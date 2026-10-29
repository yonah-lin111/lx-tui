//! Agent loop：多步模型采样与**串行**工具调度的纯编排层。
//!
//! 边界（对应 design §4 / §7 / §15.3）：
//!
//! - 本模块不依赖 `AppState` / `ui` / `db` 连接 / TUI；只依赖 [`crate::chat`] 的纯数据
//!   事件（[`ChatDelta`]、[`ChatRequest`]）与 `db::chat` 的**记录类型**（ticket / usage）。
//! - 持久化与可见状态由事件层负责：loop 通过 [`AgentHost`] 发请求并等待 ack，
//!   ack 只在“DB pending 已提交 + 状态已转 pending + 该帧已绘制”之后才返回 ticket。
//! - 文件 IO 一律经 [`ToolExecutor`]（生产实现用 `spawn_blocking`），绝不在事件循环执行。
//! - 取消在“网络 → ack → 文件任务”之间都可发生：未开始执行的调用不执行；已开始的调用
//!   必须等其返回后记录**真实结果**，不得谎报取消；随后停止后续调用与采样。
//!
//! 一轮的时序：
//!
//! ```text
//! 采样(step) -> ResponseFinished{ToolCalls} -> 校验调用均闭合
//!   -> 按 ordinal 逐个：host.begin_tool(ack=DB pending + 可见 pending + running)
//!      -> executor.execute（后台阻塞 IO）-> host.finish_tool(ack=结果落库 + 状态终态)
//!   -> 结果加入本轮上下文 -> 下一次采样
//! 无工具的 Final -> Complete；其他 -> Failed。步数/调用数不设上限（对齐 codex）；
//! 子 agent 在上下文达到阈值时自动压缩，避免无限执行把上下文撑爆。
//! ```

use std::future::Future;
use std::path::{Path, PathBuf};

use tokio::sync::{oneshot, watch};

use crate::chat::client::StreamOutcome;
use crate::chat::compact::AutoCompactNotice;
use crate::chat::events::{
    ChatDelta, ChatRequest, ModelItem, ModelStepStop, PartKind, StopReason,
    TOOL_ARGUMENTS_INVALID_CODE,
};
use crate::chat::tools::ask::{AskAnswers, AskQuestion};
use crate::chat::tools::{self, ToolContext, ToolErrorCode, ToolExecutionResult, ToolFailure};
use crate::chat::transport::tools::{TOOL_INPUT_TRUNCATED_CODE, TOOL_PROTOCOL_ERROR_CODE};
use crate::db::chat::{ToolCallTicket, Usage};

mod auto_compact;
mod executor;

use auto_compact::maybe_auto_compact;

#[cfg(test)]
mod tests;

pub use executor::{AgentTools, TurnExecution, plan_snapshot_of};

/// 自动压缩触发线：上下文达到上限的 90% 时压缩。
pub const AUTO_COMPACT_PERCENT: u64 = 90;

/// 模型未配置 `limit.context` 时的保守回退上限（token）。
pub const FALLBACK_CONTEXT_LIMIT_TOKENS: u64 = 200_000;

/// 响应要求工具、但结束原因或内容不自洽（协议层问题）。
pub const UNEXPECTED_STOP_CODE: &str = "unexpected model stop";

/// 工具执行任务异常终止（`spawn_blocking` 失败）时的稳定错误码。
pub const TOOL_EXECUTION_ABORTED_CODE: &str = "tool execution aborted";

/// 参数完整、JSON 合法（且为对象）的待执行调用。
///
/// 只有收到 `ToolInputEnded` 且参数可解析的调用才会构造本类型：正在接收参数
/// 或参数非法的调用永远不会进入 [`AgentHost::begin_tool`]，因此不会产生文件副作用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyToolCall {
    /// 供应商 call ID（无原生 ID 时由传输层合成的稳定内部 ID）。
    pub call_id: String,
    /// 稳定工具名（`read` / `search` / `edit` / `write`）。
    pub name: String,
    /// 完整参数 JSON 字符串（已校验为 JSON 对象）。
    pub arguments_json: String,
    /// 供应商响应内顺序（同一响应内串行执行顺序即此顺序）。
    pub ordinal: usize,
}

/// 事件层 ack 失败的两类原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// 请求已不再属于在途轮次（窗格关闭、会话切换、Esc 取消、旧 generation）：
    /// **不得**执行文件 IO，本轮按中断收尾。
    Cancelled,
    /// 持久化失败或状态机拒绝：本轮按失败收尾（错误码为稳定值）。
    Failed { code: String },
}

/// 一次工具调用的结算输入。
///
/// `Executed` 携带真实执行结果（成功或工具级失败）；`Cancelled` 表示调用已入库
/// pending 但**尚未执行**就被取消，绝不能写成成功结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolOutcome {
    Executed(ToolExecutionResult),
    Cancelled,
}

/// loop 发往事件层的持久化请求；每个请求都带一个 ack 通道。
///
/// 事件层实现 [`AgentHost::send`] 只需把请求转投唯一事件循环；[`AgentHost`] 的
/// `begin_tool` / `finish_tool` 默认方法负责建通道与等待回执，因此 loop 侧看不到
/// channel 细节，也永远不会直接触碰 DB 或 `AppState`。
#[derive(Debug)]
pub enum AgentHostRequest {
    /// pending 落库 + 可见 loading 门槛；ack 携带已提交的 ticket。
    BeginTool {
        call: ReadyToolCall,
        reply: oneshot::Sender<Result<ToolCallTicket, HostError>>,
    },
    /// 结果落库 + 状态终态；ack 表示真实结果已持久化。
    ///
    /// 携带 begin 阶段发放的 ticket：事件层据此校验 session/message/call 归属，
    /// 不接受 loop 侧自造的标识。
    FinishTool {
        ticket: ToolCallTicket,
        outcome: ToolOutcome,
        reply: oneshot::Sender<Result<(), HostError>>,
    },
    /// 把提问交给 UI 并等待作答；回执携带用户答案（`id -> 答案数组`）。
    ///
    /// 与 `BeginTool` 不同，这里没有 DB 写入：ask 的 pending/running 行已由
    /// `BeginTool` 提交，本请求只负责阻塞等待 UI 的答案或取消。
    AskUser {
        call_id: String,
        questions: Vec<AskQuestion>,
        reply: oneshot::Sender<Result<AskAnswers, HostError>>,
    },
    /// 下一次采样前取已落库的本地命令结果（按提交序的连续终态前缀）。
    UserShellUpdates { reply: oneshot::Sender<Vec<String>> },
    /// 子 agent 控制工具（spawn / wait / close）。
    Subagent {
        op: crate::chat::agent::SubagentOp,
        reply: oneshot::Sender<Result<crate::chat::agent::SubagentOutcome, HostError>>,
    },
}

/// loop 对事件层的依赖（持久化门槛 + 可见状态门槛）。
///
/// 实现只能把请求转投唯一事件循环并等待 ack；loop 自身从不触碰 DB 或 `AppState`。
/// 默认方法把通道细节收敛在一处，并统一处理“回执丢失 = 取消”。
pub trait AgentHost: Send {
    /// 把一次请求交给事件层（实现方不得在本地执行任何 IO）。
    fn send(&mut self, request: AgentHostRequest);

    /// pending 落库并让 UI 先看到 loading；返回 ticket 才允许执行文件 IO。
    fn begin_tool(
        &mut self,
        call: ReadyToolCall,
    ) -> impl Future<Output = Result<ToolCallTicket, HostError>> + Send
    where
        Self: Sized,
    {
        async move {
            let (reply, receiver) = oneshot::channel();
            self.send(AgentHostRequest::BeginTool { call, reply });
            match receiver.await {
                Ok(result) => result,
                // 事件循环已退出 / 回执通道被丢弃（窗格关闭、会话切换）：等价于取消。
                Err(_) => Err(HostError::Cancelled),
            }
        }
    }

    /// 结果落库并把状态推进到终态；失败时调用方必须**停止本轮且不重试**。
    fn finish_tool(
        &mut self,
        ticket: ToolCallTicket,
        outcome: ToolOutcome,
    ) -> impl Future<Output = Result<(), HostError>> + Send
    where
        Self: Sized,
    {
        async move {
            let (reply, receiver) = oneshot::channel();
            self.send(AgentHostRequest::FinishTool {
                ticket,
                outcome,
                reply,
            });
            match receiver.await {
                Ok(result) => result,
                Err(_) => Err(HostError::Cancelled),
            }
        }
    }

    /// 把提问交给 UI 并阻塞等待作答；回执丢失（窗格关闭、会话切换）等价于取消。
    ///
    /// 与 `begin_tool` 一样，通道细节收敛在这里：loop 侧看不到 oneshot。
    fn ask_user(
        &mut self,
        call_id: String,
        questions: Vec<AskQuestion>,
    ) -> impl Future<Output = Result<AskAnswers, HostError>> + Send
    where
        Self: Sized,
    {
        async move {
            let (reply, receiver) = oneshot::channel();
            self.send(AgentHostRequest::AskUser {
                call_id,
                questions,
                reply,
            });
            match receiver.await {
                Ok(result) => result,
                Err(_) => Err(HostError::Cancelled),
            }
        }
    }

    /// 执行一次子 agent 控制操作；回执丢失（事件循环退出）等价于取消。
    fn subagent(
        &mut self,
        op: crate::chat::agent::SubagentOp,
    ) -> impl Future<Output = Result<crate::chat::agent::SubagentOutcome, HostError>> + Send
    where
        Self: Sized,
    {
        async move {
            let (reply, receiver) = oneshot::channel();
            self.send(AgentHostRequest::Subagent { op, reply });
            match receiver.await {
                Ok(result) => result,
                Err(_) => Err(HostError::Cancelled),
            }
        }
    }

    /// 下一次采样前读取本次请求尚未吸收的本地命令结果。
    ///
    /// 回执丢失（事件循环退出）按空集处理：命令结果仍在库里，下一轮聊天会回放。
    fn user_shell_updates(&mut self) -> impl Future<Output = Vec<String>> + Send
    where
        Self: Sized,
    {
        async move {
            let (reply, receiver) = oneshot::channel();
            self.send(AgentHostRequest::UserShellUpdates { reply });
            receiver.await.unwrap_or_default()
        }
    }

    /// 子 agent 完成一次运行时自动压缩：事件层据此更新面板里的 compact 条目。
    ///
    /// 默认空实现（测试 host 不需要 UI）；生产 host 发一条纯数据事件即可。
    fn notify_auto_compact(&mut self, _notice: &AutoCompactNotice) {}
}

/// 工具执行入口：生产实现走 `spawn_blocking`；测试可注入可控执行器。
pub trait ToolExecutor: Send + Sync {
    /// 执行一次调用（同步实现必须自行移出事件循环线程）。
    fn execute(
        &self,
        cwd: &Path,
        call: &ReadyToolCall,
    ) -> impl Future<Output = ToolExecutionResult> + Send;

    /// 带 BeginTool 票据的执行入口：生产执行器据此把 mutation 绑定到真实
    /// session / message / call；默认实现忽略票据（测试执行器无需关心快照）。
    fn execute_tracked(
        &self,
        cwd: &Path,
        call: &ReadyToolCall,
        _ticket: &ToolCallTicket,
    ) -> impl Future<Output = ToolExecutionResult> + Send
    where
        Self: Sized,
    {
        self.execute(cwd, call)
    }
}

/// 生产执行器：文件工具是阻塞 IO，必须放入阻塞线程池。
#[derive(Debug, Clone, Copy, Default)]
pub struct FileTools;

impl ToolExecutor for FileTools {
    async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        let context = ToolContext::new(cwd.to_path_buf());
        let name = call.name.clone();
        let arguments_json = call.arguments_json.clone();
        let call_id = call.call_id.clone();
        let worker_name = name.clone();
        match tokio::task::spawn_blocking(move || {
            tools::execute(&context, &worker_name, &arguments_json)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => {
                // 只记 call_id/工具名/错误类别：不打印参数全文与文件内容。
                tracing::warn!(
                    target: "lx::agent",
                    call_id = %call_id,
                    name = %name,
                    %error,
                    "tool execution task failed"
                );
                ToolExecutionResult::Failed {
                    code: ToolErrorCode::Io,
                    message: TOOL_EXECUTION_ABORTED_CODE.to_string(),
                    changes: Vec::new(),
                }
            }
        }
    }
}

/// 一轮的最终结局（映射到 `StopReason`：Complete / Cancelled / Failed）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    /// 收到无工具的正常终态响应。
    Complete,
    /// 用户取消（含等待 ack 期间窗格关闭）。
    Interrupted,
    /// 失败；`code` 为稳定错误码。
    Failed { code: String },
}

/// 一轮结算：终态 + **整轮累计** usage（多次采样求和，未知保持 None）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSummary {
    pub outcome: TurnOutcome,
    pub usage: Usage,
}

impl TurnSummary {
    /// 正常完成。
    pub fn complete(usage: Usage) -> Self {
        Self {
            outcome: TurnOutcome::Complete,
            usage,
        }
    }

    /// 取消/中断。
    pub fn interrupted(usage: Usage) -> Self {
        Self {
            outcome: TurnOutcome::Interrupted,
            usage,
        }
    }

    /// 失败。
    pub fn failed(code: impl Into<String>, usage: Usage) -> Self {
        Self {
            outcome: TurnOutcome::Failed { code: code.into() },
            usage,
        }
    }

    /// 转成事件层使用的结束原因。
    pub fn stop_reason(&self) -> StopReason {
        match &self.outcome {
            TurnOutcome::Complete => StopReason::Complete,
            TurnOutcome::Interrupted => StopReason::Cancelled,
            TurnOutcome::Failed { code } => StopReason::Failed { code: code.clone() },
        }
    }
}

/// 本轮上下文：冻结的请求骨架 + 逐步累积的结构化历史项 + 冻结的工作区 cwd。
///
/// 多次采样共用同一上下文（当前轮的 call/result 不回到 DB 重新装配），
/// 只把本次响应产生的内容/调用与工具结果追加进来。
#[derive(Debug, Clone)]
pub struct ModelContext {
    /// 初始请求骨架（turns / model / variant_params 冻结）。
    request: ChatRequest,
    /// 结构化历史（初始为装配结果，逐步追加）。
    items: Vec<ModelItem>,
    /// 发送时冻结的工作区根；文件工具的相对路径基准。
    cwd: PathBuf,
    /// 本轮开始时的**本地运行上下文**摘要（存活进程 + 当前计划）。
    ///
    /// 它不是工具结果，也不伪装成新的调用：只在第一次采样时并入用户消息末尾，
    /// 避免历史裁剪后模型忘记 `session_id` 或把已完成的计划当成当前计划。
    runtime_summary: Option<String>,
    /// 模型上下文上限（token）；None 时用 [`FALLBACK_CONTEXT_LIMIT_TOKENS`]。
    context_limit: Option<u64>,
    /// 运行时自动压缩生成的摘要；非空时注入系统提示，替代被丢掉的旧 items。
    compact_summary: Option<String>,
}

impl ModelContext {
    /// 以装配好的请求与冻结 cwd 构造上下文。
    pub fn new(request: ChatRequest, cwd: impl Into<PathBuf>) -> Self {
        let items = request.items.clone();
        Self {
            request,
            items,
            cwd: cwd.into(),
            runtime_summary: None,
            context_limit: None,
            compact_summary: None,
        }
    }

    /// 附加本地运行上下文摘要（存活进程与当前计划的有界快照）。
    pub fn with_runtime_summary(mut self, summary: Option<String>) -> Self {
        self.runtime_summary = summary;
        self
    }

    /// 设置模型上下文上限；未设置时使用回退值。
    pub fn with_context_limit(mut self, limit: Option<u64>) -> Self {
        self.context_limit = limit.filter(|value| *value > 0);
        self
    }

    /// 冻结的工作区根。
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// 当前结构化历史（诊断与测试用）。
    pub fn items(&self) -> &[ModelItem] {
        &self.items
    }

    /// 下一次采样请求：turns/model 不变，items 为本轮累计结果。
    ///
    /// 运行上下文摘要只在**第一次**采样时并入（后续采样已包含在 items 里）。
    fn next_request(&self) -> ChatRequest {
        // 自动压缩的摘要放在系统段：与手动 compact 的注入位置一致，不伪造
        // user/assistant 消息；原始任务仍作为 items 的第一条保留。
        let system_prompt = self.request.system_prompt.as_ref().map(|base| {
            self.compact_summary.as_ref().map_or_else(
                || base.clone(),
                |summary| format!("{base}\n\n{}", compact_summary_prompt(summary)),
            )
        });
        ChatRequest {
            turns: self.request.turns.clone(),
            items: self.items.clone(),
            model: self.request.model.clone(),
            variant_params: self.request.variant_params.clone(),
            // 用途随轮次冻结：子 agent 的请求必须保持 `Subagent(role)`，
            // 否则第二次采样会重新声明主工具面（模型就能越权 spawn）。
            purpose: self.request.purpose,
            // 系统提示在 turn 创建时冻结（稳定提示 + 环境快照 + compact 摘要）。
            // 每次采样都必须原样复用：写成 None 会让 transport 现场重采集，
            // 丢摘要且同一轮看到不同环境。
            system_prompt,
        }
    }

    /// 本轮是否为子 agent（禁止递归控制工具）。
    pub fn is_subagent(&self) -> bool {
        matches!(
            self.request.purpose,
            crate::chat::events::RequestPurpose::Subagent(_)
        )
    }

    /// 运行上下文摘要（诊断与测试用）。
    pub fn runtime_summary(&self) -> Option<&str> {
        self.runtime_summary.as_deref()
    }

    /// 把运行上下文摘要并入最后一次用户消息（只并入一次）。
    fn absorb_runtime_summary(&mut self) {
        let Some(summary) = self.runtime_summary.take() else {
            return;
        };
        let Some(ModelItem::UserText(text) | ModelItem::UserContent { text, .. }) =
            self.items.last_mut()
        else {
            // 末尾不是用户消息（理论上不会发生）：不伪造位置，直接放弃摘要。
            return;
        };
        text.push_str("\n\n");
        text.push_str(&summary);
    }

    /// 追加一次模型响应：可见正文（若有）按出现顺序合并为一条 assistant 文本，
    /// 随后按 ordinal 顺序追加工具调用（结构必须保留，不能拼进文本）。
    ///
    /// 参数不是 JSON 对象时跳过该调用（`ready_calls` 已保证对象，这里只做防御）。
    fn push_response(&mut self, text: String, calls: &[ReadyToolCall]) {
        if !text.is_empty() {
            self.items.push(ModelItem::AssistantText(text));
        }
        for call in calls {
            let Ok(input) = serde_json::from_str::<serde_json::Value>(&call.arguments_json) else {
                tracing::debug!(
                    target: "lx::agent",
                    call_id = %call.call_id,
                    name = %call.name,
                    "skip tool call in current turn: arguments are not valid JSON"
                );
                continue;
            };
            self.items.push(ModelItem::AssistantToolCall {
                call_id: call.call_id.clone(),
                name: call.name.clone(),
                input,
            });
        }
    }

    /// 追加一条本地命令结果（user 角色事实，排在已闭合工具对之后）。
    ///
    /// 同时进入 `turns` 纯文本视图与 `items` 结构化视图：不支持 item 的 transport
    /// 也能读到命令事实。
    fn push_user_context(&mut self, text: String) {
        self.items.push(ModelItem::UserText(text.clone()));
        self.request.turns.push(crate::chat::events::ChatTurn {
            role: "user".into(),
            text,
            reasoning_metadata: None,
        });
    }

    /// 追加一条工具结果（与前面的调用成对；失败结果带 `is_error`）。
    ///
    /// 回放侧单条结果上限与历史装配同一口径（[`MAX_TOOL_RESULT_REPLAY_BYTES`]）：
    /// 超限截断并显式标记，不静默省略；落库结果体本身不变。
    fn push_tool_result(&mut self, call_id: &str, content: String, is_error: bool) {
        self.items.push(ModelItem::ToolResult {
            call_id: call_id.to_string(),
            content: crate::chat::context::truncate_result_for_replay(&content),
            is_error,
        });
    }

    /// 追加一条由工具（如 `read` 图片）读取出的多模态用户消息。
    fn push_user_image(&mut self, text: String, images: Vec<crate::chat::events::ModelImage>) {
        self.items.push(ModelItem::UserContent { text, images });
    }
}

/// 自动压缩摘要的系统段包裹（与手动 compact 的 `<compact_summary>` 同形）。
fn compact_summary_prompt(summary: &str) -> String {
    format!(
        "<compact_summary>\nThis is a summary of earlier work in this subagent run that was \
         compacted automatically. Treat it as established context, not as a new request.\n\n\
         {summary}\n</compact_summary>"
    )
}

/// 执行一轮 agent 任务：多步采样 + 串行工具调度。
///
/// `stream_one` 只执行**一次**模型流并返回其全部增量与结束原因（生产实现为
/// [`crate::chat::client::run_stream`]，测试可注入脚本化假 transport）。
/// 返回 [`TurnSummary`]：调用方据此提交整轮终态与累计 usage。
pub async fn run_agent_turn<H, E, S, F>(
    context: ModelContext,
    mut host: H,
    executor: E,
    stream_one: S,
    cancel: watch::Receiver<bool>,
) -> TurnSummary
where
    H: AgentHost,
    E: ToolExecutor,
    S: Fn(ChatRequest, watch::Receiver<bool>) -> F,
    F: Future<Output = StreamOutcome>,
{
    let mut context = context;
    context.absorb_runtime_summary();
    let mut usage = Usage::default();
    // 不设步数/调用数上限：子 agent 靠自动压缩控制上下文，主会话靠用户停止与
    // `/compact`。这里只保留取消与协议错误两类终止条件。
    loop {
        if *cancel.borrow() {
            return TurnSummary::interrupted(usage);
        }
        // 每次采样前吸收已落库的本地命令结果；不额外触发模型请求。
        for text in host.user_shell_updates().await {
            context.push_user_context(text);
        }
        // 工具批次已全部结算：此时压缩不会切断 call/result 对。
        if let Err(error) = maybe_auto_compact(&mut context, &mut host, &stream_one, &cancel).await
        {
            return match error {
                HostError::Cancelled => TurnSummary::interrupted(usage),
                HostError::Failed { code } => TurnSummary::failed(code, usage),
            };
        }
        let outcome = stream_one(context.next_request(), cancel.clone()).await;
        add_usage(&mut usage, &outcome.deltas);
        match &outcome.stop {
            StopReason::Cancelled => return TurnSummary::interrupted(usage),
            StopReason::Failed { code } => return TurnSummary::failed(code.clone(), usage),
            StopReason::Complete => {}
        }
        // 参数未闭合 / 非法 JSON / call ID 冲突：绝不执行文件 IO，本轮失败。
        let calls = match ready_calls(&outcome.deltas) {
            Ok(calls) => calls,
            Err(code) => return TurnSummary::failed(code, usage),
        };
        context.push_response(assistant_text(&outcome.deltas), &calls);
        let stop = model_stop(&outcome.deltas);
        if calls.is_empty() {
            return match stop {
                // 无工具的 Final（以及没有 `ResponseFinished` 的旧式纯文本流）即整轮完成。
                Some(ModelStepStop::Final) | None => TurnSummary::complete(usage),
                Some(
                    ModelStepStop::ToolCalls | ModelStepStop::Failed | ModelStepStop::Cancelled,
                ) => TurnSummary::failed(UNEXPECTED_STOP_CODE, usage),
            };
        }
        // 新协议下“要求工具”的响应必须声明 ToolCalls；否则不猜、不执行。
        if stop != Some(ModelStepStop::ToolCalls) {
            return TurnSummary::failed(UNEXPECTED_STOP_CODE, usage);
        }
        // 同一响应内**串行**执行，顺序即供应商 ordinal；前一结果对后一调用可见。
        let mut remaining = calls.into_iter();
        while let Some(call) = remaining.next() {
            if *cancel.borrow() {
                // 未开始的调用：只在库里留下 `cancelled` 记录（pending → cancelled），
                // **绝不执行**文件 IO，然后停止本轮。
                cancel_unstarted(&mut host, std::iter::once(call).chain(remaining)).await;
                return TurnSummary::interrupted(usage);
            }
            // 门槛一：pending 已提交且可见 loading。ack 前不会有任何文件副作用。
            let ticket = match host.begin_tool(call.clone()).await {
                Ok(ticket) => ticket,
                Err(HostError::Cancelled) => return TurnSummary::interrupted(usage),
                Err(HostError::Failed { code }) => return TurnSummary::failed(code, usage),
            };
            if *cancel.borrow() {
                // 已入库 pending 但尚未执行：结算为 cancelled，绝不执行文件 IO。
                let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
                cancel_unstarted(&mut host, remaining).await;
                return TurnSummary::interrupted(usage);
            }
            // 门槛二：running 已推进（事件层在 ack 内完成），此后才允许文件 IO。
            //
            // 执行层不信任模型：即使供应商/模型幻觉出未声明工具，也不能越过本角色的
            // 工具面执行文件、进程或递归控制工具。失败结果仍走 begin/finish 落库。
            let allowed = context
                .request
                .tools_surface()
                .is_some_and(|surface| surface.names().contains(&call.name.as_str()));
            let control = crate::chat::agent::parse_control(&call.name, &call.arguments_json);
            let result = if !allowed {
                ToolFailure::new(ToolErrorCode::Io, crate::chat::agent::UNAVAILABLE_CODE)
                    .into_result(Vec::new())
            } else if let Some(parsed) = control {
                // 子 agent 内禁止递归：即使模型伪造控制工具调用，也按工具级失败回给模型。
                if context.is_subagent() {
                    ToolFailure::new(ToolErrorCode::Io, crate::chat::agent::UNAVAILABLE_CODE)
                        .into_result(Vec::new())
                } else {
                    match parsed {
                        Err(failure) => failure.into_result(Vec::new()),
                        Ok(op) => match host.subagent(op).await {
                            Ok(outcome) => outcome.into_result(),
                            Err(HostError::Cancelled) => {
                                let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
                                cancel_unstarted(&mut host, remaining).await;
                                return TurnSummary::interrupted(usage);
                            }
                            // 并发超限 / 目标不存在：工具级失败，轮次继续。
                            Err(HostError::Failed { code }) => {
                                ToolFailure::new(ToolErrorCode::Io, code).into_result(Vec::new())
                            }
                        },
                    }
                }
            } else if call.name == crate::chat::tools::ask::ASK_TOOL {
                match crate::chat::tools::ask::parse_arguments(&call.arguments_json) {
                    Err(failure) => failure.into_result(Vec::new()),
                    Ok(questions) => {
                        match host.ask_user(call.call_id.clone(), questions.clone()).await {
                            Ok(answers) => {
                                crate::chat::tools::ask::answered_result(&questions, &answers)
                            }
                            Err(HostError::Cancelled) => {
                                // 本轮被取消：只结算这一行，未开始的调用按 cancelled
                                // 处理，不再采样。
                                let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
                                cancel_unstarted(&mut host, remaining).await;
                                return TurnSummary::interrupted(usage);
                            }
                            Err(HostError::Failed { code }) => {
                                let failure = crate::chat::tools::ToolFailure::new(
                                    ToolErrorCode::Io,
                                    code.clone(),
                                )
                                .into_result(Vec::new());
                                let _ = host
                                    .finish_tool(ticket, ToolOutcome::Executed(failure))
                                    .await;
                                return TurnSummary::failed(code, usage);
                            }
                        }
                    }
                }
            } else {
                executor
                    .execute_tracked(context.cwd(), &call, &ticket)
                    .await
            };
            let content = tools::format_for_model(&result);
            let is_error = result.is_failure();
            let image_from_read = match &result {
                ToolExecutionResult::Read {
                    path,
                    kind: crate::chat::tools::ReadKind::Image,
                    ..
                } => crate::platform::load_and_process_image_file(path)
                    .ok()
                    .map(|img| {
                        (
                            path.display().to_string(),
                            crate::chat::events::ModelImage {
                                mime: img.mime,
                                bytes: std::sync::Arc::new(img.bytes),
                            },
                        )
                    }),
                _ => None,
            };
            // 结果必须先落库再进入上下文；落库失败即停止本轮，**不重试写入**。
            match host
                .finish_tool(ticket, ToolOutcome::Executed(result))
                .await
            {
                Ok(()) => {}
                Err(HostError::Cancelled) => return TurnSummary::interrupted(usage),
                Err(HostError::Failed { code }) => return TurnSummary::failed(code, usage),
            }
            context.push_tool_result(&call.call_id, content, is_error);
            if let Some((path_str, model_image)) = image_from_read {
                context
                    .push_user_image(format!("Image read from `{path_str}`:"), vec![model_image]);
            }
        }
    }
}

/// 取消时把**尚未开始**的调用记为 `cancelled`（pending → cancelled），绝不执行文件 IO。
///
/// 只有真正建过 pending 行才能留下终态记录；任何 ack 失败（窗格关闭、会话切换）
/// 视为本轮已中断，不重试写入，也不影响已经记录的真实结果。
async fn cancel_unstarted<H, I>(host: &mut H, calls: I)
where
    H: AgentHost,
    I: IntoIterator<Item = ReadyToolCall>,
{
    for call in calls {
        let call_id = call.call_id.clone();
        let Ok(ticket) = host.begin_tool(call).await else {
            // 归属失效（窗格关闭/旧 generation）：不建行、不执行。
            tracing::debug!(target: "lx::agent", call_id = %call_id, "cancelled unstarted tool call");
            continue;
        };
        let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
    }
}

/// 把一次响应的全部工具输入聚合成待执行调用（按 ordinal 排序）。
///
/// 任一调用未闭合、参数非 JSON 对象、或同响应内 call ID 重复都返回稳定错误码，
/// 调用方据此结束本轮且不执行任何文件 IO。
fn ready_calls(deltas: &[ChatDelta]) -> Result<Vec<ReadyToolCall>, String> {
    struct Open {
        ordinal: usize,
        call_id: String,
        name: String,
        arguments: String,
    }
    let mut open: Vec<Open> = Vec::new();
    let mut ready: Vec<ReadyToolCall> = Vec::new();
    for delta in deltas {
        match delta {
            ChatDelta::ToolInputStarted {
                call_id,
                name,
                ordinal,
            } => {
                let duplicated = open.iter().any(|call| call.call_id == *call_id)
                    || ready.iter().any(|call| call.call_id == *call_id);
                if duplicated {
                    return Err(TOOL_PROTOCOL_ERROR_CODE.to_string());
                }
                open.push(Open {
                    ordinal: *ordinal,
                    call_id: call_id.clone(),
                    name: name.clone(),
                    arguments: String::new(),
                });
            }
            ChatDelta::ToolInputDelta { call_id, chunk } => {
                let Some(call) = open.iter_mut().find(|call| call.call_id == *call_id) else {
                    return Err(TOOL_PROTOCOL_ERROR_CODE.to_string());
                };
                call.arguments.push_str(chunk);
            }
            ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            } => {
                let Some(index) = open.iter().position(|call| call.call_id == *call_id) else {
                    return Err(TOOL_PROTOCOL_ERROR_CODE.to_string());
                };
                let call = open.remove(index);
                // 以 `ToolInputEnded` 携带的完整串为权威值（跨帧缓冲只用于展示）。
                let arguments_json = if arguments_json.is_empty() {
                    call.arguments
                } else {
                    arguments_json.clone()
                };
                let valid = serde_json::from_str::<serde_json::Value>(&arguments_json)
                    .map(|value| value.is_object())
                    .unwrap_or(false);
                if !valid {
                    return Err(TOOL_ARGUMENTS_INVALID_CODE.to_string());
                }
                ready.push(ReadyToolCall {
                    call_id: call.call_id,
                    name: call.name,
                    arguments_json,
                    ordinal: call.ordinal,
                });
            }
            _ => {}
        }
    }
    if !open.is_empty() {
        return Err(TOOL_INPUT_TRUNCATED_CODE.to_string());
    }
    ready.sort_by_key(|call| call.ordinal);
    Ok(ready)
}

/// 本次响应声明的结束原因（最后一次 `ResponseFinished`）。
fn model_stop(deltas: &[ChatDelta]) -> Option<ModelStepStop> {
    deltas.iter().rev().find_map(|delta| match delta {
        ChatDelta::ResponseFinished { stop } => Some(*stop),
        _ => None,
    })
}

/// 本次响应的可见正文（Text part 按出现顺序拼接；思考不回传）。
fn assistant_text(deltas: &[ChatDelta]) -> String {
    let mut text = String::new();
    for delta in deltas {
        if let ChatDelta::Part(part) = delta
            && part.kind == PartKind::Text
        {
            text.push_str(&part.text);
        }
    }
    text
}

/// 累加一次响应的 usage：多次采样求和；未知（None）不写 0 冒充真实值。
fn add_usage(total: &mut Usage, deltas: &[ChatDelta]) {
    for delta in deltas {
        let ChatDelta::Usage(step) = delta else {
            continue;
        };
        total.input = add_optional(total.input, step.input);
        total.output = add_optional(total.output, step.output);
        total.cache_read = add_optional(total.cache_read, step.cache_read);
        total.cache_write = add_optional(total.cache_write, step.cache_write);
    }
}

/// 可选数值累加：两侧都未知时为 None。
fn add_optional(total: Option<i64>, step: Option<i64>) -> Option<i64> {
    match (total, step) {
        (None, None) => None,
        (total, step) => Some(total.unwrap_or(0).saturating_add(step.unwrap_or(0))),
    }
}

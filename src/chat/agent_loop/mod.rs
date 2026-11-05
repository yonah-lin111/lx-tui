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
use std::path::Path;

use tokio::sync::{oneshot, watch};

use crate::chat::client::StreamOutcome;
use crate::chat::compact::AutoCompactNotice;
use crate::chat::events::{
    ChatDelta, ChatRequest, ModelStepStop, PartKind, StopReason, TOOL_ARGUMENTS_INVALID_CODE,
};
use crate::chat::tools::ask::{AskAnswers, AskQuestion};
use crate::chat::tools::{self, ToolContext, ToolErrorCode, ToolExecutionResult, ToolFailure};
use crate::chat::transport::tools::{TOOL_INPUT_TRUNCATED_CODE, TOOL_PROTOCOL_ERROR_CODE};
use crate::db::chat::{ToolCallTicket, Usage};

mod auto_compact;
mod context;
mod executor;

#[cfg(test)]
use crate::chat::events::ModelItem;
use auto_compact::maybe_auto_compact;
use context::compact_summary_prompt;

#[cfg(test)]
mod tests;

pub use context::ModelContext;
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
    /// 子 agent 控制工具（spawn / send_input / resume / wait / close / list / cancel_input）。
    Subagent {
        call_id: Option<String>,
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
        self.subagent_with_call(None, op)
    }

    /// 携带外层工具 `call_id` 执行子 agent 控制操作（用于入队幂等去重键）。
    fn subagent_with_call(
        &mut self,
        call_id: Option<String>,
        op: crate::chat::agent::SubagentOp,
    ) -> impl Future<Output = Result<crate::chat::agent::SubagentOutcome, HostError>> + Send
    where
        Self: Sized,
    {
        async move {
            let (reply, receiver) = oneshot::channel();
            self.send(AgentHostRequest::Subagent { call_id, op, reply });
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

    /// 子 agent 完成一次运行时自动压缩：事件层持久化检查点并更新面板里的 compact 条目。
    ///
    /// 默认空实现直接返回 `Ok(())`（无 DB 的单元测试 host 不需要持久化）；
    /// 生产 host 等待事件层完成 DB 检查点写入后才允许丢弃旧 items。
    fn notify_auto_compact(
        &mut self,
        _notice: &AutoCompactNotice,
    ) -> impl Future<Output = Result<(), HostError>> + Send
    where
        Self: Sized,
    {
        async { Ok(()) }
    }
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

    /// 可取消的 MCP 调用；非 MCP 工具返回 None，由调用方走通用执行入口。
    ///
    /// Esc 取消必须直达 `notifications/cancelled`，不能只丢弃 future 让 server
    /// 继续执行副作用。
    fn execute_mcp_cancellable(
        &self,
        _cwd: &Path,
        _call: &ReadyToolCall,
        _cancel: watch::Receiver<bool>,
    ) -> impl Future<Output = Option<ToolExecutionResult>> + Send
    where
        Self: Sized,
    {
        async { None }
    }

    /// 当前模型是否支持图片输入；测试执行器默认支持。
    fn supports_image_input(&self) -> bool {
        true
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

/// 一轮结算：终态 + 最后一次模型响应的 usage（上下文/消息口径）与整轮累计 usage（账单口径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSummary {
    pub outcome: TurnOutcome,
    /// 最后一次模型响应的 usage：上下文占用与消息统计只认它，不跨步累加。
    pub usage: Usage,
    /// 整轮累计 usage：会话账单与 CLI 统计使用，跨步求和。
    pub billing: Usage,
    /// 是否因 Token 预算耗尽而熔断（终态仍为 `Interrupted`，可恢复）。
    pub budget_exceeded: bool,
}

impl TurnSummary {
    /// 正常完成。
    pub fn complete(usage: Usage) -> Self {
        Self {
            outcome: TurnOutcome::Complete,
            usage,
            billing: usage,
            budget_exceeded: false,
        }
    }

    /// 取消/中断。
    pub fn interrupted(usage: Usage) -> Self {
        Self {
            outcome: TurnOutcome::Interrupted,
            usage,
            billing: usage,
            budget_exceeded: false,
        }
    }

    /// 失败。
    pub fn failed(code: impl Into<String>, usage: Usage) -> Self {
        Self {
            outcome: TurnOutcome::Failed { code: code.into() },
            usage,
            billing: usage,
            budget_exceeded: false,
        }
    }

    /// Token 预算耗尽：按中断收尾，但标记 `budget_exceeded` 供事件层生成半程汇报。
    pub fn budget_exceeded(usage: Usage) -> Self {
        Self {
            outcome: TurnOutcome::Interrupted,
            usage,
            billing: usage,
            budget_exceeded: true,
        }
    }

    /// 附加整轮累计 usage（账单口径）；默认与单步 usage 相同。
    pub fn with_billing(mut self, billing: Usage) -> Self {
        self.billing = billing;
        self
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

/// 取工具结果的图片字节（归档路径缺失或读失败时跳过，不阻断工具结果）。
fn tool_result_images(result: &ToolExecutionResult) -> Vec<crate::chat::events::ModelImage> {
    let archived: Vec<&crate::chat::tools::ArchivedImage> = match result {
        ToolExecutionResult::Mcp(mcp) => mcp.images.iter().collect(),
        ToolExecutionResult::Read {
            image: Some(image), ..
        } => vec![image],
        _ => Vec::new(),
    };
    archived
        .into_iter()
        .filter_map(|image| {
            let bytes = std::fs::read(&image.path).ok()?;
            Some(crate::chat::events::ModelImage {
                mime: image.mime.clone(),
                bytes: std::sync::Arc::new(bytes),
            })
        })
        .collect()
}

/// 执行一轮 agent 任务：多步采样 + 串行工具调度。
///
/// `stream_one` 只执行**一次**模型流并返回其全部增量与结束原因（生产实现为
/// [`crate::chat::client::run_stream`]，测试可注入脚本化假 transport）。
/// 返回 [`TurnSummary`]：调用方据此提交整轮终态与最后一次响应的 usage。
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
    let mut billing = Usage::default();
    // 不设步数/调用数上限：子 agent 靠自动压缩控制上下文，主会话靠用户停止与
    // `/compact`。这里只保留取消与协议错误两类终止条件。
    loop {
        if *cancel.borrow() {
            return TurnSummary::interrupted(usage).with_billing(billing);
        }
        // 每次采样前吸收已落库的本地命令结果；不额外触发模型请求。
        for text in host.user_shell_updates().await {
            context.push_user_context(text);
        }
        // 工具批次已全部结算：此时压缩不会切断 call/result 对。
        if let Err(error) = maybe_auto_compact(&mut context, &mut host, &stream_one, &cancel).await
        {
            return match error {
                HostError::Cancelled => TurnSummary::interrupted(usage).with_billing(billing),
                HostError::Failed { code } => {
                    TurnSummary::failed(code, usage).with_billing(billing)
                }
            };
        }
        let outcome = stream_one(context.next_request(), cancel.clone()).await;
        // 上下文占用只认最后一次模型响应的 usage；账单口径单独跨步累加。
        let mut budget_exhausted = false;
        let mut budget_reminder = None;
        if let Some(latest) = step_usage(&outcome.deltas) {
            add_usage(&mut billing, latest);
            usage = latest;
            budget_exhausted = context
                .token_budget
                .as_mut()
                .is_some_and(|budget| budget.record_step_usage(latest));
            if !budget_exhausted {
                budget_reminder = context
                    .token_budget
                    .as_mut()
                    .and_then(crate::chat::agent::SubagentTokenBudget::take_pending_reminder);
            }
        }
        match &outcome.stop {
            StopReason::Cancelled => return TurnSummary::interrupted(usage).with_billing(billing),
            StopReason::Failed { code } => {
                return TurnSummary::failed(code.clone(), usage).with_billing(billing);
            }
            StopReason::Complete => {}
        }
        // 参数未闭合 / 非法 JSON / call ID 冲突：绝不执行文件 IO，本轮失败。
        let calls = match ready_calls(&outcome.deltas) {
            Ok(calls) => calls,
            Err(code) => return TurnSummary::failed(code, usage).with_billing(billing),
        };
        context.push_response(assistant_text(&outcome.deltas), &calls);
        let stop = model_stop(&outcome.deltas);
        if calls.is_empty() {
            return match stop {
                // 无工具的 Final（以及没有 `ResponseFinished` 的旧式纯文本流）即整轮完成。
                Some(ModelStepStop::Final) | None => {
                    TurnSummary::complete(usage).with_billing(billing)
                }
                Some(
                    ModelStepStop::ToolCalls | ModelStepStop::Failed | ModelStepStop::Cancelled,
                ) => TurnSummary::failed(UNEXPECTED_STOP_CODE, usage).with_billing(billing),
            };
        }
        // 新协议下“要求工具”的响应必须声明 ToolCalls；否则不猜、不执行。
        if stop != Some(ModelStepStop::ToolCalls) {
            return TurnSummary::failed(UNEXPECTED_STOP_CODE, usage).with_billing(billing);
        }
        // 同一响应内**串行**执行，顺序即供应商 ordinal；前一结果对后一调用可见。
        let mut remaining = calls.into_iter();
        while let Some(call) = remaining.next() {
            if *cancel.borrow() {
                // 未开始的调用：只在库里留下 `cancelled` 记录（pending → cancelled），
                // **绝不执行**文件 IO，然后停止本轮。
                cancel_unstarted(&mut host, std::iter::once(call).chain(remaining)).await;
                return TurnSummary::interrupted(usage).with_billing(billing);
            }
            // 门槛一：pending 已提交且可见 loading。ack 前不会有任何文件副作用。
            let ticket = match host.begin_tool(call.clone()).await {
                Ok(ticket) => ticket,
                Err(HostError::Cancelled) => {
                    return TurnSummary::interrupted(usage).with_billing(billing);
                }
                Err(HostError::Failed { code }) => {
                    return TurnSummary::failed(code, usage).with_billing(billing);
                }
            };
            if *cancel.borrow() {
                // 已入库 pending 但尚未执行：结算为 cancelled，绝不执行文件 IO。
                let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
                cancel_unstarted(&mut host, remaining).await;
                return TurnSummary::interrupted(usage).with_billing(billing);
            }
            // 门槛二：running 已推进（事件层在 ack 内完成），此后才允许文件 IO。
            //
            // 执行层不信任模型：即使供应商/模型幻觉出未声明工具，也不能越过本角色的
            // 工具面执行文件、进程或递归控制工具。失败结果仍走 begin/finish 落库。
            // 唯一权威是本轮冻结的工具声明：内置白名单 + MCP + resources 都已在里面。
            let allowed = if context.request.tools.is_empty() {
                // 测试与旧调用路径未注入冻结快照时，回退到角色白名单（按深度收紧）。
                context
                    .request
                    .tools_surface_at_depth(context.subagent_depth, context.max_subagent_depth)
                    .is_some_and(|surface| surface.names().contains(&call.name.as_str()))
            } else {
                context
                    .request
                    .tools
                    .iter()
                    .any(|decl| decl.name == call.name)
            };
            let control = crate::chat::agent::parse_control(&call.name, &call.arguments_json);
            // 深度守卫优先于工具面校验：子 agent 伪造/残留控制工具调用时，
            // 派发工具给出稳定深度错误，其余控制工具按“不可用”拒绝。
            let result = if control.is_some() && context.is_subagent() && !context.can_delegate() {
                let code = if call.name == "spawn_agent"
                    && matches!(
                        context.request.purpose,
                        crate::chat::events::RequestPurpose::Subagent(role)
                            if role.base() == crate::chat::agent::BaseRole::Executor
                    ) {
                    crate::chat::agent::DEPTH_LIMIT_REACHED_CODE
                } else {
                    crate::chat::agent::UNAVAILABLE_CODE
                };
                ToolFailure::new(ToolErrorCode::Io, code).into_result(Vec::new())
            } else if !allowed {
                ToolFailure::new(ToolErrorCode::Io, crate::chat::agent::UNAVAILABLE_CODE)
                    .into_result(Vec::new())
            } else if let Some(parsed) = control {
                match parsed {
                    Err(failure) => failure.into_result(Vec::new()),
                    Ok(op) => match host
                        .subagent_with_call(Some(call.call_id.clone()), op)
                        .await
                    {
                        Ok(outcome) => outcome.into_result(),
                        Err(HostError::Cancelled) => {
                            let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
                            cancel_unstarted(&mut host, remaining).await;
                            return TurnSummary::interrupted(usage).with_billing(billing);
                        }
                        // 并发超限 / 目标不存在：工具级失败，轮次继续。
                        Err(HostError::Failed { code }) => {
                            ToolFailure::new(ToolErrorCode::Io, code).into_result(Vec::new())
                        }
                    },
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
                                return TurnSummary::interrupted(usage).with_billing(billing);
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
                                return TurnSummary::failed(code, usage).with_billing(billing);
                            }
                        }
                    }
                }
            } else if let Some(result) = executor
                .execute_mcp_cancellable(context.cwd(), &call, cancel.clone())
                .await
            {
                result
            } else {
                executor
                    .execute_tracked(context.cwd(), &call, &ticket)
                    .await
            };
            // MCP 取消不是工具结果：按用户取消结算，不再采样。
            if result.error_code() == Some(ToolErrorCode::Cancelled) {
                let _ = host.finish_tool(ticket, ToolOutcome::Cancelled).await;
                cancel_unstarted(&mut host, remaining).await;
                return TurnSummary::interrupted(usage).with_billing(billing);
            }
            let mut content = tools::format_for_model(&result);
            let is_error = result.is_failure();
            let mut tool_images = tool_result_images(&result);
            if !executor.supports_image_input() && !tool_images.is_empty() {
                tool_images.clear();
                content.push_str("\n[image omitted: this model does not support image input]");
            }
            // 结果必须先落库再进入上下文；落库失败即停止本轮，**不重试写入**。
            match host
                .finish_tool(ticket, ToolOutcome::Executed(result))
                .await
            {
                Ok(()) => {}
                Err(HostError::Cancelled) => {
                    return TurnSummary::interrupted(usage).with_billing(billing);
                }
                Err(HostError::Failed { code }) => {
                    return TurnSummary::failed(code, usage).with_billing(billing);
                }
            }
            context.push_tool_result(&call.call_id, content, is_error, tool_images);
        }
        // 当前步的全部工具已结算：预算耗尽时在此熔断，保证 call/result 成对落库。
        if budget_exhausted {
            return TurnSummary::budget_exceeded(usage).with_billing(billing);
        }
        // 未耗尽时把跨过 25% / 10% 阈值的提醒注入下一轮采样。
        if let Some(reminder) = budget_reminder {
            context.push_user_context(reminder);
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

/// 合并单次模型响应内分帧到达的 usage（后到的非 None 字段覆盖先到的）。
///
/// 同一次 HTTP 响应可能先报 input、再报 output（Anthropic / Google）；这里合并成
/// 一个完整快照。返回 None 表示该次响应没有任何 usage 帧。
fn step_usage(deltas: &[ChatDelta]) -> Option<Usage> {
    let mut step = Usage::default();
    let mut seen = false;
    for delta in deltas {
        let ChatDelta::Usage(part) = delta else {
            continue;
        };
        seen = true;
        if part.input.is_some() {
            step.input = part.input;
        }
        if part.output.is_some() {
            step.output = part.output;
        }
        if part.cache_read.is_some() {
            step.cache_read = part.cache_read;
        }
        if part.cache_write.is_some() {
            step.cache_write = part.cache_write;
        }
    }
    seen.then_some(step)
}

/// 累加整轮账单 usage：多次采样求和；未知（None）不写 0 冒充真实值。
fn add_usage(total: &mut Usage, step: Usage) {
    total.input = add_optional(total.input, step.input);
    total.output = add_optional(total.output, step.output);
    total.cache_read = add_optional(total.cache_read, step.cache_read);
    total.cache_write = add_optional(total.cache_write, step.cache_write);
}

/// 可选数值累加：两侧都未知时为 None。
fn add_optional(total: Option<i64>, step: Option<i64>) -> Option<i64> {
    match (total, step) {
        (None, None) => None,
        (total, step) => Some(total.unwrap_or(0).saturating_add(step.unwrap_or(0))),
    }
}

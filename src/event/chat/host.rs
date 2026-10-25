//! Agent loop 与事件层之间的握手：事件类型 + [`AgentHost`] 的生产实现。
//!
//! loop 只持有 [`mpsc::UnboundedSender<AppEvent>`]：它把 [`AgentHostEvent`] 投回唯一
//! 事件循环，然后等待事件层在 **DB 与状态都就绪后**回执。本文件不直接访问 `Database`
//! 或 `AppState`，因此 loop 的时序可被完全观测与测试。

use tokio::sync::{mpsc, oneshot, watch};

use crate::app::state::RequestKey;
use crate::chat::agent_loop::{
    AgentHost, AgentHostRequest, AgentTools, HostError, ModelContext, ReadyToolCall, ToolExecutor,
    ToolOutcome, TurnSummary, run_agent_turn,
};
use crate::chat::client::StreamOutcome;
use crate::chat::events::ChatRequest;
use crate::chat::tools::ask::{AskAnswers, AskQuestion};
use crate::db::chat::ToolCallTicket;
use crate::event::AppEvent;

/// BeginTool 阶段持久化失败时的稳定错误码前缀。
pub const BEGIN_TOOL_FAILED_CODE: &str = "tool call pending record failed";

/// running 推进失败时的稳定错误码前缀（pending 已提交，恢复时按 interrupted 处理）。
pub const RUNNING_TOOL_FAILED_CODE: &str = "tool call running record failed";

/// 工具结果落库失败时的稳定错误码前缀（文件可能已变化，绝不重试写入）。
pub const FINISH_TOOL_FAILED_CODE: &str = "tool result persistence failed";

/// 事件层收到的工具调用请求。
///
/// 两个变体都携带 oneshot 回执通道：事件层**先做 DB 与状态**，再回执；
/// 回执丢失（窗格关闭、会话切换）等价于取消，loop 不得执行文件 IO。
#[derive(Debug)]
pub enum AgentHostEvent {
    /// pending 落库 + 可见 loading 门槛；回执携带已提交的 ticket。
    BeginTool {
        key: RequestKey,
        call: ReadyToolCall,
        reply: oneshot::Sender<Result<ToolCallTicket, HostError>>,
    },
    /// 结果落库 + 状态终态；回执表示真实结果已持久化。
    ///
    /// 携带 begin 阶段发放的 ticket：事件层据此校验 session/message/call 归属，
    /// 不接受 loop 侧自造的标识。
    FinishTool {
        key: RequestKey,
        ticket: ToolCallTicket,
        outcome: ToolOutcome,
        reply: oneshot::Sender<Result<(), HostError>>,
    },
    /// 向用户提问并等待作答；回执丢失等价于取消。
    ///
    /// 与 `BeginTool`/`FinishTool` 不同，这里不改 DB：ask 的行状态已由前后两个门槛维护。
    AskUser {
        key: RequestKey,
        call_id: String,
        questions: Vec<AskQuestion>,
        reply: oneshot::Sender<Result<AskAnswers, HostError>>,
    },
    /// 文件 mutation 的 Prepare/Commit：worker 在路径锁内等待持久化回执。
    Mutation(super::file_snapshots::MutationHostEvent),
    /// 下一次采样前的本地命令结果增量。
    UserShellUpdates {
        key: RequestKey,
        reply: oneshot::Sender<Vec<String>>,
    },
}

/// [`AgentHost`] 的生产实现：把请求转投事件循环并等待回执。
pub(crate) struct EventHost {
    pub(crate) key: RequestKey,
    pub(crate) sender: mpsc::UnboundedSender<AppEvent>,
}

impl AgentHost for EventHost {
    fn send(&mut self, request: AgentHostRequest) {
        let event = match request {
            AgentHostRequest::BeginTool { call, reply } => AgentHostEvent::BeginTool {
                key: self.key.clone(),
                call,
                reply,
            },
            AgentHostRequest::FinishTool {
                ticket,
                outcome,
                reply,
            } => AgentHostEvent::FinishTool {
                key: self.key.clone(),
                ticket,
                outcome,
                reply,
            },
            AgentHostRequest::AskUser {
                call_id,
                questions,
                reply,
            } => AgentHostEvent::AskUser {
                key: self.key.clone(),
                call_id,
                questions,
                reply,
            },
            // mutation 请求不经 `EventHost`（由文件 recorder 直接投递）。
            AgentHostRequest::UserShellUpdates { reply } => AgentHostEvent::UserShellUpdates {
                key: self.key.clone(),
                reply,
            },
        };
        // 发送失败（事件循环已退出）时 drop 回执通道：调用方按取消处理。
        let _ = self.sender.send(AppEvent::AgentHost(event));
    }
}

/// 一次 agent turn 的驱动入口：注入 host / 执行器 / 单次模型流。
///
/// 生产路径用 [`drive_agent_turn`]；测试用它替换文件执行器（例如插桩 DB 顺序）。
pub(crate) async fn drive_with<H, E, S, F>(
    context: ModelContext,
    host: H,
    executor: E,
    stream_one: S,
    cancel: watch::Receiver<bool>,
) -> TurnSummary
where
    H: AgentHost,
    E: ToolExecutor,
    S: Fn(ChatRequest, watch::Receiver<bool>) -> F,
    F: std::future::Future<Output = StreamOutcome>,
{
    run_agent_turn(context, host, executor, stream_one, cancel).await
}

/// 生产轮次驱动：事件层 host + 生产工具执行器（文件 / 进程 / 计划三类分派）。
pub(crate) async fn drive_agent_turn<S, F>(
    key: RequestKey,
    context: ModelContext,
    sender: mpsc::UnboundedSender<AppEvent>,
    executor: AgentTools,
    stream_one: S,
    cancel: watch::Receiver<bool>,
) -> TurnSummary
where
    S: Fn(ChatRequest, watch::Receiver<bool>) -> F + Send + 'static,
    F: std::future::Future<Output = StreamOutcome> + Send,
{
    let host = EventHost { key, sender };
    drive_with(context, host, executor, stream_one, cancel).await
}

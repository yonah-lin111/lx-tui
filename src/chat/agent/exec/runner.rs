//! `exec` 无头宿主：把 agent loop 的请求直接落到 Database / 进程管理器 / 子 agent 池。
//!
//! 与 TUI 路径的差异只有门槛的实现位置：TUI 等一帧绘制后才回执 BeginTool，无头模式
//! 没有帧，pending → running → 回执在同一处顺序完成，loop 本身不感知差异。本模块不导入
//! `app` / `ui` / `tui` / `event`。
//!
//! 子 agent 池由 [`super::subagent::ExecSubagentPool`] 实现：`handle_op` 处理控制工具，
//! `handle_event` 处理子轮事件，`shutdown_all` 在主轮结束时收割全部子 agent。

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};

use crate::chat::agent::EXEC_ASK_UNSUPPORTED_CODE;
use crate::chat::agent_loop::{
    AgentHost, AgentHostRequest, AgentTools, HostError, ModelContext, ReadyToolCall, ToolOutcome,
    TurnExecution, TurnSummary, run_agent_turn,
};
use crate::chat::client::uncancellable;
use crate::chat::compact::AutoCompactNotice;
use crate::chat::events::{ChatDelta, ChatRequest, PartKind, RequestPurpose, StopReason};
use crate::chat::process::{ProcessManager, ProcessOwner};
use crate::chat::tools::{
    MutationError, MutationRecorder, MutationRecorderFactory, MutationTicket, ObservedMutation,
    PathMutation, PreparedMutation, ToolExecutionResult, read_registry,
};
use crate::db::Database;
use crate::db::chat::{
    ChatPart, ImageWrite, ObservedWrite, PathMutationWrite, ToolCallResult, ToolTerminalStatus,
    Usage,
};
use crate::git::{WorktreeIdentity, identify, write_blob};
use crate::platform::{RawImage, RawKind};

use super::cli::ExecModelTarget;
use super::processor::{ExecEventProcessor, SessionHeader};
use super::subagent::{ExecSubagentEvent, ExecSubagentPool};

/// 一次模型流的 future（boxed，便于注入脚本化假实现）。
pub type BoxStreamFuture =
    std::pin::Pin<Box<dyn std::future::Future<Output = crate::chat::client::StreamOutcome> + Send>>;

/// 模型流的构造入口；生产实现是 [`RealStreamFactory`]，测试注入脚本化流。
#[allow(clippy::type_complexity)]
pub trait ExecStreamFactory: Send + Sync {
    fn stream(
        &self,
        model: &super::cli::ExecModelTarget,
        request: crate::chat::events::ChatRequest,
        cancel: tokio::sync::watch::Receiver<bool>,
        on_delta: Box<dyn FnMut(&[crate::chat::events::ChatDelta]) + Send>,
    ) -> BoxStreamFuture;
}

/// 真实网络流：按冻结的模型目标调用 `run_stream`。
#[derive(Debug, Default, Clone, Copy)]
pub struct RealStreamFactory;

#[allow(clippy::type_complexity)]
impl ExecStreamFactory for RealStreamFactory {
    fn stream(
        &self,
        model: &super::cli::ExecModelTarget,
        request: crate::chat::events::ChatRequest,
        cancel: tokio::sync::watch::Receiver<bool>,
        mut on_delta: Box<dyn FnMut(&[crate::chat::events::ChatDelta]) + Send>,
    ) -> BoxStreamFuture {
        let transport = model.transport.clone();
        let base_url = model.base_url.clone();
        let api_key = model.api_key.clone();
        let idle_timeout = std::time::Duration::from_millis(model.idle_timeout_ms);
        Box::pin(async move {
            crate::chat::client::run_stream(
                &transport,
                &base_url,
                Some(&api_key),
                idle_timeout,
                request,
                cancel,
                move |deltas| on_delta(deltas),
            )
            .await
        })
    }
}

/// 文件 mutation 的持久化请求（worker 在路径锁内同步等待回执）。
pub(crate) enum ExecMutationEvent {
    Prepare {
        workspace: PathBuf,
        session_id: String,
        user_message_id: String,
        tool_call_id: String,
        changes: Vec<PathMutationWrite>,
        reply: oneshot::Sender<Result<MutationTicket, MutationError>>,
    },
    Commit {
        workspace: PathBuf,
        session_id: String,
        ticket: MutationTicket,
        observed: Vec<ObservedWrite>,
        reply: oneshot::Sender<Result<(), MutationError>>,
    },
}

/// loop / 子 agent 池与无头事件循环之间的内部消息。
pub(crate) enum ExecLoopEvent {
    MainDelta(Vec<ChatDelta>),
    Host(AgentHostRequest),
    Mutation(ExecMutationEvent),
    AutoCompact {
        notice: AutoCompactNotice,
        reply: oneshot::Sender<Result<(), HostError>>,
    },
    Subagent(ExecSubagentEvent),
}

const CHECKPOINT_BYTES: usize = 8 * 1024;
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);
const BEGIN_TOOL_FAILED_CODE: &str = "tool call pending record failed";
const RUNNING_TOOL_FAILED_CODE: &str = "tool call running record failed";
const FINISH_TOOL_FAILED_CODE: &str = "tool result persistence failed";
const AGENT_TURN_ABORTED_CODE: &str = "agent turn aborted";

/// loop 的无头宿主：请求经唯一事件循环落库后回执。
///
/// `model` / `stream_factory` 只服务于 [`Self::run_compact_summary`]；该入口预留给
/// 子 agent 的摘要请求，当前子池用 [`crate::db::Database::commit_compact`] 落检查点。
#[allow(dead_code)]
struct ExecHost {
    sender: mpsc::UnboundedSender<ExecLoopEvent>,
    model: ExecModelTarget,
    stream_factory: Arc<dyn ExecStreamFactory>,
}

impl ExecHost {
    fn new(
        sender: mpsc::UnboundedSender<ExecLoopEvent>,
        model: ExecModelTarget,
        stream_factory: Arc<dyn ExecStreamFactory>,
    ) -> Self {
        Self {
            sender,
            model,
            stream_factory,
        }
    }

    /// 独立执行一次压缩摘要请求（子 agent 运行时压缩复用同一模型目标）。
    ///
    /// 摘要请求不可取消；发送端必须活到请求结束，否则会被当成用户取消。
    #[allow(dead_code)]
    pub(crate) async fn run_compact_summary(&mut self, prompt: String) -> Result<String, String> {
        let request = ChatRequest {
            turns: Vec::new(),
            items: Vec::new(),
            model: self.model.snapshot(),
            variant_params: self.model.variant_params.clone(),
            purpose: RequestPurpose::CompactSummary,
            system_prompt: Some(prompt),
            tools: Arc::new(Vec::new()),
        };
        let (_cancel_sender, cancel) = uncancellable();
        let outcome = self
            .stream_factory
            .stream(&self.model, request, cancel, Box::new(|_| {}))
            .await;
        match &outcome.stop {
            StopReason::Failed { code } => return Err(code.clone()),
            StopReason::Cancelled => return Err("cancelled".to_string()),
            StopReason::Complete => {}
        }
        let mut text = String::new();
        for delta in &outcome.deltas {
            if let ChatDelta::Part(part) = delta
                && part.kind == PartKind::Text
            {
                text.push_str(&part.text);
            }
        }
        if text.trim().is_empty() {
            return Err("empty compact summary".to_string());
        }
        Ok(text)
    }
}

impl AgentHost for ExecHost {
    fn send(&mut self, request: AgentHostRequest) {
        if self.sender.send(ExecLoopEvent::Host(request)).is_err() {
            // 回执通道随请求一起丢弃：默认方法会把它翻译成取消。
            tracing::warn!(target: "lx::exec", "exec event loop is gone; host request dropped");
        }
    }

    fn notify_auto_compact(
        &mut self,
        notice: &AutoCompactNotice,
    ) -> impl Future<Output = Result<(), HostError>> + Send {
        let (reply, receiver) = oneshot::channel();
        let sent = self
            .sender
            .send(ExecLoopEvent::AutoCompact {
                notice: notice.clone(),
                reply,
            })
            .is_ok();
        async move {
            if !sent {
                return Err(HostError::Cancelled);
            }
            receiver.await.unwrap_or(Err(HostError::Cancelled))
        }
    }
}

/// 主轮时间线状态：parts、位置分配与工具调用登记。
struct ExecRunner<'a> {
    db: &'a Database,
    workspace: PathBuf,
    session_id: String,
    assistant_id: String,
    parts: Vec<ChatPart>,
    next_position: i64,
    open_part_index: Option<usize>,
    active_calls: HashMap<String, ReadyToolCall>,
    uncheckpointed_bytes: usize,
    last_checkpoint: Instant,
}

impl ExecRunner<'_> {
    fn new(
        db: &Database,
        workspace: PathBuf,
        session_id: String,
        assistant_id: String,
    ) -> ExecRunner<'_> {
        ExecRunner {
            db,
            workspace,
            session_id,
            assistant_id,
            parts: Vec::new(),
            next_position: 0,
            open_part_index: None,
            active_calls: HashMap::new(),
            uncheckpointed_bytes: 0,
            last_checkpoint: Instant::now(),
        }
    }

    /// 分发非子 agent 事件；`Subagent` 由 `dispatch_event` 先路由到池。
    fn handle_event(&mut self, event: ExecLoopEvent, processor: &mut dyn ExecEventProcessor) {
        match event {
            ExecLoopEvent::MainDelta(deltas) => self.handle_main_delta(&deltas, processor),
            ExecLoopEvent::Host(request) => self.handle_host(request, processor),
            ExecLoopEvent::Mutation(event) => handle_mutation_event(self.db, event),
            ExecLoopEvent::AutoCompact { notice, reply } => {
                processor.on_warning(&format!(
                    "context compacted: ~{} tokens in use (limit {})",
                    notice.estimated_tokens, notice.context_limit
                ));
                let _ = reply.send(Ok(()));
            }
            ExecLoopEvent::Subagent(_) => {
                tracing::warn!(target: "lx::exec", "subagent event reached the main runner");
            }
        }
    }

    fn handle_main_delta(&mut self, deltas: &[ChatDelta], processor: &mut dyn ExecEventProcessor) {
        append_deltas_to_parts(
            &mut self.parts,
            &mut self.next_position,
            &mut self.open_part_index,
            deltas,
            |kind, text| notify_part_closed(processor, kind, text),
        );
        self.uncheckpointed_bytes = self
            .uncheckpointed_bytes
            .saturating_add(deltas.iter().map(ChatDelta::payload_bytes).sum::<usize>());
        if self.uncheckpointed_bytes >= CHECKPOINT_BYTES
            || self.last_checkpoint.elapsed() >= CHECKPOINT_INTERVAL
        {
            self.checkpoint();
        }
    }

    fn handle_host(&mut self, request: AgentHostRequest, processor: &mut dyn ExecEventProcessor) {
        match request {
            AgentHostRequest::BeginTool { call, reply } => self.begin_tool(call, reply, processor),
            AgentHostRequest::FinishTool {
                ticket,
                outcome,
                reply,
            } => self.finish_tool(ticket, outcome, reply, processor),
            AgentHostRequest::AskUser { reply, .. } => {
                // 无头模式没有交互界面：让模型按 best-effort 继续，而不是挂起。
                let _ = reply.send(Err(HostError::Failed {
                    code: EXEC_ASK_UNSUPPORTED_CODE.to_string(),
                }));
            }
            AgentHostRequest::UserShellUpdates { reply } => {
                let _ = reply.send(Vec::new());
            }
            AgentHostRequest::Subagent { reply, .. } => {
                let _ = reply.send(Err(HostError::Cancelled));
            }
        }
    }

    /// 先闭合当前 part 并 checkpoint，保证 `begin_tool_call` 看到真实位置。
    fn begin_tool(
        &mut self,
        call: ReadyToolCall,
        reply: oneshot::Sender<Result<crate::db::chat::ToolCallTicket, HostError>>,
        processor: &mut dyn ExecEventProcessor,
    ) {
        self.close_open_part(processor);
        if let Err(error) = self
            .db
            .checkpoint(&self.workspace, &self.assistant_id, &self.parts)
        {
            tracing::warn!(target: "lx::exec", call_id = %call.call_id, code = error.code(), "tool timeline checkpoint failed");
            return fail_reply(reply, BEGIN_TOOL_FAILED_CODE, error.code());
        }
        let ticket = match self.db.begin_tool_call(
            &self.workspace,
            &self.session_id,
            &self.assistant_id,
            &call.call_id,
            &call.name,
            &call.arguments_json,
        ) {
            Ok(ticket) => ticket,
            Err(error) => {
                tracing::warn!(target: "lx::exec", call_id = %call.call_id, code = error.code(), "tool pending record failed");
                return fail_reply(reply, BEGIN_TOOL_FAILED_CODE, error.code());
            }
        };
        // 后续 part 必须严格落在该工具之后：SQLite 侧位置已由 begin_tool_call 占用。
        self.next_position = self.next_position.max(ticket.position.saturating_add(1));
        if let Err(error) = self.db.mark_tool_running(&self.workspace, &ticket) {
            tracing::warn!(target: "lx::exec", call_id = %ticket.call_id, code = error.code(), "tool running record failed");
            return fail_reply(reply, RUNNING_TOOL_FAILED_CODE, error.code());
        }
        processor.on_tool_started(&call);
        self.active_calls.insert(call.call_id.clone(), call);
        let _ = reply.send(Ok(ticket));
    }

    /// 真实结果落库后才回执；失败不重试写入。
    fn finish_tool(
        &mut self,
        ticket: crate::db::chat::ToolCallTicket,
        outcome: ToolOutcome,
        reply: oneshot::Sender<Result<(), HostError>>,
        processor: &mut dyn ExecEventProcessor,
    ) {
        match self
            .db
            .finish_tool_call(&self.workspace, &ticket, &db_result_of(&outcome))
        {
            Ok(()) => {
                if let Some(call) = self.active_calls.remove(&ticket.call_id) {
                    let duration_ms = crate::db::now_ms()
                        .saturating_sub(ticket.started_at_ms)
                        .max(0);
                    processor.on_tool_finished(&call, &outcome, duration_ms);
                }
                let _ = reply.send(Ok(()));
            }
            Err(error) => {
                tracing::warn!(target: "lx::exec", call_id = %ticket.call_id, code = error.code(), "tool result persistence failed");
                let _ = reply.send(Err(HostError::Failed {
                    code: format!("{FINISH_TOOL_FAILED_CODE} · {}", error.code()),
                }));
            }
        }
    }

    fn close_open_part(&mut self, processor: &mut dyn ExecEventProcessor) {
        close_part(
            &mut self.parts,
            &mut self.open_part_index,
            &mut |kind, text| notify_part_closed(processor, kind, text),
        );
    }

    /// 节流 checkpoint；失败只记日志，终态 `finish` 仍是权威写入。
    fn checkpoint(&mut self) {
        if let Err(error) = self
            .db
            .checkpoint(&self.workspace, &self.assistant_id, &self.parts)
        {
            tracing::warn!(target: "lx::exec", code = error.code(), "stream checkpoint failed");
        }
        self.uncheckpointed_bytes = 0;
        self.last_checkpoint = Instant::now();
    }

    fn final_message(&self) -> Option<String> {
        self.parts
            .iter()
            .rev()
            .find(|part| part.kind == PartKind::Text.as_str() && !part.content.trim().is_empty())
            .map(|part| part.content.clone())
    }
}

/// 本轮固定的执行参数（由顶层编排装配，runner 不再回读配置）。
pub struct ExecTurnConfig {
    pub workspace: PathBuf,
    pub cwd: PathBuf,
    pub session_id: String,
    pub user_message_id: String,
    pub assistant_id: String,
    pub prompt_display: String,
    pub model: ExecModelTarget,
    pub context: ModelContext,
    pub exa_api_key: String,
    pub tavily_api_key: String,
    pub mcp_snapshot: Arc<crate::mcp::McpToolSnapshot>,
}

/// 执行一整轮：装配宿主、驱动事件循环、收尾清理并提交终态。
pub async fn run_exec_turn(
    db: &Database,
    config: ExecTurnConfig,
    processor: &mut dyn ExecEventProcessor,
    stream_factory: Arc<dyn ExecStreamFactory>,
    cancel_rx: watch::Receiver<bool>,
) -> TurnSummary {
    let ExecTurnConfig {
        workspace,
        cwd,
        session_id,
        user_message_id,
        assistant_id,
        prompt_display,
        model,
        context,
        exa_api_key,
        tavily_api_key,
        mcp_snapshot,
    } = config;

    processor.on_session_started(&SessionHeader {
        version: env!("CARGO_PKG_VERSION"),
        workdir: &cwd,
        model: &model.model,
        provider: &model.provider,
        session_id: &session_id,
        prompt: &prompt_display,
    });

    let (sender, mut receiver) = mpsc::unbounded_channel::<ExecLoopEvent>();
    let process_manager = Arc::new(ProcessManager::new(
        ProcessOwner {
            workspace: cwd.clone(),
            chat_session_id: session_id.clone(),
            runtime_epoch: crate::db::new_external_id("epoch"),
        },
        None,
    ));
    let executor = AgentTools::new(
        Arc::clone(&process_manager),
        Arc::new(tokio::sync::Mutex::new(TurnExecution::default())),
    )
    .with_recorder(new_mutation_recorder_factory(
        sender.clone(),
        workspace.clone(),
        session_id.clone(),
        user_message_id.clone(),
    ))
    .with_reads(read_registry(&session_id))
    .with_image_support(model.supports_image_input)
    .with_web_search_keys(exa_api_key.clone(), tavily_api_key.clone())
    .with_mcp(
        crate::mcp::global_manager().clone(),
        Arc::clone(&mcp_snapshot),
        session_id.clone(),
    );

    let mut subagent_pool = ExecSubagentPool::new(
        workspace.clone(),
        cwd.clone(),
        session_id.clone(),
        model.clone(),
        exa_api_key,
        tavily_api_key,
        mcp_snapshot,
        sender.clone(),
        Arc::clone(&stream_factory),
    );
    let host = ExecHost::new(sender.clone(), model.clone(), Arc::clone(&stream_factory));
    let stream_one = {
        let factory = Arc::clone(&stream_factory);
        let sender = sender.clone();
        move |request: ChatRequest, cancel: watch::Receiver<bool>| {
            let factory = Arc::clone(&factory);
            let sender = sender.clone();
            let model = model.clone();
            async move {
                factory
                    .stream(
                        &model,
                        request,
                        cancel,
                        Box::new(move |deltas| {
                            let _ = sender.send(ExecLoopEvent::MainDelta(deltas.to_vec()));
                        }),
                    )
                    .await
            }
        }
    };

    let mut turn_handle = tokio::spawn(run_agent_turn(
        context,
        host,
        executor,
        stream_one,
        cancel_rx.clone(),
    ));
    let mut runner = ExecRunner::new(
        db,
        workspace.clone(),
        session_id.clone(),
        assistant_id.clone(),
    );
    let mut summary: Option<TurnSummary> = None;
    while summary.is_none() {
        tokio::select! {
            joined = &mut turn_handle => summary = Some(join_summary(joined)),
            event = receiver.recv() => match event {
                Some(event) => dispatch_event(
                    event,
                    &mut runner,
                    processor,
                    &mut subagent_pool,
                    &cancel_rx,
                ),
                // 所有发送端已释放：只等轮次本身收尾。
                None => break,
            },
        }
    }
    let summary = match summary {
        Some(summary) => summary,
        None => join_summary(turn_handle.await),
    };
    // 轮次已结束：把队列里剩下的事件处理完（Begin/Finish 回执都在这里收尾）。
    while let Ok(event) = receiver.try_recv() {
        dispatch_event(
            event,
            &mut runner,
            processor,
            &mut subagent_pool,
            &cancel_rx,
        );
    }

    runner.close_open_part(processor);
    let final_message = runner.final_message();
    let parts = std::mem::take(&mut runner.parts);
    drop(runner);

    subagent_pool.shutdown_all(db).await;
    process_manager.shutdown().await;
    if let Err(error) = db.finish(
        &workspace,
        &assistant_id,
        &parts,
        &db_turn_outcome(&summary.outcome),
        summary.usage,
    ) {
        tracing::warn!(target: "lx::exec", code = error.code(), "turn completion failed");
    }
    let _ = processor.on_turn_finished(&summary, final_message.as_deref());
    summary
}

/// loop 结局 → DB 终态（`complete` 的 stop reason 与事件层同口径）。
fn db_turn_outcome(outcome: &crate::chat::agent_loop::TurnOutcome) -> crate::db::chat::TurnOutcome {
    match outcome {
        crate::chat::agent_loop::TurnOutcome::Complete => crate::db::chat::TurnOutcome::Complete {
            stop_reason: Some("complete".into()),
        },
        crate::chat::agent_loop::TurnOutcome::Interrupted => {
            crate::db::chat::TurnOutcome::Interrupted
        }
        crate::chat::agent_loop::TurnOutcome::Failed { code } => {
            crate::db::chat::TurnOutcome::Failed {
                error_code: code.clone(),
            }
        }
    }
}

/// 事件 → 主 runner / 子 agent 池。
fn dispatch_event(
    event: ExecLoopEvent,
    runner: &mut ExecRunner<'_>,
    processor: &mut dyn ExecEventProcessor,
    subagents: &mut ExecSubagentPool,
    cancel: &watch::Receiver<bool>,
) {
    match event {
        ExecLoopEvent::Host(AgentHostRequest::Subagent { call_id, op, reply }) => {
            subagents.handle_op(
                runner.db,
                &runner.assistant_id,
                call_id.as_deref(),
                op,
                cancel,
                reply,
            );
        }
        ExecLoopEvent::Subagent(event) => subagents.handle_event(runner.db, event),
        other => runner.handle_event(other, processor),
    }
}

fn join_summary(result: Result<TurnSummary, tokio::task::JoinError>) -> TurnSummary {
    match result {
        Ok(summary) => summary,
        Err(error) => {
            tracing::error!(target: "lx::exec", %error, "agent turn task aborted");
            TurnSummary::failed(AGENT_TURN_ABORTED_CODE, Usage::default())
        }
    }
}

fn fail_reply<T>(reply: oneshot::Sender<Result<T, HostError>>, prefix: &str, code: &str) {
    let _ = reply.send(Err(HostError::Failed {
        code: format!("{prefix} · {code}"),
    }));
}

/// 闭合开放 part：写完成时刻并通知处理器（思考块/正文块各自成段）。
fn close_part(
    parts: &mut [ChatPart],
    open_part_index: &mut Option<usize>,
    on_part_closed: &mut impl FnMut(PartKind, &str),
) {
    let Some(index) = open_part_index.take() else {
        return;
    };
    let Some(part) = parts.get_mut(index) else {
        return;
    };
    part.completed_at_ms = Some(crate::db::now_ms());
    on_part_closed(part_kind_of(&part.kind), &part.content);
}

/// 存储值 → 强类型 part kind；未知值按正文处理（老库兜底）。
fn part_kind_of(kind: &str) -> PartKind {
    if kind == PartKind::Reasoning.as_str() {
        PartKind::Reasoning
    } else {
        PartKind::Text
    }
}

fn notify_part_closed(processor: &mut dyn ExecEventProcessor, kind: PartKind, text: &str) {
    match kind {
        PartKind::Reasoning => processor.on_reasoning(text),
        PartKind::Text => processor.on_assistant_text(text),
    }
}

/// 增量聚合：同 kind 续写；换 kind 先闭合旧块；终态事件闭合当前块。
pub(crate) fn append_deltas_to_parts(
    parts: &mut Vec<ChatPart>,
    next_position: &mut i64,
    open_part_index: &mut Option<usize>,
    deltas: &[ChatDelta],
    mut on_part_closed: impl FnMut(PartKind, &str),
) {
    for delta in deltas {
        match delta {
            ChatDelta::Part(delta) => {
                let same_kind = open_part_index
                    .and_then(|index| parts.get(index))
                    .is_some_and(|part| part.kind == delta.kind.as_str());
                if same_kind {
                    if let Some(part) = open_part_index.and_then(|index| parts.get_mut(index)) {
                        part.content.push_str(&delta.text);
                        if let Some(metadata) = delta.metadata.as_ref()
                            && let Ok(json) = serde_json::to_string(metadata)
                        {
                            part.provider_metadata_json = Some(json);
                        }
                        part.ended_at_ms = Some(crate::db::now_ms());
                    }
                    continue;
                }
                close_part(parts, open_part_index, &mut on_part_closed);
                let now = crate::db::now_ms();
                parts.push(ChatPart {
                    position: *next_position,
                    kind: delta.kind.as_str().to_string(),
                    content: delta.text.clone(),
                    provider_metadata_json: delta
                        .metadata
                        .as_ref()
                        .and_then(|metadata| serde_json::to_string(metadata).ok()),
                    started_at_ms: Some(now),
                    ended_at_ms: Some(now),
                    completed_at_ms: None,
                });
                *next_position = next_position.saturating_add(1);
                *open_part_index = Some(parts.len().saturating_sub(1));
            }
            ChatDelta::ReasoningEnded { .. }
            | ChatDelta::ToolInputStarted { .. }
            | ChatDelta::ResponseFinished { .. }
            | ChatDelta::Finished { .. } => {
                close_part(parts, open_part_index, &mut on_part_closed);
            }
            _ => {}
        }
    }
}

/// 工具结局 → DB 结算记录（取消不带结果体，失败与成功都带规范化 JSON）。
pub(crate) fn db_result_of(outcome: &ToolOutcome) -> ToolCallResult {
    match outcome {
        ToolOutcome::Cancelled => ToolCallResult {
            status: ToolTerminalStatus::Cancelled,
            result_json: None,
            error_text: None,
        },
        ToolOutcome::Executed(execution) => ToolCallResult {
            status: if execution.is_failure() {
                ToolTerminalStatus::Failed
            } else {
                ToolTerminalStatus::Completed
            },
            result_json: crate::chat::tools::to_result_json(execution),
            error_text: failure_text(execution),
        },
    }
}

/// 失败原因（给模型与 `--json` 的精简单行；不含文件内容与命令输出）。
fn failure_text(execution: &ToolExecutionResult) -> Option<String> {
    match execution {
        ToolExecutionResult::Failed { message, .. } => Some(message.chars().take(512).collect()),
        ToolExecutionResult::Process(result) => result
            .failure_message()
            .map(|message| message.chars().take(512).collect()),
        _ => None,
    }
}

/// 事件层 mutation 请求的直连 DB 处理；回执丢失由 recorder 按取消收尾。
pub(crate) fn handle_mutation_event(db: &Database, event: ExecMutationEvent) {
    match event {
        ExecMutationEvent::Prepare {
            workspace,
            session_id,
            user_message_id,
            tool_call_id,
            changes,
            reply,
        } => {
            let result = db
                .prepare_mutations(
                    &workspace,
                    &session_id,
                    &user_message_id,
                    &tool_call_id,
                    &changes,
                )
                .map(|ticket| MutationTicket { ids: ticket.ids })
                .map_err(mutation_error);
            let _ = reply.send(result);
        }
        ExecMutationEvent::Commit {
            workspace,
            session_id,
            ticket,
            observed,
            reply,
        } => {
            let result = db
                .commit_mutations(
                    &workspace,
                    &session_id,
                    &crate::db::chat::PreparedTicket { ids: ticket.ids },
                    &observed,
                )
                .map_err(mutation_error);
            let _ = reply.send(result);
        }
    }
}

fn mutation_error(error: crate::db::DbError) -> MutationError {
    let code = match error {
        crate::db::DbError::RewindLimit => "limit",
        crate::db::DbError::StaleRead => "stale_read",
        _ => "persist",
    };
    MutationError::new(
        code,
        format!("failed to persist file snapshot: {}", error.code()),
    )
}

/// 每轮构造一次的 recorder 工厂；每次调用克隆出按 `call_id` 绑定的 recorder。
/// worker 线程在路径锁内同步等待事件循环的 DB 回执。
#[derive(Clone)]
struct ExecRecorder {
    sender: mpsc::UnboundedSender<ExecLoopEvent>,
    workspace: PathBuf,
    session_id: String,
    user_message_id: String,
    tool_call_id: String,
}

impl MutationRecorderFactory for ExecRecorder {
    fn for_call(&self, call_id: &str) -> Arc<dyn MutationRecorder> {
        Arc::new(Self {
            tool_call_id: call_id.to_string(),
            ..self.clone()
        })
    }
}

impl MutationRecorder for ExecRecorder {
    fn prepare(&self, mutation: &PreparedMutation) -> Result<MutationTicket, MutationError> {
        let changes = mutation.paths.iter().map(change_write).collect();
        let (reply, receiver) = oneshot::channel();
        let event = ExecMutationEvent::Prepare {
            workspace: self.workspace.clone(),
            session_id: self.session_id.clone(),
            user_message_id: self.user_message_id.clone(),
            tool_call_id: self.tool_call_id.clone(),
            changes,
            reply,
        };
        if self.sender.send(ExecLoopEvent::Mutation(event)).is_err() {
            return Err(MutationError::new(
                "cancelled",
                "the exec event loop is gone; file mutation was not started",
            ));
        }
        receiver
            .blocking_recv()
            .map_err(|_| MutationError::new("cancelled", "snapshot acknowledgement lost"))?
    }

    fn commit(
        &self,
        ticket: MutationTicket,
        observed: &ObservedMutation,
    ) -> Result<(), MutationError> {
        let observed = observed
            .paths
            .iter()
            .map(|item| ObservedWrite {
                actual_after: image_write(&item.actual_after, None, false),
                success: item.success,
            })
            .collect();
        let (reply, receiver) = oneshot::channel();
        let event = ExecMutationEvent::Commit {
            workspace: self.workspace.clone(),
            session_id: self.session_id.clone(),
            ticket,
            observed,
            reply,
        };
        if self.sender.send(ExecLoopEvent::Mutation(event)).is_err() {
            return Err(MutationError::new(
                "cancelled",
                "the exec event loop is gone; file mutation was not committed",
            ));
        }
        receiver
            .blocking_recv()
            .map_err(|_| MutationError::new("cancelled", "mutation acknowledgement lost"))?
    }
}

/// 注入生产 recorder 工厂（每轮一次，按 call_id 绑定所有权）。
pub(crate) fn new_mutation_recorder_factory(
    sender: mpsc::UnboundedSender<ExecLoopEvent>,
    workspace: PathBuf,
    session_id: String,
    user_message_id: String,
) -> Arc<dyn MutationRecorderFactory> {
    Arc::new(ExecRecorder {
        sender,
        workspace,
        session_id,
        user_message_id,
        tool_call_id: String::new(),
    })
}

/// 工具侧 mutation → DB 写入参数；Git blob 在调用线程（worker）完成。
fn change_write(change: &PathMutation) -> PathMutationWrite {
    let identity = identify(&change.target);
    let anchor = crate::platform::path_anchor(&change.target);
    PathMutationWrite {
        operation: change.operation.as_str().to_string(),
        path: crate::platform::encode_path(&change.path),
        target_path: crate::platform::encode_path(&change.target),
        anchor_path: anchor
            .as_ref()
            .map(|(lexical, _)| crate::platform::encode_path(lexical)),
        anchor_canonical: anchor
            .as_ref()
            .map(|(_, canonical)| crate::platform::encode_path(canonical)),
        before: image_write(&change.before, identity.as_ref(), true),
        planned_after: image_write(&change.planned_after, identity.as_ref(), true),
        expected_version: change.expected_version.clone(),
    }
}

/// 原始镜像 → DB 镜像；`with_git` 为真时写入项目对象库（失败降级为纯字节）。
fn image_write(
    image: &RawImage,
    identity: Option<&WorktreeIdentity>,
    with_git: bool,
) -> ImageWrite {
    let git_oid = if with_git && image.kind == RawKind::Regular {
        identity.and_then(|identity| write_blob(identity, &image.bytes))
    } else {
        None
    };
    ImageWrite {
        kind: match image.kind {
            RawKind::Missing => "missing".to_string(),
            RawKind::Regular => "regular".to_string(),
            RawKind::Symlink => "symlink".to_string(),
        },
        bytes: (image.kind == RawKind::Regular).then(|| image.bytes.clone()),
        mode: image.mode.map(i64::from),
        link_target: image
            .link_target
            .as_ref()
            .map(|target| crate::platform::encode_path(target)),
        git_oid,
        worktree_root: identity.map(|identity| crate::platform::encode_path(&identity.root)),
        git_dir: identity.map(|identity| crate::platform::encode_path(&identity.git_dir)),
        head_ref: identity.and_then(|identity| identity.head_ref.clone()),
        head_oid: identity.and_then(|identity| identity.head_oid.clone()),
    }
}

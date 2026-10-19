//! lx 聊天轮次编排：取走网络效应、装配上下文、启动 agent turn 任务并回投结果。
//!
//! 本模块是**唯一**负责“DB 落库 → 状态 → 回执”顺序的地方：
//!
//! - [`pump_chat_streams`] 把 `StartChatStream` 效应变成一次 [`crate::chat::agent_loop`]
//!   任务（多步采样 + 串行工具调度）；取消句柄、冻结工作区都在这里维护。
//! - 后台任务只通过 `mpsc::UnboundedSender<AppEvent>` 投递**纯数据**，通过 oneshot 等待
//!   事件层回执；网络模块与 loop 都不修改 `AppState`。
//! - [`handle_agent_host_event`] 执行工具调用的持久化门槛：`BeginTool` 必须先提交 pending
//!   事务、再让 UI 显示 pending、最后（可见窗格还要等该帧绘制）才回执 ticket；
//!   `FinishTool` 必须先原子写真实结果、再更新状态、最后回执。
//! - delta 与整轮终态仍按 `(pane, session, generation)` 过滤，旧 generation 事件一律丢弃。

mod host;
mod process_events;
pub mod runtime;
mod timeline;
mod tool_calls;

pub use process_events::handle_process_event;
// 工具握手与时间线辅助的实现在子模块；本模块保留唯一入口。
pub(crate) use timeline::{chat_part_views_to_db, latest_user_draft, schedule_checkpoint};
#[cfg(test)]
pub(crate) use tool_calls::view_summary;
pub(crate) use tool_calls::{begin_tool, finish_tool, flush_pending_tool_begins};

/// 停止本轮进程（D9）：本轮新建集合 ∪ 当前正在等待/交互的进程。
///
/// 此前启动、本轮未操作的旧服务保持运行：只有“这一轮动过的”和“此刻正在等的”
/// 才随 Esc 停止一起清理。停止在后台任务执行，事件循环继续响应输入与渲染。
pub fn stop_turn_processes(streams: &mut ChatStreams, key: &RequestKey) {
    let Some(runtime) = streams.runtimes.get(&key.session) else {
        return;
    };
    let manager = runtime.manager().clone();
    let Some(turn) = runtime.turn_context(&key.generation.to_string()) else {
        return;
    };
    tokio::spawn(async move {
        let targets: Vec<u64> = turn.lock().await.stop_targets().into_iter().collect();
        manager.stop_many(targets).await;
    });
}

#[cfg(test)]
mod tests;

pub use host::{
    AgentHostEvent, BEGIN_TOOL_FAILED_CODE, FINISH_TOOL_FAILED_CODE, RUNNING_TOOL_FAILED_CODE,
};

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, watch};

use crate::app::effects::Effect;
use crate::app::state::{AppState, RequestKey, RequestState};
use crate::app::update;
use crate::chat::agent_loop::{
    AgentTools, HostError, ModelContext, ReadyToolCall, ToolOutcome, TurnSummary,
};
use crate::chat::context::{self, AssemblyInput};
use crate::chat::events::{ChatDelta, ChatRequest, StopReason};
use crate::chat::process::ProcessUiEvent;
use crate::chat::tools::ToolExecutionResult;
use crate::db::chat::{ToolCallResult, ToolCallTicket, ToolTerminalStatus, Usage};
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::layout::PaneId;
use crate::ui::text;

/// checkpoint 落盘间隔与累计字节阈值（设计 5.3）。
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);
const CHECKPOINT_BYTES: usize = 4 * 1024;

/// 在途轮次的取消句柄与 pending ack 门槛（按 request key 存放）。
#[derive(Default)]
pub struct ChatStreams {
    /// 取消信号发送端。
    cancels: HashMap<RequestKey, watch::Sender<bool>>,
    /// checkpoint 合并状态：最近一次落盘时刻 + 累计字节。
    checkpoints: HashMap<RequestKey, (std::time::Instant, usize)>,
    /// 冻结的工作区根（工具行落库基准；发送时固定，后续切换窗格不影响）。
    workspaces: HashMap<RequestKey, PathBuf>,
    /// 已提交 pending 但**尚未**回执 ticket 的调用：等该帧绘制后推进 running。
    pending_begins: Vec<PendingBegin>,
    /// 上一帧几何中可见的窗格；不可见窗格无需等待渲染即可回执。
    visible_panes: Vec<PaneId>,
    /// 会话进程运行实例（跨轮保留；切会话/关窗格/退出时清理）。
    pub(crate) runtimes: runtime::ProcessRuntimes,
    /// 进程 UI 事件通道（由事件循环在启动时注入）。
    process_sender: Option<mpsc::UnboundedSender<ProcessUiEvent>>,
    /// 每进程上次输出快照落库时刻（限流合并）。
    process_snapshot_at: HashMap<u64, std::time::Instant>,
}

/// 一次待回执的 BeginTool：DB pending 已提交，UI 已转 Pending。
struct PendingBegin {
    key: RequestKey,
    ticket: ToolCallTicket,
    /// 回执通道；`None` 表示已被丢弃（窗格关闭，不得再执行）。
    reply: Option<oneshot::Sender<Result<ToolCallTicket, HostError>>>,
}

impl ChatStreams {
    /// 记录本帧可见窗格（由事件循环在几何计算后写入）。
    pub fn set_visible_panes(&mut self, rects: &[(PaneId, ratatui::layout::Rect)]) {
        self.visible_panes = rects.iter().map(|(pane, _)| *pane).collect();
    }

    /// 该窗格本帧是否可见（不可见时 loading 无需等渲染）。
    fn pane_visible(&self, pane: PaneId) -> bool {
        self.visible_panes.contains(&pane)
    }

    /// 注入进程 UI 事件通道（由事件循环在启动时调用一次）。
    pub fn set_process_sender(&mut self, sender: mpsc::UnboundedSender<ProcessUiEvent>) {
        self.process_sender = Some(sender);
    }

    /// 该进程本次输出快照是否到落库时间（每进程最多一个待合并快照）。
    pub(crate) fn process_snapshot_due(&mut self, session_id: u64, interval: Duration) -> bool {
        let now = std::time::Instant::now();
        match self.process_snapshot_at.get(&session_id) {
            Some(last) if now.saturating_duration_since(*last) < interval => false,
            _ => {
                self.process_snapshot_at.insert(session_id, now);
                true
            }
        }
    }

    /// 按进程会话 ID 找到持有它的运行实例（旧代次/已清理返回 None）。
    pub(crate) fn runtime_by_session_id(
        &self,
        session_id: u64,
    ) -> Option<&runtime::SessionRuntime> {
        self.runtimes
            .sessions()
            .into_iter()
            .filter_map(|session| self.runtimes.get(&session))
            .find(|runtime| runtime.manager.snapshot(session_id).is_some())
    }

    /// 取（必要时新建）某会话的运行实例。
    fn runtime_for(
        &mut self,
        workspace: PathBuf,
        session_id: &str,
    ) -> Option<&mut runtime::SessionRuntime> {
        let sender = self.process_sender.clone()?;
        Some(self.runtimes.get_or_create(workspace, session_id, sender))
    }

    /// 请求取消一轮（Esc / 窗格关闭 / 会话切换）。返回是否确有在途轮次。
    ///
    /// 只发出取消信号：已提交 pending、尚未执行的调用由 loop 在下一检查点结算为
    /// `cancelled`（**不执行**文件 IO），回执通道保持可用以便它完成该结算。
    fn cancel(&mut self, key: &RequestKey) -> bool {
        let Some(cancel) = self.cancels.get(key) else {
            return false;
        };
        let _ = cancel.send(true);
        true
    }

    /// 轮次结束：清除该 key 的取消句柄、checkpoint、冻结工作区与未回执的 pending。
    ///
    /// **不清理进程**：正常结束一轮模型回答时，所属服务继续运行（下一轮复用同一实例）。
    fn finish(&mut self, key: &RequestKey) {
        self.cancels.remove(key);
        self.checkpoints.remove(key);
        self.workspaces.remove(key);
        self.pending_begins.retain(|pending| &pending.key != key);
        // 只丢弃该轮执行上下文；进程留给会话（正常结束一轮不清理服务）。
        self.runtimes
            .finish_turn(&key.session, &key.generation.to_string());
    }

    /// 丢弃某 key 尚未回执的 BeginTool（窗格关闭时）：
    /// 回执通道被 drop，loop 得到 `Err(RecvError)` 即等于取消，**不会执行**文件 IO。
    fn drop_pending(&mut self, key: &RequestKey) {
        self.pending_begins.retain(|pending| &pending.key != key);
    }
}

/// 网络回投事件。
#[derive(Debug)]
pub enum ChatNetEvent {
    /// 一批流增量（可合并；避免逐 token 无界堆积）。
    Delta {
        key: RequestKey,
        deltas: Vec<ChatDelta>,
    },
    /// 整轮结束（含终态原因与**累计** usage）。
    Finished {
        key: RequestKey,
        reason: StopReason,
        usage: Usage,
    },
}

/// 取走网络效应并启动后台 agent turn 任务；返回是否有变化。
///
/// `sender` 把结果投回唯一事件循环；本函数只做装配、取消与 spawn。
pub fn pump_chat_streams(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
) -> bool {
    // 会话切换、`/new`、窗格关闭：在途轮次不再归属于任何可见请求 → 取消。
    reap_stale_streams(state, streams);
    // 已不再被任何窗格绑定的会话运行实例：停止其全部进程并释放句柄。
    reap_orphan_runtimes(state, streams);
    let mut changed = false;
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::StartChatStream { .. } | Effect::CancelChatStream { .. }
            )
        })
        .cloned()
        .collect();
    if effects.is_empty() {
        return false;
    }
    state.effects.retain(|effect| {
        !matches!(
            effect,
            Effect::StartChatStream { .. } | Effect::CancelChatStream { .. }
        )
    });
    for effect in effects {
        match effect {
            Effect::StartChatStream { key, assistant_id } => {
                changed |= start_stream(state, streams, sender, key, assistant_id);
            }
            Effect::CancelChatStream { key } => {
                // loop 在下一个检查点停止未开始的工具与后续采样。
                changed |= streams.cancel(&key);
                // D9：同时停止本轮新建的进程与当前正在等待/交互的进程
                // （此前启动、本轮未操作的旧服务保持运行）。
                stop_turn_processes(streams, &key);
            }
            _ => {}
        }
    }
    changed
}

/// 取消已不再属于任何在途请求的轮次（窗格删除、会话切换、generation 前进）。
fn reap_stale_streams(state: &AppState, streams: &mut ChatStreams) {
    let stale: Vec<RequestKey> = streams
        .cancels
        .keys()
        .filter(|key| active_assistant_id(state, key).is_none())
        .cloned()
        .collect();
    for key in stale {
        streams.cancel(&key);
        streams.drop_pending(&key);
        // 窗格关闭/会话切换：取消已不足以表达“清理进程”，这里按本轮范围停止。
        stop_turn_processes(streams, &key);
    }
}

/// 回收不再被任何窗格绑定的会话运行实例（`/new`、切换会话、关窗格）。
///
/// 只移除运行实例并**异步停止**其进程：事件循环不阻塞等待，回执由后台任务完成。
fn reap_orphan_runtimes(state: &AppState, streams: &mut ChatStreams) {
    let bound: Vec<String> = state
        .all_pane_ids()
        .into_iter()
        .filter_map(|pane| state.pane_anywhere(pane))
        .filter_map(|pane| pane.lx.chat.session_id.clone())
        .collect();
    let orphans: Vec<String> = streams
        .runtimes
        .sessions()
        .into_iter()
        .filter(|session| !bound.contains(session))
        .collect();
    for session in orphans {
        if let Some(runtime) = streams.runtimes.take(&session) {
            tokio::spawn(async move {
                runtime.manager.shutdown().await;
            });
        }
    }
}

/// 该 request key 仍是窗格的在途流时，返回本轮 assistant 消息 id。
///
/// 只有通过这一检查的调用才允许写库与执行文件工具：旧 generation、其它窗格、
/// 会话切换后的迟到请求都在这里被拒绝。
pub(crate) fn active_assistant_id(state: &AppState, key: &RequestKey) -> Option<String> {
    let pane = state.pane_anywhere(key.pane)?;
    match &pane.lx.chat.request {
        RequestState::Streaming {
            key: active,
            assistant_id,
        } if active == key => Some(assistant_id.clone()),
        _ => None,
    }
}

/// 由窗格消息视图装配本轮请求；返回（请求，省略的完整轮次数，草稿是否单独超限）。
///
/// 装配来源固定为**窗格内存视图**（经 [`chat_part_views_to_db`] 映射为记录），
/// 因此 `/resume` 加载的历史与本轮新增步骤走同一条路径：
///
/// - 本轮草稿取**最近一条 user 消息**——`on_turn_committed` 已在其后追加了 streaming
///   的 assistant 占位，故不能取 `messages.last()`（那会得到占位、过滤 user 后变空串，
///   导致请求携带空 user 消息被供应商拒绝）。
/// - 只有「user 与 assistant 均 complete」的轮次参与回放；当前轮 assistant 仍在
///   streaming，不会被当作历史（其工具步骤由 `ModelContext` 在同一轮内累计）。
/// - 工具步骤也一并映射（[`crate::db::chat::chat_message_views_to_db`]），使已完成轮次的
///   call/result 能成对回放，interrupted/pending 调用不进入请求。
pub(crate) fn assemble_turn_request(
    messages: &[crate::app::state::ChatMessageView],
    frozen: &crate::app::state::FrozenModel,
) -> AssembledTurn {
    let draft = latest_user_draft(messages);
    let db_messages = chat_part_views_to_db(messages);
    let model = crate::db::chat::ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let assembly = context::assemble(&AssemblyInput {
        messages: &db_messages,
        draft: &draft,
        model: &model,
        context_limit: frozen.context_limit,
        output_limit: frozen.output_limit,
        variant_params: frozen.variant_params.clone(),
    });
    AssembledTurn {
        request: ChatRequest {
            turns: assembly.request.turns,
            items: assembly.request.items,
            model,
            variant_params: frozen.variant_params.clone(),
        },
        omitted_turns: assembly.omitted_turns,
        draft_exceeds: assembly.draft_exceeds,
    }
}

/// 装配结果：请求 + 需要调用方处理的两种信号。
pub(crate) struct AssembledTurn {
    pub request: ChatRequest,
    /// 省略的完整轮次数（一次性提示）。
    pub omitted_turns: usize,
    /// 当前用户消息单独超限：调用方报错，**不截断**用户输入。
    pub draft_exceeds: bool,
}

/// 当前用户消息超出模型上下文预算时的稳定错误码（调用方报错，不截断输入）。
pub const DRAFT_EXCEEDS_CONTEXT_CODE: &str = "message exceeds model context";

/// 启动一轮：装配上下文、冻结模型与工作区并 spawn agent turn。
fn start_stream(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    _assistant_id: String,
) -> bool {
    // 已有同 key 在途流：不重复启动。
    if streams.cancels.contains_key(&key) {
        return false;
    }
    let Some(pane) = state.pane_anywhere(key.pane) else {
        return false;
    };
    // 冻结模型快照：从窗格当前消息流状态读取该轮的选择（发送时冻结语义：
    // Committing 已把模型写入 DB；流启动时按窗格 effective_selection 重解析——
    // 在途请求持有启动时快照，设置页中途保存不影响本轮）。
    // 必须走 `effective_selection`：它会把模型默认思考等级（variant）补上，
    // 否则「未显式选等级」的窗格请求不会带思考参数（reasoning 恒为空）。
    let selection = pane.lx.chat.effective_selection(&state.models);
    let Some(selection) = selection else {
        // 无有效模型：直接按失败收尾。
        fail_turn(sender, key, "no model available");
        return true;
    };
    let Some(frozen) = crate::app::state::FrozenModel::resolve(&state.models, &selection) else {
        fail_turn(sender, key, "model configuration invalid");
        return true;
    };
    // 上下文：从窗格内存中的完整轮次装配（DB 历史与内存一致；partial 排除）。
    let assembled = assemble_turn_request(&pane.lx.chat.messages, &frozen);
    // 计划快照先克隆：运行上下文摘要要在后面的可变借用窗外拼装。
    let current_plan = pane.lx.chat.plan.clone();
    // 省略提示（一次性显示）。
    if assembled.omitted_turns > 0
        && let Some(pane) = state.pane_mut_anywhere(key.pane)
    {
        pane.lx.chat.omitted_turns_notice = Some(assembled.omitted_turns);
    }
    // 当前用户消息单独超限：整轮失败并报错，绝不截断用户输入。
    if assembled.draft_exceeds {
        fail_turn(sender, key, DRAFT_EXCEEDS_CONTEXT_CODE);
        return true;
    }
    let request = assembled.request;
    // 冻结工作区根：本轮所有工具的相对路径都以它为准（后续切换窗格不影响）。
    let Some(workspace) = state.workspace_cwd_for_pane(key.pane) else {
        fail_turn(sender, key, "workspace path unavailable");
        return true;
    };
    let mut prepared = prepare_turn(request, workspace);
    // 取消通道；sender 由 ChatStreams 持有。
    let (cancel_sender, cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender);
    streams.workspaces.insert(key.clone(), prepared.cwd.clone());
    // 运行实例：同一聊天会话跨轮复用同一进程管理器；本轮执行上下文独立。
    let turn_slot = key.session.clone();
    let Some(runtime) = streams.runtime_for(prepared.cwd.clone(), &turn_slot) else {
        // 没有进程事件通道（测试或非进程环境）：仍可执行文件工具，进程工具按失败返回。
        fail_turn(sender, key, "process runtime unavailable");
        return true;
    };
    let turn = runtime.turn(&key.generation.to_string());
    let executor = AgentTools::new(runtime.manager.clone(), turn);
    // 本地运行上下文：存活进程清单 + 当前计划（有界，避免历史裁剪后模型忘记 session_id）。
    let summary = runtime_context_summary(runtime.manager.as_ref(), &current_plan);
    prepared.context = prepared.context.with_runtime_summary(summary);
    let transport = frozen.transport.clone();
    let base_url = frozen.base_url.clone();
    let api_key = frozen.api_key.clone();
    let idle_timeout = Duration::from_millis(frozen.idle_timeout_ms);
    let net_sender = sender.clone();
    let stream_key = key.clone();
    tokio::spawn(async move {
        // 每次采样一次模型流；增量即时回投（打字机效果），整轮终态在 loop 结束时上报。
        let live_sender = net_sender.clone();
        let live_key = stream_key.clone();
        let stream_one = move |request: ChatRequest, cancel: watch::Receiver<bool>| {
            let sender = live_sender.clone();
            let key = live_key.clone();
            let transport = transport.clone();
            let base_url = base_url.clone();
            let api_key = api_key.clone();
            async move {
                crate::chat::client::run_stream(
                    &transport,
                    &base_url,
                    Some(api_key.as_str()),
                    idle_timeout,
                    request,
                    cancel,
                    move |deltas| {
                        let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Delta {
                            key: key.clone(),
                            deltas: deltas.to_vec(),
                        }));
                    },
                )
                .await
            }
        };
        let summary = host::drive_agent_turn(
            stream_key.clone(),
            prepared.context,
            net_sender.clone(),
            executor,
            stream_one,
            cancel,
        )
        .await;
        report_turn(net_sender, stream_key, summary);
    });
    true
}

/// 本轮运行上下文摘要：存活进程（有界）与当前计划（有界）。
///
/// 它是**本地运行状态**，不是工具结果：不新增 call/result 对，也不触发新的模型请求。
fn runtime_context_summary(
    manager: &crate::chat::process::ProcessManager,
    plan: &Option<crate::app::state::PlanSnapshotView>,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let running: Vec<crate::chat::process::ProcessSnapshot> = manager
        .snapshots()
        .into_iter()
        .filter(|snapshot| !snapshot.state.is_terminal())
        .take(MAX_CONTEXT_PROCESSES)
        .collect();
    if !running.is_empty() {
        lines.push(
            "[local runtime] running processes (use write_stdin with session_id):".to_string(),
        );
        for snapshot in running {
            let command: String = snapshot.command.chars().take(120).collect();
            lines.push(format!(
                "- session_id={} cwd={} · {}",
                snapshot.session_id, snapshot.cwd, command
            ));
        }
    }
    if let Some(plan) = plan
        && !plan.is_empty()
    {
        lines.push("[local runtime] current plan:".to_string());
        for (step, status) in plan.steps.iter().take(MAX_CONTEXT_PLAN_STEPS) {
            let marker = text::agent::plan_marker(*status);
            lines.push(format!("- {marker} {step}"));
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

/// 摘要里最多列出的运行进程数。
const MAX_CONTEXT_PROCESSES: usize = 8;
/// 摘要里最多列出的计划步骤数。
const MAX_CONTEXT_PLAN_STEPS: usize = 16;

/// 一轮的可执行快照：冻结的上下文 + 冻结的工作区根。
///
/// 与 spawn 分离，使测试能用假 transport 直接驱动同一 agent turn（不经真实 HTTP）。
pub(crate) struct PreparedTurn {
    pub context: ModelContext,
    pub cwd: PathBuf,
}

/// 由已装配请求与冻结 cwd 构造可执行快照。
pub(crate) fn prepare_turn(request: ChatRequest, cwd: PathBuf) -> PreparedTurn {
    let context = ModelContext::new(request, cwd.clone());
    PreparedTurn { context, cwd }
}

/// 启动前失败：直接回投整轮失败终态。
fn fail_turn(sender: &mpsc::UnboundedSender<AppEvent>, key: RequestKey, code: &str) {
    let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
        key,
        reason: StopReason::Failed {
            code: code.to_string(),
        },
        usage: Usage::default(),
    }));
}

/// 整轮结束回投：只补一个终态（增量已随网络分块逐批投递）。
fn report_turn(sender: mpsc::UnboundedSender<AppEvent>, key: RequestKey, summary: TurnSummary) {
    tracing::debug!(
        target: "lx::agent",
        pane = key.pane.raw(),
        generation = key.generation,
        usage_input = summary.usage.input,
        usage_output = summary.usage.output,
        "agent turn finished"
    );
    let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
        key,
        reason: summary.stop_reason(),
        usage: summary.usage,
    }));
}

/// 网络事件回投处理：按 key 过滤后转 update。
pub fn handle_net_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    event: ChatNetEvent,
) -> bool {
    match event {
        ChatNetEvent::Delta { key, deltas } => {
            // 累计字节：超过间隔或阈值时入队一次 checkpoint（合并写盘）。
            let bytes: usize = deltas.iter().map(ChatDelta::payload_bytes).sum();
            let entry = streams
                .checkpoints
                .entry(key.clone())
                .or_insert_with(|| (std::time::Instant::now(), 0));
            entry.1 += bytes;
            let due = entry.1 >= CHECKPOINT_BYTES || entry.0.elapsed() >= CHECKPOINT_INTERVAL;
            if due {
                *entry = (std::time::Instant::now(), 0);
                schedule_checkpoint(state, &key);
            }
            update::on_stream_delta(state, &key, &deltas)
        }
        ChatNetEvent::Finished { key, reason, usage } => {
            // 清除在途取消句柄、checkpoint 与冻结工作区。
            streams.finish(&key);
            update::on_stream_finished(state, &key, &reason, usage)
        }
    }
}

/// 工具调用请求（BeginTool / FinishTool）的事件层处理：**唯一**写库与状态来源。
///
/// 顺序不可交换：
///
/// - `BeginTool`：① `begin_tool_call`（pending 事务提交）→ ② 状态转 `Pending` 并安排下一帧
///   → ③（可见窗格在该帧绘制后）`mark_tool_running` + 状态转 `Running` → 回执 ticket。
/// - `FinishTool`：① `finish_tool_call`（真实结果原子落库）→ ② 状态转终态 + 摘要
///   → ③ 回执 `Ok`。落库失败时回执 `Err`，本轮失败且**不重试**写入。
pub fn handle_agent_host_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    event: AgentHostEvent,
) -> bool {
    match event {
        AgentHostEvent::BeginTool { key, call, reply } => {
            begin_tool(state, streams, persistence, key, call, reply)
        }
        AgentHostEvent::FinishTool {
            key,
            ticket,
            outcome,
            reply,
        } => finish_tool(state, streams, persistence, key, &ticket, outcome, reply),
    }
}

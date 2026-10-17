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
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::chat::agent_loop::{HostError, ModelContext, ReadyToolCall, ToolOutcome, TurnSummary};
use crate::chat::context::{self, AssemblyInput};
use crate::chat::events::{ChatDelta, ChatRequest, StopReason};
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
    fn finish(&mut self, key: &RequestKey) {
        self.cancels.remove(key);
        self.checkpoints.remove(key);
        self.workspaces.remove(key);
        self.pending_begins.retain(|pending| &pending.key != key);
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
    let prepared = prepare_turn(request, workspace);
    // 取消通道；sender 由 ChatStreams 持有。
    let (cancel_sender, cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender);
    streams.workspaces.insert(key.clone(), prepared.cwd.clone());
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
            stream_one,
            cancel,
        )
        .await;
        report_turn(net_sender, stream_key, summary);
    });
    true
}

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

/// BeginTool：DB pending → UI pending → （帧后）DB running + UI running → ticket。
fn begin_tool(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    call: ReadyToolCall,
    reply: oneshot::Sender<Result<ToolCallTicket, HostError>>,
) -> bool {
    // 归属校验：旧 generation / 窗格关闭 / 会话切换一律不执行文件 IO。
    let Some(assistant_id) = active_assistant_id(state, &key) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let Some(workspace) = streams.workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    // 时间线对齐：`begin_tool_call` 按该消息内 parts/tools 的 `MAX(position)+1` 分配工具
    // 位置。先把「该工具之前」的 parts 快照提交，使 DB 分配值与 app 时间线一致，
    // 后续 checkpoint/finish 才不会撞 `PositionConflict`。
    if let Some(position) = tool_step_position(state, &key, &call.call_id) {
        let parts = parts_before(state, &key, &assistant_id, position);
        if let Err(error) = persistence.checkpoint_parts(&workspace, &assistant_id, &parts) {
            tracing::warn!(
                target: "lx::agent",
                call_id = %call.call_id,
                code = error.code(),
                "tool call timeline sync failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            return true;
        }
    }
    // ① pending 事务提交：失败即不执行文件 IO，本轮失败。
    let ticket = match persistence.begin_tool_call(
        &workspace,
        &key.session,
        &assistant_id,
        &call.call_id,
        &call.name,
        &call.arguments_json,
    ) {
        Ok(ticket) => ticket,
        Err(error) => {
            tracing::warn!(
                target: "lx::agent",
                call_id = %call.call_id,
                name = %call.name,
                code = error.code(),
                "tool call pending record failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            return true;
        }
    };
    // ② 状态转 Pending（DB 已提交）；调用方随后安排下一帧，用户先看到 loading。
    update::on_tool_pending(state, &key, &call.call_id);
    let pending = PendingBegin {
        key,
        ticket,
        reply: Some(reply),
    };
    // ③ 可见窗格：回执推迟到该帧绘制之后；不可见窗格无需等待不可见的一帧。
    if streams.pane_visible(pending.key.pane) {
        streams.pending_begins.push(pending);
        true
    } else {
        settle_begin(state, streams, persistence, pending)
    }
}

/// 绘制完成后推进 pending → running 并回执 ticket（可见窗格的 loading 门槛）。
///
/// 返回是否有状态变化（需要再画一帧显示 running）。
pub fn flush_pending_tool_begins(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
) -> bool {
    if streams.pending_begins.is_empty() {
        return false;
    }
    let pending = std::mem::take(&mut streams.pending_begins);
    let mut changed = false;
    for item in pending {
        changed |= settle_begin(state, streams, persistence, item);
    }
    changed
}

/// pending 帧已绘制：推进 running 并把 ticket 交给 loop。
fn settle_begin(
    state: &mut AppState,
    streams: &ChatStreams,
    persistence: &Persistence,
    pending: PendingBegin,
) -> bool {
    let Some(reply) = pending.reply else {
        // 回执通道已被丢弃（窗格关闭）：loop 会按取消结束，不再执行。
        return false;
    };
    let Some(workspace) = streams.workspaces.get(&pending.key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    // 帧间归属可能变化（会话切换/`/new`）：已提交的 pending 记为 cancelled，不执行。
    if active_assistant_id(state, &pending.key).is_none() {
        cancel_pending(persistence, &workspace, &pending.ticket);
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    }
    match persistence.mark_tool_running(&workspace, &pending.ticket) {
        Ok(()) => {
            let changed = update::on_tool_running(state, &pending.key, &pending.ticket.call_id);
            let _ = reply.send(Ok(pending.ticket));
            changed
        }
        Err(error) => {
            // running 入库失败：不执行文件 IO，本轮失败。
            tracing::warn!(
                target: "lx::agent",
                call_id = %pending.ticket.call_id,
                code = error.code(),
                "tool call running record failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{RUNNING_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            false
        }
    }
}

/// 已提交 pending 但不再允许执行的调用：落为 `cancelled`（恢复时不会显示 running）。
fn cancel_pending(persistence: &Persistence, workspace: &std::path::Path, ticket: &ToolCallTicket) {
    let cancelled = ToolCallResult {
        status: ToolTerminalStatus::Cancelled,
        result_json: None,
        error_text: None,
    };
    if let Err(error) = persistence.finish_tool_call(workspace, ticket, &cancelled) {
        tracing::warn!(
            target: "lx::agent",
            call_id = %ticket.call_id,
            code = error.code(),
            "tool call cancel record failed"
        );
    }
}

/// FinishTool：真实结果原子落库 → 状态终态 + 摘要 → 回执。
fn finish_tool(
    state: &mut AppState,
    streams: &ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    ticket: &ToolCallTicket,
    outcome: ToolOutcome,
    reply: oneshot::Sender<Result<(), HostError>>,
) -> bool {
    let Some(workspace) = streams.workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let result = db_result_of(&outcome);
    let started = std::time::Instant::now();
    // ① 真实结果原子落库（DB 是唯一真相）。
    match persistence.finish_tool_call(&workspace, ticket, &result) {
        Ok(()) => {
            let (status, summary, error) = view_settlement(&outcome);
            // ② 状态转终态并写摘要/错误/结果体（不伪造结果：结果体来自刚落库的记录）。
            let changed = update::on_tool_settled(
                state,
                &key,
                &ticket.call_id,
                status,
                summary,
                result.result_json.clone(),
                error,
            );
            tracing::debug!(
                target: "lx::agent",
                call_id = %ticket.call_id,
                status = status_text(status),
                duration_ms = started.elapsed().as_millis() as u64,
                "tool call settled"
            );
            let _ = reply.send(Ok(()));
            changed
        }
        Err(db_error) => {
            // ③ 落库失败：不重试写入，本轮失败；UI 明示文件状态需核查。
            tracing::warn!(
                target: "lx::agent",
                call_id = %ticket.call_id,
                code = db_error.code(),
                "tool result persistence failed"
            );
            update::show_toast(
                state,
                Toast::new(
                    ToastKind::Error,
                    text::LX_CHAT_TOOL_RESULT_SAVE_FAILED,
                    Some(key.pane),
                    std::time::Instant::now(),
                ),
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{FINISH_TOOL_FAILED_CODE} · {}", db_error.code()),
            }));
            true
        }
    }
}

/// 工具执行结局 → DB 结算记录（取消不带结果体，失败与成功都带规范化 JSON）。
fn db_result_of(outcome: &ToolOutcome) -> ToolCallResult {
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

/// 工具执行结局 → 视图状态 + 摘要 + 错误码。
fn view_settlement(
    outcome: &ToolOutcome,
) -> (
    crate::app::state::ToolStatus,
    Option<crate::app::state::ToolSummary>,
    Option<String>,
) {
    use crate::app::state::ToolStatus;
    match outcome {
        ToolOutcome::Cancelled => (ToolStatus::Cancelled, None, None),
        ToolOutcome::Executed(execution) => {
            let summary = view_summary(&crate::chat::tools::to_summary(execution));
            if execution.is_failure() {
                (ToolStatus::Failed, Some(summary), failure_text(execution))
            } else {
                (ToolStatus::Completed, Some(summary), None)
            }
        }
    }
}

/// `chat::tools::ToolSummary` → `app` 层 `ToolSummary`（一次字段映射，保持单向依赖）。
fn view_summary(summary: &crate::chat::tools::ToolSummary) -> crate::app::state::ToolSummary {
    crate::app::state::ToolSummary {
        path: summary.path.clone(),
        range: summary.range,
        added: summary.added,
        removed: summary.removed,
        matches: summary.matches,
        truncated: summary.truncated,
    }
}

/// 失败原因（给模型与 UI 的精简单行；不含文件内容）。
fn failure_text(execution: &crate::chat::tools::ToolExecutionResult) -> Option<String> {
    let crate::chat::tools::ToolExecutionResult::Failed { message, .. } = execution else {
        return None;
    };
    Some(message.chars().take(512).collect())
}

/// 稳定状态字符串（只进 tracing，不当用户文案）。
fn status_text(status: crate::app::state::ToolStatus) -> &'static str {
    use crate::app::state::ToolStatus;
    match status {
        ToolStatus::InputStreaming => "input_streaming",
        ToolStatus::Pending => "pending",
        ToolStatus::Running => "running",
        ToolStatus::Completed => "completed",
        ToolStatus::Failed => "failed",
        ToolStatus::Cancelled => "cancelled",
        ToolStatus::Interrupted => "interrupted",
    }
}

/// 该 call 步骤在 app 时间线内的 position（找不到返回 None）。
fn tool_step_position(state: &AppState, key: &RequestKey, call_id: &str) -> Option<i64> {
    let assistant_id = active_assistant_id(state, key)?;
    let pane = state.pane_anywhere(key.pane)?;
    let message = pane
        .lx
        .chat
        .messages
        .iter()
        .find(|message| message.id == assistant_id)?;
    message.tool(call_id).map(|call| call.position)
}

/// 该工具步骤**之前**的 parts 快照（保持真实 position，不重新编号）。
fn parts_before(
    state: &AppState,
    key: &RequestKey,
    assistant_id: &str,
    position: i64,
) -> Vec<crate::db::chat::ChatPart> {
    let Some(pane) = state.pane_anywhere(key.pane) else {
        return Vec::new();
    };
    let Some(message) = pane
        .lx
        .chat
        .messages
        .iter()
        .find(|message| message.id == assistant_id)
    else {
        return Vec::new();
    };
    crate::db::chat::chat_part_views(message.steps.iter().filter_map(|step| match step {
        crate::app::state::AssistantStepView::Part { position: at, part } if *at < position => {
            Some((*at, part))
        }
        _ => None,
    }))
}

/// 入队一次 checkpoint：把窗格当前内存 parts 快照交给持久化层。
fn schedule_checkpoint(state: &mut AppState, key: &RequestKey) {
    let Some(target) = state.pane_anywhere(key.pane) else {
        return;
    };
    let assistant_id = match &target.lx.chat.request {
        RequestState::Streaming {
            key: active,
            assistant_id,
        } if active == key => assistant_id.clone(),
        _ => return,
    };
    let parts = target
        .lx
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.id == assistant_id)
        .map(crate::db::chat::chat_message_part_views)
        .unwrap_or_default();
    let Some(workspace) = state.workspace_cwd_for_pane(key.pane) else {
        return;
    };
    state
        .effects
        .push(crate::app::effects::Effect::CheckpointChatTurn {
            workspace,
            assistant_id,
            parts,
        });
}

/// 本轮用户草稿：取**最近一条 user 消息**的正文拼接。
///
/// 发送时该轮 assistant 占位已追加在末尾，故不能用 `messages.last()`；
/// 必须按 role 反向查找 user 消息，否则得到空串（请求携带空 user 消息被拒）。
fn latest_user_draft(messages: &[crate::app::state::ChatMessageView]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.text())
        .unwrap_or_default()
}

/// 窗格聊天视图 → DB 消息记录（上下文装配输入）。
///
/// 委托给 `db::chat::chat_message_views_to_db`：把 parts 与 tools 同时映射成记录，
/// 使「当前轮内存视图」与「`/resume` 加载的历史」走同一条装配来源。
fn chat_part_views_to_db(
    messages: &[crate::app::state::ChatMessageView],
) -> Vec<crate::db::chat::ChatMessage> {
    crate::db::chat::chat_message_views_to_db(messages)
}

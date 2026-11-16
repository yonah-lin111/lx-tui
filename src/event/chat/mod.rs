//! lx 聊天轮次编排：取走网络效应、装配上下文、启动 agent turn 任务并回投结果。
//!
//! 统一负责“DB 落库 → 状态 → 回执”顺序；delta 与终态按 `(pane, session, generation)` 过滤。

pub mod ask;
pub mod btw;
pub mod compact;
pub mod connect;
mod context_summary;
pub(crate) mod detail_compact;
pub(crate) mod detail_rewind;
mod file_snapshots;
mod host;
pub mod mcp;
mod process_events;
mod process_page;
pub mod recap;
pub mod rewind;
mod runtime;
pub(crate) mod skills;
pub(crate) mod steer;
mod stream_start;
pub(crate) mod subagent;
mod timeline;
pub mod title;
mod tool_calls;
pub mod user_shell;
pub mod web;

use crate::chat::ModelItem;
pub use process_events::{handle_process_event, persist_terminal_snapshots};
pub use process_page::{
    apply_process_effects, refresh_open_task_pages, stop_turn_processes,
    sync_background_task_counts, tick_task_pages,
};
pub use recap::apply_recap_effects;
pub use user_shell::pump_user_shells;
// 工具握手与时间线辅助的实现在子模块；本模块保留唯一入口。
pub use timeline::DRAFT_EXCEEDS_CONTEXT_CODE;
#[cfg(test)]
pub(crate) use timeline::{assemble_turn_request, latest_user_draft_for_test};
pub(crate) use timeline::{assemble_turn_request_with_context, prepare_turn, schedule_checkpoint};
#[cfg(test)]
pub(crate) use tool_calls::view_summary;
pub(crate) use tool_calls::{begin_tool, finish_tool, flush_pending_tool_begins};

#[cfg(test)]
mod tests;

pub use compact::{apply_compact_effects, handle_compact_finished, handle_compact_reverted};
pub use host::{
    AgentHostEvent, BEGIN_TOOL_FAILED_CODE, FINISH_TOOL_FAILED_CODE, RUNNING_TOOL_FAILED_CODE,
};
pub use rewind::{RewindOutcome, apply_rewind_effects, handle_rewind_finished};
use stream_start::start_stream;

#[cfg(test)]
pub(crate) use subagent::finish_subagent;
pub(crate) use subagent::{
    apply_subagent_effects, finish_subagent_with_billing, handle_subagent_auto_compact,
    handle_subagent_net_event, handle_subagent_wait_ready,
};
pub use title::{TitleGeneratedEvent, apply_title_effects, handle_title_generated};

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot, watch};

use crate::app::effects::Effect;
use crate::app::state::{AppState, RequestKey, RequestState};
use crate::app::toast::{Toast, ToastKind};
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
use crate::tiling::PaneId;
use crate::ui::text;
use context_summary::{background_tasks, runtime_context_summary};

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
    /// 会话的 **DB 工作区键**（`/resume` 共享键，通常是仓库主 checkout），不是工具执行
    /// cwd：linked worktree 里执行时仍用共享键落库，用执行 cwd 会报 `workspace missing`。
    db_workspaces: HashMap<RequestKey, PathBuf>,
    /// 本轮 mutation 归属的当前 user message id；steer 生效时原地更新。
    mutation_users: HashMap<RequestKey, std::sync::Arc<std::sync::RwLock<String>>>,
    /// 本轮持有的跨进程 session guard；直到终态才释放。
    guards: HashMap<RequestKey, crate::platform::FileGuard>,
    /// `BeginChatTurn` 之前就取得的 guard：续轮必须在 DB 事务之前锁住会话。
    /// 以 `(pane, generation)` 暂存，`start_stream` 取走；轮次失败时由清理函数释放。
    pending_guards: HashMap<(PaneId, u64), crate::platform::FileGuard>,
    /// 已提交 pending 但**尚未**回执 ticket 的调用：等该帧绘制后推进 running。
    pending_begins: Vec<PendingBegin>,
    /// 上一帧几何中可见的窗格；不可见窗格无需等待渲染即可回执。
    visible_panes: Vec<PaneId>,
    /// 会话进程运行实例（跨轮保留；切会话/关窗格/退出时清理）。
    pub(crate) runtimes: runtime::ProcessRuntimes,
    /// 进程 UI 事件通道（由事件循环在启动时注入）。
    process_sender: Option<mpsc::UnboundedSender<ProcessUiEvent>>,
    /// 应用事件通道（子 agent 的异步等待与终态回投用）。
    event_sender: Option<mpsc::UnboundedSender<AppEvent>>,
    /// 每进程上次输出快照落库时刻（限流合并）。
    process_snapshot_at: HashMap<u64, std::time::Instant>,
    /// 在途本地命令（DB 预登记后、真实进程退出前）。
    pub(crate) user_shell: user_shell::UserShellJobs,
    /// 每轮模型请求的命令注入记忆（按 RequestKey）。
    pub(crate) user_shell_injected: HashMap<RequestKey, user_shell::ShellInjectionState>,
    /// 每轮最多一个挂起的 `ask` 提问（loop 串行）；回执通道的唯一持有者。
    pub(crate) asks: HashMap<RequestKey, PendingAsk>,
    /// 子 agent 句柄：`child_session_id -> handle`。
    pub(crate) subagents: HashMap<String, subagent::SubagentHandle>,
    /// 旁路问答（`/btw`）的独立运行状态与队列。
    pub(crate) btw: btw::BtwStreams,
    /// 已显式恢复（或刚 spawn 处于活动生命周期）的子 agent ID 集合（`agent_id`）。
    pub(crate) resumed_subagents: std::collections::HashSet<String>,
    /// 因 `send_input(interrupt=true)` 正在中断当前轮、封口后需立即消费新输入的 `agent_id` 集合。
    pub(crate) interrupted_subagents: std::collections::HashSet<String>,
    /// 因 `interrupt_agent` 正在中断当前轮、封口后保持 `idle` 且暂停自动消费队列的 `agent_id` 集合。
    pub(crate) paused_by_interrupt: std::collections::HashSet<String>,
    /// 被用户从面板主动停止、尚待显式恢复的 `agent_id`（仅内存）。
    pub(crate) user_stopped: std::collections::HashSet<String>,
    /// 下一轮是 `/report` 报告轮、完成后只填主输入框的 `agent_id`（仅内存）。
    pub(crate) report_to_input: std::collections::HashSet<String>,
    /// spawn 时冻结的模型（父会话与子会话各登记一份）。
    pub(crate) frozen: HashMap<String, crate::app::state::FrozenModel>,
    /// 显式覆写或角色绑定了专属模型的子会话（`child_session_id -> FrozenModel`）。
    pub(crate) pinned_subagent_models: HashMap<String, crate::app::state::FrozenModel>,
    /// spawn 时冻结的单轮 Token 预算覆写（`agent_id -> max_tokens`），
    /// 供后续排队/恢复轮次沿用，避免多轮累计预算退回默认值。
    pub(crate) subagent_budgets: HashMap<String, Option<u64>>,
}

/// 一次等待作答的提问：回执通道 + 归属用的 `call_id`。
pub(crate) struct PendingAsk {
    pub call_id: String,
    /// 回执通道由本结构独占；轮次结束/取消时取走并回 `Cancelled`。
    pub reply: oneshot::Sender<Result<crate::chat::tools::AskAnswers, HostError>>,
}

/// 一次待回执的 BeginTool：DB pending 已提交，UI 已转 Pending。
pub(crate) struct PendingBegin {
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

    /// 注入应用事件通道（子 agent 的异步等待与终态回投用）。
    pub fn set_event_sender(&mut self, sender: mpsc::UnboundedSender<AppEvent>) {
        self.event_sender = Some(sender);
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
    /// 只发出取消信号：已提交 pending 的调用由 loop 在下一检查点结算为 `cancelled`
    /// （**不执行**文件 IO）；挂起的 `ask` 在这里唤醒为 `Err(Cancelled)`，否则永久阻塞。
    fn cancel(&mut self, key: &RequestKey) -> bool {
        // Esc 只停主轮；子 agent 不级联取消（用户可在 Agents 面板按选中项停止）。
        let woke = self.wake_ask(key);
        let Some(cancel) = self.cancels.get(key) else {
            return woke;
        };
        // `watch::Sender::send` 在没有接收者时**不写入值**却仍返回 `Err`。早先
        // `let _ =` 吞掉它后依然 `return true`，调用方会误以为取消已生效，
        // 而 loop 其实早就不在了（任务 panic 等），界面就永久卡在 streaming。
        if cancel.send(true).is_err() {
            self.cancels.remove(key);
            return woke;
        }
        tracing::debug!(
            target: "lx::agent",
            pane = key.pane.raw(),
            session = %key.session,
            generation = key.generation,
            "agent turn cancel requested"
        );
        true
    }

    /// 取走挂起的提问并回 `Cancelled`；无挂起项返回 `false`。
    pub(crate) fn wake_ask(&mut self, key: &RequestKey) -> bool {
        let Some(pending) = self.asks.remove(key) else {
            return false;
        };
        let _ = pending.reply.send(Err(HostError::Cancelled));
        true
    }

    /// 轮次结束：清除该 key 的取消句柄、checkpoint、冻结工作区与未回执的 pending。
    ///
    /// **不清理进程**：正常结束一轮模型回答时，所属服务继续运行（下一轮复用同一实例）。
    fn finish(&mut self, key: &RequestKey) {
        // 轮次结束不应还有提问在等；有就是没人再来回执了，一律按取消唤醒。
        self.wake_ask(key);
        self.cancels.remove(key);
        self.checkpoints.remove(key);
        self.db_workspaces.remove(key);
        self.mutation_users.remove(key);
        self.guards.remove(key);
        self.user_shell_injected.remove(key);
        self.pending_begins.retain(|pending| &pending.key != key);
        // 子轮结束：句柄与冻结模型一起移除；主轮 key 不受影响（session 不同）。
        self.subagents.remove(&key.session);
        self.frozen.remove(&key.session);
        // 只丢弃该轮执行上下文；进程留给会话（正常结束一轮不清理服务）。
        self.runtimes
            .finish_turn(&key.session, &key.generation.to_string());
    }

    /// 抽取某 key 尚未回执的 BeginTool（窗格关闭 / 会话切换 / `/new` 时）。
    ///
    /// **不能直接丢掉**：`begin_tool` 已把 `agent_tool_call` 写成 `pending` 并提交，丢条目会让
    /// `reply` 被 drop、loop 拿 `Err(RecvError)` 返回，`finish_tool` 永不再调用，库里那一行
    /// **永久停在 `pending`**。所以交给调用方，由 [`settle_begin`] 走「无归属 → `cancel_pending`」
    /// 那条唯一合法的 pending→cancelled 落库路径。
    fn take_pending(&mut self, key: &RequestKey) -> Vec<PendingBegin> {
        let (taken, kept) = std::mem::take(&mut self.pending_begins)
            .into_iter()
            .partition(|pending| &pending.key == key);
        self.pending_begins = kept;
        taken
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
    /// 整轮结束（含终态原因、最后一次响应 usage 与整轮累计 billing）。
    Finished {
        key: RequestKey,
        reason: StopReason,
        /// 是否因多步累计 Token 预算耗尽而熔断（仅子 agent 会置真）。
        budget_exceeded: bool,
        /// 最后一次模型响应：上下文占用与消息统计口径。
        usage: Usage,
        /// 整轮累计：会话账单与 CLI 统计口径。
        billing: Usage,
    },
}

/// 取走网络效应并启动后台 agent turn 任务；返回是否有变化。
///
/// `sender` 把结果投回唯一事件循环；本函数只做装配、取消与 spawn。
pub fn pump_chat_streams(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
) -> bool {
    // 会话切换、`/new`、窗格关闭：在途轮次不再归属于任何可见请求 → 取消。
    let mut changed = reap_stale_streams(state, streams, persistence);
    // 旁路问答：补启动释放出的并发槽（批内并行、批间串行）。
    changed |= btw::pump(state, streams, persistence, sender);
    // 已不再被任何窗格绑定的会话运行实例：停止其全部进程、释放句柄，并把终态回投落库。
    changed |= process_page::reap_orphan_runtimes(state, streams, sender);
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            web::is_ai_web_effect(state, effect)
                || matches!(
                    effect,
                    Effect::StartChatStream { .. }
                        | Effect::CancelChatStream { .. }
                        | Effect::AnswerAsk { .. }
                        | Effect::Btw(_)
                )
        })
        .cloned()
        .collect();
    if effects.is_empty() {
        // 没有网络效应要取，但运行实例回收本身可能就是本帧唯一的变化。
        return changed;
    }
    let taken: Vec<Effect> = std::mem::take(&mut state.effects);
    state.effects = taken
        .into_iter()
        .filter(|effect| {
            !web::is_ai_web_effect(state, effect)
                && !matches!(
                    effect,
                    Effect::StartChatStream { .. }
                        | Effect::CancelChatStream { .. }
                        | Effect::AnswerAsk { .. }
                        | Effect::Btw(_)
                )
        })
        .collect();
    for effect in effects {
        match effect {
            Effect::StartChatStream { key, assistant_id } => {
                changed |= start_stream(state, streams, sender, persistence, key, assistant_id);
            }
            Effect::CancelChatStream { key } => {
                // loop 在下一个检查点停止未开始的工具与后续采样。
                changed |= streams.cancel(&key);
                changed |= ask::cancel(state, streams, &key);
                // D9：同时停止本轮新建的进程与当前正在等待/交互的进程
                // （此前启动、本轮未操作的旧服务保持运行）。
                stop_turn_processes(streams, &key);
            }
            Effect::AnswerAsk {
                key,
                call_id,
                answers,
            } => {
                changed |= ask::answer(state, streams, &key, &call_id, answers);
            }
            Effect::Btw(effect) => {
                changed |= btw::apply_effect(state, streams, persistence, sender, effect);
            }
            other => {
                changed |= web::apply_effect(state, streams, sender, other);
            }
        }
    }
    changed
}

/// 在 `BeginChatTurn` 落库之前取得/复核 session guard。
///
/// 续轮的 guard 必须早于 `begin_turn` 事务：否则另一实例可在事务提交与流启动之间插入同一
/// 会话，revision 只能事后发现。取不到或存在未完成撤销时移除效应、回到 Idle 并提示原因。
pub fn guard_begin_turns(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
) -> bool {
    let targets: Vec<(PaneId, u64, String)> = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::BeginChatTurn {
                pane,
                generation,
                session_id: Some(session_id),
                ..
            } => Some((*pane, *generation, session_id.clone())),
            _ => None,
        })
        .collect();
    let mut changed = false;
    for (pane, generation, session_id) in targets {
        if streams.pending_guards.contains_key(&(pane, generation)) {
            continue;
        }
        let mut reason = persistence
            .unfinished_rewind(&session_id)
            .ok()
            .flatten()
            .map(|operation| format!("{} ({})", text::LX_REWIND_RECOVERY_BLOCKED, operation.phase));
        if reason.is_none() {
            match crate::platform::try_lock_session(&session_id) {
                Ok(Some(guard)) => {
                    streams.pending_guards.insert((pane, generation), guard);
                    continue;
                }
                Ok(None) => reason = Some(text::LX_CHAT_SESSION_BUSY.to_string()),
                Err(error) => {
                    tracing::warn!(%error, "session guard unavailable");
                    reason = Some(text::LX_CHAT_SESSION_BUSY.to_string());
                }
            }
        }
        state.effects.retain(|effect| {
            !matches!(
                effect,
                Effect::BeginChatTurn { pane: p, generation: g, .. }
                    if *p == pane && *g == generation
            )
        });
        update::on_turn_failed(state, pane, generation);
        // lx 页的反馈落在该窗格自己的 toast 槽（不占用全局覆盖层）。
        update::show_lx_toast(
            state,
            pane,
            Toast::new(
                ToastKind::Danger,
                reason.unwrap_or_default(),
                Some(pane),
                Instant::now(),
            ),
        );
        changed = true;
    }
    changed
}

/// 释放没有对应在途请求的暂存 guard（`begin_turn` 失败、效应被移除等）。
pub fn prune_pending_guards(state: &AppState, streams: &mut ChatStreams) -> bool {
    let stale: Vec<(PaneId, u64)> = streams
        .pending_guards
        .keys()
        .copied()
        .filter(|(pane, generation)| {
            !state
                .pane_anywhere(*pane)
                .is_some_and(|target| match &target.lx.chat.request {
                    RequestState::Committing {
                        generation: active, ..
                    } => active == generation,
                    RequestState::Streaming { key, .. } => key.generation == *generation,
                    _ => false,
                })
        })
        .collect();
    for key in &stale {
        streams.pending_guards.remove(key);
    }
    !stale.is_empty()
}

/// 取消已不再属于任何在途请求的轮次（窗格删除、会话切换、generation 前进）。
fn reap_stale_streams(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
) -> bool {
    let stale: Vec<RequestKey> = streams
        .cancels
        .keys()
        // 子 agent 的 key 没有对应的主窗格请求，但它有自己的句柄与生命周期；
        // 不能被「窗格里没有这个在途 assistant」这条规则误杀。
        .filter(|key| !streams.subagents.contains_key(&key.session))
        .filter(|key| !streams.btw.runs.contains_key(*key))
        .filter(|key| active_assistant_id(state, key).is_none())
        .cloned()
        .collect();
    let mut changed = false;
    for key in stale {
        // 取消通道与挂起提问一并唤醒（`cancel` 内部会取走 `asks` 并回 Cancelled）。
        streams.cancel(&key);
        // 归属已失效：面板也必须跟着消失，否则用户会对一个已无人接收的提问作答。
        changed |= ask::clear(state, &key);
        // 已提交 pending 的调用必须落成 cancelled，否则库里永久停在 `pending`。
        for pending in streams.take_pending(&key) {
            changed |= tool_calls::settle_begin(state, streams, persistence, pending);
        }
        // 窗格关闭/会话切换：取消已不足以表达“清理进程”，这里按本轮范围停止。
        stop_turn_processes(streams, &key);
    }
    changed | ask::sweep_orphans(state, streams)
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

/// 启动前失败：直接回投整轮失败终态。
fn fail_turn(sender: &mpsc::UnboundedSender<AppEvent>, key: RequestKey, code: &str) {
    tracing::warn!(
        target: "lx::agent",
        pane = key.pane.raw(),
        session = %key.session,
        generation = key.generation,
        code,
        "agent turn failed before stream"
    );
    let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
        key,
        reason: StopReason::Failed {
            code: code.to_string(),
        },
        budget_exceeded: false,
        usage: Usage::default(),
        billing: Usage::default(),
    }));
}

/// 整轮结束回投：只补一个终态（增量已随网络分块逐批投递）。
fn report_turn(
    sender: mpsc::UnboundedSender<AppEvent>,
    key: RequestKey,
    summary: TurnSummary,
    turn_index: Option<i64>,
) {
    let reason = summary.stop_reason();
    if matches!(reason, StopReason::Failed { .. }) {
        tracing::warn!(
            target: "lx::agent",
            pane = key.pane.raw(),
            session = %key.session,
            generation = key.generation,
            turn = ?turn_index,
            reason = reason.as_str(),
            usage_input = summary.usage.input,
            usage_output = summary.usage.output,
            "agent turn failed"
        );
    } else {
        tracing::info!(
            target: "lx::agent",
            pane = key.pane.raw(),
            session = %key.session,
            generation = key.generation,
            turn = ?turn_index,
            reason = reason.as_str(),
            usage_input = summary.usage.input,
            usage_output = summary.usage.output,
            "agent turn finished"
        );
    }
    let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
        key,
        reason: summary.stop_reason(),
        budget_exceeded: false,
        usage: summary.usage,
        billing: summary.billing,
    }));
}

/// 网络事件回投处理：按 key 过滤后转 update。
pub fn handle_net_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    event: ChatNetEvent,
) -> bool {
    // 旁路问答：独立 child session；未知/已结束的 BTW 事件直接丢弃，绝不进入主对话。
    if let ChatNetEvent::Delta { key, .. } | ChatNetEvent::Finished { key, .. } = &event
        && (streams.btw.runs.contains_key(key) || streams.btw.sessions.contains(&key.session))
    {
        return btw::handle_net_event(state, streams, persistence, event);
    }
    // 子 agent 增量走独立分流：绝不进入主对话。
    if let ChatNetEvent::Delta { key, .. } | ChatNetEvent::Finished { key, .. } = &event
        && streams.subagents.contains_key(&key.session)
    {
        return handle_subagent_net_event(state, streams, event);
    }
    match event {
        ChatNetEvent::Delta { key, deltas } => {
            // AI Web 是纯内存会话：不写 checkpoint，也不触发任何 DB 快照调度。
            if !web::is_ai_web_session(&key.session) {
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
            }
            update::on_stream_delta(state, &key, &deltas)
        }
        ChatNetEvent::Finished {
            key,
            reason,
            budget_exceeded: _,
            usage,
            billing,
        } => {
            // 登录 / 驱动错误在 lx 窗格内立即可见（AI Web 没有 DB 历史可回溯）。
            web::notify_failure(state, &key, &reason);
            // 清除在途取消句柄、checkpoint 与冻结工作区。
            streams.finish(&key);
            // 防御 sweep：流程走完时不应还剩面板；留下就是悬挂 sender。
            let cleared = ask::cancel(state, streams, &key);
            let settled = update::on_stream_finished(state, &key, &reason, usage, billing);
            settled | cleared
        }
    }
}

/// 工具调用请求（BeginTool / FinishTool）的事件层处理：**唯一**写库与状态来源。
///
/// 顺序不可交换：`BeginTool` 先提交 pending 事务、再转 `Pending`、（可见窗格待该帧绘制后）
/// 转 `Running` 并回执 ticket；`FinishTool` 先原子写真实结果、再转终态、最后回执 `Ok`。
/// 落库失败时回执 `Err`，本轮失败且**不重试**写入。
pub fn handle_agent_host_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    event: AgentHostEvent,
) -> bool {
    // 旁路问答：只读握手，结算与状态更新都走独立路径。
    let btw_key = match &event {
        AgentHostEvent::BeginTool { key, .. }
        | AgentHostEvent::FinishTool { key, .. }
        | AgentHostEvent::AskUser { key, .. }
        | AgentHostEvent::UserShellUpdates { key, .. }
        | AgentHostEvent::SteerInputs { key, .. }
        | AgentHostEvent::HasPendingSteers { key, .. }
        | AgentHostEvent::Subagent { key, .. } => Some(key.clone()),
        AgentHostEvent::Mutation(_) => None,
    };
    if let Some(key) = btw_key
        && (streams.btw.runs.contains_key(&key) || streams.btw.sessions.contains(&key.session))
    {
        return match event {
            AgentHostEvent::BeginTool { key, call, reply } => {
                btw::begin_tool(state, streams, persistence, key, call, reply)
            }
            AgentHostEvent::FinishTool {
                key,
                ticket,
                outcome,
                reply,
            } => btw::finish_tool(state, streams, persistence, key, &ticket, outcome, reply),
            other => btw::reject_host_request(other),
        };
    }
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
        AgentHostEvent::Mutation(event) => file_snapshots::handle_mutation(persistence, event),
        AgentHostEvent::AskUser {
            key,
            call_id,
            questions,
            reply,
        } => ask::request(state, streams, key, call_id, questions, reply),
        AgentHostEvent::UserShellUpdates { key, reply } => {
            // 只读握手：命令结果仍以数据库为事实来源。
            let updates = user_shell::poll_updates(state, streams, persistence, &key);
            let _ = reply.send(updates);
            false
        }
        AgentHostEvent::SteerInputs { key, reply } => {
            steer::deliver(state, streams, persistence, key, reply)
        }
        AgentHostEvent::HasPendingSteers { key, reply } => {
            let _ = reply.send(steer::has_pending(state, &key));
            false
        }
        AgentHostEvent::Subagent {
            key,
            call_id,
            op,
            reply,
        } => subagent::handle_subagent_op_with_call(
            state,
            streams,
            persistence,
            key,
            call_id.as_deref(),
            op,
            reply,
        ),
    }
}

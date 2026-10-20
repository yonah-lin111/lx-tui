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

pub use process_events::{handle_process_event, persist_terminal_snapshots};
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
    // 已不再被任何窗格绑定的会话运行实例：停止其全部进程、释放句柄，并把终态回投落库。
    let reaped = reap_orphan_runtimes(state, streams, sender);
    let mut changed = reaped;
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
        // 没有网络效应要取，但运行实例回收本身可能就是本帧唯一的变化。
        return changed;
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

/// 执行进程类效应（后台任务页的列表与停止）。
///
/// 与 `pump_chat_streams` 同一处置方式：从 `state.effects` 取出自己关心的效应执行，
/// 其余的留给持久化泵/网络泵。进程句柄只在事件层的运行实例里，所以这两件事只能
/// 在这里做——`app/` 与 `db/` 都碰不到进程。
///
/// 返回是否产生状态变化。
pub fn apply_process_effects(state: &mut AppState, streams: &mut ChatStreams) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::ListSessionProcesses { .. } | Effect::StopSessionProcess { .. }
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
            Effect::ListSessionProcesses { .. } | Effect::StopSessionProcess { .. }
        )
    });
    let mut changed = false;
    for effect in effects {
        match effect {
            Effect::ListSessionProcesses { pane } => {
                changed |= list_session_processes(state, streams, pane);
            }
            Effect::StopSessionProcess {
                pane,
                process_session_id,
            } => {
                stop_session_process(state, streams, pane, process_session_id);
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// 把每个 lx 窗格的会话后台任务数同步到底部状态行指标。
///
/// 每轮循环都算（`running_count` 只是一次加锁 + 计数），这样无论用户是否打开
/// 后台任务页，都能随时看到"还有几个服务在跑"——这正是此前"忘了清理"的根因：
/// 进程一直活着，界面上却没有任何痕迹。
///
/// 返回是否有窗格的计数发生变化（用于置脏重绘）。
pub fn sync_background_task_counts(state: &mut AppState, streams: &ChatStreams) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_anywhere(pane) else {
            continue;
        };
        let count = target
            .lx
            .chat
            .session_id
            .as_deref()
            .and_then(|session_id| streams.runtimes.get(session_id))
            .map(|runtime| runtime.manager().running_count())
            .unwrap_or(0);
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        if target.lx.background_tasks != count {
            target.lx.background_tasks = count;
            changed = true;
        }
    }
    changed
}

/// 面板打开时每轮重新拉取一次进程列表。
///
/// `ListSessionProcesses` 原来只在 `open_task_page` 时发一次——面板**打开之后**
/// 才启动的进程永远进不了列表（用户报的"启动任务后没有在面板中显示"）。
///
/// 改成每轮嗅探：只要某窗格的面板开着且绑定了会话，就发一次列表请求。列表回投
/// 是幂等的（`set_processes` 按内容比较），无变化不会置脏，因此不会引起重绘风暴。
///
/// 返回是否新压入了请求（供调用方判断要不要多跑一轮效应泵）。
pub fn refresh_open_task_pages(state: &mut AppState) -> bool {
    let mut requested = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_anywhere(pane) else {
            continue;
        };
        if target.lx.task.is_none() || target.lx.chat.session_id.is_none() {
            continue;
        }
        state
            .effects
            .push(crate::app::effects::Effect::ListSessionProcesses { pane });
        requested = true;
    }
    requested
}

/// 后台任务面板的运行时长节拍：把各面板的 `now_ms` 推到当前时刻。
///
/// 面板每行右侧显示 `running 1m 30s`，而它是 `now_ms - started_at_ms` 现算的。
/// `now_ms` 原来只在**列表回投**时更新——进程不变就再也不会变，时长会冻住
/// （用户报的"时间不更新"）。
///
/// 每轮循环调一次（一次 `now_ms()` + 几个比较，成本可忽略）；只在面板打开时
/// 才追高 `now_ms`，返回值用于置脏重绘。按秒对齐，避免每帧都重绘。
pub fn tick_task_pages(state: &mut AppState) -> bool {
    let now = crate::db::now_ms();
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(page) = target.lx.task.as_mut() else {
            continue;
        };
        // 同一秒内不重绘：时长显示精度是秒。
        if now.saturating_sub(page.now_ms) >= 1_000 {
            page.now_ms = now;
            changed = true;
        }
    }
    changed
}

/// 列出某窗格所绑会话仍在运行的进程，回投给后台任务页的进程 tab。
///
/// 两条准入条件（任一成立即列出）：
///
/// 1. 进程本身还没到终态（`starting`/`running`）；
/// 2. **shell 已退出，但它拉起的后台作业还活着**（`background_alive`）。
///
/// 第 2 条是必须的：`nohup bash task.sh &` 之后 shell 立刻退出，用户的服务却还在
/// 跑。只看 `state` 会把它过滤掉，面板显示「no running processes」而任务明明活着
/// ——这正是用户报的情形（进程组仍存活，组长 shell 已退出）。
///
/// 已彻底结束（终态且组内无成员）的进程不列：没有可停的东西，列出来只会让用户
/// 以为还有服务在跑。窗格没有绑定会话（空白草稿）时回投空列表。
///
/// 会话切换后页面可能还开着：此时回投的是新会话的进程，页面据此整体翻新。
fn list_session_processes(state: &mut AppState, streams: &ChatStreams, pane: PaneId) -> bool {
    let Some(session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return update::on_process_listed(state, pane, Vec::new());
    };
    let snapshots: Vec<crate::chat::process::ProcessSnapshot> = streams
        .runtimes
        .get(&session_id)
        .map(|runtime| runtime.manager().snapshots())
        .unwrap_or_default()
        .into_iter()
        .filter(|snapshot| !snapshot.state.is_terminal() || snapshot.background_alive)
        .collect();
    // 高亮任务的输出尾部：与列表同一次回投，保证"上面选谁、下面看谁"永远同步。
    // `app/` 不持有进程句柄，所以输出也必须由这里切好行交过去。
    let active = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| page.selected().map(|process| process.session_id));
    let output = active
        .and_then(|session| {
            snapshots
                .iter()
                .find(|snapshot| snapshot.session_id == session)
        })
        .map(|snapshot| process_output_lines(&snapshot.tail))
        .unwrap_or_default();
    let entries = snapshots
        .into_iter()
        .map(|snapshot| crate::app::state::ProcessEntry {
            session_id: snapshot.session_id,
            command: snapshot.command,
            cwd: snapshot.cwd,
            tty: snapshot.tty,
            started_at_ms: snapshot.started_at_ms,
        })
        .collect();
    let listed = update::on_process_listed(state, pane, entries);
    let shown = update::on_task_output(state, pane, output);
    listed || shown
}

/// 进程输出尾部 → 展示行（保留中间空行；末尾换行不额外占行）。
///
/// 面板输出区**不做软换行**（终端日志按行读，换行会让行数与滚动错位），
/// 超宽由渲染层硬切，因此这里只需要按 `\n` 切分。
fn process_output_lines(tail: &str) -> Vec<String> {
    if tail.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = tail
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// 停止后台任务页选中的进程，并给出结果反馈。
///
/// 停止是异步的（真实退出码只能由 `wait` 得到）；这里不阻塞事件循环。
fn stop_session_process(
    state: &mut AppState,
    streams: &ChatStreams,
    pane: PaneId,
    process_session_id: u64,
) {
    let session_id = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone());
    let manager = session_id
        .as_deref()
        .and_then(|session_id| streams.runtimes.get(session_id))
        .map(|runtime| runtime.manager().clone());
    let Some(manager) = manager else {
        // 会话运行实例已经不在（刚切会话/刚退出）：页面上还挂着这一行，
        // 不能静默失败，否则用户会以为按了没生效。
        //
        // 用 toast 而不是输入框上方的 hint：这一页已经把输入框换成了控制面板，
        // hint 的锚点是输入框，面板打开时会飘在错误的位置。
        update::show_toast(
            state,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Error,
                crate::ui::text::LX_TASK_STOP_FAILED,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return;
    };
    tokio::spawn(async move {
        match manager.stop(process_session_id).await {
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(
                    target: "lx::process",
                    process_session_id,
                    code = error.code,
                    "stopping a background process failed"
                );
            }
        }
    });
    // 停止是异步的，但"按了就要有反馈"。
    //
    // 这里只确认"已发出停止请求"；真正的结果由进程终态事件驱动列表刷新。
    // 用 Info 而不是 Error：这是用户主动要求的动作，不是出错。
    update::show_toast(
        state,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Info,
            crate::ui::text::LX_TASK_STOPPED,
            Some(pane),
            std::time::Instant::now(),
        ),
    );
}

/// 回收不再被任何窗格绑定的会话运行实例（`/new`、切换会话、关窗格）。
///
/// 只移除运行实例并**异步停止**其进程：事件循环不阻塞等待，回执由后台任务完成。
///
/// 停止完成后把可获取的终态快照经 [`AppEvent::ProcessesReaped`] 回投事件层落库。
/// 设计 §8 要求「关闭旧运行实例，清理其进程**并记录终态**」——只杀不记，库里会
/// 永远留着 `running` 行：重开该会话时卡片按记录判活显示 Running，而后台任务面板
/// 读的是内存运行实例（已随会话回收而空），两处对同一件事给出相反答案。
///
/// 返回是否回收了运行实例（后台任务计数随之变化，调用方需要重绘）。
fn reap_orphan_runtimes(
    state: &AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
) -> bool {
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
    let mut reaped = false;
    for session in orphans {
        if let Some(runtime) = streams.runtimes.take(&session) {
            reaped = true;
            let manager = runtime.manager.clone();
            let workspace = manager.owner().workspace.clone();
            let epoch = runtime.epoch.clone();
            let sender = sender.clone();
            tokio::spawn(async move {
                manager.shutdown().await;
                // 停止之后才读快照：此时才是真正的终态（Stopped / Interrupted）。
                let snapshots = manager.snapshots();
                if snapshots.is_empty() {
                    return;
                }
                let _ = sender.send(AppEvent::ProcessesReaped {
                    workspace,
                    epoch,
                    snapshots,
                });
            });
        }
    }
    reaped
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

//! 进程事件的判定与落库：UI 与 DB 两条出口共用一次读取。
//!
//! 顺序（对应 design §5 / §9.2）：
//!
//! ```text
//! ProcessUiEvent::Output -> 读快照 -> UI 更新原命令卡片 -> （限流）合并输出快照落库
//! ProcessUiEvent::Exited -> 读快照 -> UI 终态       -> 终态立即落库（不等待周期）
//! ```
//!
//! 输出快照落库**每进程最多每秒一次**；终态不等待该周期。后台输出只更新数据，
//! 绝不触发新的模型请求。

use std::collections::HashMap;
use std::path::Path;

use crate::app::state::AppState;
use crate::app::update;
use crate::chat::process::{ProcessState, ProcessUiEvent};
use crate::db::chat::{NewProcess, OutputSnapshot};
use crate::db::service::Persistence;

use super::ChatStreams;

/// 单个进程输出落库的最小间隔。
const PROCESS_SNAPSHOT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// 处理一条进程 UI 事件：先更新 UI，再按需落库。
///
/// 返回是否有状态变化（供事件循环安排重绘）。
pub fn handle_process_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    event: ProcessUiEvent,
) -> bool {
    let (session_id, origin_call_id) = match &event {
        ProcessUiEvent::Output {
            session_id,
            origin_call_id,
        } => (*session_id, origin_call_id.clone()),
        ProcessUiEvent::Exited {
            session_id,
            origin_call_id,
            ..
        } => (*session_id, origin_call_id.clone()),
    };
    let Some(runtime) = streams.runtime_by_session_id(session_id) else {
        // 旧代次/已清理会话的迟到事件：不改状态、不复活进程。
        return false;
    };
    // **先**复位合并标志、**再**读快照：反过来会丢掉「快照读完之后、标志复位之前」
    // 到达的那一批输出（`push_output` 看到 `ui_pending` 为真就不发事件，紧接着标志
    // 被清掉，那批输出再也没有事件通知 UI，卡片预览永久停在前一段）。
    // 复位后到达的输出最多多触发一次空转，代价可以忽略。
    runtime.manager.acknowledge_output(session_id);
    let Some(snapshot) = runtime.manager.snapshot(session_id) else {
        return false;
    };
    let workspace = runtime.manager.owner().workspace.clone();
    let epoch = runtime.epoch.clone();
    let terminal = matches!(event, ProcessUiEvent::Exited { .. });
    // 终态同时收敛 `write_stdin` 卡片：它们记的是「当次调用时进程在跑」，
    // 进程结束后必须改成终态，否则 `Send input` 会一直显示 `Running`。
    let terminal_state = terminal.then(|| snapshot.state.clone());
    let changed = match &event {
        ProcessUiEvent::Output { .. } => {
            update::apply_process_output(state, session_id, &origin_call_id, snapshot.clone())
        }
        ProcessUiEvent::Exited { .. } => {
            let applied = update::apply_process_exit(state, &event, Some(snapshot.clone()));
            // 终态事件可能没改到工具卡片（该卡片已不在屏幕上），
            // 但仍需刷新打开着的后台任务页——否则用户会看到已停的进程还挂在列表里。
            applied | refresh_task_pages(state, session_id)
        }
    };
    let changed = changed
        | terminal_state
            .as_ref()
            .is_some_and(|terminal| update::apply_interaction_exit(state, session_id, terminal));
    // 落库：终态立即写，运行态按间隔合并（每进程最多一个待合并快照）。
    if terminal || streams.process_snapshot_due(session_id, PROCESS_SNAPSHOT_INTERVAL) {
        persist_snapshot(
            persistence,
            &workspace,
            &epoch,
            session_id,
            &snapshot,
            terminal,
        );
    }
    changed
}

/// 登记新建进程并写入首帧快照（由事件层在工具结算成功后调用）。
///
/// 登记失败只记日志：进程仍在运行，但历史恢复时看不到它（不阻断本轮工具结果落库）。
pub fn register_process(
    persistence: &Persistence,
    workspace: &Path,
    epoch: &str,
    chat_session_id: &str,
    origin_call_id: &str,
    snapshot: &crate::chat::process::ProcessSnapshot,
) {
    let new = NewProcess {
        chat_session_id,
        origin_call_id,
        runtime_epoch: epoch,
        process_session_id: snapshot.session_id,
        command: &snapshot.command,
        cwd: Path::new(&snapshot.cwd),
        tty: snapshot.tty,
    };
    if let Err(error) = persistence.begin_process(workspace, &new) {
        tracing::warn!(
            target: "lx::agent",
            session_id = snapshot.session_id,
            code = error.code(),
            "process record insert failed"
        );
        return;
    }
    // 首帧快照**不是**退出事件：写 `terminal = true` 会无条件走 `finish_process`，
    // 于是刚启动的进程就被盖上 `ended_at_ms`——库里出现 "state=running 但已结束"
    // 的自相矛盾行。是否终态交给 `persist_snapshot` 按快照状态自己判断。
    persist_snapshot(
        persistence,
        workspace,
        epoch,
        snapshot.session_id,
        snapshot,
        false,
    );
}

/// 把一批**已停止**的进程快照写入终态（会话关闭 / 退出 lx 时清理后的可获取终态）。
///
/// 设计 §8 要求「清理其进程并记录终态」：只杀不记，库里会留下永远 `running` 的行，
/// 重开该会话后卡片显示 Running，而后台任务面板（读内存运行实例）却是空的——
/// 两处对同一件事给出相反答案。非终态快照一律跳过，绝不凭空给活进程盖终态。
pub fn persist_terminal_snapshots(
    persistence: &Persistence,
    workspace: &Path,
    epoch: &str,
    snapshots: &[crate::chat::process::ProcessSnapshot],
) {
    for snapshot in snapshots {
        if !snapshot.state.is_terminal() {
            continue;
        }
        persist_snapshot(
            persistence,
            workspace,
            epoch,
            snapshot.session_id,
            snapshot,
            true,
        );
    }
}

/// 写入一次输出/终态快照（revision 自增，较旧写入零行）。
fn persist_snapshot(
    persistence: &Persistence,
    workspace: &Path,
    epoch: &str,
    session_id: u64,
    snapshot: &crate::chat::process::ProcessSnapshot,
    terminal: bool,
) {
    let revision = next_revision(session_id);
    let output = snapshot.tail.as_bytes();
    // 写**真实**原始字节偏移，不是 `tail.len()`：`tail` 已剥离控制序列并做过
    // lossy 转码，长度与原始流无关；拿文本长度当偏移会让这两列说谎。
    let snapshot_ref = OutputSnapshot {
        output_tail: output,
        output_start: i64::try_from(snapshot.output_start).unwrap_or(i64::MAX),
        output_end: i64::try_from(snapshot.output_end).unwrap_or(i64::MAX),
        dropped_bytes: i64::try_from(snapshot.dropped_bytes).unwrap_or(i64::MAX),
    };
    let result = if terminal || snapshot.state.is_terminal() {
        let finish = crate::db::chat::ProcessFinish {
            runtime_epoch: epoch,
            process_session_id: session_id,
            state: snapshot.state.as_str(),
            exit_code: snapshot.exit_code,
            stop_reason: stop_reason_of(&snapshot.state),
            snapshot: snapshot_ref.clone(),
            revision,
        };
        persistence.finish_process(workspace, &finish)
    } else {
        persistence.update_process_output(workspace, epoch, session_id, &snapshot_ref, revision)
    };
    if let Err(error) = result {
        tracing::warn!(
            target: "lx::agent",
            session_id,
            code = error.code(),
            "process snapshot write failed"
        );
    }
}

/// 停止原因（只对显式停止/超时写；其余保持 NULL）。
fn stop_reason_of(state: &ProcessState) -> Option<&'static str> {
    match state {
        ProcessState::Stopped => Some("stopped"),
        ProcessState::TimedOut => Some("timed_out"),
        ProcessState::Interrupted => Some("interrupted"),
        _ => None,
    }
}

/// 每进程单调修订号（进程内全局，跨运行实例共享；revision 只增不减）。
fn next_revision(session_id: u64) -> i64 {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicI64, Ordering};
    static COUNTER: AtomicI64 = AtomicI64::new(0);
    static PER_PROCESS: Mutex<Option<HashMap<u64, i64>>> = Mutex::new(None);
    let mut guard = match PER_PROCESS.lock() {
        Ok(guard) => guard,
        Err(poison) => poison.into_inner(),
    };
    let map = guard.get_or_insert_with(HashMap::new);
    let entry = map.entry(session_id).or_insert(0);
    *entry = entry.saturating_add(1);
    COUNTER.fetch_add(1, Ordering::Relaxed);
    *entry
}

/// 进程终态后把该进程从打开着的后台任务页里摘掉。
///
/// 这是"停止后刷新列表"的**唯一可靠时机**：停止是异步的，按了 `^d` 之后立刻
/// 重读会读到停止前的快照（进程还没真正退出），列表看起来像"停不掉"。
///
/// 只处理列表里含该进程的窗格；不重建页面、不重置 tab，高亮由 `clamp` 收尾。
fn refresh_task_pages(state: &mut AppState, session_id: u64) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(page) = target.lx.task.as_mut() else {
            continue;
        };
        changed |= page.retain_process(session_id);
    }
    changed
}

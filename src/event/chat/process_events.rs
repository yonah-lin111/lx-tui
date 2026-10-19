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
    let Some(snapshot) = runtime.manager.snapshot(session_id) else {
        return false;
    };
    let workspace = runtime.manager.owner().workspace.clone();
    let epoch = runtime.epoch.clone();
    let manager = runtime.manager.clone();
    let terminal = matches!(event, ProcessUiEvent::Exited { .. });
    let changed = match &event {
        ProcessUiEvent::Output { .. } => {
            update::apply_process_output(state, session_id, &origin_call_id, snapshot.clone())
        }
        ProcessUiEvent::Exited { .. } => {
            update::apply_process_exit(state, &event, Some(snapshot.clone()))
        }
    };
    // UI 已取走本次输出通知：允许下一次输出重新通知。
    manager.acknowledge_output(session_id);
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
    persist_snapshot(
        persistence,
        workspace,
        epoch,
        snapshot.session_id,
        snapshot,
        true,
    );
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
    let snapshot_ref = OutputSnapshot {
        output_tail: output,
        output_start: 0,
        output_end: snapshot.tail.len() as i64,
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

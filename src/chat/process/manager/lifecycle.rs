//! 停止请求、工作目录解析与停止信号轮询。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::oneshot;

use super::super::ProcessState;
use super::types::{ProcessError, STOP_GRACE};
use super::{ProcessHandle, lock};

/// 停止原因（决定终态标记）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopReason {
    /// 用户/模型显式停止。
    Stopped,
    /// 命令寿命超时。
    TimedOut,
}

/// 停止并 reap：温和停止 → 有界等待 → 未确认则保留 Interrupted（绝不伪报 Stopped）。
///
/// 这是自由函数（而非方法）的原因：命令寿命超时任务需要在不持有 `&self` 的情况下
/// 停止进程（spawn 出去的 `'static` 任务不能借用管理器）。
pub(super) async fn stop_handle(handle: &Arc<ProcessHandle>, reason: StopReason) {
    if handle.state().is_terminal() {
        // 组长（shell）已经退了，但它拉起的后台作业可能还在进程组里跑
        // （`nohup cmd &` 的典型结果）。此时没有 supervisor 可通知，
        // 也没有可等的子进程，只能按进程组直接终止——否则面板里的这一行
        // 永远停不掉，用户会以为按钮没生效。
        // pgid 可能已被内核回收并分配给别的进程组。组长仍存活时用启动时刻
        // 指纹识破复用；组长已退出的 `nohup … &` 场景无法证明，仍是 best-effort
        // （详见 `docs/feature/lx-agent-command-plan-tools/review.md`）。
        // 指纹**惰性**取：不能放在 `set_pid` 的 spawn 热路径上（macOS 的 `ps`
        // 调用会拖慢「spawn → 等待」，让读端 drain 落后于结算）。
        if handle.has_living_group()
            && handle.leader_pid_not_recycled()
            && let Some(pid) = *lock(&handle.pid)
        {
            crate::platform::terminate_process_group(pid, true);
        }
        return;
    }
    let sender = {
        let mut guard = handle.stop.lock().await;
        guard.take()
    };
    match sender {
        Some(sender) => {
            handle.stop_requested.store(true, Ordering::Relaxed);
            let _ = sender.send(reason);
        }
        // `None` 有两种语义：已被别的调用者认领（应当一起等 supervisor 落终态），
        // 或压根没有 supervisor（才该自己标 Interrupted）。区分开才不会把正常的
        // `Stopped` / `TimedOut` 覆盖成 `Interrupted`。
        None if handle.stop_requested.load(Ordering::Relaxed) => {}
        None => {
            // 没有 supervisor（理论上不会发生）：退回直接标记，绝不伪报已停止。
            handle.settle(ProcessState::Interrupted);
            return;
        }
    }
    // 等待 supervisor 落终态（有界），期间不阻塞事件循环。
    let deadline = tokio::time::Instant::now() + STOP_GRACE * 2;
    while !handle.state().is_terminal() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if !handle.state().is_terminal() {
        // 清理未确认：保留 Interrupted 并记录；不伪报 Stopped。
        handle.settle(ProcessState::Interrupted);
        tracing::warn!(
            target: "lx::agent",
            session_id = handle.session_id,
            "process did not confirm termination"
        );
    }
}

/// 解析工作目录：相对路径基于冻结工作区，不修改进程全局 cwd。
pub(super) fn resolve_cwd(
    workspace: &Path,
    workdir: Option<&str>,
) -> Result<PathBuf, ProcessError> {
    let Some(workdir) = workdir else {
        return Ok(workspace.to_path_buf());
    };
    let path = Path::new(workdir);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    if !resolved.is_dir() {
        return Err(ProcessError::new(
            "invalid_arguments",
            format!("workdir is not a directory: {}", resolved.display()),
        ));
    }
    Ok(resolved)
}

/// 短时停止请求轮询器（PTY 等待线程无法 select）。
pub(super) struct StopWaiter<'a> {
    receiver: &'a mut oneshot::Receiver<StopReason>,
}

impl<'a> StopWaiter<'a> {
    pub(super) fn new(receiver: &'a mut oneshot::Receiver<StopReason>) -> Self {
        Self { receiver }
    }

    pub(super) fn poll(&mut self) -> Option<StopReason> {
        match self.receiver.try_recv() {
            Ok(reason) => Some(reason),
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => None,
        }
    }
}

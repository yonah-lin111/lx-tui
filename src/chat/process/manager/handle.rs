//! 受管进程的共享状态句柄（`ProcessHandle`）与进程终止流程。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tokio::sync::{Mutex, Notify, mpsc, oneshot};

use super::super::ProcessState;
use super::super::output::{MAX_MODEL_OUTPUT_BYTES, OutputBuffer, OutputRead, OutputSource};
use super::{GLOBAL_RUNNING, ProcessSnapshot, ProcessUiEvent, STOP_GRACE, lock};

/// 停止原因（决定终态标记）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopReason {
    /// 用户/模型显式停止。
    Stopped,
    /// 命令寿命超时。
    TimedOut,
}

/// 一次 stdin 写入请求。
pub(super) struct StdinRequest {
    pub(super) bytes: Vec<u8>,
    pub(super) reply: oneshot::Sender<Result<(), String>>,
}

/// 一个受管进程的共享状态。
pub(super) struct ProcessHandle {
    pub(super) session_id: u64,
    pub(super) origin_call_id: String,
    pub(super) command: String,
    pub(super) cwd: String,
    pub(super) tty: bool,
    pub(super) started_at_ms: i64,
    /// lx 直接 spawn 的 shell 的 pid（也是进程组 id，`process_group(0)` 建立）。
    ///
    /// spawn 之后才知道，所以是 `Mutex<Option<u32>>`；PTY 分支没有独立进程组。
    /// 用途：shell 退出后判断它拉起的后台作业是否还活着。
    pub(super) pid: std::sync::Mutex<Option<u32>>,
    /// 该 pid 的启动时刻（身份指纹，见 [`crate::platform::process_start_time`]）。
    ///
    /// pid 会被回收再分配：只凭 pid 对「已退出的 shell 的进程组」发 SIGKILL 有
    /// 误杀无关进程的风险。杀之前比对指纹，对不上就放弃。
    pid_start_time: std::sync::Mutex<Option<u64>>,
    pub(super) output: std::sync::Mutex<OutputBuffer>,
    state: std::sync::Mutex<ProcessState>,
    pub(super) ended_at_ms: std::sync::Mutex<Option<i64>>,
    /// 模型读取游标（绝对字节偏移）。
    pub(super) cursor: AtomicU64,
    /// 串行化「读区间 + 推进游标」：否则并发读会重复输出或把游标写回较小值。
    ///
    /// 不能复用 `output` 锁：`wait_window` 已在另一处持 `output` 锁时调用其它方法，
    /// 反向加锁会死锁。
    cursor_lock: std::sync::Mutex<()>,
    /// 该 handle 是否已计入 [`GLOBAL_RUNNING`]。
    ///
    /// `settle()` 只在「确实加过」时递减：spawn 失败路径会 settle 一个从未 `fetch_add`
    /// 的 handle，若无条件递减会让 `u64` 下溢成 `u64::MAX`，此后所有 `exec_command`
    /// 都返回 `resource_limit`（整个进程生命周期内不可恢复）。
    counted: AtomicBool,
    /// 停止请求是否已被某个调用者认领。
    ///
    /// `stop` 是 `Option<Sender>`，`take()` 之后第二个调用者看到的 `None` 既可能是
    /// 「已被别人认领」也可能是「压根没有 supervisor」——两者语义完全不同：
    /// 前者应当等待 supervisor 落终态，后者才该自己标 `Interrupted`。
    pub(super) stop_requested: AtomicBool,
    /// 停止信号（被 supervisor 监听）。
    pub(super) stop: Mutex<Option<oneshot::Sender<StopReason>>>,
    /// stdin 写入通道（写入任务拥有真实句柄）。
    pub(super) stdin_slot: std::sync::Mutex<Option<mpsc::UnboundedSender<StdinRequest>>>,
    /// 输出/退出通知。
    pub(super) notify: Notify,
    /// UI 事件通道（限流：每个进程最多一个在途输出通知）。
    ui: Option<mpsc::UnboundedSender<ProcessUiEvent>>,
    /// 是否已发出「有输出」通知（合并后续输出）。
    ui_pending: AtomicBool,
}

impl ProcessHandle {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        session_id: u64,
        origin_call_id: String,
        command: String,
        cwd: String,
        tty: bool,
        started_at_ms: i64,
        ui: Option<mpsc::UnboundedSender<ProcessUiEvent>>,
    ) -> Self {
        Self {
            session_id,
            origin_call_id,
            command,
            cwd,
            tty,
            started_at_ms,
            output: std::sync::Mutex::new(OutputBuffer::default()),
            state: std::sync::Mutex::new(ProcessState::Starting),
            ended_at_ms: std::sync::Mutex::new(None),
            pid: std::sync::Mutex::new(None),
            pid_start_time: std::sync::Mutex::new(None),
            cursor: AtomicU64::new(0),
            cursor_lock: std::sync::Mutex::new(()),
            counted: AtomicBool::new(false),
            stop_requested: AtomicBool::new(false),
            stdin_slot: std::sync::Mutex::new(None),
            stop: Mutex::new(None),
            notify: Notify::new(),
            ui,
            ui_pending: AtomicBool::new(false),
        }
    }

    pub(super) fn state(&self) -> ProcessState {
        lock(&self.state).clone()
    }

    /// 记录 spawn 出来的 shell pid（也是进程组 id）。
    pub(super) fn set_pid(&self, pid: u32) {
        *lock(&self.pid) = Some(pid);
        // 指纹**惰性**取：`set_pid` 在 spawn 的热路径上，同步调用 `ps`（macOS）
        // 会拖慢「spawn → 进入等待」这一段，实测会让读端 drain 落后于结算。
    }

    /// 组长 pid 是否**没有**被内核复用（`true` = 可以安全按 pgid 杀）。
    pub(super) fn leader_pid_not_recycled(&self) -> bool {
        let Some(pid) = *lock(&self.pid) else {
            return false;
        };
        let current = crate::platform::process_start_time(pid);
        let mut recorded = lock(&self.pid_start_time);
        match (current, *recorded) {
            (Some(now), Some(saved)) => now == saved,
            (Some(now), None) => {
                *recorded = Some(now);
                true
            }
            (None, _) => true,
        }
    }

    /// shell 已退出的进程，其进程组里是否还有存活的后台作业。
    pub(super) fn has_living_group(&self) -> bool {
        if !self.state().is_terminal() {
            return false;
        }
        lock(&self.pid)
            .map(crate::platform::process_group_has_members)
            .unwrap_or(false)
    }

    /// 写入输出并通知等待者与 UI（UI 通知按进程合并）。
    pub(super) fn push_output(&self, source: OutputSource, bytes: &[u8]) {
        {
            let mut output = lock(&self.output);
            output.push(source, bytes);
        }
        self.notify.notify_waiters();
        if let Some(ui) = &self.ui
            && !self.ui_pending.swap(true, Ordering::Relaxed)
        {
            let _ = ui.send(ProcessUiEvent::Output {
                session_id: self.session_id,
                origin_call_id: self.origin_call_id.clone(),
            });
        }
    }

    /// UI 已取走本次输出通知：复位合并标志。
    pub(super) fn clear_ui_pending(&self) {
        self.ui_pending.store(false, Ordering::Relaxed);
    }

    /// 标记终态（幂等：已终态时不覆盖真实结果）。
    pub(super) fn settle(&self, next: ProcessState) -> bool {
        let mut state = lock(&self.state);
        if state.is_terminal() {
            return false;
        }
        *state = next.clone();
        *lock(&self.ended_at_ms) = Some(crate::db::now_ms());
        let counted = self.counted.swap(false, Ordering::Relaxed);
        drop(state);
        if counted {
            GLOBAL_RUNNING.fetch_sub(1, Ordering::Relaxed);
        }
        self.notify.notify_waiters();
        if let Some(ui) = &self.ui {
            let _ = ui.send(ProcessUiEvent::Exited {
                session_id: self.session_id,
                origin_call_id: self.origin_call_id.clone(),
                state: next.clone(),
                exit_code: next.exit_code(),
            });
        }
        true
    }

    /// `Starting → Running` 的条件转移：成功时计入全局运行数并返回 true。
    pub(super) fn mark_running(&self) -> bool {
        let mut state = lock(&self.state);
        if !matches!(*state, ProcessState::Starting) {
            return false;
        }
        *state = ProcessState::Running;
        GLOBAL_RUNNING.fetch_add(1, Ordering::Relaxed);
        self.counted.store(true, Ordering::Relaxed);
        true
    }

    /// 从模型游标起读取新输出（推进模型游标，不影响 UI 预览）。
    pub(super) fn read_for_model(&self, max_bytes: usize) -> OutputRead {
        let _guard = lock(&self.cursor_lock);
        let cursor = self.cursor.load(Ordering::Relaxed);
        let read = lock(&self.output).read_from(cursor, max_bytes, self.state().is_terminal());
        self.cursor.store(read.end, Ordering::Relaxed);
        read
    }

    /// 当前保留输出的**尾部**文本（UI 预览用；不推进任何游标）。
    pub(super) fn tail_text(&self) -> (String, u64, u64, u64) {
        let output = lock(&self.output);
        let from = output.end().saturating_sub(MAX_MODEL_OUTPUT_BYTES as u64);
        let read = output.read_from(from, MAX_MODEL_OUTPUT_BYTES, true);
        (read.text, read.start, read.end, output.dropped_bytes())
    }

    pub(super) fn snapshot(&self) -> ProcessSnapshot {
        let state = self.state();
        let (tail, output_start, output_end, dropped_bytes) = self.tail_text();
        ProcessSnapshot {
            session_id: self.session_id,
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            tty: self.tty,
            exit_code: state.exit_code(),
            state,
            tail,
            output_start,
            output_end,
            dropped_bytes,
            started_at_ms: self.started_at_ms,
            ended_at_ms: *lock(&self.ended_at_ms),
            origin_call_id: self.origin_call_id.clone(),
            background_alive: self.has_living_group(),
        }
    }
}

/// 停止并 reap：温和停止 → 有界等待 → 未确认则保留 Interrupted（绝不伪报 Stopped）。
pub(super) async fn stop_handle(handle: &Arc<ProcessHandle>, reason: StopReason) {
    if handle.state().is_terminal() {
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
        None if handle.stop_requested.load(Ordering::Relaxed) => {}
        None => {
            handle.settle(ProcessState::Interrupted);
            return;
        }
    }
    let deadline = tokio::time::Instant::now() + STOP_GRACE * 2;
    while !handle.state().is_terminal() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if !handle.state().is_terminal() {
        handle.settle(ProcessState::Interrupted);
        tracing::warn!(
            target: "lx::agent",
            session_id = handle.session_id,
            "process did not confirm termination"
        );
    }
}

/// 当前输出量（近似 token：4 字节 ≈ 1 token）。
pub(super) fn estimated_tokens(handle: &Arc<ProcessHandle>) -> usize {
    let output = lock(&handle.output);
    let cursor = handle.cursor.load(Ordering::Relaxed);
    (output.end().saturating_sub(cursor) as usize).div_ceil(4)
}

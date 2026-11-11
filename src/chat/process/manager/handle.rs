//! 受管进程的共享句柄：输出缓冲、模型游标、尾部预览与 UI 通知。

use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{Mutex, Notify, mpsc, oneshot};

use super::super::ProcessState;
use super::super::output::{MAX_MODEL_OUTPUT_BYTES, OutputBuffer, OutputSource};
use super::types::{ProcessSnapshot, ProcessUiEvent};
use super::{GLOBAL_RUNNING, StopReason, lock};

/// 一个受管进程的共享状态。
pub(in crate::chat::process) struct ProcessHandle {
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
    pub(super) pid_start_time: std::sync::Mutex<Option<u64>>,
    pub(super) output: std::sync::Mutex<OutputBuffer>,
    pub(super) state: std::sync::Mutex<ProcessState>,
    pub(super) ended_at_ms: std::sync::Mutex<Option<i64>>,
    /// 模型读取游标（绝对字节偏移）。
    pub(super) cursor: AtomicU64,
    /// 串行化「读区间 + 推进游标」：否则并发读会重复输出或把游标写回较小值。
    ///
    /// 不能复用 `output` 锁：`wait_window` 已在另一处持 `output` 锁时调用其它方法，
    /// 反向加锁会死锁。
    pub(super) cursor_lock: std::sync::Mutex<()>,
    /// 该 handle 是否已计入 [`GLOBAL_RUNNING`]。
    ///
    /// `settle()` 只在「确实加过」时递减：spawn 失败路径会 settle 一个从未 `fetch_add`
    /// 的 handle，若无条件递减会让 `u64` 下溢成 `u64::MAX`，此后所有 `exec_command`
    /// 都返回 `resource_limit`（整个进程生命周期内不可恢复）。
    pub(super) counted: std::sync::atomic::AtomicBool,
    /// 停止请求是否已被某个调用者认领。
    ///
    /// `stop` 是 `Option<Sender>`，`take()` 之后第二个调用者看到的 `None` 既可能是
    /// 「已被别人认领」也可能是「压根没有 supervisor」——两者语义完全不同：
    /// 前者应当等待 supervisor 落终态，后者才该自己标 `Interrupted`。
    pub(super) stop_requested: std::sync::atomic::AtomicBool,
    /// 停止信号（被 supervisor 监听）。
    pub(super) stop: Mutex<Option<oneshot::Sender<StopReason>>>,
    /// stdin 写入通道（写入任务拥有真实句柄）。
    pub(super) stdin_slot: std::sync::Mutex<Option<mpsc::UnboundedSender<StdinRequest>>>,
    /// 输出/退出通知。
    pub(super) notify: Notify,
    /// UI 事件通道（限流：每个进程最多一个在途输出通知）。
    pub(super) ui: Option<mpsc::UnboundedSender<ProcessUiEvent>>,
    /// 是否已发出「有输出」通知（合并后续输出）。
    pub(super) ui_pending: std::sync::atomic::AtomicBool,
}

/// 一次 stdin 写入请求。
pub(in crate::chat::process) struct StdinRequest {
    pub(super) bytes: Vec<u8>,
    pub(super) reply: oneshot::Sender<Result<(), String>>,
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
            counted: std::sync::atomic::AtomicBool::new(false),
            stop_requested: std::sync::atomic::AtomicBool::new(false),
            stdin_slot: std::sync::Mutex::new(None),
            stop: Mutex::new(None),
            notify: Notify::new(),
            ui,
            ui_pending: std::sync::atomic::AtomicBool::new(false),
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
    ///
    /// 判据分三种情形：
    ///
    /// - 组长仍存活：比对启动时刻指纹，相同才放行（pid 被复用会被识破）；
    /// - 组长已退出（`nohup … &` 的常态，`process_start_time` 返回 None）：
    ///   此时**无法**证明 pgid 未被复用，但用户明确要停这个后台作业，
    ///   只能 best-effort 按 pgid 杀。这是本模块已知的残余风险。
    /// - 平台不支持指纹：同样退化为 best-effort。
    pub(super) fn leader_pid_not_recycled(&self) -> bool {
        let Some(pid) = *lock(&self.pid) else {
            return false;
        };
        let current = crate::platform::process_start_time(pid);
        let mut recorded = lock(&self.pid_start_time);
        match (current, *recorded) {
            // 组长还在：指纹必须一致。
            (Some(now), Some(saved)) => now == saved,
            // 首次拿到指纹：记下来，本帧放行。
            (Some(now), None) => {
                *recorded = Some(now);
                true
            }
            // 组长已退出 / 平台不支持：无法证明，按 best-effort 放行。
            (None, _) => true,
        }
    }

    /// shell 已退出的进程，其进程组里是否还有存活的后台作业。
    ///
    /// 这是回答「`nohup cmd &` 起的服务还在不在」的唯一可靠方式：lx 只 wait 得到
    /// 直接子进程（shell），后台作业一旦脱离就再无句柄，只能靠进程组存在性探测。
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
    ///
    /// 全局运行计数只在**确实计入过**时才递减（见 [`Self::counted`]）。
    pub(super) fn settle(&self, next: ProcessState) -> bool {
        let mut state = lock(&self.state);
        if state.is_terminal() {
            return false;
        }
        *state = next.clone();
        *lock(&self.ended_at_ms) = Some(crate::db::now_ms());
        // `counted` 的读写与 `mark_running` 一样都在 state 锁内，两者互斥：
        // 否则「置位后、加计数前」被 settle 抢先，会让计数先减后加而下溢。
        let counted = self.counted.swap(false, Ordering::Relaxed);
        // 注意：**不要**在这里抓「组内成员指纹」。`settle` 在 state 锁内、且是本
        // 语言层的热路径，同步 `ps` 调用会拖慢每一次进程收尾（实测会让读端 drain
        // 落后于 settle，卡片输出变空）。pid 复用的防护见 `stop_handle` 的注释。
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
    ///
    /// 必须是条件转移：supervisor 可能在主任务走到这里之前就已经 `settle(Exited)`
    /// （`true` / `echo hi` 这类秒退命令），无条件覆盖会把终态改回 `Running`，
    /// 而 supervisor 已经返回、没人再 settle —— 进程永远显示运行中、全局计数永久 +1。
    pub(super) fn mark_running(&self) -> bool {
        let mut state = lock(&self.state);
        if !matches!(*state, ProcessState::Starting) {
            return false;
        }
        *state = ProcessState::Running;
        // 计数与 `counted` 在同一次 state 锁内一起落定，见 `settle`。
        GLOBAL_RUNNING.fetch_add(1, Ordering::Relaxed);
        self.counted.store(true, Ordering::Relaxed);
        true
    }

    /// 从模型游标起读取新输出（推进模型游标，不影响 UI 预览）。
    ///
    /// 「读区间 + 推进游标」在同一把锁内完成：否则两个并发调用者（模型正在
    /// `write_stdin` 等待、用户在面板按停止）会读到同一游标，把同一段输出作为两次
    /// 工具结果返回；先完成者还可能后写游标，把它写回较小值导致再次重复。
    pub(super) fn read_for_model(&self, max_bytes: usize) -> super::super::output::OutputRead {
        let _guard = lock(&self.cursor_lock);
        let cursor = self.cursor.load(Ordering::Relaxed);
        let read = lock(&self.output).read_from(cursor, max_bytes, self.state().is_terminal());
        self.cursor.store(read.end, Ordering::Relaxed);
        read
    }

    /// 当前保留输出的**尾部**文本（UI 预览用；不推进任何游标）。
    ///
    /// 必须从 `end − 预算` 起读：从保留区间起点读会在输出超过预算后永远返回
    /// 「最早的那一段」，卡片与面板预览就此冻结在旧内容上。
    pub(super) fn tail_text(&self) -> (String, u64, u64, u64) {
        let output = lock(&self.output);
        let from = output.end().saturating_sub(MAX_MODEL_OUTPUT_BYTES as u64);
        let read = output.read_from(from, MAX_MODEL_OUTPUT_BYTES, true);
        // 第 2、3 项是 `tail` 在**原始字节流**中的区间：落库要写真实偏移，
        // 不能拿 `tail.len()` 冒充（文本已剥离控制序列，长度与原始流无关）。
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

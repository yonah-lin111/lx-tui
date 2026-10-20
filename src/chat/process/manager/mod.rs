//! 会话进程管理器：spawn、输出收集、stdin 写入、等待窗口与停止。
//!
//! 生命周期（对应 design §5 / §8）：
//!
//! ```text
//! exec()  -> 校验限额 -> spawn（pipe 或 PTY）-> 登记 owner + session_id
//!         -> 两路 drain 线程/任务持续读取 -> 等待窗口到期或进程退出 -> 返回结果
//! write_stdin() -> owner 校验 -> （可选）写入 stdin / 显式停止 -> 等待窗口 -> 返回区间
//! stop()/drop   -> 温和停止 -> 超时强制终止 -> reap（真实退出码只来自 wait）
//! ```
//!
//! 不变量：
//!
//! 1. 真实退出码只能由 `wait` 得到：读端 EOF 不代表成功退出；
//! 2. 等待窗口只结束本次工具等待，**不杀进程**；只有 `timeout_ms` 或显式停止才清理；
//! 3. 模型读取游标与 UI 预览分开：UI 读多少次都不影响模型下次拿到的区间；
//! 4. 停止幂等：重复停止已退出进程只返回已知终态，不再发信号；
//! 5. 创建成功但登记失败时清理资源，不留下无人管理的后台进程。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::{Mutex, Notify, mpsc, oneshot};

mod spawn;

use super::output::{MAX_MODEL_OUTPUT_BYTES, OutputBuffer, OutputSource};
use super::{
    ExecArgs, MAX_INPUT_PREVIEW_CHARS, ProcessOwner, ProcessResult, ProcessResultKind,
    ProcessState, StdinArgs, preview_input,
};

/// 默认等待窗口（毫秒）。
///
/// 对齐 codex：`exec_command` 默认等 10s（`write_stdin` 另有默认，见
/// [`DEFAULT_STDIN_YIELD_MS`]）。窗口只决定**本次工具调用等多久**，到期不杀进程。
pub const DEFAULT_YIELD_MS: u64 = 10_000;
/// `write_stdin` 的默认等待窗口（毫秒）：交互写要快速返回，对齐 codex 的 250ms。
///
/// 与 `exec_command` 分开：交互是「打字 → 看反应」，等 10s 完全没有意义；
/// 而首轮命令往往要几秒才吐第一行输出，等 250ms 只会立刻返回空。
pub const DEFAULT_STDIN_YIELD_MS: u64 = 250;
/// 等待窗口上限（毫秒）。
pub const MAX_YIELD_MS: u64 = 30_000;
/// 命令总寿命上限（毫秒）：24 小时。
pub const MAX_TIMEOUT_MS: u64 = 24 * 60 * 60 * 1000;
/// 返回模型的文本近似 token 上限。
pub const MAX_OUTPUT_TOKENS: usize = 16_000;
/// 每次 stdin 输入字节上限。
pub const MAX_STDIN_BYTES: usize = 64 * 1024;
/// 温和停止后等待强制终止的时间。
pub const STOP_GRACE: Duration = Duration::from_millis(1000);
/// 终态记录保留时长。
pub const FINISHED_TTL: Duration = Duration::from_secs(600);

/// 资源限额。
#[derive(Debug, Clone, Copy)]
pub struct ProcessLimits {
    /// 单会话同时运行的进程上限。
    pub max_running: usize,
    /// 进程内全局运行进程上限（多窗格会话累加）。
    pub global_max_running: usize,
    /// 终态记录保留条数上限。
    pub max_finished: usize,
}

impl Default for ProcessLimits {
    fn default() -> Self {
        Self {
            max_running: 8,
            global_max_running: 32,
            max_finished: 32,
        }
    }
}

/// 进程内全局运行进程计数（跨会话共享）。
pub(super) static GLOBAL_RUNNING: AtomicU64 = AtomicU64::new(0);

/// 进程内全局进程 ID 计数器（**跨会话共享**）。
///
/// 必须是全局的：`ProcessUiEvent` 只带 `session_id`，事件层靠它在运行实例里反查归属。
/// 若每个管理器各自从 1 起算，两个并存会话的第一个进程都叫 `1`，事件会被判给错误的
/// 会话，进而用别人的 `runtime_epoch` 写坏库里的进程行。
static NEXT_PROCESS_ID: AtomicU64 = AtomicU64::new(1);

/// 取下一个全局唯一的进程 ID。
fn next_process_id() -> u64 {
    NEXT_PROCESS_ID.fetch_add(1, Ordering::Relaxed)
}

/// 当前全局运行进程数（仅供测试断言）。
///
/// 生产代码不该读它（限额判定在内部分支里做），因此只在测试构建下存在。
#[cfg(test)]
pub(crate) fn global_running() -> u64 {
    GLOBAL_RUNNING.load(Ordering::Relaxed)
}

/// UI 侧进程事件（纯数据；由事件层映射成 `AppEvent`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessUiEvent {
    /// 有新输出可读（限流后每进程最多一个在途通知）。
    Output {
        session_id: u64,
        origin_call_id: String,
    },
    /// 进程进入终态。
    Exited {
        session_id: u64,
        origin_call_id: String,
        state: ProcessState,
        exit_code: Option<i32>,
    },
}

/// 进程操作失败原因（稳定错误码，可回给模型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessError {
    /// 稳定错误码（`process_not_found` / `process_expired` / `resource_limit` / …）。
    pub code: &'static str,
    /// 单行说明（不含命令输出）。
    pub message: String,
}

impl ProcessError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// 一次进程的 UI 快照（只读，不推进模型游标）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSnapshot {
    pub session_id: u64,
    pub command: String,
    pub cwd: String,
    pub tty: bool,
    pub state: ProcessState,
    pub exit_code: Option<i32>,
    /// 保留输出的尾部文本（已剥离控制序列）。
    pub tail: String,
    /// `tail` 在**原始字节流**中的区间 `[output_start, output_end)`。
    ///
    /// 落库时写这两列。它们不是 `tail.len()`——`tail` 是剥掉控制序列、lossy 转码
    /// 之后的文本，长度与原始偏移无关。留在库里的是真实偏移，恢复侧才能据此判断
    /// 「模型读到哪」。
    pub output_start: u64,
    pub output_end: u64,
    pub dropped_bytes: u64,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    /// 发起该进程的 `exec_command` 工具调用 ID（卡片归属）。
    pub origin_call_id: String,
    /// shell 已退出，但它拉起的后台作业仍在进程组里存活。
    ///
    /// 真实场景：`nohup bash task.sh &` 之后 shell 立刻退出。此时 lx 的直接子进程
    /// 已是 `exited`，可用户的服务还在跑——只看 `state` 会把它从后台任务面板里
    /// 过滤掉，面板显示「no running processes」而任务其实活着。
    pub background_alive: bool,
}

/// 停止原因（决定终态标记）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StopReason {
    /// 用户/模型显式停止。
    Stopped,
    /// 命令寿命超时。
    TimedOut,
}

/// 一个受管进程的共享状态。
pub(super) struct ProcessHandle {
    session_id: u64,
    origin_call_id: String,
    command: String,
    cwd: String,
    tty: bool,
    started_at_ms: i64,
    /// lx 直接 spawn 的 shell 的 pid（也是进程组 id，`process_group(0)` 建立）。
    ///
    /// spawn 之后才知道，所以是 `Mutex<Option<u32>>`；PTY 分支没有独立进程组。
    /// 用途：shell 退出后判断它拉起的后台作业是否还活着。
    pid: std::sync::Mutex<Option<u32>>,
    /// 该 pid 的启动时刻（身份指纹，见 [`crate::platform::process_start_time`]）。
    ///
    /// pid 会被回收再分配：只凭 pid 对「已退出的 shell 的进程组」发 SIGKILL 有
    /// 误杀无关进程的风险。杀之前比对指纹，对不上就放弃。
    pid_start_time: std::sync::Mutex<Option<u64>>,
    output: std::sync::Mutex<OutputBuffer>,
    state: std::sync::Mutex<ProcessState>,
    ended_at_ms: std::sync::Mutex<Option<i64>>,
    /// 模型读取游标（绝对字节偏移）。
    cursor: AtomicU64,
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
    counted: std::sync::atomic::AtomicBool,
    /// 停止请求是否已被某个调用者认领。
    ///
    /// `stop` 是 `Option<Sender>`，`take()` 之后第二个调用者看到的 `None` 既可能是
    /// 「已被别人认领」也可能是「压根没有 supervisor」——两者语义完全不同：
    /// 前者应当等待 supervisor 落终态，后者才该自己标 `Interrupted`。
    stop_requested: std::sync::atomic::AtomicBool,
    /// 停止信号（被 supervisor 监听）。
    stop: Mutex<Option<oneshot::Sender<StopReason>>>,
    /// stdin 写入通道（写入任务拥有真实句柄）。
    stdin_slot: std::sync::Mutex<Option<mpsc::UnboundedSender<StdinRequest>>>,
    /// 输出/退出通知。
    notify: Notify,
    /// UI 事件通道（限流：每个进程最多一个在途输出通知）。
    ui: Option<mpsc::UnboundedSender<ProcessUiEvent>>,
    /// 是否已发出「有输出」通知（合并后续输出）。
    ui_pending: std::sync::atomic::AtomicBool,
}

/// 一次 stdin 写入请求。
pub(super) struct StdinRequest {
    pub(super) bytes: Vec<u8>,
    pub(super) reply: oneshot::Sender<Result<(), String>>,
}

impl ProcessHandle {
    #[allow(clippy::too_many_arguments)]
    fn new(
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

    fn state(&self) -> ProcessState {
        lock(&self.state).clone()
    }

    /// 记录 spawn 出来的 shell pid（也是进程组 id）。
    fn set_pid(&self, pid: u32) {
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
    fn leader_pid_not_recycled(&self) -> bool {
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
    fn has_living_group(&self) -> bool {
        if !self.state().is_terminal() {
            return false;
        }
        lock(&self.pid)
            .map(crate::platform::process_group_has_members)
            .unwrap_or(false)
    }

    /// 写入输出并通知等待者与 UI（UI 通知按进程合并）。
    fn push_output(&self, source: OutputSource, bytes: &[u8]) {
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
    fn clear_ui_pending(&self) {
        self.ui_pending.store(false, Ordering::Relaxed);
    }

    /// 标记终态（幂等：已终态时不覆盖真实结果）。
    ///
    /// 全局运行计数只在**确实计入过**时才递减（见 [`Self::counted`]）。
    fn settle(&self, next: ProcessState) -> bool {
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
    fn mark_running(&self) -> bool {
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
    fn read_for_model(&self, max_bytes: usize) -> super::output::OutputRead {
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
    fn tail_text(&self) -> (String, u64, u64, u64) {
        let output = lock(&self.output);
        let from = output.end().saturating_sub(MAX_MODEL_OUTPUT_BYTES as u64);
        let read = output.read_from(from, MAX_MODEL_OUTPUT_BYTES, true);
        // 第 2、3 项是 `tail` 在**原始字节流**中的区间：落库要写真实偏移，
        // 不能拿 `tail.len()` 冒充（文本已剥离控制序列，长度与原始流无关）。
        (read.text, read.start, read.end, output.dropped_bytes())
    }

    fn snapshot(&self) -> ProcessSnapshot {
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

/// 会话进程管理器（一个聊天运行实例持有唯一一份）。
pub struct ProcessManager {
    owner: ProcessOwner,
    limits: ProcessLimits,
    processes: std::sync::Mutex<HashMap<u64, Arc<ProcessHandle>>>,
    ui: Option<mpsc::UnboundedSender<ProcessUiEvent>>,
}

impl ProcessManager {
    /// 新建管理器；`ui` 为 UI 事件通道（无 UI 的测试环境可为 None）。
    pub fn new(owner: ProcessOwner, ui: Option<mpsc::UnboundedSender<ProcessUiEvent>>) -> Self {
        Self::with_limits(owner, ui, ProcessLimits::default())
    }

    /// 指定限额构造（测试用小限额）。
    pub fn with_limits(
        owner: ProcessOwner,
        ui: Option<mpsc::UnboundedSender<ProcessUiEvent>>,
        limits: ProcessLimits,
    ) -> Self {
        Self {
            owner,
            limits,
            processes: std::sync::Mutex::new(HashMap::new()),
            ui,
        }
    }

    /// 归属键（事件层据此过滤旧代次事件）。
    pub fn owner(&self) -> &ProcessOwner {
        &self.owner
    }

    /// 该会话当前运行中的进程数。
    pub fn running_count(&self) -> usize {
        lock(&self.processes)
            .values()
            .filter(|handle| !handle.state().is_terminal())
            .count()
    }

    /// UI 取走输出通知后复位合并标志（下一次输出会重新通知）。
    pub fn acknowledge_output(&self, session_id: u64) {
        if let Some(handle) = self.handle(session_id) {
            handle.clear_ui_pending();
        }
    }

    /// 指定进程的 UI 快照；未知 ID 返回 None。
    pub fn snapshot(&self, session_id: u64) -> Option<ProcessSnapshot> {
        self.handle(session_id).map(|handle| handle.snapshot())
    }

    /// 该会话的全部进程快照（UI 重载/清理用）。
    ///
    /// **按启动时刻排序**（次键 `session_id`）：面板用下标记住高亮，而 `HashMap`
    /// 迭代顺序不稳定——列表顺序一抖，`^d` 就会停掉另一个进程（破坏性操作）。
    pub fn snapshots(&self) -> Vec<ProcessSnapshot> {
        let mut snapshots: Vec<ProcessSnapshot> = lock(&self.processes)
            .values()
            .map(|handle| handle.snapshot())
            .collect();
        snapshots.sort_by(|left, right| {
            left.started_at_ms
                .cmp(&right.started_at_ms)
                .then(left.session_id.cmp(&right.session_id))
        });
        snapshots
    }

    fn handle(&self, session_id: u64) -> Option<Arc<ProcessHandle>> {
        lock(&self.processes).get(&session_id).cloned()
    }

    /// 执行 `exec_command`：spawn 后最多等待 `yield_time_ms`，到期则把运行中的进程交回。
    pub async fn exec(
        &self,
        args: ExecArgs,
        workspace: &Path,
        origin_call_id: &str,
    ) -> Result<ProcessResult, ProcessError> {
        self.exec_observed(args, workspace, origin_call_id, &|_| {})
            .await
    }

    /// 同 [`Self::exec`]，但 spawn 成功后**立即**回调新进程 id。
    ///
    /// 用途：本轮停止语义必须能在命令**等待期间**就杀掉它。若等 `exec` 返回后
    /// 才登记（`TurnExecution::observe` 的时机），用户在等待窗口里按 Esc 时
    /// 目标集合是空的，命令会继续在后台跑。
    pub async fn exec_observed(
        &self,
        args: ExecArgs,
        workspace: &Path,
        origin_call_id: &str,
        on_spawn: &(dyn Fn(u64) + Sync),
    ) -> Result<ProcessResult, ProcessError> {
        self.prune_finished();
        if self.running_count() >= self.limits.max_running {
            return Err(ProcessError::new(
                "resource_limit",
                format!(
                    "this chat already has {} running processes",
                    self.limits.max_running
                ),
            ));
        }
        if GLOBAL_RUNNING.load(Ordering::Relaxed) >= self.limits.global_max_running as u64 {
            return Err(ProcessError::new(
                "resource_limit",
                "too many running processes across all chats",
            ));
        }
        let cwd = resolve_cwd(workspace, args.workdir.as_deref())?;
        let shell = args
            .shell
            .clone()
            .unwrap_or_else(crate::platform::default_shell);
        let session_id = next_process_id();
        let handle = Arc::new(ProcessHandle::new(
            session_id,
            origin_call_id.to_string(),
            args.cmd.clone(),
            cwd.display().to_string(),
            args.tty,
            crate::db::now_ms(),
            self.ui.clone(),
        ));
        let spawned = if args.tty {
            self.spawn_pty(&handle, &shell, &args, &cwd)
        } else {
            self.spawn_pipe(&handle, &shell, &args, &cwd)
        };
        if let Err(error) = spawned {
            handle.settle(ProcessState::Failed {
                code: error.code.to_string(),
            });
            return Err(error);
        }
        // 登记失败（重名）时立即停止新进程，绝不留下无人管理的后台进程。
        let duplicated = {
            let mut processes = lock(&self.processes);
            processes.insert(session_id, Arc::clone(&handle)).is_some()
        };
        if duplicated {
            self.stop_handle(&handle, StopReason::Stopped).await;
            return Err(ProcessError::new("spawn_failed", "duplicate process id"));
        }
        // 条件转移：supervisor 可能已经落过终态（秒退命令），绝不能覆盖回 Running。
        handle.mark_running();
        // spawn 成功、进程已在管理表里：此刻就登记给调用方（等待窗口期间可被停止）。
        on_spawn(session_id);
        // 命令总寿命：即使等待窗口先到期（工具已返回 running），后台仍按超时清理。
        if let Some(timeout_ms) = args.timeout_ms {
            let lifetime_handle = Arc::clone(&handle);
            let lifetime = Duration::from_millis(timeout_ms.min(MAX_TIMEOUT_MS));
            tokio::spawn(async move {
                tokio::time::sleep(lifetime).await;
                if !lifetime_handle.state().is_terminal() {
                    stop_handle(&lifetime_handle, StopReason::TimedOut).await;
                }
            });
        }
        let started = tokio::time::Instant::now();
        self.wait_window(
            &handle,
            args.yield_time_ms,
            args.max_output_tokens,
            args.timeout_ms,
        )
        .await;
        Ok(self.result_of(
            &handle,
            ProcessResultKind::ExecCommand,
            None,
            false,
            args.max_output_tokens,
            started,
        ))
    }

    /// 进程交互：空写只读输出；非空写 stdin；`terminate` 显式停止。
    pub async fn interact(&self, args: StdinArgs) -> Result<ProcessResult, ProcessError> {
        let Some(handle) = self.handle(args.session_id) else {
            return Err(ProcessError::new(
                "process_not_found",
                "unknown or expired session_id",
            ));
        };
        if args.terminate && !args.chars.is_empty() {
            return Err(ProcessError::new(
                "invalid_arguments",
                "'terminate' cannot be combined with a non-empty 'chars'",
            ));
        }
        if args.chars.len() > MAX_STDIN_BYTES {
            return Err(ProcessError::new(
                "too_large",
                format!("'chars' exceeds {MAX_STDIN_BYTES} bytes"),
            ));
        }
        let started = tokio::time::Instant::now();
        if args.terminate {
            self.stop_handle(&handle, StopReason::Stopped).await;
            let mut result = self.result_of(
                &handle,
                ProcessResultKind::WriteStdin,
                Some(preview_input(&args.chars)),
                true,
                args.max_output_tokens,
                started,
            );
            if result.output.is_empty() {
                // 显式停止要能看到“最后输出”：读游标已经取空时，补上保留的尾部
                // （不推进模型游标——它已经指向末尾）。
                let (tail, start, end, dropped) = handle.tail_text();
                result.output = tail;
                result.output_start = start;
                result.output_end = end;
                result.dropped_bytes = dropped;
            }
            return Ok(result);
        }
        if !args.chars.is_empty() {
            self.write_stdin(&handle, args.chars.as_bytes()).await?;
        }
        self.wait_window(&handle, args.yield_time_ms, args.max_output_tokens, None)
            .await;
        let wrote_input = !args.chars.is_empty();
        let input = wrote_input.then(|| {
            preview_input(
                &args
                    .chars
                    .chars()
                    .take(MAX_INPUT_PREVIEW_CHARS)
                    .collect::<String>(),
            )
        });
        let mut result = self.result_of(
            &handle,
            ProcessResultKind::WriteStdin,
            input,
            false,
            args.max_output_tokens,
            started,
        );
        result.wrote_input = wrote_input;
        Ok(result)
    }

    /// 显式停止一个进程（幂等：已终态只返回已知终态）。
    pub async fn stop(&self, session_id: u64) -> Result<ProcessResult, ProcessError> {
        let Some(handle) = self.handle(session_id) else {
            return Err(ProcessError::new(
                "process_not_found",
                "unknown or expired session_id",
            ));
        };
        let started = tokio::time::Instant::now();
        self.stop_handle(&handle, StopReason::Stopped).await;
        Ok(self.result_of(
            &handle,
            ProcessResultKind::WriteStdin,
            None,
            true,
            MAX_OUTPUT_TOKENS,
            started,
        ))
    }

    /// 停止一批进程（会话关闭、Esc 停止当前轮）。
    pub async fn stop_many(&self, session_ids: impl IntoIterator<Item = u64>) {
        let handles: Vec<Arc<ProcessHandle>> = {
            let processes = lock(&self.processes);
            session_ids
                .into_iter()
                .filter_map(|id| processes.get(&id).cloned())
                .collect()
        };
        for handle in handles {
            self.stop_handle(&handle, StopReason::Stopped).await;
        }
    }

    /// 关闭运行实例：停止全部进程（幂等）。
    pub async fn shutdown(&self) {
        // 先对**所有**目标并发送出停止信号，再统一有界等待：逐个 `await` 是
        // N × 2s 的串行预算，进程一多就会在「还没轮到第 4 个」时超时返回，
        // 剩下的进程连 SIGTERM 都没收到，退出后变成孤儿进程组。
        let all: Vec<u64> = lock(&self.processes).keys().copied().collect();
        let handles: Vec<Arc<ProcessHandle>> =
            all.iter().filter_map(|id| self.handle(*id)).collect();
        // 先广播温和停止信号（不等待），让所有进程并行进入收尾。
        for handle in &handles {
            if handle.state().is_terminal() {
                continue;
            }
            let mut guard = handle.stop.lock().await;
            if let Some(sender) = guard.take() {
                handle.stop_requested.store(true, Ordering::Relaxed);
                let _ = sender.send(StopReason::Stopped);
            }
        }
        // 再统一等待：总预算与单个进程的最坏耗时同阶（不再随进程数线性增长）。
        let deadline = tokio::time::Instant::now() + STOP_GRACE * 2;
        for handle in &handles {
            while !handle.state().is_terminal() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            if !handle.state().is_terminal() {
                stop_handle(handle, StopReason::Stopped).await;
            }
        }
    }

    /// 写入 stdin；半写/BrokenPipe 以稳定错误码返回。
    async fn write_stdin(
        &self,
        handle: &Arc<ProcessHandle>,
        bytes: &[u8],
    ) -> Result<(), ProcessError> {
        if handle.state().is_terminal() {
            return Err(ProcessError::new(
                "process_exited",
                "process has already exited",
            ));
        }
        let sender = lock(&handle.stdin_slot).clone();
        let Some(sender) = sender else {
            return Err(ProcessError::new(
                "process_exited",
                "stdin is no longer writable",
            ));
        };
        let (reply, receiver) = oneshot::channel();
        if sender
            .send(StdinRequest {
                bytes: bytes.to_vec(),
                reply,
            })
            .is_err()
        {
            return Err(ProcessError::new(
                "process_exited",
                "stdin writer has stopped",
            ));
        }
        match receiver.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(ProcessError::new("stdin_failed", error)),
            Err(_) => Err(ProcessError::new("stdin_failed", "stdin writer dropped")),
        }
    }

    /// 等待窗口：直到有新输出、进程终态或窗口到期（到期**不杀进程**）。
    async fn wait_window(
        &self,
        handle: &Arc<ProcessHandle>,
        yield_ms: u64,
        max_tokens: usize,
        timeout_ms: Option<u64>,
    ) {
        let deadline =
            tokio::time::Instant::now() + Duration::from_millis(yield_ms.min(MAX_YIELD_MS));
        // 命令寿命上限：与等待窗口并行计时，到期后按超时清理。
        let life = timeout_ms
            .map(|ms| tokio::time::Instant::now() + Duration::from_millis(ms.min(MAX_TIMEOUT_MS)));
        loop {
            if handle.state().is_terminal() {
                return;
            }
            // 命令寿命上限优先于“继续累积输出”：到期即清理进程，不当成等待窗口到期。
            if life.is_some_and(|life| tokio::time::Instant::now() >= life) {
                self.stop_handle(handle, StopReason::TimedOut).await;
                return;
            }
            let has_output = {
                let output = lock(&handle.output);
                output.end() > handle.cursor.load(Ordering::Relaxed)
                    && output.end() > output.start()
            };
            if has_output {
                // 已有可读输出：先交回模型，后台输出继续由事件层更新卡片。
                let enough = estimated_tokens(handle) >= max_tokens;
                if enough || tokio::time::Instant::now() >= deadline {
                    return;
                }
                // 仍在窗口内且输出未达预算：继续累积一小段（避免逐字节返回）。
                let slice = tokio::time::sleep(Duration::from_millis(30));
                tokio::pin!(slice);
                tokio::select! {
                    _ = &mut slice => continue,
                    _ = handle.notify.notified() => continue,
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return;
            }
            let mut wait_until = deadline;
            if let Some(life) = life
                && life < wait_until
            {
                wait_until = life;
            }
            let sleep = tokio::time::sleep_until(wait_until);
            tokio::pin!(sleep);
            tokio::select! {
                _ = &mut sleep => {
                    if life.is_some_and(|life| tokio::time::Instant::now() >= life) {
                        self.stop_handle(handle, StopReason::TimedOut).await;
                        return;
                    }
                    return;
                }
                _ = handle.notify.notified() => {
                    if life.is_some_and(|life| tokio::time::Instant::now() >= life) {
                        self.stop_handle(handle, StopReason::TimedOut).await;
                        return;
                    }
                }
            }
        }
    }

    /// 停止并 reap：温和停止 → 超时强制终止 → 等待真实退出状态。
    async fn stop_handle(&self, handle: &Arc<ProcessHandle>, reason: StopReason) {
        stop_handle(handle, reason).await;
    }

    /// 由共享状态构造结果（读取本次模型区间）。
    fn result_of(
        &self,
        handle: &Arc<ProcessHandle>,
        kind: ProcessResultKind,
        input: Option<String>,
        terminate: bool,
        max_tokens: usize,
        started: tokio::time::Instant,
    ) -> ProcessResult {
        // `max_output_tokens` 是**返回模型的文本预算**，不是「何时返回」的开关：
        // 只用于早退判断会让模型显式请求 200 token 时仍收到 64 KiB 正文。
        // 按 4 字节 ≈ 1 token 换算，并与硬上限取 min。
        let budget = max_tokens
            .max(1)
            .saturating_mul(4)
            .min(MAX_MODEL_OUTPUT_BYTES);
        let read = handle.read_for_model(budget);
        let state = handle.state();
        ProcessResult {
            kind,
            session_id: handle.session_id,
            command: handle.command.clone(),
            cwd: handle.cwd.clone(),
            tty: handle.tty,
            exit_code: state.exit_code(),
            state,
            output: read.text,
            output_start: read.start,
            output_end: read.end,
            dropped_bytes: read.dropped_bytes,
            truncated: read.truncated,
            input,
            wrote_input: false,
            terminate,
            chunk_id: crate::chat::process::generate_chunk_id(),
            // 与 codex 的 `wall_time_seconds` 同口径：本次调用在等待窗口内真正花掉的墙钟。
            wall_time_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }

    /// 淘汰过期的终态记录（条数与时间双限）。
    fn prune_finished(&self) {
        let now = crate::db::now_ms();
        let mut processes = lock(&self.processes);
        // 「shell 已退出但后台作业仍在跑」的记录**不能淘汰**：那正是用户要停的服务，
        // 淘汰掉就等于面板不显示、也停不了，而进程组还在后台烧。
        let mut finished: Vec<(u64, i64)> = processes
            .iter()
            .filter(|(_, handle)| handle.state().is_terminal() && !handle.has_living_group())
            .map(|(id, handle)| (*id, lock(&handle.ended_at_ms).unwrap_or(now)))
            .collect();
        finished.sort_by_key(|(_, ended)| *ended);
        let excess = finished.len().saturating_sub(self.limits.max_finished);
        let mut remove: Vec<u64> = finished.iter().take(excess).map(|(id, _)| *id).collect();
        let ttl_ms = FINISHED_TTL.as_millis() as i64;
        for (id, ended) in &finished {
            if now.saturating_sub(*ended) > ttl_ms {
                remove.push(*id);
            }
        }
        for id in remove {
            processes.remove(&id);
        }
    }
}

/// 停止并 reap：温和停止 → 有界等待 → 未确认则保留 Interrupted（绝不伪报 Stopped）。
///
/// 这是自由函数（而非方法）的原因：命令寿命超时任务需要在不持有 `&self` 的情况下
/// 停止进程（spawn 出去的 `'static` 任务不能借用管理器）。
async fn stop_handle(handle: &Arc<ProcessHandle>, reason: StopReason) {
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

/// 读端 → 缓冲的异步任务（管道）。
pub(super) fn spawn_pipe_reader<R>(handle: Arc<ProcessHandle>, mut reader: R, source: OutputSource)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut buffer = vec![0u8; 8192];
        loop {
            match reader.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => handle.push_output(source, &buffer[..read]),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    });
}

/// 退出码 → 状态（未知退出码保持 `Exited { code: None }`，绝不伪造 0）。
pub(super) fn exit_state(code: Option<i32>) -> ProcessState {
    ProcessState::Exited { code }
}

/// 当前输出量（近似 token：4 字节 ≈ 1 token）。
pub(super) fn estimated_tokens(handle: &Arc<ProcessHandle>) -> usize {
    let output = lock(&handle.output);
    let cursor = handle.cursor.load(Ordering::Relaxed);
    (output.end().saturating_sub(cursor) as usize).div_ceil(4)
}

/// 解析工作目录：相对路径基于冻结工作区，不修改进程全局 cwd。
fn resolve_cwd(workspace: &Path, workdir: Option<&str>) -> Result<PathBuf, ProcessError> {
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

/// PTY 固定尺寸（聊天卡片不做全屏终端仿真，窗格缩放不改变后台程序尺寸）。
pub(super) const PTY_COLS: u16 = 120;
pub(super) const PTY_ROWS: u16 = 40;

/// 短时停止请求轮询器（PTY 等待线程无法 select）。
pub(super) struct StopWaiter<'a> {
    receiver: &'a mut oneshot::Receiver<StopReason>,
}

impl<'a> StopWaiter<'a> {
    fn new(receiver: &'a mut oneshot::Receiver<StopReason>) -> Self {
        Self { receiver }
    }

    fn poll(&mut self) -> Option<StopReason> {
        match self.receiver.try_recv() {
            Ok(reason) => Some(reason),
            Err(oneshot::error::TryRecvError::Empty) => None,
            Err(oneshot::error::TryRecvError::Closed) => None,
        }
    }
}

/// 短锁包装：中毒锁按内部数据继续使用（本模块不 panic 持有锁）。
pub(super) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod tests;

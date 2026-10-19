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
    pub dropped_bytes: u64,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    /// 发起该进程的 `exec_command` 工具调用 ID（卡片归属）。
    pub origin_call_id: String,
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
    output: std::sync::Mutex<OutputBuffer>,
    state: std::sync::Mutex<ProcessState>,
    ended_at_ms: std::sync::Mutex<Option<i64>>,
    /// 模型读取游标（绝对字节偏移）。
    cursor: AtomicU64,
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
            cursor: AtomicU64::new(0),
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
    fn settle(&self, next: ProcessState) -> bool {
        let mut state = lock(&self.state);
        if state.is_terminal() {
            return false;
        }
        *state = next.clone();
        *lock(&self.ended_at_ms) = Some(crate::db::now_ms());
        drop(state);
        GLOBAL_RUNNING.fetch_sub(1, Ordering::Relaxed);
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

    /// 从模型游标起读取新输出（推进模型游标，不影响 UI 预览）。
    fn read_for_model(&self, max_bytes: usize) -> super::output::OutputRead {
        let cursor = self.cursor.load(Ordering::Relaxed);
        let read = lock(&self.output).read_from(cursor, max_bytes, self.state().is_terminal());
        self.cursor.store(read.end, Ordering::Relaxed);
        read
    }

    /// 当前保留输出的尾部文本（UI 预览用；不推进任何游标）。
    fn tail_text(&self) -> (String, u64) {
        let output = lock(&self.output);
        let start = output.start();
        let read = output.read_from(start, MAX_MODEL_OUTPUT_BYTES, true);
        (read.text, output.dropped_bytes())
    }

    fn snapshot(&self) -> ProcessSnapshot {
        let state = self.state();
        let (tail, dropped_bytes) = self.tail_text();
        ProcessSnapshot {
            session_id: self.session_id,
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            tty: self.tty,
            exit_code: state.exit_code(),
            state,
            tail,
            dropped_bytes,
            started_at_ms: self.started_at_ms,
            ended_at_ms: *lock(&self.ended_at_ms),
            origin_call_id: self.origin_call_id.clone(),
        }
    }
}

/// 会话进程管理器（一个聊天运行实例持有唯一一份）。
pub struct ProcessManager {
    owner: ProcessOwner,
    limits: ProcessLimits,
    processes: std::sync::Mutex<HashMap<u64, Arc<ProcessHandle>>>,
    next_id: AtomicU64,
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
            next_id: AtomicU64::new(1),
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
    pub fn snapshots(&self) -> Vec<ProcessSnapshot> {
        lock(&self.processes)
            .values()
            .map(|handle| handle.snapshot())
            .collect()
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
        let session_id = self.next_id.fetch_add(1, Ordering::Relaxed);
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
        GLOBAL_RUNNING.fetch_add(1, Ordering::Relaxed);
        *lock(&handle.state) = ProcessState::Running;
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
                started,
            );
            if result.output.is_empty() {
                // 显式停止要能看到“最后输出”：读游标已经取空时，补上保留的尾部
                // （不推进模型游标——它已经指向末尾）。
                let (tail, dropped) = handle.tail_text();
                result.output = tail;
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
        Ok(self.result_of(&handle, ProcessResultKind::WriteStdin, None, true, started))
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
        let all: Vec<u64> = lock(&self.processes).keys().copied().collect();
        self.stop_many(all).await;
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
        started: tokio::time::Instant,
    ) -> ProcessResult {
        let budget = MAX_MODEL_OUTPUT_BYTES;
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
        let mut finished: Vec<(u64, i64)> = processes
            .iter()
            .filter(|(_, handle)| handle.state().is_terminal())
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
        return;
    }
    let sender = {
        let mut guard = handle.stop.lock().await;
        guard.take()
    };
    match sender {
        Some(sender) => {
            let _ = sender.send(reason);
        }
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

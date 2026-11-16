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

mod handle;
mod lifecycle;
mod spawn;
mod types;

pub use types::*;

use handle::*;
use lifecycle::*;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::sync::{mpsc, oneshot};

use super::output::{MAX_MODEL_OUTPUT_BYTES, MAX_PROCESS_OUTPUT_BYTES, OutputSource};
use super::{
    ExecArgs, MAX_INPUT_PREVIEW_CHARS, ProcessOwner, ProcessResult, ProcessResultKind,
    ProcessState, StdinArgs, preview_input,
};

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
        let mut result = self
            .exec_observed(args, workspace, origin_call_id, None, &|_| {})
            .await?;
        result.enforce_output_bounds();
        Ok(result)
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
        sandbox: Option<&crate::platform::sandbox::SandboxSpec<'_>>,
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
            self.spawn_pty(&handle, &shell, &args, &cwd, sandbox)
        } else {
            self.spawn_pipe(&handle, &shell, &args, &cwd, sandbox)
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
            result.enforce_output_bounds();
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
        result.enforce_output_bounds();
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
        let mut result = self.result_of(
            &handle,
            ProcessResultKind::WriteStdin,
            None,
            true,
            MAX_OUTPUT_TOKENS,
            started,
        );
        result.enforce_output_bounds();
        Ok(result)
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

    /// 由共享状态构造结果（读取本次模型区间；最终输出边界由调用方在 `rtk` 过滤后统一施加）。
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
        // 终态且使用默认预算时读取完整保留缓冲（上限 256 KiB），交由后续 `rtk` 过滤与
        // `bound_model_output`（head + tail 50 KiB）裁剪，确保尾部退出摘要不会在过滤前被腰斩。
        let state = handle.state();
        let budget = if state.is_terminal() && max_tokens >= MAX_OUTPUT_TOKENS {
            MAX_PROCESS_OUTPUT_BYTES
        } else {
            max_tokens
                .max(1)
                .saturating_mul(4)
                .min(MAX_MODEL_OUTPUT_BYTES)
        };
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

/// PTY 固定尺寸（聊天卡片不做全屏终端仿真，窗格缩放不改变后台程序尺寸）。
pub(super) const PTY_COLS: u16 = 120;
pub(super) const PTY_ROWS: u16 = 40;

/// 短锁包装：中毒锁按内部数据继续使用（本模块不 panic 持有锁）。
pub(super) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod tests;

//! 进程 spawn 与 supervisor：管道两路 drain、PTY 读写线程、stdin 写入任务与停止升级。
//!
//! 拆成独立子模块的原因：这部分只与 OS/子进程句柄打交道，和「管理器状态与生命周期」
//! 是两类职责；单文件行数上限也要求分开。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};

use super::super::output::OutputSource;
use super::super::{ExecArgs, ProcessState};
use super::{
    PTY_COLS, PTY_ROWS, ProcessError, ProcessHandle, ProcessManager, STOP_GRACE, StdinRequest,
    StopReason, StopWaiter, exit_state, lock, spawn_pipe_reader,
};

impl ProcessManager {
    /// 普通管道 spawn：两路 drain + stdin 写入任务 + supervisor。
    pub(super) fn spawn_pipe(
        &self,
        handle: &Arc<ProcessHandle>,
        shell: &str,
        args: &ExecArgs,
        cwd: &Path,
    ) -> Result<(), ProcessError> {
        let mut command = tokio::process::Command::new(shell);
        // 登录 shell 与普通 shell 的启动参数不同：只影响环境与启动耗时。
        if args.login {
            command.arg("-l");
        }
        command
            .arg("-c")
            .arg(&args.cmd)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|error| {
            ProcessError::new("spawn_failed", format!("failed to start shell: {error}"))
        })?;
        let pid = child.id();
        // 记下组长 pid：shell 退出后要靠它探测进程组里是否还有后台作业。
        if let Some(pid) = pid {
            handle.set_pid(pid);
        }
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        if let Some(stdin) = stdin {
            self.spawn_stdin_task(handle, stdin);
        }
        if let Some(stdout) = stdout {
            spawn_pipe_reader(Arc::clone(handle), stdout, OutputSource::Stdout);
        }
        if let Some(stderr) = stderr {
            spawn_pipe_reader(Arc::clone(handle), stderr, OutputSource::Stderr);
        }
        self.spawn_pipe_supervisor(handle, child, pid);
        Ok(())
    }

    /// PTY spawn：固定 120×40，输出合流，独立读线程 + wait 线程。
    pub(super) fn spawn_pty(
        &self,
        handle: &Arc<ProcessHandle>,
        shell: &str,
        args: &ExecArgs,
        cwd: &Path,
    ) -> Result<(), ProcessError> {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: PTY_ROWS,
                cols: PTY_COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| {
                ProcessError::new("spawn_failed", format!("failed to open pty: {error}"))
            })?;
        let mut builder = CommandBuilder::new(shell);
        builder.env("TERM", "xterm-256color");
        builder.cwd(cwd);
        if args.login {
            builder.arg("-l");
        }
        builder.arg("-c");
        builder.arg(&args.cmd);
        let child = pair.slave.spawn_command(builder).map_err(|error| {
            ProcessError::new("spawn_failed", format!("failed to start shell: {error}"))
        })?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader().map_err(|error| {
            ProcessError::new("spawn_failed", format!("failed to read pty: {error}"))
        })?;
        let writer = pair.master.take_writer().map_err(|error| {
            ProcessError::new("spawn_failed", format!("failed to write pty: {error}"))
        })?;
        // PTY 读线程：持续 drain，避免子进程因满管道阻塞。
        {
            let handle = Arc::clone(handle);
            std::thread::Builder::new()
                .name(format!("lx-pty-read-{}", handle.session_id))
                .spawn(move || {
                    let mut reader = reader;
                    let mut buffer = [0u8; 8192];
                    loop {
                        match std::io::Read::read(&mut reader, &mut buffer) {
                            Ok(0) => break,
                            Ok(read) => {
                                handle.push_output(OutputSource::Pty, &buffer[..read]);
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| {
                    ProcessError::new("spawn_failed", format!("failed to spawn reader: {error}"))
                })?;
        }
        self.spawn_pty_stdin_task(handle, writer);
        self.spawn_pty_supervisor(handle, child);
        Ok(())
    }

    /// 管道 stdin 写入任务：拥有 `ChildStdin`，按请求写入并回报结果。
    fn spawn_stdin_task(&self, handle: &Arc<ProcessHandle>, stdin: tokio::process::ChildStdin) {
        let (sender, mut receiver) = mpsc::unbounded_channel::<StdinRequest>();
        let handle = Arc::clone(handle);
        // 同步登记：spawn 后立即调用，写线程稍后才可能需要它。
        let slot_handle = Arc::clone(&handle);
        *lock(&handle.stdin_slot) = Some(sender);
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(request) = receiver.recv().await {
                // 不自动补换行：模型显式给出 `\n` 才算提交。
                let result = match stdin.write_all(&request.bytes).await {
                    Ok(()) => stdin.flush().await,
                    Err(error) => Err(error),
                };
                let failed = result.is_err();
                let reported = result.map_err(|error| {
                    if error.kind() == std::io::ErrorKind::BrokenPipe {
                        format!("broken pipe: {error}")
                    } else {
                        error.to_string()
                    }
                });
                let _ = request.reply.send(reported);
                if failed {
                    // 写端已坏：后续请求立即失败，不再排队等待。
                    *lock(&slot_handle.stdin_slot) = None;
                    break;
                }
            }
            *lock(&slot_handle.stdin_slot) = None;
            let _ = handle.session_id;
        });
    }

    /// PTY stdin 写入任务：阻塞写放在专用线程，半写/BrokenPipe 回报。
    fn spawn_pty_stdin_task(
        &self,
        handle: &Arc<ProcessHandle>,
        writer: Box<dyn std::io::Write + Send>,
    ) {
        let (sender, mut receiver) = mpsc::unbounded_channel::<StdinRequest>();
        let handle_for_task = Arc::clone(handle);
        *lock(&handle.stdin_slot) = Some(sender);
        std::thread::Builder::new()
            .name(format!("lx-pty-write-{}", handle.session_id))
            .spawn(move || {
                let mut writer = writer;
                while let Some(request) = receiver.blocking_recv() {
                    let result = std::io::Write::write_all(&mut writer, &request.bytes)
                        .and_then(|()| std::io::Write::flush(&mut writer));
                    let failed = result.is_err();
                    let reported = result.map_err(|error| {
                        if error.kind() == std::io::ErrorKind::BrokenPipe {
                            format!("broken pipe: {error}")
                        } else {
                            error.to_string()
                        }
                    });
                    let _ = request.reply.send(reported);
                    if failed {
                        break;
                    }
                }
                let _ = handle_for_task.session_id;
            })
            .ok();
    }

    /// 管道 supervisor：等待退出或停止请求，按需温和/强制终止并 reap。
    fn spawn_pipe_supervisor(
        &self,
        handle: &Arc<ProcessHandle>,
        mut child: tokio::process::Child,
        pid: Option<u32>,
    ) {
        let (stop_sender, mut stop_receiver) = oneshot::channel::<StopReason>();
        if let Ok(mut guard) = handle.stop.try_lock() {
            *guard = Some(stop_sender);
        }
        let handle = Arc::clone(handle);
        tokio::spawn(async move {
            tokio::select! {
                status = child.wait() => {
                    let next = exit_state(status.ok().and_then(|status| status.code()));
                    handle.settle(next);
                }
                reason = &mut stop_receiver => {
                    let reason = reason.unwrap_or(StopReason::Stopped);
                    if let Some(pid) = pid {
                        crate::platform::terminate_process_group(pid, false);
                    }
                    let waited = tokio::time::timeout(STOP_GRACE, child.wait()).await;
                    if waited.is_err() {
                        if let Some(pid) = pid {
                            crate::platform::terminate_process_group(pid, true);
                        }
                        let _ = child.start_kill();
                        let _ = child.wait().await;
                    }
                    handle.settle(match reason {
                        StopReason::Stopped => ProcessState::Stopped,
                        StopReason::TimedOut => ProcessState::TimedOut,
                    });
                }
            }
        });
    }

    /// PTY supervisor：专用线程 `wait` 取真实退出状态，停止走 ChildKiller。
    fn spawn_pty_supervisor(
        &self,
        handle: &Arc<ProcessHandle>,
        mut child: Box<dyn portable_pty::Child + Send + Sync>,
    ) {
        let killer = child.clone_killer();
        let (stop_sender, mut stop_receiver) = oneshot::channel::<StopReason>();
        if let Ok(mut guard) = handle.stop.try_lock() {
            *guard = Some(stop_sender);
        }
        let handle = Arc::clone(handle);
        std::thread::Builder::new()
            .name(format!("lx-pty-wait-{}", handle.session_id))
            .spawn(move || {
                let mut waiter = StopWaiter::new(&mut stop_receiver);
                // 轮询 `try_wait` 以便同时响应停止请求（`wait` 不可中断）。
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            let code = i32::try_from(status.exit_code()).ok();
                            handle.settle(exit_state(code));
                            return;
                        }
                        Ok(None) => {}
                        Err(_) => {
                            handle.settle(ProcessState::Interrupted);
                            return;
                        }
                    }
                    if let Some(reason) = waiter.poll() {
                        let mut killer = killer;
                        let _ = killer.kill();
                        let state = match reason {
                            StopReason::Stopped => ProcessState::Stopped,
                            StopReason::TimedOut => ProcessState::TimedOut,
                        };
                        // 尽力 reap，避免僵尸进程；未确认时保留真实状态。
                        for _ in 0..100 {
                            match child.try_wait() {
                                Ok(Some(_)) => break,
                                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                                Err(_) => break,
                            }
                        }
                        handle.settle(state);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            })
            .ok();
    }
}

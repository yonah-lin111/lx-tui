//! 进程管理器测试：短命令、非零退出、等待窗口、跨轮复用、stdin、停止与限额。
//!
//! 全部用真实子进程（`sh -c`），但命令都是「立刻结束/短暂运行」的小命令：
//! 不依赖网络、不读写用户文件、不启动开发服务器。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::chat::process::{
    ExecArgs, MAX_STDIN_BYTES, ProcessManager, ProcessOwner, ProcessState, ProcessUiEvent,
    StdinArgs, preview_input,
};

/// 测试用的独立临时目录（进程 cwd 落在这里，drop 时删除）。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let sequence = std::sync::atomic::AtomicU64::new(0);
        let id = sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("lx-process-{}-{label}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// 构造一个绑定到临时工作区的管理器。
fn manager(dir: &TempDir) -> Arc<ProcessManager> {
    Arc::new(ProcessManager::new(
        ProcessOwner {
            workspace: dir.path.clone(),
            chat_session_id: "chat-1".into(),
            runtime_epoch: "epoch-1".into(),
        },
        None,
    ))
}

/// 命令行参数（默认 shell = /bin/sh，保证测试可移植到 POSIX 环境）。
fn exec(cmd: &str, yield_ms: u64) -> ExecArgs {
    ExecArgs {
        cmd: cmd.to_string(),
        workdir: None,
        shell: Some("/bin/sh".to_string()),
        login: false,
        tty: false,
        yield_time_ms: yield_ms,
        timeout_ms: None,
        max_output_tokens: MAX_OUTPUT_TOKENS,
    }
}

/// 交互参数（空写即只读输出）。
fn read_stdin(session_id: u64, yield_ms: u64) -> StdinArgs {
    StdinArgs {
        session_id,
        chars: String::new(),
        terminate: false,
        yield_time_ms: yield_ms,
        max_output_tokens: MAX_OUTPUT_TOKENS,
    }
}

#[tokio::test]
async fn short_command_returns_real_exit_code_and_output() {
    let dir = TempDir::new("short");
    let manager = manager(&dir);
    let result = manager
        .exec(exec("echo hello", 5_000), &dir.path, "call-1")
        .await
        .expect("exec");
    assert_eq!(result.session_id, 1, "进程会话 ID 不是 OS PID");
    assert_eq!(result.state, ProcessState::Exited { code: Some(0) });
    assert_eq!(result.exit_code, Some(0));
    assert!(!result.is_failure(), "退出码 0 不是失败");
    assert!(
        result.output.contains("hello"),
        "output: {:?}",
        result.output
    );
    assert!(result.command.contains("echo hello"));
}

/// 非零退出**不是**失败：命令跑完并给出了结果，退出码本身就是结果的一部分。
///
/// 真实案例：`pgrep -af ... ; ... | grep -c ...` 的 `grep -c` 在没有匹配时返回 1，
/// 这是"没找到"这个答案的正常表达，把它标成失败会让界面显示红叉。
#[tokio::test]
async fn non_zero_exit_is_a_result_not_a_failure() {
    let dir = TempDir::new("nonzero");
    let manager = manager(&dir);
    let result = manager
        .exec(exec("echo boom; exit 3", 5_000), &dir.path, "call-1")
        .await
        .expect("exec");
    assert_eq!(result.state, ProcessState::Exited { code: Some(3) });
    assert_eq!(result.exit_code, Some(3));
    assert!(!result.is_failure(), "非零退出不是工具失败");
    assert!(result.output.contains("boom"), "输出仍要如实返回");
    assert!(
        result.failure_message().is_none(),
        "非零退出不编造失败原因（退出码自己就是答案）"
    );
}

#[tokio::test]
async fn long_command_returns_running_session_and_stays_alive() {
    let dir = TempDir::new("long");
    let manager = manager(&dir);
    let result = manager
        .exec(
            exec("echo started; sleep 5; echo done", 300),
            &dir.path,
            "call-1",
        )
        .await
        .expect("exec");
    assert_eq!(result.state, ProcessState::Running, "等待窗口到期仍运行");
    assert!(result.output.contains("started"), "已产生的输出先返回");
    let session_id = result.session_id;
    // 等待窗口到期**不杀进程**：它仍在管理器里运行。
    let snapshot = manager.snapshot(session_id).expect("still tracked");
    assert!(!snapshot.state.is_terminal());
    manager.shutdown().await;
    let snapshot = manager.snapshot(session_id).expect("still tracked");
    assert!(snapshot.state.is_terminal(), "关闭后落终态");
}

#[tokio::test]
async fn same_session_id_can_be_read_across_turns() {
    let dir = TempDir::new("cross");
    let manager = manager(&dir);
    let first = manager
        .exec(
            exec("echo first; sleep 1; echo second", 200),
            &dir.path,
            "call-1",
        )
        .await
        .expect("exec");
    let session_id = first.session_id;
    // 第二轮：同一实例、同一 ID 读取后续输出；不重复启动命令。
    let second = manager
        .interact(read_stdin(session_id, 3_000))
        .await
        .expect("read further output");
    assert_eq!(second.session_id, session_id);
    assert!(
        second.output.contains("second"),
        "后续输出必须可读：{:?}",
        second.output
    );
    assert!(second.output_start >= first.output_end, "游标连续推进");
    manager.shutdown().await;
}

#[tokio::test]
async fn stdout_and_stderr_are_both_drained() {
    let dir = TempDir::new("both");
    let manager = manager(&dir);
    // 大量双流输出：任一管道不 drain 就会阻塞。
    let result = manager
        .exec(
            exec(
                "i=0; while [ $i -lt 200 ]; do echo out-$i; echo err-$i >&2; i=$((i+1)); done",
                10_000,
            ),
            &dir.path,
            "call-1",
        )
        .await
        .expect("exec");
    assert_eq!(result.state, ProcessState::Exited { code: Some(0) });
    assert!(result.output.contains("out-0"), "stdout 已 drain");
    assert!(result.output.contains("err-0"), "stderr 已 drain");
}

#[tokio::test]
async fn stdin_write_is_delivered_verbatim_without_newline() {
    let dir = TempDir::new("stdin");
    let manager = manager(&dir);
    let started = manager
        .exec(
            exec("read line; echo \"got:$line\"", 200),
            &dir.path,
            "call-1",
        )
        .await
        .expect("exec");
    assert_eq!(started.state, ProcessState::Running, "等待输入");
    let mut args = read_stdin(started.session_id, 5_000);
    args.chars = "answer\n".to_string();
    let result = manager.interact(args).await.expect("write stdin");
    assert_eq!(result.state, ProcessState::Exited { code: Some(0) });
    assert!(
        result.output.contains("got:answer"),
        "stdin 内容必须送达：{:?}",
        result.output
    );
    assert!(result.wrote_input, "非空 chars 视为写入");
    assert!(result.input.is_some(), "交互视图保留输入预览");
}

#[tokio::test]
async fn terminate_stops_a_running_process_and_is_idempotent() {
    let dir = TempDir::new("terminate");
    let manager = manager(&dir);
    let started = manager
        .exec(exec("echo up; sleep 30", 200), &dir.path, "call-1")
        .await
        .expect("exec");
    let session_id = started.session_id;
    let mut args = read_stdin(session_id, 200);
    args.terminate = true;
    let stopped = manager.interact(args).await.expect("terminate");
    assert_eq!(stopped.state, ProcessState::Stopped);
    assert!(stopped.terminate);
    assert!(stopped.output.contains("up"), "停止也返回已产生的输出");
    // 重复停止：只返回已知终态，不重发信号、不报错。
    let again = manager
        .interact(read_stdin(session_id, 100))
        .await
        .expect("again");
    assert_eq!(again.state, ProcessState::Stopped);
    manager.shutdown().await;
}

#[tokio::test]
async fn unknown_session_id_is_a_clear_error() {
    let dir = TempDir::new("unknown");
    let manager = manager(&dir);
    let error = manager
        .interact(read_stdin(999, 100))
        .await
        .expect_err("unknown id");
    assert_eq!(error.code, "process_not_found");
    manager.shutdown().await;
}

#[tokio::test]
async fn explicit_timeout_stops_the_command() {
    let dir = TempDir::new("timeout");
    let manager = manager(&dir);
    let mut args = exec("echo before; sleep 30", 20);
    args.timeout_ms = Some(150);
    let result = manager.exec(args, &dir.path, "call-1").await.expect("exec");
    assert!(
        result.output.contains("before"),
        "等待窗口内已产生的输出先返回"
    );
    // 等待窗口（20ms）先到期时工具返回 running；命令寿命（150ms）到期后由后台清理。
    tokio::time::sleep(Duration::from_millis(400)).await;
    let snapshot = manager.snapshot(result.session_id).expect("tracked");
    assert_eq!(snapshot.state, ProcessState::TimedOut, "超时后清理进程");
    let after = manager
        .interact(read_stdin(result.session_id, 100))
        .await
        .expect("read timed out process");
    assert!(after.is_failure(), "超时进程继续交互仍报失败状态");
    manager.shutdown().await;
}

#[tokio::test]
async fn empty_chars_only_reads_new_output() {
    let dir = TempDir::new("emptyread");
    let manager = manager(&dir);
    let started = manager
        .exec(exec("sleep 1; echo later", 100), &dir.path, "call-1")
        .await
        .expect("exec");
    let mut args = read_stdin(started.session_id, 3_000);
    args.chars = String::new();
    let result = manager.interact(args).await.expect("read");
    assert!(!result.wrote_input, "空写不写 stdin");
    assert!(result.input.is_none(), "空写没有输入预览");
    assert!(result.output.contains("later"), "仍能读到后续输出");
    manager.shutdown().await;
}

#[tokio::test]
async fn tty_mode_runs_in_a_dedicated_pty() {
    let dir = TempDir::new("pty");
    let manager = manager(&dir);
    let mut args = exec("echo pty-ok; exit 0", 5_000);
    args.tty = true;
    let result = manager
        .exec(args, &dir.path, "call-1")
        .await
        .expect("pty exec");
    assert!(result.tty);
    assert!(
        result.output.contains("pty-ok"),
        "output: {:?}",
        result.output
    );
    assert_eq!(result.exit_code, Some(0), "真实退出码来自 wait");
}

#[tokio::test]
async fn workdir_resolves_against_the_workspace() {
    let dir = TempDir::new("workdir");
    std::fs::create_dir_all(dir.path.join("sub")).expect("mkdir");
    let manager = manager(&dir);
    let mut args = exec("pwd", 5_000);
    args.workdir = Some("sub".to_string());
    let result = manager.exec(args, &dir.path, "call-1").await.expect("exec");
    assert!(
        result.output.contains("sub"),
        "cwd 必须落到子目录：{:?}",
        result.output
    );
    assert!(result.cwd.ends_with("sub"));
}

#[tokio::test]
async fn missing_workdir_is_rejected_before_spawning() {
    let dir = TempDir::new("badworkdir");
    let manager = manager(&dir);
    let mut args = exec("echo should-not-run", 100);
    args.workdir = Some("does-not-exist".to_string());
    let error = manager
        .exec(args, &dir.path, "call-1")
        .await
        .expect_err("bad workdir");
    assert_eq!(error.code, "invalid_arguments");
    assert_eq!(manager.running_count(), 0, "校验失败不产生进程");
}

#[tokio::test]
async fn running_limit_returns_resource_limit_without_killing_services() {
    let dir = TempDir::new("limit");
    let manager = Arc::new(ProcessManager::with_limits(
        ProcessOwner {
            workspace: dir.path.clone(),
            chat_session_id: "chat-limit".into(),
            runtime_epoch: "epoch-limit".into(),
        },
        None,
        ProcessLimits {
            max_running: 1,
            global_max_running: 32,
            max_finished: 32,
        },
    ));
    let _first = manager
        .exec(exec("sleep 30", 100), &dir.path, "call-1")
        .await
        .expect("first");
    let error = manager
        .exec(exec("echo second", 100), &dir.path, "call-1")
        .await
        .expect_err("limit");
    assert_eq!(error.code, "resource_limit");
    assert_eq!(manager.running_count(), 1, "已有服务不被自动杀掉");
    manager.shutdown().await;
}

#[tokio::test]
async fn output_events_fire_once_per_process_until_acknowledged() {
    let dir = TempDir::new("events");
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<ProcessUiEvent>();
    let manager = Arc::new(ProcessManager::new(
        ProcessOwner {
            workspace: dir.path.clone(),
            chat_session_id: "chat-events".into(),
            runtime_epoch: "epoch-events".into(),
        },
        Some(sender),
    ));
    let started = manager
        .exec(exec("echo tick; sleep 2", 150), &dir.path, "call-1")
        .await
        .expect("exec");
    let mut output_events = 0;
    while let Ok(event) = receiver.try_recv() {
        match event {
            ProcessUiEvent::Output { .. } => output_events += 1,
            ProcessUiEvent::Exited { .. } => {}
        }
    }
    assert_eq!(output_events, 1, "未确认前同类输出事件合并为一个");
    manager.acknowledge_output(started.session_id);
    let _ = manager
        .interact(read_stdin(started.session_id, 100))
        .await
        .expect("read");
    manager.shutdown().await;
}

#[tokio::test]
async fn stdin_limit_is_enforced_before_writing() {
    let dir = TempDir::new("stdinlimit");
    let manager = manager(&dir);
    let started = manager
        .exec(exec("cat", 100), &dir.path, "call-1")
        .await
        .expect("exec");
    let mut args = read_stdin(started.session_id, 100);
    args.chars = "x".repeat(MAX_STDIN_BYTES + 1);
    let error = manager.interact(args).await.expect_err("too large");
    assert_eq!(error.code, "too_large");
    manager.shutdown().await;
}

#[test]
fn input_preview_escapes_control_characters() {
    assert_eq!(preview_input("a\nb\tc\\d"), "a\\nb\\tc\\\\d");
    assert_eq!(preview_input("\u{3}"), "\\u{3}");
    let long = preview_input(&"x".repeat(200));
    assert!(long.ends_with('…'), "超长输入被截止并标记");
}

#[tokio::test]
async fn shutdown_reaps_every_process() {
    let dir = TempDir::new("shutdown");
    let manager = manager(&dir);
    let first = manager
        .exec(exec("sleep 30", 100), &dir.path, "call-1")
        .await
        .expect("first");
    let second = manager
        .exec(exec("sleep 30", 100), &dir.path, "call-1")
        .await
        .expect("second");
    assert_eq!(manager.running_count(), 2);
    manager.shutdown().await;
    for id in [first.session_id, second.session_id] {
        let snapshot = manager.snapshot(id).expect("tracked");
        assert!(snapshot.state.is_terminal(), "关闭后必须落终态");
    }
    assert_eq!(manager.running_count(), 0);
}

#[tokio::test]
async fn wait_window_does_not_kill_the_process() {
    let dir = TempDir::new("nowait");
    let manager = manager(&dir);
    let started = manager
        .exec(exec("sleep 3; echo done", 50), &dir.path, "call-1")
        .await
        .expect("exec");
    assert_eq!(started.state, ProcessState::Running);
    tokio::time::sleep(Duration::from_millis(120)).await;
    let snapshot = manager.snapshot(started.session_id).expect("tracked");
    assert!(!snapshot.state.is_terminal(), "等待窗口到期不杀进程");
    manager.shutdown().await;
}

/// 回归：`grep -c` / `pgrep` 无匹配返回 1，这是**答案**，不是失败。
///
/// 真实案例（design §14.7）：模型为确认"测试进程是否全部停止"执行
/// `pgrep -af "IFS= read" ; echo ---; ps -ef | grep -c "[r]ead -r line"`，
/// 输出 `35293 / --- / 0`，退出码 1（`pgrep` 无匹配）。界面把它渲染成红色
/// `exit 1` 失败卡片，而模型自己读懂了"退出码 1 表示未找到"。
#[tokio::test]
async fn no_match_grep_exit_code_one_is_a_normal_result() {
    let dir = TempDir::new("grep-no-match");
    let manager = manager(&dir);
    let result = manager
        .exec(
            exec("grep -c nomatch /dev/null; exit 1", 5_000),
            &dir.path,
            "call-1",
        )
        .await
        .expect("exec");
    assert_eq!(result.exit_code, Some(1), "退出码如实记录");
    assert!(
        !result.is_failure(),
        "grep 无匹配的 exit 1 是正常结果，不能标成失败"
    );
    assert!(result.failure_message().is_none(), "不编造失败原因");
    // 输出（这里是 `0`）照常返回给模型。
    assert_eq!(result.output.trim(), "0");
}

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
    // ID 由**进程内全局**计数器分配（跨会话唯一），所以不可能是固定值 1；
    // 这里只断言它确实是个「管理器分配的小整数」而不是 OS PID。
    assert!(
        result.session_id < 10_000,
        "进程会话 ID 是管理器分配的序号，不是 OS PID：{}",
        result.session_id
    );
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

// ===== 后台任务：shell 退了但作业还活着，必须仍算「在跑」 =====

/// 复刻用户真实场景：`nohup bash script &`，shell 立刻退出，后台作业继续跑。
///
/// 修复前：`snapshot.state` 是 `exited`，面板按「非终态」过滤 → 显示
/// `no running processes`，而任务其实活着（PID 还在、日志还在写）。
/// 修复后：`background_alive` 为 true，面板能列出它，`stop` 能停掉它。
#[tokio::test]
async fn detached_background_job_stays_visible_and_stoppable() {
    let dir = TempDir::new("detached-bg");
    let manager = manager(&dir);
    let script = dir.path.join("bg.sh");
    std::fs::write(&script, "#!/bin/bash\nwhile true; do sleep 5; done\n").expect("write script");
    let log = dir.path.join("bg.log");
    let args = ExecArgs {
        cmd: format!(
            "nohup bash {} > {} 2>&1 & echo LAUNCHED",
            script.display(),
            log.display()
        ),
        workdir: None,
        shell: Some("/bin/sh".to_string()),
        login: false,
        tty: false,
        yield_time_ms: 1500,
        timeout_ms: None,
        max_output_tokens: 1000,
    };
    let out = manager
        .exec(args, &dir.path, "call-detached")
        .await
        .expect("exec");
    // shell 自己退出了：这是 `nohup ... &` 的正常结果。
    assert_eq!(out.state, ProcessState::Exited { code: Some(0) });

    // 给后台作业一点时间去写日志并落稳。
    tokio::time::sleep(Duration::from_millis(600)).await;

    // 关键断言：快照必须报告「组里还有活体」。
    let snapshots = manager.snapshots();
    assert_eq!(snapshots.len(), 1);
    assert!(
        snapshots[0].background_alive,
        "shell 已退出但后台作业仍在跑，background_alive 必须为 true：{:?}",
        snapshots[0]
    );

    // 面板的准入条件（与 list_session_processes 同一判据）。
    let listed = snapshots
        .iter()
        .filter(|snapshot| !snapshot.state.is_terminal() || snapshot.background_alive)
        .count();
    assert_eq!(listed, 1, "后台任务必须出现在面板里");

    // 停止必须真的杀掉后台作业，尽管 lx 的直接子进程早已退出。
    manager.stop(snapshots[0].session_id).await.expect("stop");
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        !manager.snapshots()[0].background_alive,
        "stop 之后进程组不该再有存活成员"
    );
}

/// 纯前景命令不该被误判成「还有后台作业」。
#[tokio::test]
async fn plain_foreground_command_reports_no_living_group() {
    let dir = TempDir::new("fg-only");
    let manager = manager(&dir);
    let args = ExecArgs {
        cmd: "echo done".to_string(),
        workdir: None,
        shell: Some("/bin/sh".to_string()),
        login: false,
        tty: false,
        yield_time_ms: 2000,
        timeout_ms: None,
        max_output_tokens: 1000,
    };
    let out = manager
        .exec(args, &dir.path, "call-fg")
        .await
        .expect("exec");
    assert_eq!(out.state, ProcessState::Exited { code: Some(0) });
    tokio::time::sleep(Duration::from_millis(200)).await;
    let snapshots = manager.snapshots();
    assert!(
        !snapshots[0].background_alive,
        "前景命令结束后进程组应已空：{:?}",
        snapshots[0]
    );
    manager.shutdown().await;
}

// ===== review 回归：生命周期与计数的所有权 =====

#[tokio::test]
async fn failed_spawn_never_underflows_the_global_running_counter() {
    // 真实缺陷：spawn 失败时 settle() 无条件递减 GLOBAL_RUNNING，而该 handle 从未
    // fetch_add 过 → u64 下溢成 u64::MAX → 此后所有 exec_command 都返回
    // resource_limit，且整个进程生命周期内不可恢复。
    let dir = TempDir::new("bad-shell");
    let manager = manager(&dir);
    let mut args = exec("echo hi", 1_000);
    args.shell = Some("/definitely/not/a/shell".to_string());
    assert!(manager.exec(args, &dir.path, "call-bad").await.is_err());
    // 下溢的唯一症状是计数变成一个巨大的数（u64::MAX 附近）。
    assert!(
        crate::chat::process::manager::global_running() < u64::MAX / 2,
        "全局运行计数被下溢了：{}",
        crate::chat::process::manager::global_running()
    );
}

#[tokio::test]
async fn a_short_lived_command_never_stays_running() {
    // 真实缺陷：登记后无条件写回 `Running`，会把 supervisor 已落定的 `Exited`
    // 覆盖掉 —— 进程永远显示运行中、全局计数永久 +1。秒退命令最容易撞上。
    let dir = TempDir::new("instant");
    let manager = manager(&dir);
    for _ in 0..20 {
        let result = manager
            .exec(exec("true", 5_000), &dir.path, "call-fast")
            .await
            .expect("exec");
        assert!(
            result.state.is_terminal(),
            "秒退命令必须是终态，实际 {:?}",
            result.state
        );
        // 快照也必须收敛（面板与卡片读的是它）。
        let snapshot = manager.snapshot(result.session_id).expect("snapshot");
        assert!(
            snapshot.state.is_terminal(),
            "快照状态也必须终态，实际 {:?}",
            snapshot.state
        );
    }
}

#[tokio::test]
async fn process_ids_are_unique_across_managers() {
    // 真实缺陷：id 每管理器从 1 起算，两个并存会话的第一个进程都叫 1，
    // 事件层按 id 反查归属时会串台（并用别人的 epoch 写坏库里的行）。
    let dir = TempDir::new("ids");
    let first = manager(&dir);
    let second = manager(&dir);
    let left = first
        .exec(exec("true", 5_000), &dir.path, "a")
        .await
        .expect("exec");
    let right = second
        .exec(exec("true", 5_000), &dir.path, "b")
        .await
        .expect("exec");
    assert_ne!(
        left.session_id, right.session_id,
        "不同管理器的进程 id 必须互不相同"
    );
}

#[tokio::test]
async fn tail_snapshot_tracks_the_newest_output_past_the_budget() {
    // 真实缺陷：tail_text() 从保留区间**起点**读 64 KiB，输出超过预算后
    // 卡片与面板预览永远停在最早那一段。
    let dir = TempDir::new("tail");
    let manager = manager(&dir);
    // 输出量超过 64 KiB 预算，末尾放一个可识别的标记。
    let result = manager
        .exec(
            exec(
                "yes 0123456789abcdef | head -c 80000; echo TAIL-MARKER",
                30_000,
            ),
            &dir.path,
            "call-tail",
        )
        .await
        .expect("exec");
    // 输出量超过 token 预算时 `exec_command` 会先返回 `running`（设计如此），
    // 因此这里只轮询快照收敛，不断言终态。
    let _ = result;
    // 读端 drain 与快照读取之间没有强序：重试到快照收敛为止。
    let mut tail = String::new();
    for _ in 0..200 {
        tail = manager.snapshot(result.session_id).expect("snapshot").tail;
        if tail.contains("TAIL-MARKER") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        tail.contains("TAIL-MARKER"),
        "快照必须是最新输出的尾部，实际尾部 {:?}",
        &tail[tail.len().saturating_sub(48)..]
    );
}

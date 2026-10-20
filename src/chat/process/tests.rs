//! 进程结果形状与「后台任务是否真跑起来」判定的单元测试。
//!
//! 这些是纯数据测试（不 spawn 子进程）：只验证 `ProcessResult` 上的判定逻辑与
//! 结果 JSON 形状。真实子进程行为在 `manager/tests.rs`。

use super::*;

/// 构造一个已退出的 `exec_command` 结果。
fn exited_result(command: &str, tty: bool) -> ProcessResult {
    ProcessResult {
        kind: ProcessResultKind::ExecCommand,
        session_id: 1,
        command: command.to_string(),
        cwd: "/w".to_string(),
        tty,
        state: ProcessState::Exited { code: Some(0) },
        exit_code: Some(0),
        output: String::new(),
        output_start: 0,
        output_end: 0,
        dropped_bytes: 0,
        truncated: false,
        input: None,
        wrote_input: false,
        terminate: false,
        chunk_id: "abcdef".to_string(),
        wall_time_ms: 0,
    }
}

/// 真实故障：模型用 `setsid ... &` 起常驻任务，shell 返回 0 却什么都没启动。
///
/// 提示必须出现，且要指向可执行的验证手段，而不是只说"可能失败了"。
#[test]
fn background_command_that_already_exited_is_flagged() {
    let result = exited_result("setsid bash -c 'while true; do sleep 30; done' &", false);
    let warning = result.background_job_warning().expect("应给出后台任务提示");
    assert!(
        warning.contains("pgrep"),
        "要给出可执行的验证手段：{warning}"
    );
    assert!(
        warning.contains("cannot tell"),
        "不夸大结论：lx 确实无法追踪脱离进程组的后台作业：{warning}"
    );
}

/// `tty: true` 时点名 SIGHUP：这是用户实际踩到的那个坑。
#[test]
fn tty_background_job_warning_names_sighup() {
    let result = exited_result("sleep 300 & echo PID:$!", true);
    let warning = result.background_job_warning().expect("应给出提示");
    assert!(warning.contains("SIGHUP"), "要点明原因：{warning}");
    assert!(warning.contains("nohup"), "要给出可用写法：{warning}");
}

/// 前景命令不该被误报：`&&`、`&>`、`2>&1` 都不是后台作业。
#[test]
fn foreground_commands_are_not_flagged() {
    for command in [
        "cargo build && cargo test",
        "cargo test >out.txt 2>&1",
        "make &> build.log",
        "grep -rn foo src",
    ] {
        let result = exited_result(command, false);
        assert_eq!(
            result.background_job_warning(),
            None,
            "前景命令不该被误报：{command}"
        );
    }
}

/// 仍在运行的进程不需要提示：它本来就在背景任务面板里。
#[test]
fn running_process_is_not_flagged() {
    let mut result = exited_result("sleep 300 &", false);
    result.state = ProcessState::Running;
    result.exit_code = None;
    assert_eq!(result.background_job_warning(), None);
}

/// `write_stdin` 结果不重复提示：第一次 `exec_command` 已经说过了。
#[test]
fn write_stdin_result_is_not_flagged() {
    let mut result = exited_result("sleep 300 &", false);
    result.kind = ProcessResultKind::WriteStdin;
    assert_eq!(result.background_job_warning(), None);
}

/// 结果 JSON 带 `warning` 键；前景命令为 null（形状稳定，不随判定变化）。
#[test]
fn result_json_carries_a_stable_warning_key() {
    let flagged = exited_result("sleep 300 &", false).to_json();
    assert!(
        flagged["warning"].is_string(),
        "后台作业要带提示：{flagged}"
    );
    let clean = exited_result("echo hi", false).to_json();
    assert!(clean["warning"].is_null(), "前景命令为 null：{clean}");
}

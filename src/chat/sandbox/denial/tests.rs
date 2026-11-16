//! 沙箱拒绝识别：关键词、退出码排除与状态过滤。

use super::*;
use crate::chat::process::{ProcessResult, ProcessResultKind, ProcessState};

fn result(state: ProcessState, output: &str) -> ProcessResult {
    ProcessResult {
        kind: ProcessResultKind::ExecCommand,
        session_id: 1,
        command: "touch ~/x".into(),
        cwd: "/ws".into(),
        tty: false,
        state: state.clone(),
        exit_code: match state {
            ProcessState::Exited { code } => code,
            _ => None,
        },
        output: output.into(),
        output_start: 0,
        output_end: 0,
        dropped_bytes: 0,
        truncated: false,
        input: None,
        wrote_input: false,
        terminate: false,
        chunk_id: "000001".into(),
        sandbox_retry: None,
        wall_time_ms: 1,
    }
}

#[test]
fn denial_keywords_with_nonzero_exit_are_detected() {
    assert!(is_likely_sandbox_denied(
        &result(
            ProcessState::Exited { code: Some(1) },
            "touch: Operation not permitted"
        ),
        true
    ));
    assert!(is_likely_sandbox_denied(
        &result(
            ProcessState::Exited { code: Some(1) },
            "sandbox denied write"
        ),
        true
    ));
}

#[test]
fn success_missing_command_and_usage_errors_are_not_denials() {
    for code in [0, 2, 126, 127] {
        assert!(!is_likely_sandbox_denied(
            &result(
                ProcessState::Exited { code: Some(code) },
                "permission denied"
            ),
            true
        ));
    }
    assert!(!is_likely_sandbox_denied(
        &result(ProcessState::Exited { code: Some(1) }, "no such file"),
        true
    ));
}

#[test]
fn non_exited_states_and_unsandboxed_commands_are_ignored() {
    for state in [
        ProcessState::TimedOut,
        ProcessState::Stopped,
        ProcessState::Interrupted,
        ProcessState::Running,
    ] {
        assert!(!is_likely_sandbox_denied(
            &result(state, "operation not permitted"),
            true
        ));
    }
    assert!(!is_likely_sandbox_denied(
        &result(
            ProcessState::Exited { code: Some(1) },
            "operation not permitted"
        ),
        false
    ));
}

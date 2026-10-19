//! 类型化视图测试：从进程结果/结果 JSON 生成卡片数据，历史重载与结算同源。

use super::*;
use crate::chat::process::{ProcessResult, ProcessResultKind, ProcessState, preview_input};

/// 一个运行中的命令结果。
fn running_result() -> ProcessResult {
    ProcessResult {
        kind: ProcessResultKind::ExecCommand,
        session_id: 42,
        command: "cargo test".into(),
        cwd: "/workspace".into(),
        tty: false,
        state: ProcessState::Running,
        exit_code: None,
        output: "running 1 test\n".into(),
        output_start: 0,
        output_end: 16,
        dropped_bytes: 0,
        truncated: false,
        input: None,
        wrote_input: false,
        terminate: false,
    }
}

#[test]
fn command_view_carries_command_state_and_preview() {
    let view = ToolDetailView::from_process(&running_result());
    let ToolDetailView::Command(command) = view else {
        panic!("expected command view");
    };
    assert_eq!(command.process_session_id, Some(42));
    assert_eq!(command.command, "cargo test");
    assert_eq!(command.cwd, "/workspace");
    assert!(command.state.is_running());
    assert_eq!(command.output_preview, vec!["running 1 test"]);
    assert!(!command.state.is_failure());
}

#[test]
fn non_zero_exit_marks_the_view_as_failure() {
    let mut result = running_result();
    result.state = ProcessState::Exited { code: Some(2) };
    result.exit_code = Some(2);
    let view = ToolDetailView::from_process(&result);
    let ToolDetailView::Command(command) = view else {
        panic!("expected command view");
    };
    assert!(command.state.is_failure(), "非零退出在卡片上必须显示为失败");
    assert_eq!(command.state.exit_code(), Some(2));
}

#[test]
fn write_stdin_view_distinguishes_read_send_and_stop() {
    let mut result = running_result();
    result.kind = ProcessResultKind::WriteStdin;

    // 空写：只读输出。
    let ToolDetailView::Interaction(read) = ToolDetailView::from_process(&result) else {
        panic!("expected interaction");
    };
    assert_eq!(read.action, InteractionAction::ReadOutput);
    assert!(read.input_preview.is_none());

    // 非空写：发送输入（输入预览已转义）。
    result.wrote_input = true;
    result.input = Some(preview_input("yes\n"));
    let ToolDetailView::Interaction(send) = ToolDetailView::from_process(&result) else {
        panic!("expected interaction");
    };
    assert_eq!(send.action, InteractionAction::SendInput);
    assert_eq!(send.input_preview.as_deref(), Some("yes\\n"));

    // 显式停止：Stop command。
    result.wrote_input = false;
    result.input = None;
    result.terminate = true;
    let ToolDetailView::Interaction(stop) = ToolDetailView::from_process(&result) else {
        panic!("expected interaction");
    };
    assert_eq!(stop.action, InteractionAction::StopCommand);
}

#[test]
fn result_json_round_trip_matches_the_live_view() {
    let result = running_result();
    let json = result.to_json().to_string();
    let restored = ToolDetailView::from_result_json(Some(&json)).expect("reload");
    assert_eq!(restored, ToolDetailView::from_process(&result));
}

#[test]
fn plan_result_json_round_trip_preserves_steps_and_revision() {
    let snapshot = crate::chat::plan::PlanSnapshot {
        explanation: Some("note".into()),
        steps: vec![
            crate::chat::plan::PlanStep {
                step: "done".into(),
                status: crate::chat::plan::PlanStepStatus::Completed,
            },
            crate::chat::plan::PlanStep {
                step: "doing".into(),
                status: crate::chat::plan::PlanStepStatus::InProgress,
            },
        ],
        revision: 5,
    };
    let json = snapshot.to_json().to_string();
    let view = ToolDetailView::from_result_json(Some(&json)).expect("plan view");
    let ToolDetailView::Plan(plan) = view else {
        panic!("expected plan view");
    };
    assert_eq!(plan.revision, 5);
    assert_eq!(plan.completed(), 1);
    assert_eq!(plan.steps[1].1, PlanStepView::InProgress);
    assert_eq!(plan.explanation.as_deref(), Some("note"));
}

#[test]
fn foreign_or_broken_result_json_yields_no_view() {
    assert!(ToolDetailView::from_result_json(None).is_none());
    assert!(ToolDetailView::from_result_json(Some("not json")).is_none());
    assert!(ToolDetailView::from_result_json(Some(r#"{"tool":"read","content":"x"}"#)).is_none());
    assert!(ToolDetailView::from_result_json(Some(r#"{"tool":"exec_command"}"#)).is_none());
}

#[test]
fn preview_shows_the_last_lines_only() {
    let output: String = (1..=20).map(|index| format!("line {index}\n")).collect();
    let preview = preview_lines(&output);
    assert_eq!(preview.len(), COMMAND_PREVIEW_LINES);
    assert_eq!(preview.first().map(String::as_str), Some("line 15"));
    assert_eq!(preview.last().map(String::as_str), Some("line 20"));
}

#[test]
fn preview_omits_the_trailing_blank_line() {
    let preview = preview_lines("a\nb\n");
    assert_eq!(preview, vec!["a", "b"]);
}

#[test]
fn empty_plan_view_is_clear_marker() {
    let snapshot = crate::chat::plan::PlanSnapshot {
        explanation: None,
        steps: Vec::new(),
        revision: 1,
    };
    let view = PlanSnapshotView::from_snapshot(&snapshot);
    assert!(view.is_empty());
    assert_eq!(view.completed(), 0);
}

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
        chunk_id: "abc123".into(),
        wall_time_ms: 5,
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
    assert!(!command.state.is_process_failure());
}

#[test]
fn non_zero_exit_is_not_a_process_failure() {
    // 非零退出是命令跑完后的正常结果（`grep` 无匹配 = 1、`diff` 有差异 = 1），
    // 不能把它当成进程失败——否则界面会把一次成功的查询标成红叉。
    let mut result = running_result();
    result.state = ProcessState::Exited { code: Some(1) };
    result.exit_code = Some(1);
    let view = ToolDetailView::from_process(&result);
    let ToolDetailView::Command(command) = view else {
        panic!("expected command view");
    };
    assert!(
        !command.state.is_process_failure(),
        "非零退出不是进程级失败"
    );
    assert!(command.state.is_nonzero_exit(), "但仍然记录着非零退出");
    assert_eq!(command.state.exit_code(), Some(1), "退出码照常可见");
}

#[test]
fn timeout_and_interrupt_stay_process_failures() {
    // 真正的进程级失败必须仍然算失败，不能被上一条放宽掉。
    for state in [
        ProcessState::TimedOut,
        ProcessState::Interrupted,
        ProcessState::Failed {
            code: "spawn_failed".into(),
        },
    ] {
        let mut result = running_result();
        result.state = state.clone();
        let view = ToolDetailView::from_process(&result);
        let ToolDetailView::Command(command) = view else {
            panic!("expected command view");
        };
        assert!(command.state.is_process_failure(), "{state:?} 必须仍是失败");
    }
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
    // 窗口贴着输出的末尾（最后一条一定是末行）。
    assert_eq!(preview.last().map(String::as_str), Some("line 20"));
    assert_eq!(
        preview.first().map(String::as_str),
        Some(format!("line {}", 21 - COMMAND_PREVIEW_LINES).as_str()),
        "预览是尾部窗口，条数由 COMMAND_PREVIEW_LINES 决定"
    );
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

/// `ask` 的实时视图与历史重载逐字段相等（沿用 roundtrip 风格）。
#[test]
fn ask_result_json_round_trip_matches_the_live_view() {
    let questions = vec![crate::chat::tools::AnsweredQuestion {
        id: "layout".into(),
        header: "布局".into(),
        question: "侧栏用哪种布局？".into(),
        answers: vec!["双栏".into(), "自己画一版".into()],
    }];
    let result = crate::chat::tools::ToolExecutionResult::Ask {
        questions: questions.clone(),
    };
    let json = crate::chat::tools::to_result_json(&result).expect("json");
    let live = ToolDetailView::Ask(AskDetailView::from_answered(&questions));
    assert_eq!(
        ToolDetailView::from_result_json(Some(&json)).expect("reload"),
        live
    );
}

/// `ask` 结果缺字段或不是 ask 时不给视图（旧记录兜底）。
#[test]
fn ask_result_json_with_missing_fields_yields_no_view() {
    assert!(ToolDetailView::from_result_json(Some(r#"{"tool":"ask"}"#)).is_none());
    assert!(
        ToolDetailView::from_result_json(Some(
            r#"{"tool":"ask","questions":[{"id":"q","answers":[]}]}"#
        ))
        .is_none(),
        "缺 header/question 必须降级，而不是伪造空题干"
    );
    assert!(
        ToolDetailView::from_result_json(Some(
            r#"{"tool":"ask","questions":[{"id":"q","header":"h","question":"q","answers":[1]}]}"#
        ))
        .is_none(),
        "answer 非字符串必须降级"
    );
}

/// 空答案（异常数据）仍能解码，交由渲染层容错。
#[test]
fn ask_view_keeps_empty_answers() {
    let json =
        r#"{"tool":"ask","questions":[{"id":"q","header":"h","question":"q?","answers":[]}]}"#;
    let Some(ToolDetailView::Ask(view)) = ToolDetailView::from_result_json(Some(json)) else {
        panic!("expected ask view");
    };
    assert_eq!(view.questions.len(), 1);
    assert!(view.questions[0].answers.is_empty());
    assert_eq!(view.questions[0].header, "h");
}

#[test]
fn search_result_json_restores_match_lines() {
    let json = r#"{
        "tool": "search",
        "matches": [
            {"path": "src/foo.rs", "line": 12, "text": "pub fn render() {}", "line_truncated": false},
            {"path": "src/bar.rs", "line": 7, "text": "render", "line_truncated": true}
        ],
        "truncated": true,
        "files_scanned": 2
    }"#;
    let Some(ToolDetailView::Search(view)) = ToolDetailView::from_result_json(Some(json)) else {
        panic!("search detail");
    };
    assert_eq!(view.matches.len(), 2);
    assert_eq!(view.matches[0].path, "src/foo.rs");
    assert_eq!(view.matches[0].line, 12);
    assert_eq!(view.matches[0].text, "pub fn render() {}");
    assert!(view.matches[1].line_truncated);
    assert!(view.truncated);
    assert_eq!(view.files_scanned, 2);
}

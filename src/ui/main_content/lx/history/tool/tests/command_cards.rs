//! 进程卡片与计划卡片的渲染测试：宽/窄/零宽、中文宽字符、无输出、展开与几何一致。
//!
//! 断言口径与文件工具一致：`rows` 与 `draw` 推进行数必须相等（否则滚动条、
//! 置顶行与命中矩形都会漂移）。

use ratatui::layout::Rect;

use crate::app::state::{
    CommandView, InteractionAction, PlanSnapshotView, PlanStepView, ProcessInteractionView,
    ProcessStateView, ToolCallView, ToolDetailView, ToolStatus,
};
use crate::ui::main_content::lx::grid::Grid;
use crate::ui::main_content::lx::history::tool::{draw, rows};

#[allow(unused_imports)]
use super::*;

/// 构造一个运行中的命令卡片（六行以上输出）。
fn running_command(output: &str) -> ToolCallView {
    ToolCallView {
        call_id: "call-1".into(),
        name: "exec_command".into(),
        position: 0,
        arguments_json: r#"{"cmd":"cargo test"}"#.into(),
        status: ToolStatus::Running,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::Command(CommandView {
            process_session_id: Some(12),
            command: "cargo test chat::tools".into(),
            cwd: "/workspace/lx".into(),
            tty: false,
            state: ProcessStateView::Running,
            output_preview: Vec::new(),
            truncated: false,
            dropped_bytes: 0,
            output_tail: output.to_string(),
            started_at_ms: Some(crate::db::now_ms()),
            ended_at_ms: None,
        }))),
    }
}

/// 交互卡片。
fn interaction(output: &str, input: Option<&str>) -> ToolCallView {
    ToolCallView {
        call_id: "call-2".into(),
        name: "write_stdin".into(),
        position: 0,
        arguments_json: r#"{"session_id":12}"#.into(),
        status: ToolStatus::Completed,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::Interaction(
            ProcessInteractionView {
                action: if input.is_some() {
                    InteractionAction::SendInput
                } else {
                    InteractionAction::ReadOutput
                },
                process_session_id: 12,
                command: "cargo test chat::tools".into(),
                input_preview: input.map(str::to_string),
                output: output.to_string(),
                state: ProcessStateView::Running,
                exit_code: None,
                truncated: false,
                dropped_bytes: 0,
            },
        ))),
    }
}

/// 计划卡片。
fn plan(steps: Vec<(&str, PlanStepView)>) -> ToolCallView {
    ToolCallView {
        call_id: "call-3".into(),
        name: "update_plan".into(),
        position: 0,
        arguments_json: "{}".into(),
        status: ToolStatus::Completed,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::Plan(PlanSnapshotView {
            revision: 1,
            explanation: Some("checking behavior".into()),
            steps: steps
                .into_iter()
                .map(|(text, status)| (text.to_string(), status))
                .collect(),
        }))),
    }
}

/// 渲染卡片并返回（整块文本，推进行数）。
///
/// 宽度 0 时网格不可用（`Grid` 要求非零尺寸），仍调用 `draw` 验证「不推进不 panic」。
fn render(call: &ToolCallView, width: u16, expanded: bool) -> (String, usize) {
    let height = rows(call, width, expanded).max(1) as u16;
    let rect = Rect::new(0, 0, width, height);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, call, 0, expanded);
    let mut lines: Vec<String> = Vec::new();
    for y in rect.y..rect.bottom() {
        let mut line = String::new();
        for x in rect.x..rect.right() {
            let glyph = grid.cell(x, y);
            if glyph.ch != crate::ui::main_content::lx::grid::CONTINUATION {
                line.push(glyph.ch);
            }
        }
        lines.push(line.trim_end().to_string());
    }
    (lines.join("\n"), row_index)
}

#[test]
fn rows_and_draw_agree_for_every_width_and_state() {
    let long_output: String = (0..40).map(|index| format!("line {index}\n")).collect();
    let cases = vec![
        running_command(&long_output),
        running_command(""),
        interaction("hello\n", None),
        interaction("", Some("yes\\n")),
        plan(vec![
            ("one", PlanStepView::Completed),
            ("two", PlanStepView::InProgress),
            ("three", PlanStepView::Pending),
        ]),
    ];
    for call in &cases {
        for width in [0u16, 1, 2, 4, 10, 20, 40, 120] {
            for expanded in [false, true] {
                let (_, advanced) = render(call, width, expanded);
                let expected = rows(call, width, expanded);
                // 宽度 0 不占行、也不推进（外层布局已跳过该块）。
                assert_eq!(
                    advanced, expected,
                    "{} 宽 {width} 展开 {expanded}：绘制行数必须等于 rows",
                    call.name
                );
            }
        }
    }
}

#[test]
fn running_command_card_shows_command_status_and_preview() {
    let output = "running 24 tests\ntest read::empty_file ... ok\ntest result: ok\n";
    let (text, _) = render(&running_command(output), 60, false);
    assert!(text.contains("Run command"), "标题动词：{text}");
    assert!(
        text.contains("cargo test chat::tools"),
        "命令在详情行：{text}"
    );
    assert!(text.contains("Running"), "状态文字必须可见：{text}");
    assert!(text.contains("test result: ok"), "输出预览：{text}");
    assert!(!text.contains("cwd:"), "折叠时不显示 cwd：{text}");
}

#[test]
fn expanded_command_card_shows_cwd_and_full_tail() {
    let output: String = (0..10).map(|index| format!("line {index}\n")).collect();
    let (folded, _) = render(&running_command(&output), 60, false);
    assert!(!folded.contains("line 0"), "折叠只显示最近六行：{folded}");
    assert!(folded.contains("line 9"));
    let (expanded, _) = render(&running_command(&output), 60, true);
    assert!(
        expanded.contains("cwd: /workspace/lx"),
        "展开显示 cwd：{expanded}"
    );
    assert!(
        expanded.contains("line 0"),
        "展开显示全部保留输出：{expanded}"
    );
}

#[test]
fn command_without_output_shows_a_placeholder() {
    let (text, _) = render(&running_command(""), 60, false);
    assert!(
        text.contains("Waiting for output"),
        "无输出必须显式占位：{text}"
    );
}

#[test]
fn interaction_card_distinguishes_read_send_and_empty_output() {
    let (read, _) = render(&interaction("later output\n", None), 60, false);
    assert!(read.contains("Read output"), "空写是 Read output：{read}");
    assert!(read.contains("later output"));

    let (send, _) = render(&interaction("", Some("yes\\n")), 60, false);
    assert!(send.contains("Send input"), "非空写是 Send input：{send}");
    assert!(send.contains("stdin: yes\\n"), "输入预览转义换行：{send}");
    assert!(send.contains("No new output"), "无输出占位：{send}");
}

#[test]
fn plan_card_renders_three_markers_and_count() {
    let call = plan(vec![
        ("Implement command tools", PlanStepView::Completed),
        ("Verify process cleanup", PlanStepView::InProgress),
        ("Review terminal display", PlanStepView::Pending),
    ]);
    let (text, _) = render(&call, 70, false);
    assert!(text.contains("[x] Implement command tools"), "{text}");
    assert!(text.contains("[>] Verify process cleanup"), "{text}");
    assert!(text.contains("[ ] Review terminal display"), "{text}");
    assert!(
        text.contains("1/3 completed"),
        "完成计数只认 completed：{text}"
    );
    assert!(text.contains("checking behavior"), "解释行：{text}");
}

#[test]
fn empty_plan_card_says_cleared() {
    let (text, _) = render(&plan(Vec::new()), 50, false);
    assert!(text.contains("Plan cleared"), "{text}");
    assert!(!text.contains("completed"), "清空不显示计数：{text}");
}

#[test]
fn wide_characters_and_long_steps_wrap_without_breaking_geometry() {
    let call = plan(vec![(
        "执行命令工具并验证进程清理与终端显示的中文步骤名称",
        PlanStepView::InProgress,
    )]);
    for width in [10u16, 20, 30] {
        let (text, advanced) = render(&call, width, false);
        assert_eq!(advanced, rows(&call, width, false), "中文换行后几何仍一致");
        assert!(!text.contains('\u{fffd}'), "不得按字节切断宽字符：{text}");
    }
}

#[test]
fn long_command_is_truncated_with_an_ellipsis() {
    let mut call = running_command("");
    if let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
        view.command = "x".repeat(400);
    }
    let (text, _) = render(&call, 40, false);
    assert!(text.contains('…'), "超长命令尾部省略：{text}");
    assert!(!text.contains(&"x".repeat(80)), "不应整段铺开");
}

#[test]
fn zero_width_card_draws_nothing() {
    let call = running_command("output\n");
    assert_eq!(rows(&call, 0, false), 0);
    let rect = Rect::new(0, 0, 0, 1);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, &call, 0, false);
    assert_eq!(row_index, 0, "宽度 0 不推进任何行");
}

#[test]
fn failed_exit_is_visible_on_the_card() {
    let mut call = running_command("boom\n");
    if let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
        view.state = ProcessStateView::Exited { code: Some(2) };
        view.ended_at_ms = Some(crate::db::now_ms());
    }
    let (text, _) = render(&call, 60, false);
    assert!(
        text.contains("exit 2"),
        "非零退出必须显示真实退出码：{text}"
    );
}

#[test]
fn header_shows_details_or_collapse_marker() {
    let call = running_command("out\n");
    let (folded, _) = render(&call, 60, false);
    assert!(
        folded.contains(crate::ui::text::agent::DETAILS_OPEN),
        "折叠时显示 [details]：{folded}"
    );
    let (expanded, _) = render(&call, 60, true);
    assert!(
        expanded.contains(crate::ui::text::agent::DETAILS_CLOSE),
        "展开时显示 [collapse]：{expanded}"
    );
    // 极窄：标记被省略，但方块与动词仍在（不挤掉动词）。
    let (narrow, _) = render(&call, 14, false);
    assert!(narrow.contains('■'), "极窄保留方块：{narrow:?}");
    assert!(narrow.contains("Run"), "极窄保留动词：{narrow:?}");
}

#[test]
fn details_marker_text_is_ascii_and_short() {
    // 详情按钮文案为纯 ASCII，窄卡片也不会变成乱码。
    for label in [
        crate::ui::text::agent::DETAILS_OPEN,
        crate::ui::text::agent::DETAILS_CLOSE,
    ] {
        assert!(label.is_ascii());
        assert!(!label.is_empty());
    }
}

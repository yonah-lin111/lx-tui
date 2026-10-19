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

// ===== 树形版式（`│` 中间行 + `└─` 收口）=====

#[test]
fn command_body_uses_a_tree_with_a_single_leaf_at_the_end() {
    let output = "aaa\nbbb\nccc\nddd\n";
    let (text, _) = render(&running_command(output), 60, false);
    let lines: Vec<&str> = text.lines().collect();
    // 第 1 行是标题，其余都是树体。
    assert!(lines[0].starts_with('■'), "第 1 行是标题：{text}");
    for line in &lines[1..lines.len() - 1] {
        assert!(
            line.starts_with(text::agent::TREE_BAR),
            "中间行都以竖线开头：{line:?}"
        );
    }
    assert!(
        lines[lines.len() - 1].starts_with(text::agent::TREE_LEAF),
        "最后一行用直角收口：{text}"
    );
    // 只出现一次收口——树干不会分叉成多根。
    assert_eq!(
        text.matches(text::agent::TREE_LEAF).count(),
        1,
        "整张卡片只有一根树干：{text}"
    );
}

#[test]
fn tree_prefixes_are_equal_width_so_content_stays_aligned() {
    // `  │  ` 与 `  └─ ` 必须等宽，否则正文会左右错位（视觉上树干断裂）。
    assert_eq!(
        text::agent::TREE_BAR.width(),
        text::agent::TREE_LEAF.width(),
        "竖线与收口前缀必须等宽"
    );
    let (text, _) = render(&running_command("out\n"), 60, false);
    let content_columns: Vec<usize> = text
        .lines()
        .skip(1)
        .map(|line| {
            let prefix = if line.starts_with(text::agent::TREE_LEAF) {
                text::agent::TREE_LEAF
            } else {
                text::agent::TREE_BAR
            };
            line.strip_prefix(prefix).map(str::len).unwrap_or(0)
        })
        .collect();
    assert!(
        content_columns.iter().all(|cols| *cols > 0),
        "每行都有正文：{text}"
    );
}

#[test]
fn folded_preview_states_how_many_lines_are_hidden() {
    let output: String = (0..10).map(|index| format!("line {index}\n")).collect();
    let (text, _) = render(&running_command(&output), 60, false);
    assert!(
        text.contains(text::agent::EARLIER_OUTPUT_HINT),
        "折叠必须显式说明省略了多少行：{text}"
    );
    assert!(text.contains("7 more lines"), "10 行留 3 行 = 省 7：{text}");
    // 展开后不再需要这条提示（全部输出都在）。
    let (expanded, _) = render(&running_command(&output), 60, true);
    assert!(
        !expanded.contains(text::agent::EARLIER_OUTPUT_HINT),
        "展开后不再提示省略：{expanded}"
    );
}

#[test]
fn command_line_has_a_shell_prompt_marker() {
    let (text, _) = render(&running_command("out\n"), 60, false);
    assert!(
        text.contains(&format!(
            "{}cargo test chat::tools",
            text::agent::COMMAND_PROMPT_PREFIX
        )),
        "命令行带 `$ ` 提示符：{text}"
    );
}

#[test]
fn interaction_body_keys_input_before_output_under_the_same_tree() {
    let (text, _) = render(&interaction("new line\n", Some("yes\\n")), 60, false);
    let lines: Vec<&str> = text.lines().collect();
    let stdin_line = lines
        .iter()
        .position(|line| line.contains("stdin: yes\\n"))
        .expect("输入预览在树体里");
    let output_line = lines
        .iter()
        .position(|line| line.contains("new line"))
        .expect("输出在树体里");
    assert!(stdin_line < output_line, "先输入后输出：{text}");
    assert!(
        lines[stdin_line].starts_with(text::agent::TREE_BAR),
        "输入行是中间行：{text}"
    );
    assert!(
        lines[output_line].starts_with(text::agent::TREE_LEAF),
        "输出行收口：{text}"
    );
}

#[test]
fn plan_steps_use_the_tree_and_leaf_lands_on_the_explanation() {
    let call = plan(vec![
        ("one", PlanStepView::Completed),
        ("two", PlanStepView::Pending),
    ]);
    let (text, _) = render(&call, 60, false);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[1].starts_with(text::agent::TREE_BAR) && lines[1].contains("[x] one"),
        "步骤是中间行：{text}"
    );
    assert!(
        lines[2].starts_with(text::agent::TREE_BAR) && lines[2].contains("[ ] two"),
        "有解释时步骤不收口：{text}"
    );
    assert!(
        lines[3].starts_with(text::agent::TREE_LEAF) && lines[3].contains("checking behavior"),
        "解释行收口：{text}"
    );
    // 没有解释时，最后一条步骤自己收口（树干照样只有一根）。
    let mut without = call.clone();
    if let Some(ToolDetailView::Plan(view)) = without.detail.as_deref_mut() {
        view.explanation = None;
    }
    let (text, _) = render(&without, 60, false);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[lines.len() - 1].starts_with(text::agent::TREE_LEAF),
        "无解释时最后一条步骤收口：{text}"
    );
    assert_eq!(text.matches(text::agent::TREE_LEAF).count(), 1, "{text}");
}

#[test]
fn empty_plan_also_closes_with_a_leaf() {
    let (text, _) = render(&plan(Vec::new()), 50, false);
    let last = text.lines().last().expect("至少两行");
    assert!(
        last.starts_with(text::agent::TREE_LEAF),
        "空计划同样用直角收口：{text}"
    );
}

#[test]
fn running_process_keeps_breathing_even_after_the_tool_settled() {
    // 命令已经交回（工具 `completed`），但进程仍在后台跑：方块必须继续呼吸。
    let mut call = running_command("out\n");
    call.status = ToolStatus::Completed;
    let rect = Rect::new(0, 0, 60, 30);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, &call, 0, false);
    let filled = grid.cell(rect.x, rect.y);
    assert_eq!(
        filled.ch,
        text::agent::STEP_MARK_FILLED,
        "运行中的进程在第一相位是实心方块"
    );
    // 第二相位翻空心：仍在跑 → 继续呼吸。
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        &call,
        text::agent::STEP_BREATH_TICKS,
        false,
    );
    assert_eq!(
        grid.cell(rect.x, rect.y).ch,
        text::agent::STEP_MARK_HOLLOW,
        "运行中的进程跟随呼吸相位，不因工具已结算而静止"
    );
}

#[test]
fn finished_process_stops_breathing() {
    let mut call = running_command("out\n");
    call.status = ToolStatus::Completed;
    if let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
        view.state = ProcessStateView::Exited { code: Some(0) };
    }
    let rect = Rect::new(0, 0, 60, 30);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        &call,
        text::agent::STEP_BREATH_TICKS,
        false,
    );
    assert_eq!(
        grid.cell(rect.x, rect.y).ch,
        text::agent::STEP_MARK_FILLED,
        "已退出的进程停为实心方块（不随相位变化）"
    );
}

/// 折叠预览行数只有一个口径：视图快照常量与渲染常量必须相等。
///
/// 两处若漂移，卡片会「预览 3 行、快照存 6 行」——摘要与绘制说的不是同一件事。
#[test]
fn preview_line_budget_has_a_single_source_of_truth() {
    use super::super::command::PREVIEW_LINES;
    assert_eq!(
        PREVIEW_LINES,
        crate::app::state::lx::chat::tool_view::COMMAND_PREVIEW_LINES,
        "渲染层预览行数必须与视图常量一致"
    );
}

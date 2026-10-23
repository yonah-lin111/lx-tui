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
    let height = rows(call, width, folds(expanded)).max(1) as u16;
    let rect = Rect::new(0, 0, width, height);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, call, 0, folds(expanded));
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
                let expected = rows(call, width, folds(expanded));
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
fn running_command_card_shows_only_the_command_and_guide() {
    // 长任务：命令 + 一行引导；输出一律不进消息列表（完整日志在面板里）。
    let output = "running 24 tests\ntest read::empty_file ... ok\ntest result: ok\n";
    let (text, _) = render(&running_command(output), 60, false);
    assert!(text.contains("Run command"), "标题动词：{text}");
    assert!(
        text.contains("cargo test chat::tools"),
        "命令在树体首行：{text}"
    );
    assert!(text.contains("Running"), "状态文字必须可见：{text}");
    assert!(!text.contains("test result: ok"), "不渲染任何输出：{text}");
    assert!(!text.contains("cwd:"), "不再渲染 cwd：{text}");
    assert!(
        text.contains(crate::ui::text::agent::TASK_OPEN_GUIDE),
        "末行给出打开面板的引导：{text}"
    );
    assert_eq!(text.lines().count(), 3, "标题 + 命令 + 引导：{text}");
}

#[test]
fn running_card_never_prints_output_rows() {
    // 运行中的卡片只留引导行：输出只在后台任务面板里。
    let output: String = (0..10).map(|index| format!("line {index}\n")).collect();
    let (text, _) = render(&running_command(&output), 60, false);
    assert!(!text.contains("line 9"), "不渲染输出：{text}");
    assert!(!text.contains("line 0"), "不渲染输出：{text}");
    assert!(!text.contains("… +"), "运行中不给省略提示：{text}");
}

#[test]
fn command_without_output_has_no_placeholder() {
    // 尚无输出时不画「等待中」占位（那是本地命令卡片的口径）；只留命令 + 引导。
    let (text, _) = render(&running_command(""), 60, false);
    assert!(!text.contains("Waiting for output"), "无输出占位：{text}");
    assert!(
        text.contains(crate::ui::text::agent::TASK_OPEN_GUIDE),
        "引导行始终在：{text}"
    );
    assert!(
        !text.lines().any(|line| line.trim_end().ends_with(" open")),
        "{text}"
    );
}

#[test]
fn interaction_card_distinguishes_verbs_and_hides_io() {
    // 动词仍按动作区分（Read output / Send input），但 stdin 与输出都不再上卡片。
    let (read, _) = render(&interaction("later output\n", None), 60, false);
    assert!(read.contains("Read output"), "空写是 Read output：{read}");
    assert!(!read.contains("later output"), "输出只在面板里看：{read}");

    let (send, _) = render(&interaction("", Some("yes\\n")), 60, false);
    assert!(send.contains("Send input"), "非空写是 Send input：{send}");
    assert!(!send.contains("stdin:"), "输入预览不再上卡片：{send}");
    for text in [read, send] {
        assert!(
            !text.lines().any(|line| line.trim_end().ends_with(" open")),
            "交互卡片也没有 open 入口行：{text}"
        );
    }
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
        assert_eq!(
            advanced,
            rows(&call, width, folds(false)),
            "中文换行后几何仍一致"
        );
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
    assert_eq!(rows(&call, 0, folds(false)), 0);
    let rect = Rect::new(0, 0, 0, 1);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, &call, 0, folds(false));
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
fn header_shows_status_inline_and_has_no_corner_details_marker() {
    let call = running_command("out\n");
    let (folded, _) = render(&call, 60, false);
    // 状态紧贴动词、以 ` · ` 分隔，且**不在行尾**（右上角已没有 [details]）。
    assert!(
        folded.contains(&format!(
            "Run command{}Running",
            text::LX_CHAT_TOOL_STATUS_SEP
        )),
        "状态紧跟动词：{folded}"
    );
    assert!(
        !folded.contains(text::agent::FILE_DIFF_COLLAPSE),
        "进程卡片没有展开态，不该出现 collapse：{folded}"
    );
    // 极窄：状态整段丢弃，但方块与动词仍在（不挤掉动词）。
    let (narrow, _) = render(&call, 14, false);
    assert!(narrow.contains('■'), "极窄保留方块：{narrow:?}");
    assert!(narrow.contains("Run"), "极窄保留动词：{narrow:?}");
    assert!(!narrow.contains("Running"), "极窄整项丢弃状态：{narrow:?}");
}

#[test]
fn running_card_guide_is_not_wrapped_by_the_tree() {
    // 运行中的树体：`└─` 只收口参数（命令），引导行用等宽空白对齐、不再被树干包围。
    let call = running_command("line\n");
    let (text, _) = render(&call, 60, false);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with('■'), "首行是标题：{text}");
    assert!(
        lines[1].starts_with(text::agent::TREE_LEAF),
        "命令行是参数区的收口行：{text}"
    );
    assert!(
        lines.last().is_some_and(|line| {
            line.starts_with(text::agent::TREE_PAD) && line.contains(text::agent::TASK_OPEN_GUIDE)
        }),
        "引导行只缩进、不画连接符：{text}"
    );
    assert_eq!(
        text.matches(text::agent::TREE_LEAF).count(),
        1,
        "整张卡片只有一根树干：{text}"
    );
}

#[test]
fn command_prompt_marker_is_ascii_and_short() {
    let label = text::agent::COMMAND_PROMPT_PREFIX;
    assert!(label.is_ascii());
    assert!(!label.is_empty());
}

// ===== 树形版式（`│` 中间行 + `└─` 收口）=====

#[test]
fn leaf_closes_the_params_block_and_output_is_not_wrapped() {
    let output = "aaa\nbbb\nccc\nddd\n";
    let (text, _) = render(&running_command(output), 60, false);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].starts_with('■'), "第 1 行是标题：{text}");
    // 参数区收口：命令只有一行，它就是 `└─`。
    assert!(
        lines[1].starts_with(text::agent::TREE_LEAF),
        "参数区最后一行收口：{text}"
    );
    // 输出/引导区块不画树：只用等宽空白对齐到内容列。
    for line in &lines[2..] {
        assert!(
            line.starts_with(text::agent::TREE_PAD),
            "输出区不被 `│` / `└─` 包裹：{line:?}"
        );
    }
    assert_eq!(
        text.matches(text::agent::TREE_LEAF).count(),
        1,
        "整张卡片只有一根树干：{text}"
    );
}

#[test]
fn tree_never_mixes_branch_symbols() {
    // 用户反馈：一段输出里同时混 `├─` / `│` / `└─` 比原来更花。
    // 卡片只用两种连接符：中间行 `│`、末行 `└─`，不出现 `├─`。
    let (text, _) = render(&running_command("out\nsecond\n"), 60, false);
    assert!(
        !text.contains(text::agent::TREE_BRANCH),
        "不应再出现 `├─`：{text}"
    );
    // `│` 与 `└` 落在同一列，树干连成一条线。
    let bar_col = text::agent::TREE_BAR
        .find(text::agent::MARKER_TREE_BAR)
        .expect("`│` 位置");
    let leaf_col = text::agent::TREE_LEAF.find('└').expect("`└` 位置");
    assert_eq!(bar_col, leaf_col, "`│` 与 `└` 必须同列：{text}");
}

#[test]
fn tree_prefixes_are_equal_width_so_content_stays_aligned() {
    // `  │  ` / `  └─ ` / 空白前缀必须等宽，否则正文会左右错位（视觉上树干断裂）。
    for prefix in [text::agent::TREE_LEAF, text::agent::TREE_PAD] {
        assert_eq!(
            text::agent::TREE_BAR.width(),
            prefix.width(),
            "树体与空白前缀必须等宽"
        );
    }
    let (text, _) = render(&running_command("out\n"), 60, false);
    let content_columns: Vec<usize> = text
        .lines()
        .skip(1)
        .map(|line| {
            let prefix = [
                text::agent::TREE_LEAF,
                text::agent::TREE_BAR,
                text::agent::TREE_PAD,
            ]
            .into_iter()
            .find(|prefix| line.starts_with(prefix))
            .unwrap_or(text::agent::TREE_PAD);
            line.strip_prefix(prefix).map(str::len).unwrap_or(0)
        })
        .collect();
    assert!(
        content_columns.iter().all(|cols| *cols > 0),
        "每行都有正文：{text}"
    );
}

#[test]
fn running_card_never_shows_output_or_ellipsis_hint() {
    // 「还有多少行没显示」由面板回答：卡片只画命令与引导，不画输出与省略提示。
    let output: String = (0..40).map(|index| format!("line {index}\n")).collect();
    let (text, _) = render(&running_command(&output), 60, false);
    assert!(!text.contains("… +"), "运行中没有 `… +N` 省略提示：{text}");
    assert!(!text.contains("more lines"), "不再提 more lines：{text}");
    assert!(!text.contains("line 39"), "输出不上卡片：{text}");
    assert!(!text.contains("line 0"), "输出不上卡片：{text}");
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
fn interaction_card_keeps_only_the_command_row() {
    // 树体只剩命令：stdin 预览与输出都在面板里看；不再有 `open` 入口行。
    let (text, _) = render(&interaction("new line\n", Some("yes\\n")), 60, false);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "标题 + 命令：{text}");
    let command_line = lines
        .iter()
        .position(|line| line.contains("cargo test chat::tools"))
        .expect("命令在树体首行");
    assert!(command_line > 0, "命令在标题行之下：{text}");
    assert!(
        lines[command_line].starts_with(text::agent::TREE_LEAF),
        "命令行用直角收口：{text}"
    );
    assert!(!text.contains("stdin:"), "stdin 预览不再上卡片：{text}");
    assert!(!text.contains("new line"), "输出不再上卡片：{text}");
    assert!(
        !text.lines().any(|line| line.trim_end().ends_with(" open")),
        "没有 open 入口行：{text}"
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
    draw(&mut grid, rect, 0, &mut row_index, &call, 0, folds(false));
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
        folds(false),
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
        folds(false),
    );
    assert_eq!(
        grid.cell(rect.x, rect.y).ch,
        text::agent::STEP_MARK_FILLED,
        "已退出的进程停为实心方块（不随相位变化）"
    );
}

// ===== 非零退出不应被画成失败（真实案例回归，design §14.7）=====

/// 构造一个已退出的命令卡片（指定退出码与工具状态）。
fn exited_command(code: i32, status: ToolStatus, error: Option<&str>) -> ToolCallView {
    let mut call = running_command("35293\n---\n0\n");
    call.status = status;
    call.error = error.map(str::to_string);
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() else {
        panic!("expected command view");
    };
    view.state = ProcessStateView::Exited { code: Some(code) };
    view.ended_at_ms = Some(crate::db::now_ms());
    call
}

/// 首行方块的前景色（行首标记）。
fn mark_style(call: &ToolCallView) -> ratatui::style::Style {
    let area = Rect::new(0, 0, 100, 12);
    let mut grid = Grid::new(area);
    let mut row = 0usize;
    draw(&mut grid, area, 0, &mut row, call, 0, folds(false));
    grid.cell(area.x, 0).style
}

#[test]
fn non_zero_exit_keeps_the_tool_color_instead_of_red() {
    // `grep -c` 无匹配返回 1、`pgrep` 无匹配返回 1：这是答案，不是失败。
    // 工具状态是 Completed，方块必须保持工具色（不是红色）。
    let call = exited_command(1, ToolStatus::Completed, None);
    let style = mark_style(&call);
    assert_ne!(
        style.fg,
        Some(ratatui::style::Color::Red),
        "非零退出不得画成红色失败方块"
    );
    // 退出码仍然显示在标题行上（信息不丢）。
    let text = render_row(&call, 100).0;
    assert!(text.contains("exit 1"), "退出码仍可见：{text}");
    assert!(!text.contains("Failed"), "不显示 Failed：{text}");
}

#[test]
fn timeout_still_renders_the_red_failure_mark() {
    // 真正的失败（超时）必须仍然是红的，不能被上一条放宽掉。
    let mut call = exited_command(1, ToolStatus::Failed, Some("Timed out"));
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() else {
        panic!("expected command view");
    };
    view.state = ProcessStateView::TimedOut;
    let style = mark_style(&call);
    assert_eq!(
        style.fg,
        Some(ratatui::style::Color::Red),
        "超时仍必须是红色"
    );
}

// ===== 消息列表不再显示运行时长（时长只在后台任务面板）=====

#[test]
fn running_card_shows_status_without_elapsed_time() {
    // 用户要求：移除消息列表里的时长，只留状态词；时长改由后台任务面板定时刷新。
    let call = running_command("out\n");
    let (text, _) = render(&call, 80, false);
    assert!(text.contains("Running"), "状态词仍然可见：{text}");
    // 状态紧跟动词，但不拼接任何时长（`Running · 12s` 这类形态）。
    assert!(
        text.contains(&format!(
            "Run command{}Running",
            text::LX_CHAT_TOOL_STATUS_SEP
        )),
        "状态以 · 分隔后紧跟动词：{text}"
    );
    assert!(!text.contains("Running ·"), "状态后不得再跟时长：{text}");
    assert!(!text.contains("12s"), "不应出现时长：{text}");
}

#[test]
fn finished_card_shows_exit_code_without_elapsed_time() {
    let mut call = running_command("out\n");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() else {
        panic!("expected command view");
    };
    view.state = ProcessStateView::Exited { code: Some(0) };
    view.started_at_ms = Some(1);
    view.ended_at_ms = Some(200_000);
    let (text, _) = render(&call, 80, false);
    assert!(text.contains("Exited 0"), "终态显示退出码：{text}");
    assert!(!text.contains("Exited 0 ·"), "退出码后不得跟时长：{text}");
    assert!(!text.contains("3m 19s"), "不应出现时长：{text}");
}

// ===== 末尾展开/收起行的宽度让位（整项丢弃，不截半段）=====

#[test]
fn finished_process_card_has_no_guide_and_no_ellipsis() {
    // 终态：不再有打开面板的引导（面板是给「还在跑」的东西看的），
    // 改为命令 + 输出首尾预览，行数与 `rows()` 一致。
    let mut call = running_command("out\n");
    let Some(ToolDetailView::Command(view)) = call.detail.as_deref_mut() else {
        panic!("expected command view");
    };
    view.state = ProcessStateView::Exited { code: Some(0) };
    let (text, advanced) = render(&call, 60, false);
    assert_eq!(advanced, rows(&call, 60, folds(false)), "几何口径一致");
    assert!(!text.contains(text::agent::TASK_OPEN_GUIDE), "{text}");
    assert!(
        !text.lines().any(|line| line.trim_end().ends_with(" open")),
        "{text}"
    );
    assert!(text.contains("out"), "终态保留输出预览：{text}");
    assert!(
        text.lines()
            .last()
            .is_some_and(|line| line.starts_with(text::agent::TREE_PAD)),
        "输出区不画树、只用空白对齐：{text}"
    );
}

// ===== review 回归：计划卡片的行数口径必须与绘制一致 =====

#[test]
fn empty_plan_with_an_explanation_keeps_rows_and_draw_in_step() {
    // 真实缺陷：`rows()` 无条件把解释行算进去，而 `draw()` 在空计划分支画完
    // `Plan cleared` 就 return，永不画解释。多算的 1 行会让该卡片之后的所有内容
    // 与命中矩形整体错位一行。模型发 `{steps: [], explanation: "..."}`（收尾
    // 清空计划时很常见）就会触发。
    let call = ToolCallView {
        call_id: "call-empty-plan".into(),
        name: "update_plan".into(),
        position: 0,
        arguments_json: "{}".into(),
        status: ToolStatus::Completed,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::Plan(PlanSnapshotView {
            revision: 1,
            steps: Vec::new(),
            explanation: Some("wrapped up".into()),
        }))),
    };
    for width in [0u16, 1, 2, 10, 40, 120] {
        let (_, advanced) = render(&call, width, false);
        assert_eq!(
            advanced,
            rows(&call, width, folds(false)),
            "空计划 + 解释：宽 {width} 的绘制行数必须等于 rows"
        );
    }
}

#[test]
fn plan_row_indices_advance_even_when_rows_scroll_out_of_view() {
    // 真实缺陷：解释行的 `row_index += 1` 被写进了 `visible_row` 分支内，
    // 滚出视口时少加一行，其后所有内容都比 `layout` 认为的位置高一行。
    // 这里把滚动偏移推到卡片之外，验证推进行数仍与 `rows()` 相等。
    let call = plan(vec![
        ("one", PlanStepView::Completed),
        ("two", PlanStepView::InProgress),
    ]);
    let width = 40u16;
    let expected = rows(&call, width, folds(false));
    for scroll in [0usize, 1, 2, 5, 50] {
        let area = Rect::new(0, 0, width, 1);
        let mut grid = Grid::new(area);
        let mut row = 0usize;
        draw(&mut grid, area, scroll, &mut row, &call, 0, folds(false));
        assert_eq!(
            row, expected,
            "scroll={scroll} 时推进行数必须与 rows 相等（命中靠它）"
        );
    }
}

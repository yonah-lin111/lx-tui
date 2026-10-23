//! 后台任务卡片新形态的回归测试：橙色存活态、动画停止后实心、运行中实时尾部与整卡引导。
//!
//! 用户反馈的两组现象：
//! 1. 运行中的后台卡片是绿的，且会定格成镂空 `□`（看起来像失败），但进程其实还在跑；
//! 2. 卡片曾把命令、cwd、输出、`… +N more lines` 全画在消息列表里，把历史撑得很长。
//!    现在输出只保留受控预览：运行中给实时尾部 + 打开面板的引导，终态给首尾预览。

use ratatui::layout::Rect;
use ratatui::style::Color;

use super::folds;
use crate::app::state::{
    CommandView, PlanStepView, ProcessInteractionView, ProcessStateView, ToolCallView,
    ToolDetailView, ToolStatus,
};
use crate::ui::main_content::lx::grid::Grid;
use crate::ui::main_content::lx::history::{StepMark, tool};
use crate::ui::style;
use crate::ui::text;

/// 构造一条命令卡片。
fn command_call(state: ProcessStateView, status: ToolStatus, output: &str) -> ToolCallView {
    ToolCallView {
        call_id: "call-alive".into(),
        name: "exec_command".into(),
        position: 0,
        arguments_json: r#"{"cmd":"serve"}"#.into(),
        status,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::Command(CommandView {
            process_session_id: Some(7),
            command: "while true; do echo heartbeat; sleep 10; done".into(),
            cwd: "/workspace/lx".into(),
            tty: false,
            state,
            output_preview: Vec::new(),
            truncated: false,
            dropped_bytes: 0,
            output_tail: output.to_string(),
            started_at_ms: Some(1),
            ended_at_ms: None,
        }))),
    }
}

/// 构造一条交互卡片。
fn interaction_call(state: ProcessStateView, status: ToolStatus) -> ToolCallView {
    ToolCallView {
        call_id: "call-stdin".into(),
        name: "write_stdin".into(),
        position: 0,
        arguments_json: r#"{"session_id":7}"#.into(),
        status,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::Interaction(
            ProcessInteractionView {
                action: crate::app::state::InteractionAction::ReadOutput,
                process_session_id: 7,
                command: "while true; do echo heartbeat; sleep 10; done".into(),
                input_preview: None,
                output: "heartbeat\n".into(),
                state,
                exit_code: None,
                truncated: false,
                dropped_bytes: 0,
            },
        ))),
    }
}

/// 构造一条计划卡片（`update_plan`）。
fn plan_call(steps: Vec<(String, PlanStepView)>) -> ToolCallView {
    ToolCallView {
        call_id: "call-plan".into(),
        name: "update_plan".into(),
        position: 0,
        arguments_json: r#"{"steps":[]}"#.into(),
        status: ToolStatus::Running,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(ToolDetailView::Plan(
            crate::app::state::PlanSnapshotView {
                revision: 1,
                steps,
                explanation: None,
            },
        ))),
    }
}

/// 渲染并返回网格（用于取颜色）。
fn render_grid(call: &ToolCallView, width: u16) -> Grid {
    let height = tool::rows(call, width, folds(false)).max(1) as u16;
    let area = Rect::new(0, 0, width, height);
    let mut grid = Grid::new(area);
    let mut row = 0usize;
    tool::draw(&mut grid, area, 0, &mut row, call, 0, folds(false));
    grid
}

/// 渲染并返回整块文本。
fn render_text(call: &ToolCallView, width: u16) -> String {
    let grid = render_grid(call, width);
    let area = Rect::new(
        0,
        0,
        width,
        tool::rows(call, width, folds(false)).max(1) as u16,
    );
    let mut lines = Vec::new();
    for y in area.y..area.bottom() {
        let mut line = String::new();
        for x in area.x..area.right() {
            let glyph = grid.cell(x, y);
            if glyph.ch != crate::ui::main_content::lx::grid::CONTINUATION {
                line.push(glyph.ch);
            }
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

/// 命令行（树体中段 y = 1）里**命令正文**的样式：跳到 `$ ` 提示符之后取色，
/// 跳过树干的 `│` 连接符（那是 dim 灰，不属于命令正文）。
fn command_row_style(grid: &Grid, width: u16) -> Option<ratatui::style::Style> {
    let mut seen_prompt = false;
    for x in 0..width {
        let cell = grid.cell(x, 1);
        let glyph = cell.ch;
        if glyph == crate::ui::main_content::lx::grid::CONTINUATION || glyph == ' ' {
            continue;
        }
        if !seen_prompt {
            if glyph
                == text::agent::COMMAND_PROMPT_PREFIX
                    .chars()
                    .next()
                    .unwrap_or('$')
            {
                seen_prompt = true;
            }
            continue;
        }
        return Some(cell.style);
    }
    None
}

// ===== 决策 1：存活期为深黄，停止后恢复绿色 =====

#[test]
fn alive_command_card_is_orange_and_stopped_card_is_green() {
    let alive = command_call(ProcessStateView::Running, ToolStatus::Completed, "beat\n");
    let mark = render_grid(&alive, 60).cell(0, 0).style;
    assert_eq!(
        mark.fg,
        style::chat_step_mark_alive().fg,
        "进程仍存活时方块必须是橙色：{mark:?}"
    );
    assert_ne!(
        mark.fg,
        style::chat_step_mark_done().fg,
        "存活期不得沿用完成的翡翠绿"
    );

    let stopped = command_call(ProcessStateView::Stopped, ToolStatus::Completed, "beat\n");
    let mark = render_grid(&stopped, 60).cell(0, 0).style;
    assert_eq!(
        mark.fg,
        style::chat_step_mark_done().fg,
        "进程停止后必须恢复绿色：{mark:?}"
    );
}

#[test]
fn command_row_keeps_the_param_colour_in_every_state() {
    // 用户要求「参数颜色不要改变，颜色变化始终是小卡片和 title」：
    // 命令行（参数）**恒为终端默认前景**，运行中与停止后完全一致。
    let running = command_call(ProcessStateView::Running, ToolStatus::Completed, "beat\n");
    let running_style =
        command_row_style(&render_grid(&running, 70), 70).expect("命令行必须画出来");
    assert_eq!(
        running_style.fg,
        style::chat_tool_param().fg,
        "运行中命令行用默认字体色：{running_style:?}"
    );

    let stopped = command_call(ProcessStateView::Stopped, ToolStatus::Completed, "beat\n");
    let stopped_style =
        command_row_style(&render_grid(&stopped, 70), 70).expect("命令行必须画出来");
    assert_eq!(
        running_style.fg, stopped_style.fg,
        "运行态变化不得影响参数色"
    );

    // 变的是**标题行**：方块与动词从橙黄（运行）切到翡翠绿（完成）。
    let running_mark = render_grid(&running, 70).cell(0, 0).style;
    let stopped_mark = render_grid(&stopped, 70).cell(0, 0).style;
    assert_eq!(running_mark.fg, style::chat_step_mark_alive().fg);
    assert_eq!(stopped_mark.fg, style::chat_step_mark_done().fg);
    assert_ne!(running_mark.fg, stopped_mark.fg, "标题档位必须变化");
}

#[test]
fn alive_interaction_card_is_deep_yellow_too() {
    let alive = interaction_call(ProcessStateView::Running, ToolStatus::Completed);
    let mark = render_grid(&alive, 60).cell(0, 0).style;
    assert_eq!(
        mark.fg,
        style::chat_step_mark_alive().fg,
        "交互卡片同样按进程存活染橙：{mark:?}"
    );
}

#[test]
fn running_file_tool_cards_are_amber_too() {
    // 用户要求「所有正在运行中的 tool 的方块和 title 统一为黄色」：文件工具
    // 运行中同样是橙黄，不再单独保持绿色。
    let call = ToolCallView {
        call_id: "call-read".into(),
        name: "read".into(),
        position: 0,
        arguments_json: r#"{"path":"src/main.rs"}"#.into(),
        status: ToolStatus::Running,
        summary: None,
        error: None,
        result_json: None,
        detail: None,
    };
    let mark = render_grid(&call, 60).cell(0, 0).style;
    assert_eq!(
        mark.fg,
        style::chat_step_mark_alive().fg,
        "运行中的文件工具同样是橙黄：{mark:?}"
    );
    assert_ne!(mark.fg, style::chat_step_mark_done().fg, "不再用完成态的绿");
}

// ===== 决策 2、3：动画停止时恒实心，不再定格镂空 =====

#[test]
fn alive_mark_never_renders_hollow_when_the_animation_stops() {
    // 用户截图里的 `□ Run command · Running`：流一停，相位就冻在空心那一相。
    // 动画停止（agent 不再输出）时，任何相位都必须画实心。
    let call = command_call(ProcessStateView::Running, ToolStatus::Completed, "beat\n");
    for frame in 0..12usize {
        let grid = {
            let height = tool::rows(&call, 60, folds(false)).max(1) as u16;
            let area = Rect::new(0, 0, 60, height);
            let mut grid = Grid::new(area);
            let mut row = 0usize;
            // animating = false：动画已停止。
            tool::draw_animated(
                &mut grid,
                area,
                0,
                &mut row,
                &call,
                frame,
                folds(false),
                false,
                None,
            );
            grid
        };
        assert_eq!(
            grid.cell(0, 0).ch,
            text::agent::STEP_MARK_FILLED,
            "第 {frame} 帧：动画停止时必须实心"
        );
    }
}

#[test]
fn alive_mark_still_blinks_while_the_animation_runs() {
    // 用户要求保留闪烁：动画推进期间两帧交替，实心/空心都要出现。
    let mut glyphs = Vec::new();
    for frame in 0..(text::agent::STEP_BREATH_TICKS * 2) {
        glyphs.push(StepMark::Alive.glyph(frame, true));
    }
    assert!(
        glyphs.contains(&text::agent::STEP_MARK_FILLED),
        "闪烁必须包含实心帧：{glyphs:?}"
    );
    assert!(
        glyphs.contains(&text::agent::STEP_MARK_HOLLOW),
        "闪烁必须包含空心帧：{glyphs:?}"
    );
}

#[test]
fn hollow_glyph_is_reserved_for_cancelled() {
    // 非 Cancelled 的终态一律实心，且动画停止时实心与相位无关。
    for mark in [StepMark::Done, StepMark::Failed, StepMark::Interrupted] {
        for frame in 0..12 {
            assert_eq!(
                mark.glyph(frame, false),
                text::agent::STEP_MARK_FILLED,
                "{mark:?} 在动画停止时必须实心"
            );
        }
    }
    assert_eq!(
        StepMark::Cancelled.glyph(0, false),
        text::agent::STEP_MARK_HOLLOW,
        "取消仍是空心（语义就是没产出结果）"
    );
}

// ===== 卡片：命令 + 运行中实时尾部 + 整卡引导（终态改为输出预览） =====

#[test]
fn running_command_card_shows_only_the_panel_guide() {
    let output = "heartbeat 1\nheartbeat 2\nheartbeat 3\nheartbeat 4\n";
    let call = command_call(ProcessStateView::Running, ToolStatus::Completed, output);
    let text = render_text(&call, 70);
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[0].contains("Run command"), "第 1 行标题：{text}");
    assert!(lines[1].contains("while true"), "第 2 行是命令原文：{text}");
    assert!(
        lines[1].starts_with(text::agent::TREE_LEAF),
        "命令是参数区的收口行：{text}"
    );
    // 长任务不在消息列表铺输出：命令 + 一行引导，实时日志去面板看。
    assert!(
        !text.contains("heartbeat 1"),
        "运行中不渲染任何输出：{text}"
    );
    assert_eq!(lines.len(), 3, "标题 + 命令 + 引导：{text}");
    assert!(
        lines
            .last()
            .is_some_and(|line| line.contains(text::agent::TASK_OPEN_GUIDE)),
        "末行是整卡点击引导：{text}"
    );
    assert!(
        lines
            .last()
            .is_some_and(|line| line.starts_with(text::agent::TREE_PAD)),
        "引导行只缩进、不画连接符：{text}"
    );
}

#[test]
fn running_card_never_prints_output_or_ellipsis_rows() {
    // 任何长度的输出都不进消息列表：长任务的完整日志在后台任务面板里。
    let output: String = (0..40).map(|index| format!("line {index}\n")).collect();
    let call = command_call(ProcessStateView::Running, ToolStatus::Completed, &output);
    let text = render_text(&call, 70);
    assert!(!text.contains("line 0"), "不渲染输出：{text}");
    assert!(!text.contains("line 39"), "不渲染输出：{text}");
    assert!(
        !text.contains("… +") && !text.contains("more lines"),
        "不画省略提示（面板里看全量）：{text}"
    );
    assert_eq!(
        tool::rows(&call, 70, folds(false)),
        text.lines().count(),
        "几何口径一致"
    );
}

#[test]
fn command_wraps_to_two_rows_when_folded() {
    // 用户带截图反馈：长命令被单行截断成 `$ { ...`。现在 `$ 命令` 按列宽折行，
    // 折叠态最多 2 行；展开态铺开更多。
    // 终态卡片：没有实时输出与引导行，几何只由命令折行决定。
    let call = command_call(
        ProcessStateView::Exited { code: Some(0) },
        ToolStatus::Completed,
        "",
    );
    // 足够长：折叠 2 行装不下，展开才铺得开（否则「展开」不会多出行）。
    let long = format!(
        "$ {{ {} echo \"==== 项目概览 ====\" && echo \"Rust 版本: $(rustc --version)\" \
         && echo \"文件数\" && echo \"目录结构\" && ls -R && echo \"依赖\" && cat Cargo.toml }}",
        " ".repeat(40)
    );
    let mut call = call;
    if let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
        view.command = long.clone();
    }
    let folded = tool::rows(&call, 50, folds(false));
    let expanded = tool::rows(&call, 50, folds(true));
    assert_eq!(folded, 1 + 2 + 1, "折叠态：标题 + 2 行命令 + 剩余行数");
    assert!(expanded > folded, "展开更多行：{folded} → {expanded}");
    let text = render_text(&call, 50);
    assert!(text.lines().count() >= 3, "命令确实折行：{text}");
    // 续行不重复 `$ ` 提示符。
    let dollars = text.matches(text::agent::COMMAND_PROMPT_PREFIX).count();
    assert_eq!(dollars, 1, "提示符只出现一次：{text}");
}

#[test]
fn interaction_card_wraps_its_command_too() {
    // `write_stdin` 是调用快照：只画命令，不跟随后台输出，也没有面板引导。
    let call = interaction_call(ProcessStateView::Running, ToolStatus::Completed);
    let text = render_text(&call, 70);
    assert!(text.contains("while true"), "命令可见：{text}");
    assert!(
        !text.contains(text::agent::TASK_OPEN_GUIDE),
        "交互卡片没有整卡引导：{text}"
    );
    assert!(
        !text
            .lines()
            .any(|line| line.contains("heartbeat") && !line.contains("echo")),
        "不渲染调用快照之外的输出：{text}"
    );
}

#[test]
fn stopped_card_keeps_the_green_exit_code_label() {
    // 停止后除了颜色回绿，文案仍是退出码，不引入新状态词。
    let call = command_call(
        ProcessStateView::Exited { code: Some(0) },
        ToolStatus::Completed,
        "beat\n",
    );
    let text = render_text(&call, 70);
    assert!(text.contains("Exited 0"), "终态显示退出码：{text}");
    let mark = render_grid(&call, 70).cell(0, 0).style;
    assert_eq!(mark.fg, style::chat_step_mark_done().fg, "终态为绿");
    assert_ne!(mark.fg, Some(Color::Red), "正常退出不得画红");
}

// ===== review 回归：计划卡片配色与行数口径 =====

#[test]
fn plan_in_progress_step_uses_the_running_amber() {
    // 真实缺陷：`InProgress` 取了 `chat_tool_verb()`（完成态翡翠绿），
    // 与「运行中必须橙黄、完成才绿」的规则直接冲突——一张卡两种语义。
    let call = plan_call(vec![
        ("build the thing".to_string(), PlanStepView::InProgress),
        ("then ship it".to_string(), PlanStepView::Pending),
    ]);
    let grid = render_grid(&call, 70);
    let mut found_amber = false;
    for x in 0..70 {
        for y in 0..6 {
            if grid.cell(x, y).style.fg == style::chat_tool_verb_running().fg {
                found_amber = true;
            }
        }
    }
    assert!(
        found_amber,
        "计划里「进行中」的步骤必须用运行态橙黄，而不是完成态绿"
    );
}

#[test]
fn folded_command_reports_remaining_lines_and_hover_lifts() {
    // 折叠态：命令下方单独一行写剩余行数；悬停时命令从 dim 提亮。
    let mut call = command_call(ProcessStateView::Running, ToolStatus::Completed, "beat\n");
    if let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
        view.command = format!("$ {{ {} }}", "echo 项目概览; ".repeat(20));
    }
    let text = render_text(&call, 40);
    assert!(text.contains("more line"), "折叠态给剩余行数：{text}");
    // 展开态不再有剩余行数行。
    let expanded = render_text(&call, 40);
    assert!(!expanded.contains("more line") || expanded.lines().count() > 3);
}

/// 参数区不响应 hover（恒为浅黄），但「剩余行数」提示行仍属于参数区的点击面。
#[test]
fn params_section_has_no_hover_face_and_keeps_its_hint() {
    let mut call = command_call(
        ProcessStateView::Exited { code: Some(0) },
        ToolStatus::Completed,
        "",
    );
    if let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref_mut() {
        view.command = format!("$ {{ {} }}", "echo 项目概览; ".repeat(20));
    }
    let width = 40u16;
    let step_rows = tool::rows(&call, width, folds(false));
    let regions = tool::card_regions(&call, width, folds(false), step_rows);
    let params = regions
        .iter()
        .find(|region| region.section == crate::app::state::CardSection::Body)
        .expect("参数区");
    assert_eq!(params.fold.rows, 0, "参数区不做 hover 高亮");
    assert!(params.clickable, "参数区仍可点击展开");
    let text = render_text(&call, width);
    assert!(text.contains("more line"), "折叠态给剩余行数：{text}");
}

/// 参数区的 `… N more lines` 提示行与参数同色（不是 meta 灰）。
#[test]
fn params_more_lines_hint_uses_the_param_color() {
    let command = format!("$ {{ {} }}", "echo 项目概览; ".repeat(20));
    let lines = tool::param_lines(&command, 40, false);
    let hint = lines.last().expect("折叠态必须有提示行");
    assert!(hint.text.contains("more line"), "{:?}", hint.text);
    assert_eq!(hint.style, style::chat_tool_param(), "提示行与参数同色");
}

/// 短输出（没达到折叠额度）不登记 hover 面：悬停不会提亮任何东西。
#[test]
fn short_output_has_no_hover_face() {
    let call = command_call(
        ProcessStateView::Exited { code: Some(0) },
        ToolStatus::Completed,
        "one\ntwo\n",
    );
    let width = 60u16;
    let step_rows = tool::rows(&call, width, folds(false));
    let regions = tool::card_regions(&call, width, folds(false), step_rows);
    let output = regions
        .iter()
        .find(|region| region.section == crate::app::state::CardSection::Output)
        .expect("输出区");
    assert_eq!(output.fold.rows, 0, "短输出不做 hover 高亮");
    assert!(!output.clickable, "短输出没有可展开内容");
}

/// 参数区恒为浅黄（不随 hover/展开变色）；输出区折叠 dim、展开为默认前景；
/// 两块互不连带，且都没有整行底色。
#[test]
fn params_stay_light_yellow_and_output_lifts_independently() {
    let output: String = (0..40)
        .map(|index| format!("heartbeat {index}\n"))
        .collect();
    let call = command_call(
        ProcessStateView::Exited { code: Some(0) },
        ToolStatus::Completed,
        &output,
    );
    let folded = render_grid(&call, 60);
    assert_eq!(
        command_row_style(&folded, 60),
        Some(style::chat_tool_param()),
        "折叠态命令是浅黄"
    );
    let first_output = 2; // 标题 + 1 行命令。
    let content_col = 5; // 树体前缀占前 5 列，正文从第 5 列起。
    assert_eq!(
        folded.cell(content_col, first_output).style,
        style::muted(),
        "折叠态输出是 dim"
    );
    // 只展开输出：命令仍是浅黄，输出首行升为默认前景，且不铺底色。
    let only_output = tool::Folds {
        body: false,
        output: true,
    };
    let height = tool::rows(&call, 60, only_output).max(1) as u16;
    let area = Rect::new(0, 0, 60, height);
    let mut grid = Grid::new(area);
    let mut row = 0usize;
    tool::draw_animated(
        &mut grid,
        area,
        0,
        &mut row,
        &call,
        0,
        only_output,
        true,
        None,
    );
    assert_eq!(
        command_row_style(&grid, 60),
        Some(style::chat_tool_param()),
        "展开输出不得改变参数颜色"
    );
    assert_eq!(
        grid.cell(content_col, first_output).style,
        style::text(),
        "展开的输出是默认前景"
    );
    assert_eq!(
        grid.cell(content_col, first_output).style.bg,
        None,
        "展开的输出不铺任何底色"
    );
    // 悬停参数区：参数仍是同一档浅黄（用户要求去掉 hover 高亮）。
    let height = tool::rows(&call, 60, folds(false)).max(1) as u16;
    let area = Rect::new(0, 0, 60, height);
    let mut grid = Grid::new(area);
    let mut row = 0usize;
    tool::draw_animated(
        &mut grid,
        area,
        0,
        &mut row,
        &call,
        0,
        folds(false),
        true,
        Some(crate::app::state::CardSection::Body),
    );
    assert_eq!(
        command_row_style(&grid, 60),
        Some(style::chat_tool_param()),
        "悬停不改变参数颜色"
    );
    // 只展开参数：输出仍 dim。
    let only_params = tool::Folds {
        body: true,
        output: false,
    };
    let height = tool::rows(&call, 60, only_params).max(1) as u16;
    let area = Rect::new(0, 0, 60, height);
    let mut grid = Grid::new(area);
    let mut row = 0usize;
    tool::draw_animated(
        &mut grid,
        area,
        0,
        &mut row,
        &call,
        0,
        only_params,
        true,
        None,
    );
    assert_eq!(
        command_row_style(&grid, 60),
        Some(style::chat_tool_param()),
        "展开参数仍是浅黄"
    );
    assert_eq!(
        grid.cell(content_col, first_output).style,
        style::muted(),
        "展开参数不得连带展开输出"
    );
}

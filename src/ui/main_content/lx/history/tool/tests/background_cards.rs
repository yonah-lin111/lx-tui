//! 后台任务卡片新形态的回归测试：橙色存活态、动画停止后实心、存活三行 / 终态两行与 `open` 入口。
//!
//! 用户反馈的两组现象：
//! 1. 运行中的后台卡片是绿的，且会定格成镂空 `□`（看起来像失败），但进程其实还在跑；
//! 2. 卡片把命令、cwd、输出、`… +N more lines` 全画在消息列表里，把历史撑得很长，
//!    而这些内容应该只在后台任务面板里看。

use ratatui::layout::Rect;
use ratatui::style::Color;

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
    let height = tool::rows(call, width, false).max(1) as u16;
    let area = Rect::new(0, 0, width, height);
    let mut grid = Grid::new(area);
    let mut row = 0usize;
    tool::draw(&mut grid, area, 0, &mut row, call, 0, false);
    grid
}

/// 渲染并返回整块文本。
fn render_text(call: &ToolCallView, width: u16) -> String {
    let grid = render_grid(call, width);
    let area = Rect::new(0, 0, width, tool::rows(call, width, false).max(1) as u16);
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
            let height = tool::rows(&call, 60, false).max(1) as u16;
            let area = Rect::new(0, 0, 60, height);
            let mut grid = Grid::new(area);
            let mut row = 0usize;
            // animating = false：动画已停止。
            tool::draw_animated(
                &mut grid, area, 0, &mut row, &call, frame, false, false, false,
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

// ===== 卡片收敛为三行，open 独占末行且为次要色 =====

#[test]
fn command_card_is_title_command_and_open_row_only() {
    let output = "heartbeat 1\nheartbeat 2\nheartbeat 3\nheartbeat 4\n";
    let call = command_call(ProcessStateView::Running, ToolStatus::Completed, output);
    let text = render_text(&call, 70);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3, "卡片恒为三行：{text}");
    assert!(lines[0].contains("Run command"), "第 1 行标题：{text}");
    assert!(lines[1].contains("while true"), "第 2 行是命令原文：{text}");
    assert!(
        lines[2].starts_with(text::agent::TREE_LEAF),
        "第 3 行是 open 且用直角收口：{text}"
    );
    assert!(
        lines[2].contains(text::agent::TASK_OPEN_LABEL),
        "open 文案：{text}"
    );
}

#[test]
fn command_card_no_longer_renders_any_output_or_ellipsis_row() {
    // 输出全部搬到后台任务面板；消息列表里连省略提示都不该出现。
    let output: String = (0..40).map(|index| format!("line {index}\n")).collect();
    let call = command_call(ProcessStateView::Running, ToolStatus::Completed, &output);
    let text = render_text(&call, 70);
    assert!(!text.contains("line 0"), "不得渲染输出正文：{text}");
    assert!(
        !text.contains(text::agent::EARLIER_OUTPUT_HINT),
        "不得出现 `… +N more lines`：{text}"
    );
    assert!(
        !text.contains(text::agent::WAITING_FOR_OUTPUT),
        "无输出占位也不再需要：{text}"
    );
    assert_eq!(text.lines().count(), 3, "行数恒为三行：{text}");
    assert_eq!(tool::rows(&call, 70, false), 3, "几何口径恒为三行");
}

#[test]
fn open_row_is_grey_and_open_offset_points_at_it() {
    let call = command_call(ProcessStateView::Running, ToolStatus::Completed, "beat\n");
    let grid = render_grid(&call, 70);
    // 定位 `open` 所在列（第 3 行，即 y=2）。
    let label = text::agent::TASK_OPEN_LABEL;
    let mut found = None;
    for x in 0..70u16 {
        let cell = grid.cell(x, 2);
        if cell.ch == label.chars().next().unwrap_or('[') {
            found = Some(cell.style);
            break;
        }
    }
    let style_grey = found.expect("open 必须画出来");
    assert_eq!(
        style_grey.fg,
        style::agent_step_detail().fg,
        "open 必须是左下角上下文统计色：{style_grey:?}"
    );
    assert_eq!(
        tool::open_offset(&call),
        Some(2),
        "open 位于卡片内第 2 行偏移（0 = 标题）"
    );
}

#[test]
fn interaction_card_also_shows_the_open_row() {
    let call = interaction_call(ProcessStateView::Running, ToolStatus::Completed);
    let text = render_text(&call, 70);
    assert!(
        text.lines()
            .last()
            .is_some_and(|line| line.contains(text::agent::TASK_OPEN_LABEL)),
        "交互卡片同样给 open 入口：{text}"
    );
    assert_eq!(text.lines().count(), 3, "行数同样恒为三行：{text}");
}

#[test]
fn open_row_is_dropped_instead_of_truncated_when_extremely_narrow() {
    // 极窄时整项丢弃文案，但行数与几何口径不变（滚动条/命中不会漂移）。
    let call = command_call(ProcessStateView::Running, ToolStatus::Completed, "beat\n");
    for width in [1u16, 4, 6, 9] {
        assert_eq!(tool::rows(&call, width, false), 3, "宽 {width}：行数不变");
        // 用 `rows()` 校验几何（`render_text` 会 trim 掉末尾空行，窄到画不出内容时
        // 空行会被折叠掉，不能用来数行）。
        let grid = render_grid(&call, width);
        let area = Rect::new(0, 0, width, 3);
        let mut painted = 0;
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if grid.cell(x, y).ch != crate::ui::main_content::lx::grid::CONTINUATION
                    && grid.cell(x, y).ch != ' '
                {
                    painted += 1;
                }
            }
        }
        assert!(painted > 0 || width == 0, "宽 {width}：至少画出方块");
        let rendered = render_text(&call, width);
        assert!(
            !rendered.contains("[ope"),
            "宽 {width}：不得画半个 [open]：{rendered}"
        );
    }
}

// ===== 决策 7：只有 [open] 行可点 =====

#[test]
fn only_the_open_row_is_a_click_target() {
    let call = command_call(ProcessStateView::Running, ToolStatus::Completed, "beat\n");
    assert_eq!(
        tool::open_offset(&call),
        Some(2),
        "只有 [open] 行提供命中偏移"
    );
    // 标题行不再可点：卡片没有"展开/收起"语义，`open_offset` 是唯一命中来源。
    assert_eq!(tool::rows(&call, 70, false), 3, "三行里只有末行是 [open]");
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

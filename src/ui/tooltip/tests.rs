//! 单元测试：全局按钮命中、锚点几何与渲染。

use std::time::Instant;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

use super::*;
use crate::app::tooltip::{BlockButtonKind, TOOLTIP_DELAY, TemplateButtonKind, Tooltip};
use crate::config::Config;

fn view_for(state: &AppState) -> ViewLayout {
    crate::ui::layout::compute(
        Rect::new(0, 0, 100, 24),
        &Config::default(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    )
}

fn pane_rects(state: &AppState, view: &ViewLayout) -> Vec<(PaneId, Rect)> {
    crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        Config::default().min_pane_width,
        Config::default().min_pane_height,
    )
}

/// 渲染一帧并返回缓冲。
fn render_buffer(state: &AppState) -> ratatui::buffer::Buffer {
    let config = Config::default();
    let mut terminal =
        Terminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| crate::ui::render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    terminal.backend().buffer().clone()
}

fn collapse_area(view: &ViewLayout, target: ui::CollapseTarget) -> Rect {
    ui::collapse_buttons(view, false)
        .into_iter()
        .find(|button| button.target == target)
        .expect("collapse button visible")
        .area
}

#[test]
fn hits_collapse_buttons_and_misses_content() {
    let state = AppState::demo();
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let area = collapse_area(&view, ui::CollapseTarget::Sidebar);
    assert_eq!(
        target_at(&view, &state, &rects, area.x + 1, area.y),
        Some(TooltipTarget::CollapseSidebar)
    );
    let pane_point = (view.panes.x + 2, view.panes.y + 2);
    assert_eq!(
        target_at(&view, &state, &rects, pane_point.0, pane_point.1),
        None
    );
}

#[test]
fn hits_swap_prompt_button() {
    let state = AppState::demo();
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let button = ui::swap_button(&view).expect("swap button visible");
    assert_eq!(
        target_at(&view, &state, &rects, button.x, button.y),
        Some(TooltipTarget::SwapPromptSide)
    );
}

#[test]
fn hits_pane_toggle_add_workspace_tab_bar_and_exit() {
    let state = AppState::demo();
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let pane = state.active_tab().layout.focus();
    let toggle = main_content::toggle_button(
        rects
            .iter()
            .find(|(id, _)| *id == pane)
            .expect("focused pane has rect")
            .1,
    )
    .expect("toggle button visible");
    assert_eq!(
        target_at(&view, &state, &rects, toggle.x, toggle.y),
        Some(TooltipTarget::PaneToggle(pane))
    );

    let add_workspace = sidebar::add_workspace_button(&view).expect("add workspace visible");
    assert_eq!(
        target_at(&view, &state, &rects, add_workspace.x + 1, add_workspace.y),
        Some(TooltipTarget::AddWorkspace)
    );

    let bar = tab_bar::layout(&view, state.active_workspace(), state.tab_scroll);
    let add = bar.add.expect("add tab visible");
    assert_eq!(
        target_at(&view, &state, &rects, add.x, add.y),
        Some(TooltipTarget::AddTab)
    );
    let exit = ui::exit_button(&view).expect("exit visible");
    assert_eq!(
        target_at(&view, &state, &rects, exit.x + 1, exit.y),
        Some(TooltipTarget::Exit)
    );
}

#[test]
fn hits_prompt_toolbar_and_pin_but_not_when_collapsed() {
    let mut state = AppState::demo();
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let undo =
        layout::prompt_toolbar_button_rect(view.prompt, layout::PromptToolbarButton::Undo, false)
            .expect("undo button visible");
    assert_eq!(
        target_at(&view, &state, &rects, undo.x, undo.y),
        Some(TooltipTarget::PromptUndo)
    );
    let pin =
        layout::prompt_toolbar_button_rect(view.prompt, layout::PromptToolbarButton::Pin, false)
            .expect("pin button visible");
    assert_eq!(
        target_at(&view, &state, &rects, pin.x, pin.y),
        Some(TooltipTarget::PromptPin)
    );

    state.prompt_collapsed = true;
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    assert!(
        targets(&view, &state, &rects)
            .iter()
            .all(|(target, _)| !matches!(
                target,
                TooltipTarget::PromptUndo
                    | TooltipTarget::PromptRedo
                    | TooltipTarget::PromptSelectAll
                    | TooltipTarget::PromptSave
                    | TooltipTarget::PromptPin
            )),
        "折叠后 prompt 按钮不参与命中"
    );
}

#[test]
fn rect_prefers_below_and_flips_above_last_row() {
    let screen = Rect::new(0, 0, 100, 24);
    let hint = "new tab";
    let title = text::TOOLTIP_TITLE;
    let below = rect(screen, Rect::new(10, 5, 6, 1), title, hint).expect("rect resolves");
    assert_eq!(below, Rect::new(10, 6, 11, 3));
    let above = rect(screen, Rect::new(10, 23, 6, 1), title, hint).expect("rect resolves");
    assert_eq!(above.y, 20);
    assert!(above.bottom() <= screen.bottom());
}

#[test]
fn rect_clamps_to_screen_right() {
    let screen = Rect::new(0, 0, 100, 24);
    let hint = "scroll tabs right";
    let area =
        rect(screen, Rect::new(94, 0, 6, 1), text::TOOLTIP_TITLE, hint).expect("rect resolves");
    assert_eq!(area.width, 21);
    assert_eq!(area.x, 79);
    assert_eq!(area.right(), screen.right());
}

#[test]
fn render_skips_until_visible_then_paints_panel_and_hint() {
    let mut state = AppState::demo();
    let now = Instant::now();
    let view = view_for(&state);
    let anchor = collapse_area(&view, ui::CollapseTarget::Sidebar);
    let hint = text::tooltip_hint(TooltipTarget::CollapseSidebar, &state);
    let area =
        rect(Rect::new(0, 0, 100, 24), anchor, text::TOOLTIP_TITLE, hint).expect("rect resolves");

    state.tooltip = Some(Tooltip::new(TooltipTarget::CollapseSidebar, now));
    let buffer = render_buffer(&state);
    assert_ne!(
        buffer[(area.x, area.y)].bg,
        Color::Indexed(236),
        "未到显示时刻不绘制"
    );

    assert!(crate::app::update::tick(&mut state, now + TOOLTIP_DELAY));
    let buffer = render_buffer(&state);
    assert_eq!(buffer[(area.x, area.y)].symbol(), "╭");
    assert_eq!(buffer[(area.x, area.y)].bg, Color::Indexed(236));
    let top: String = (area.x..area.right())
        .map(|x| buffer[(x, area.y)].symbol())
        .collect();
    assert!(top.contains(text::TOOLTIP_TITLE), "顶边框标题：{top:?}");
    let row: String = (area.x + 1..area.right() - 1)
        .map(|x| buffer[(x, area.y + 1)].symbol())
        .collect();
    assert!(row.contains(hint), "正文包含提示文案：{row:?}");
}

#[test]
fn render_skips_while_overlay_open() {
    let mut state = AppState::demo();
    let now = Instant::now();
    state.tooltip = Some(Tooltip::new(TooltipTarget::CollapseSidebar, now));
    assert!(crate::app::update::tick(&mut state, now + TOOLTIP_DELAY));
    let view = view_for(&state);
    let anchor = collapse_area(&view, ui::CollapseTarget::Sidebar);
    let hint = text::tooltip_hint(TooltipTarget::CollapseSidebar, &state);
    let area =
        rect(Rect::new(0, 0, 100, 24), anchor, text::TOOLTIP_TITLE, hint).expect("rect resolves");

    crate::app::update::open_tab_menu(&mut state, 0, (2, 2));
    let buffer = render_buffer(&state);
    assert_ne!(
        buffer[(area.x, area.y)].bg,
        Color::Indexed(236),
        "浮层打开时模态优先"
    );
}

#[test]
fn hint_flips_with_state() {
    let mut state = AppState::demo();
    assert_eq!(
        text::tooltip_hint(TooltipTarget::PromptPin, &state),
        "pin prompt"
    );
    state.prompt_pinned = Some(0);
    assert_eq!(
        text::tooltip_hint(TooltipTarget::PromptPin, &state),
        "unpin prompt"
    );
    assert_eq!(
        text::tooltip_hint(TooltipTarget::CollapseSidebar, &state),
        "collapse sidebar"
    );
    state.sidebar_collapsed = true;
    assert_eq!(
        text::tooltip_hint(TooltipTarget::CollapseSidebar, &state),
        "expand sidebar"
    );
    let pane = state.active_tab().layout.focus();
    assert_eq!(
        text::tooltip_hint(TooltipTarget::PaneToggle(pane), &state),
        "switch to terminal"
    );
    for (button, hint) in [
        (TemplateButtonKind::Status, "cycle block status"),
        (TemplateButtonKind::Copy, "copy block"),
        (TemplateButtonKind::Clean, "clean block"),
        (TemplateButtonKind::Delete, "delete block"),
    ] {
        assert_eq!(
            text::tooltip_hint(TooltipTarget::TemplateButton { line: 0, button }, &state),
            hint
        );
    }
}

#[test]
fn hits_template_block_buttons_via_prompt_text_area() {
    let mut state = AppState::demo();
    state.prompt.resize(28, 20);
    state
        .prompt
        .insert_str("&&& addTemplate --start\n# add\n&&& addTemplate --end");
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let area = layout::prompt_text_rect(view.prompt);
    let hits = ui::prompt::template_buttons(&state.prompt, area);
    assert_eq!(hits.len(), 4, "状态、复制、清理、删除按钮可见");
    let copy = hits
        .iter()
        .find(|hit| hit.button == ui::prompt::TemplateBlockButton::Copy)
        .expect("copy button visible");
    assert_eq!(
        target_at(&view, &state, &rects, copy.rect.x, copy.rect.y),
        Some(TooltipTarget::TemplateButton {
            line: copy.line,
            button: TemplateButtonKind::Copy,
        })
    );
    assert_eq!(target_at(&view, &state, &rects, 0, 0), None, "边框角不命中");
}

#[test]
fn block_button_tooltips_have_hints() {
    let state = AppState::demo();
    for (button, hint) in [
        (BlockButtonKind::Clean, "clean block"),
        (BlockButtonKind::Delete, "delete block"),
        (BlockButtonKind::Merge, "merge into first variable block"),
        (BlockButtonKind::Top, "move to top"),
        (BlockButtonKind::Fold, "collapse / expand variable block"),
    ] {
        assert_eq!(
            text::tooltip_hint(TooltipTarget::BlockButton { line: 0, button }, &state),
            hint
        );
    }
}

#[test]
fn hits_block_buttons_via_prompt_text_area() {
    let mut state = AppState::demo();
    state.prompt.resize(40, 20);
    state
        .prompt
        .insert_str("$$$ varTemplate --start\n{title: }\nkey: \"v\"\n$$$ varTemplate --end");
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let area = layout::prompt_text_rect(view.prompt);
    let hits = ui::prompt::block_buttons(&state.prompt, area);
    assert_eq!(hits.len(), 5, "折叠、清理、删除、合并、置顶按钮可见");
    let clean = hits
        .iter()
        .find(|hit| hit.button == ui::prompt::BlockButton::Clean)
        .expect("clean button visible");
    assert_eq!(
        target_at(&view, &state, &rects, clean.rect.x, clean.rect.y),
        Some(TooltipTarget::BlockButton {
            line: clean.line,
            button: BlockButtonKind::Clean,
        })
    );
}

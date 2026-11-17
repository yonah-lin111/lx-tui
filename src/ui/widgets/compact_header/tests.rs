use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::{AppState, FocusedContainer};
use crate::config::Config;
use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};

fn draw(state: &AppState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("backend");
    terminal
        .draw(|frame| crate::ui::render(frame, state, &Config::default()))
        .expect("draw");
    terminal.backend().buffer().clone()
}

fn row(buffer: &Buffer, y: u16, width: u16) -> String {
    (0..width)
        .map(|x| buffer.cell((x, y)).expect("cell").symbol())
        .collect()
}

fn view(state: &AppState, width: u16, height: u16) -> ViewLayout {
    crate::ui::layout::compute_with_panel(
        Rect::new(0, 0, width, height),
        &Config::default(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
        state.compact_panel(),
    )
}

#[test]
fn nav_row_has_left_button_tabs_and_right_button() {
    let state = AppState::demo();
    let view = view(&state, 120, 24);
    assert!(view.compact);
    assert_eq!(view.compact_header, Rect::new(0, 0, 120, 2));
    assert_eq!(view.compact_left_button, Rect::new(0, 0, 4, 1));
    assert_eq!(view.compact_right_button, Rect::new(113, 0, 7, 1));
    assert_eq!(view.tab_bar, Rect::new(5, 0, 107, 1));
    assert_eq!(view.compact_status, Rect::new(0, 1, 120, 1));

    let buffer = draw(&state, 120, 24);
    let nav = row(&buffer, 0, 120);
    assert!(nav.starts_with(text::COMPACT_LEFT_LABEL), "got: {nav}");
    assert!(nav.ends_with(text::COMPACT_RIGHT_LABEL), "got: {nav}");
    assert!(nav.contains("tab 1"), "got: {nav}");
}

#[test]
fn short_header_is_single_nav_row() {
    let state = AppState::demo();
    let view = view(&state, 40, 10);
    assert_eq!(view.compact_header, Rect::new(0, 0, 40, 1));
    assert_eq!(view.compact_status, Rect::new(0, 0, 0, 0));

    let buffer = draw(&state, 40, 10);
    let nav = row(&buffer, 0, 40);
    assert!(nav.contains(text::COMPACT_LEFT_LABEL), "got: {nav}");
    assert!(nav.contains(text::COMPACT_RIGHT_LABEL), "got: {nav}");
}

#[test]
fn left_button_is_filled_when_sidebar_focused() {
    let mut state = AppState::demo();
    state.set_focus(FocusedContainer::Sidebar);
    let buffer = draw(&state, 120, 24);
    assert_eq!(buffer[(0, 0)].bg, ratatui::style::Color::Cyan);
    assert_eq!(buffer[(0, 0)].fg, ratatui::style::Color::Black);
    assert_ne!(buffer[(113, 0)].bg, ratatui::style::Color::Cyan);
}

#[test]
fn right_button_is_filled_when_prompt_focused() {
    let mut state = AppState::demo();
    state.set_focus(FocusedContainer::Prompt);
    let buffer = draw(&state, 120, 24);
    assert_eq!(buffer[(113, 0)].bg, ratatui::style::Color::Cyan);
    assert_ne!(buffer[(0, 0)].bg, ratatui::style::Color::Cyan);
}

#[test]
fn fullscreen_right_panel_keeps_nav_row_and_status() {
    let mut state = AppState::demo();
    state.set_focus(FocusedContainer::Prompt);
    let view = view(&state, 120, 24);
    assert_eq!(view.main_content.width, 0);
    assert_eq!(view.right_panel, Rect::new(0, 2, 120, 22));
    assert_eq!(view.compact_status.height, 1);

    let buffer = draw(&state, 120, 24);
    assert!(row(&buffer, 0, 120).contains(text::COMPACT_RIGHT_LABEL));
}

#[test]
fn long_workspace_name_does_not_push_buttons_off_screen() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "w".repeat(300);
    let buffer = draw(&state, 120, 24);
    let nav = row(&buffer, 0, 120);
    assert!(nav.ends_with(text::COMPACT_RIGHT_LABEL), "got: {nav}");
    assert!(nav.starts_with(text::COMPACT_LEFT_LABEL), "got: {nav}");
}

#[test]
fn agent_summary_uses_severity_order_and_counts() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.active_tab_mut().pane_mut(pane).expect("pane").agent = Some(PaneAgentSnapshot {
        kind: AgentKind::Claude,
        state: AgentState::Working,
        title: None,
    });
    let buffer = draw(&state, 120, 24);
    let status = row(&buffer, 1, 120);
    assert!(status.contains("1 working"), "got: {status}");

    assert_eq!(
        aggregate([AgentState::Idle, AgentState::Working, AgentState::Blocked].into_iter()),
        Some(AgentState::Blocked)
    );
    assert_eq!(
        aggregate([AgentState::Idle, AgentState::Working].into_iter()),
        Some(AgentState::Working)
    );
    assert_eq!(aggregate(std::iter::empty()), None);
}

#[test]
fn no_agents_shows_empty_summary() {
    let state = AppState::demo();
    let buffer = draw(&state, 120, 24);
    assert!(row(&buffer, 1, 120).contains(text::COMPACT_NO_AGENTS));
}

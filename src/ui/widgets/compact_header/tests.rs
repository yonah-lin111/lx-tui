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
fn chat_header_has_two_rows_and_switch() {
    let state = AppState::demo();
    let view = view(&state, 80, 24);
    let layout = layout(&view, &state);
    assert_eq!(layout.area, Rect::new(0, 0, 80, 2));
    assert_eq!(layout.switch, Some(Rect::new(72, 0, 8, 2)));
    assert!(layout.back.is_none());

    let buffer = draw(&state, 80, 24);
    let first = row(&buffer, 0, 80);
    assert!(first.contains(&state.active_workspace().name));
    assert!(first.contains("[switch]"));
}

#[test]
fn short_header_is_single_row() {
    let state = AppState::demo();
    let view = view(&state, 60, 10);
    let layout = layout(&view, &state);
    assert_eq!(layout.area.height, 1);
    assert!(layout.switch.is_some());

    let buffer = draw(&state, 60, 10);
    assert!(row(&buffer, 0, 60).contains("[switch]"));
}

#[test]
fn fullscreen_panel_header_shows_back_and_title() {
    let mut state = AppState::demo();
    state.set_focus(FocusedContainer::Prompt);
    let view = view(&state, 60, 20);
    let layout = layout(&view, &state);
    assert_eq!(layout.area.height, 1);
    assert_eq!(layout.back, Some(Rect::new(0, 0, 6, 1)));
    assert!(layout.switch.is_none());

    let buffer = draw(&state, 60, 20);
    let first = row(&buffer, 0, 60);
    assert!(first.contains("[back]"));
    assert!(first.contains(text::RIGHT_TAB_PROMPT_LABEL));
}

#[test]
fn long_workspace_name_does_not_push_switch_off_screen() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "w".repeat(200);
    let buffer = draw(&state, 80, 24);
    let first = row(&buffer, 0, 80);
    assert!(first.ends_with("[switch]"), "got: {first}");
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
    let buffer = draw(&state, 80, 24);
    let second = row(&buffer, 1, 80);
    assert!(second.contains("1 working"), "got: {second}");

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
    let buffer = draw(&state, 80, 24);
    assert!(row(&buffer, 1, 80).contains(text::COMPACT_NO_AGENTS));
}

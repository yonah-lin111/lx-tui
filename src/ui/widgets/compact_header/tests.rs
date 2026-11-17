use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::{AppState, FocusedContainer};
use crate::config::Config;

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
    assert_eq!(view.compact_header, Rect::new(0, 0, 120, 1));
    assert_eq!(view.compact_left_button, Rect::new(0, 0, 4, 1));
    assert_eq!(view.compact_right_button, Rect::new(113, 0, 7, 1));
    assert_eq!(view.tab_bar, Rect::new(5, 0, 107, 1));

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
fn right_button_is_filled_when_prompt_or_todo_focused() {
    let mut state = AppState::demo();
    state.set_focus(FocusedContainer::Prompt);
    let buffer = draw(&state, 120, 24);
    assert_eq!(buffer[(113, 0)].bg, ratatui::style::Color::Cyan);
    assert_ne!(buffer[(0, 0)].bg, ratatui::style::Color::Cyan);

    state.right_tab = crate::app::state::RightTab::Todo;
    state.set_focus(FocusedContainer::Todo);
    let buffer = draw(&state, 120, 24);
    assert_eq!(buffer[(113, 0)].bg, ratatui::style::Color::Cyan);
}

#[test]
fn fullscreen_right_panel_keeps_nav_row_and_body_from_row_1() {
    let mut state = AppState::demo();
    state.set_focus(FocusedContainer::Prompt);
    let view = view(&state, 120, 24);
    assert_eq!(view.main_content.width, 0);
    assert_eq!(view.right_panel, Rect::new(0, 1, 120, 23));

    let buffer = draw(&state, 120, 24);
    assert!(row(&buffer, 0, 120).contains(text::COMPACT_RIGHT_LABEL));
}

#[test]
fn long_tab_titles_do_not_push_buttons_off_screen() {
    let mut state = AppState::demo();
    state.workspaces[0].tabs[0].name = Some("t".repeat(120));
    let buffer = draw(&state, 120, 24);
    let nav = row(&buffer, 0, 120);
    assert!(nav.ends_with(text::COMPACT_RIGHT_LABEL), "got: {nav}");
    assert!(nav.starts_with(text::COMPACT_LEFT_LABEL), "got: {nav}");
    let v = view(&state, 120, 24);
    assert_eq!(
        buffer[(v.compact_left_button.right(), 0)].symbol(),
        text::TAB_SEPARATOR
    );
    assert_eq!(buffer[(v.tab_bar.right(), 0)].symbol(), text::TAB_SEPARATOR);
}

#[test]
fn ultra_narrow_header_without_tab_space_skips_separators() {
    let state = AppState::demo();
    let v = view(&state, 11, 10);
    assert_eq!(v.tab_bar.width, 0);
    let mut terminal = Terminal::new(TestBackend::new(11, 10)).expect("backend");
    terminal
        .draw(|frame| render(frame, &v, &state))
        .expect("draw header");
    let nav = row(terminal.backend().buffer(), 0, 11);
    assert_eq!(nav, "[ws][panel]", "分隔符不得覆盖 [panel]: {nav:?}");
}

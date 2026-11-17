use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::AppState;
use crate::app::update;
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

fn open(state: &mut AppState) {
    update::open_compact_switcher(state, 10);
}

fn switcher(state: &AppState) -> &CompactSwitcher {
    match state.overlay.as_ref() {
        Some(crate::app::overlay::Overlay::CompactSwitcher(switcher)) => switcher,
        _ => panic!("switcher is not open"),
    }
}

#[test]
fn layout_reserves_title_and_rule_rows() {
    let mut state = AppState::demo();
    open(&mut state);
    let layout = layout(Rect::new(0, 0, 60, 20), switcher(&state));
    assert_eq!(layout.viewport, Rect::new(0, 2, 60, 18));
    assert_eq!(layout.close, Some(Rect::new(53, 0, 7, 1)));
}

#[test]
fn row_at_uses_scroll_offset() {
    let mut state = AppState::demo();
    update::open_compact_switcher(&mut state, 4);
    let rows = switcher(&state).rows.len();
    let screen = Rect::new(0, 0, 60, 8);
    update::set_compact_switcher_scroll(&mut state, 1);
    let index = row_at(screen, switcher(&state), 0, 2).expect("row");
    assert_eq!(index, 1);
    assert!(row_at(screen, switcher(&state), 0, 1).is_none());
    assert!(index < rows);
}

#[test]
fn close_at_hits_close_button_only() {
    let mut state = AppState::demo();
    open(&mut state);
    let screen = Rect::new(0, 0, 60, 20);
    assert!(close_at(screen, switcher(&state), 55, 0));
    assert!(!close_at(screen, switcher(&state), 10, 0));
}

#[test]
fn render_shows_title_sections_and_close() {
    let mut state = AppState::demo();
    open(&mut state);
    let buffer = draw(&state, 60, 20);
    let title = row(&buffer, 0, 60);
    assert!(title.contains(text::COMPACT_TITLE));
    assert!(title.contains(text::COMPACT_CLOSE_LABEL));
    let body: String = (2..20).map(|y| row(&buffer, y, 60)).collect();
    assert!(body.contains(text::COMPACT_SECTION_SPACES));
    assert!(body.contains(text::COMPACT_NEW_WORKSPACE));
    assert!(body.contains(text::COMPACT_NEW_TAB));
}

#[test]
fn render_highlights_selected_row_with_marker() {
    let mut state = AppState::demo();
    open(&mut state);
    let buffer = draw(&state, 60, 20);
    let body: String = (2..20).map(|y| row(&buffer, y, 60)).collect();
    assert!(body.contains(text::CURRENT_ITEM_MARKER));
}

#[test]
fn long_list_shows_scrollbar_thumb() {
    let mut state = AppState::demo();
    for _ in 0..14 {
        update::create_tab(&mut state);
    }
    update::open_compact_switcher(&mut state, 4);
    let buffer = draw(&state, 60, 12);
    let body: String = (2..12).map(|y| row(&buffer, y, 60)).collect();
    assert!(body.contains('▐'));
}

#[test]
fn narrow_width_keeps_close_and_marker() {
    let mut state = AppState::demo();
    open(&mut state);
    let buffer = draw(&state, 40, 12);
    let title = row(&buffer, 0, 40);
    assert!(title.contains(text::COMPACT_CLOSE_LABEL));
    let body: String = (2..12).map(|y| row(&buffer, y, 40)).collect();
    assert!(body.contains(text::CURRENT_ITEM_MARKER));
}

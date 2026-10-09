//! 单元测试；仅测试构建编译。

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

#[test]
fn ascii_view_shows_all_and_cursor_at_end() {
    let view = view("abc", 3, 10);
    assert_eq!(view.visible, "abc");
    assert_eq!(view.cursor_col, 3);
}

#[test]
fn wide_chars_count_two_columns() {
    let view = view("中文名称", 4, 20);
    assert_eq!(view.visible, "中文名称");
    assert_eq!(view.cursor_col, 8);
}

#[test]
fn long_text_scrolls_to_keep_cursor_visible() {
    let text = "x".repeat(60);
    let view = view(&text, 60, 38);
    assert_eq!(view.visible.chars().count(), 37);
    assert_eq!(view.cursor_col, 37);
}

#[test]
fn wide_text_scrolls_by_display_width() {
    let text = "中".repeat(30);
    let view = view(&text, 30, 38);
    assert_eq!(view.visible.chars().count(), 18);
    assert_eq!(view.cursor_col, 36);
}

#[test]
fn cursor_beyond_text_clamps_to_end() {
    let view = view("ab", 99, 10);
    assert_eq!(view.visible, "ab");
    assert_eq!(view.cursor_col, 2);
}

#[test]
fn empty_width_returns_empty_view() {
    let view = view("abc", 1, 0);
    assert_eq!(view.visible, "");
    assert_eq!(view.cursor_col, 0);
}

#[test]
fn render_draws_text_and_returns_hardware_cursor() {
    let mut terminal = Terminal::new(TestBackend::new(20, 3)).expect("test backend is infallible");
    let mut cursor = None;
    if let Err(error) = terminal.draw(|frame| {
        cursor = render(frame, Rect::new(2, 1, 10, 1), "中文", 2, None);
    }) {
        panic!("draw failed: {error}");
    }
    assert_eq!(cursor, Some((6, 1)));
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(buffer[(2, 1)].symbol(), "中");
    assert_eq!(buffer[(4, 1)].symbol(), "文");
}

#[test]
fn empty_text_renders_placeholder_dimmed() {
    let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("test backend is infallible");
    let mut cursor = None;
    if let Err(error) = terminal.draw(|frame| {
        cursor = render(frame, Rect::new(0, 0, 20, 1), "", 0, Some("new name"));
    }) {
        panic!("draw failed: {error}");
    }
    assert_eq!(cursor, Some((0, 0)));
    let buffer = terminal.backend().buffer().clone();
    let line: String = (0..20).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(line.starts_with("new name"), "line={line:?}");
    assert!(buffer[(0, 0)].modifier.contains(Modifier::DIM));
}

#[test]
fn non_empty_text_suppresses_placeholder() {
    let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| {
        render(frame, Rect::new(0, 0, 20, 1), "abc", 3, Some("hint"));
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let line: String = (0..20).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(line.starts_with("abc"), "line={line:?}");
    assert!(!line.contains("hint"), "line={line:?}");
    assert!(!buffer[(0, 0)].modifier.contains(Modifier::DIM));
}

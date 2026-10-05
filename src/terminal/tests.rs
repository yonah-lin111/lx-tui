//! 单元测试；仅测试构建编译。

use super::*;
use alacritty_terminal::index::{Column, Line};

#[test]
fn feeds_text_into_grid() {
    let mut term = Terminal::new(10, 3);
    term.feed(b"hi");
    let grid = term.grid();
    assert_eq!(grid[Line(0)][Column(0)].c, 'h');
    assert_eq!(grid[Line(0)][Column(1)].c, 'i');
}

#[test]
fn captures_osc_title() {
    let mut term = Terminal::new(10, 3);
    term.feed(b"\x1b]0;agent\x07");
    assert_eq!(term.title(), Some("agent"));
    term.feed(b"\x1b]0;\x07");
    assert_eq!(term.title(), Some(""));
}

#[test]
fn tracks_deckm_and_cursor_visibility() {
    let mut term = Terminal::new(10, 3);
    assert!(!term.mode().contains(TermMode::APP_CURSOR));
    term.feed(b"\x1b[?1h");
    assert!(term.mode().contains(TermMode::APP_CURSOR));
    assert!(term.cursor_viewport().is_some());
    term.feed(b"\x1b[?25l");
    assert!(term.cursor_viewport().is_none());
}

#[test]
fn responds_to_device_status_query() {
    let mut term = Terminal::new(10, 3);
    let responses = term.feed(b"\x1b[6n");
    assert_eq!(String::from_utf8_lossy(&responses), "\x1b[1;1R");
}

#[test]
fn resize_updates_size() {
    let mut term = Terminal::new(10, 3);
    term.resize(40, 12);
    assert_eq!(term.size(), GridSize { cols: 40, rows: 12 });
    term.feed(b"x");
    assert_eq!(term.grid()[Line(0)][Column(0)].c, 'x');
}

#[test]
fn extracts_text_from_range() {
    let mut term = Terminal::new(20, 3);
    term.feed(b"hello world");
    assert_eq!(term.text_in_range((0, 0), (0, 4)).as_deref(), Some("hello"));
    assert_eq!(term.text_in_range((0, 4), (0, 0)).as_deref(), Some("hello"));
}

#[test]
fn extracts_multi_line_range() {
    let mut term = Terminal::new(20, 3);
    term.feed(b"abc\r\ndef");
    assert_eq!(
        term.text_in_range((0, 0), (1, 2)).as_deref(),
        Some("abc\ndef")
    );
}

#[test]
fn extracts_wide_chars_fully() {
    let mut term = Terminal::new(10, 2);
    term.feed("你好".as_bytes());
    assert_eq!(term.text_in_range((0, 0), (0, 3)).as_deref(), Some("你好"));
    assert_eq!(term.text_in_range((0, 0), (0, 1)).as_deref(), Some("你"));
}

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

/// 从视口起点拖到终点并提取文本。
fn extract(term: &mut Terminal, start: (u16, u16), end: (u16, u16)) -> Option<String> {
    term.start_selection(start.0, start.1);
    term.update_selection(end.0, end.1);
    term.take_selection_text()
}

#[test]
fn extracts_text_from_range() {
    let mut term = Terminal::new(20, 3);
    term.feed(b"hello world");
    assert_eq!(extract(&mut term, (0, 0), (0, 4)).as_deref(), Some("hello"));
    assert_eq!(extract(&mut term, (0, 4), (0, 0)).as_deref(), Some("hello"));
}

#[test]
fn extracts_multi_line_range() {
    let mut term = Terminal::new(20, 3);
    term.feed(b"abc\r\ndef");
    assert_eq!(
        extract(&mut term, (0, 0), (1, 2)).as_deref(),
        Some("abc\ndef")
    );
    assert_eq!(
        extract(&mut term, (1, 2), (0, 0)).as_deref(),
        Some("abc\ndef")
    );
}

#[test]
fn extracts_wide_chars_fully() {
    let mut term = Terminal::new(10, 2);
    term.feed("你好".as_bytes());
    assert_eq!(extract(&mut term, (0, 0), (0, 3)).as_deref(), Some("你好"));
    assert_eq!(extract(&mut term, (0, 0), (0, 1)).as_deref(), Some("你"));
}

#[test]
fn selection_rotates_with_output_scroll() {
    let mut term = Terminal::new(20, 3);
    for i in 0..10 {
        term.feed(format!("l{i:02}\r\n").as_bytes());
    }
    // 选中屏幕首行 l08；输出新行后网格行号整体下移，选区应仍指向 l08。
    term.start_selection(0, 0);
    term.update_selection(0, 3);
    term.feed(b"tail\r\n");
    assert_eq!(
        term.take_selection_text().as_deref(),
        Some("l08"),
        "selection must stay pinned to its content"
    );
}

#[test]
fn wheel_routing_follows_terminal_mode() {
    let mut term = Terminal::new(10, 3);
    assert_eq!(term.wheel_routing(), WheelRouting::HostScroll);

    term.feed(b"\x1b[?1049h\x1b[?1007h");
    assert_eq!(term.wheel_routing(), WheelRouting::AlternateScroll);

    term.feed(b"\x1b[?1000h");
    assert_eq!(term.wheel_routing(), WheelRouting::MouseReport);

    term.feed(b"\x1b[?1000l\x1b[?1049l");
    assert_eq!(term.wheel_routing(), WheelRouting::HostScroll);
}

#[test]
fn host_scroll_moves_viewport_and_pins_to_content() {
    let mut term = Terminal::new(20, 3);
    for i in 0..10 {
        term.feed(format!("l{i:02}\r\n").as_bytes());
    }
    assert_eq!(term.display_offset(), 0);

    assert!(term.scroll_display(1));
    assert_eq!(term.display_offset(), 1);

    // 新输出到达时视口钉在原内容上（offset 与网格行同步移动），不把用户踢回底部。
    term.feed(b"tail\r\n");
    assert_eq!(term.display_offset(), 2);

    assert!(!term.scroll_display(0));
    assert!(term.scroll_display(100));
    let top = term.display_offset();
    assert!(!term.scroll_display(1));
    assert_eq!(term.display_offset(), top);
}

#[test]
fn scroll_to_bottom_resets_viewport() {
    let mut term = Terminal::new(20, 3);
    for i in 0..10 {
        term.feed(format!("l{i:02}\r\n").as_bytes());
    }
    assert!(term.scroll_display(3));
    assert!(term.scroll_to_bottom());
    assert_eq!(term.display_offset(), 0);
    assert!(!term.scroll_to_bottom());
}

#[test]
fn selection_extracts_scrolled_back_content() {
    let mut term = Terminal::new(20, 3);
    for i in 0..10 {
        term.feed(format!("l{i:02}\r\n").as_bytes());
    }
    // 滚回两行后视口首行是 l06，按视口坐标选择提取的仍是屏幕上看到的内容。
    assert!(term.scroll_display(2));
    assert_eq!(extract(&mut term, (0, 0), (0, 3)).as_deref(), Some("l06"));
}

#[test]
fn clear_screen_drops_scrollback_and_blanks_viewport() {
    let mut term = Terminal::new(20, 5);
    term.feed(b"$ one\r\n$ two\r\n$ three\r\n$ ");
    assert_eq!(term.history_size(), 0);
    term.feed(b"\x1b[H\x1b[2J");
    term.feed(b"$ ");
    assert_eq!(term.history_size(), 0);
    assert_eq!(term.display_offset(), 0);
    let grid = term.grid();
    assert_eq!(grid[Line(0)][Column(0)].c, '$');
    assert_eq!(grid[Line(1)][Column(0)].c, ' ');
}

#[test]
fn clear_on_alternate_screen_keeps_primary_scrollback() {
    let mut term = Terminal::new(20, 5);
    for i in 0..8 {
        term.feed(format!("line {i}\r\n").as_bytes());
    }
    let history = term.history_size();
    assert!(history > 0);
    term.feed(b"\x1b[?1049h");
    term.feed(b"\x1b[H\x1b[2J");
    term.feed(b"\x1b[?1049l");
    assert_eq!(term.history_size(), history);
}

#[test]
fn scroll_to_content_offset_moves_viewport_from_top() {
    let mut term = Terminal::new(20, 5);
    for i in 0..10 {
        term.feed(format!("line {i}\r\n").as_bytes());
    }
    let history = term.history_size();
    assert!(history > 0);
    assert!(term.scroll_to_content_offset(0));
    assert_eq!(term.display_offset(), history);
    assert!(term.scroll_to_content_offset(history));
    assert_eq!(term.display_offset(), 0);
    assert!(!term.scroll_to_content_offset(history));
}

//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn renders_text_and_skips_wide_spacers() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed("hi 你好".as_bytes());
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false);
    assert_eq!(cursor, None);
    assert_eq!(buf[(0, 0)].symbol(), "h");
    assert_eq!(buf[(1, 0)].symbol(), "i");
    assert_eq!(buf[(3, 0)].symbol(), "你");
    assert_eq!(buf[(4, 0)].diff_option, CellDiffOption::Skip);
    assert_eq!(buf[(5, 0)].symbol(), "好");
    assert_eq!(buf[(6, 0)].diff_option, CellDiffOption::Skip);
}

#[test]
fn focused_terminal_returns_cursor_position() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"ab");
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, true);
    assert_eq!(cursor, Some((0, 2)));
}

#[test]
fn hidden_cursor_is_not_returned() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"\x1b[?25l");
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, true);
    assert_eq!(cursor, None);
}

#[test]
fn cursor_outside_area_is_not_returned() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"ab");
    let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
    let cursor = render(Rect::new(0, 0, 1, 1), &mut buf, &terminal, true);
    assert_eq!(cursor, None);
}

#[test]
fn selection_marks_cells_reversed() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"hello");
    terminal.start_selection(0, 1);
    terminal.update_selection(0, 3);
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    let cursor = render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false);
    assert_eq!(cursor, None);
    assert!(buf[(1, 0)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(3, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(4, 0)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn selection_follows_content_when_scrolled() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"one\r\ntwo\r\nthree");
    // 屏幕显示 two/three；选中 two 后滚回一行，选区仍钉在 two 上（换到屏幕第 2 行）。
    terminal.start_selection(0, 0);
    terminal.update_selection(0, 2);
    assert!(terminal.scroll_display(1));
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false);
    assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED), "one");
    assert!(buf[(0, 1)].modifier.contains(Modifier::REVERSED), "two");
}

#[test]
fn scrollbar_appears_only_for_local_scrollback() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"only");
    assert_eq!(scrollbar(Rect::new(0, 0, 10, 2), &terminal), None);

    for i in 0..8 {
        terminal.feed(format!("line {i}\r\n").as_bytes());
    }
    let bar = scrollbar(Rect::new(0, 0, 10, 2), &terminal).expect("scrollbar");
    assert_eq!(bar.track, Rect::new(9, 0, 1, 2));
    assert_eq!(bar.thumb.bottom(), bar.track.bottom());

    // 鼠标上报模式（opencode/claude 等）：应用自己滚动，隐藏滚动条。
    terminal.feed(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(scrollbar(Rect::new(0, 0, 10, 2), &terminal), None);

    // 备用屏且应用不接管滚轮时也没有本地内容可滚。
    terminal.feed(b"\x1b[?1000l\x1b[?1006l\x1b[?1049h");
    assert_eq!(scrollbar(Rect::new(0, 0, 10, 2), &terminal), None);
}

//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn renders_text_and_skips_wide_spacers() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed("hi 你好".as_bytes());
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, false, None);
    assert_eq!(buf[(0, 0)].symbol(), "h");
    assert_eq!(buf[(1, 0)].symbol(), "i");
    assert_eq!(buf[(3, 0)].symbol(), "你");
    assert_eq!(buf[(4, 0)].diff_option, CellDiffOption::Skip);
    assert_eq!(buf[(5, 0)].symbol(), "好");
    assert_eq!(buf[(6, 0)].diff_option, CellDiffOption::Skip);
}

#[test]
fn focused_terminal_paints_reversed_cursor() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"ab");
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    render(Rect::new(0, 0, 10, 2), &mut buf, &terminal, true, None);
    assert!(buf[(2, 0)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn selection_marks_cells_reversed() {
    let mut terminal = Terminal::new(10, 2);
    terminal.feed(b"hello");
    let mut selection = Selection::begin(crate::layout::PaneId::from_raw_for_test(1), 0, 1);
    selection.drag(0, 3);
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    render(
        Rect::new(0, 0, 10, 2),
        &mut buf,
        &terminal,
        false,
        Some(&selection),
    );
    assert!(buf[(1, 0)].modifier.contains(Modifier::REVERSED));
    assert!(buf[(3, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(4, 0)].modifier.contains(Modifier::REVERSED));
}

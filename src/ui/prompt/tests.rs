//! 单元测试；仅测试构建编译。

use super::*;
use crate::layout::PaneId;
use ratatui::style::Color;

fn prompt(cols: u16, rows: u16, text: &str) -> Prompt {
    let mut prompt = Prompt::new(PaneId::from_raw_for_test(1));
    prompt.resize(cols, rows);
    prompt.insert_str(text);
    prompt
}

fn selection(prompt: &Prompt, start: (u16, u16), end: (u16, u16)) -> Selection {
    let mut selection = Selection::begin(prompt.id(), start.0, start.1);
    selection.drag(end.0, end.1);
    selection
}

#[test]
fn paints_markdown_styles() {
    let area = Rect::new(0, 0, 20, 10);
    let prompt = prompt(20, 10, "## t\n**b**\n`c`\n> q\n- i\n[a](u)");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, false, None);

    assert_eq!(buf[(0, 0)].symbol(), "#");
    assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(3, 0)].symbol(), "t");
    assert_eq!(buf[(3, 0)].fg, Color::Yellow);
    assert!(buf[(3, 0)].modifier.contains(Modifier::BOLD));

    assert_eq!(buf[(0, 1)].symbol(), "*");
    assert_eq!(buf[(2, 1)].symbol(), "b");
    assert_eq!(buf[(2, 1)].fg, Color::Yellow);
    assert!(buf[(2, 1)].modifier.contains(Modifier::BOLD));

    assert_eq!(buf[(1, 2)].symbol(), "c");
    assert_eq!(buf[(1, 2)].fg, Color::LightRed);

    assert_eq!(buf[(2, 3)].symbol(), "q");
    assert_eq!(buf[(2, 3)].fg, Color::LightMagenta);
    assert!(buf[(2, 3)].modifier.contains(Modifier::ITALIC));

    assert_eq!(buf[(2, 4)].symbol(), "i");
    assert_eq!(buf[(2, 4)].fg, Color::Reset);
    assert!(!buf[(2, 4)].modifier.contains(Modifier::DIM));

    assert_eq!(buf[(1, 5)].symbol(), "a");
    assert_eq!(buf[(1, 5)].fg, Color::LightBlue);
    assert!(buf[(1, 5)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(buf[(4, 5)].symbol(), "u");
    assert_eq!(buf[(4, 5)].fg, Color::Cyan);
}

#[test]
fn fence_lines_are_markers_and_content_stays_default() {
    let area = Rect::new(0, 0, 10, 4);
    let prompt = prompt(10, 4, "```\nx\n```");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, false, None);
    assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(0, 1)].symbol(), "x");
    assert_eq!(buf[(0, 1)].fg, Color::Reset);
    assert!(!buf[(0, 1)].modifier.contains(Modifier::DIM));
    assert!(buf[(0, 2)].modifier.contains(Modifier::DIM));
}

#[test]
fn cursor_is_reversed_only_when_focused() {
    let area = Rect::new(0, 0, 10, 3);
    let prompt = prompt(10, 3, "ab");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, false, None);
    assert!(!buf[(2, 0)].modifier.contains(Modifier::REVERSED));

    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, true, None);
    assert!(buf[(2, 0)].modifier.contains(Modifier::REVERSED));
    assert!(!buf[(0, 0)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn renders_scrolled_viewport_and_cursor_cell() {
    let area = Rect::new(0, 0, 4, 2);
    let prompt = prompt(4, 2, "abcdefghij");
    assert_eq!(prompt.scroll(), 1);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, true, None);
    assert_eq!(buf[(0, 0)].symbol(), "e");
    assert_eq!(buf[(3, 0)].symbol(), "h");
    assert_eq!(buf[(0, 1)].symbol(), "i");
    assert!(buf[(2, 1)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn selection_reverses_cells_including_empty_trailing() {
    let area = Rect::new(0, 0, 10, 3);
    let prompt = prompt(10, 3, "abc");
    let mut buf = Buffer::empty(area);
    let selection = selection(&prompt, (0, 0), (0, 5));
    render(area, &mut buf, &prompt, false, Some(&selection));
    for col in 0..=5 {
        assert!(
            buf[(col, 0)].modifier.contains(Modifier::REVERSED),
            "column {col} should be selected"
        );
    }
    assert!(!buf[(6, 0)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn wide_chars_mark_spacer_cells() {
    let area = Rect::new(0, 0, 10, 3);
    let prompt = prompt(10, 3, "你好");
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, false, None);
    assert_eq!(buf[(0, 0)].symbol(), "你");
    assert_eq!(buf[(1, 0)].diff_option, CellDiffOption::Skip);
    assert_eq!(buf[(2, 0)].symbol(), "好");
    assert_eq!(buf[(3, 0)].diff_option, CellDiffOption::Skip);
}

//! 屏幕文本取样与复制片段测试。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use super::*;
use crate::app::selection::ScreenPoint;

fn buffer(lines: &[&str], width: u16) -> Buffer {
    let area = Rect::new(0, 0, width, lines.len() as u16);
    let mut buffer = Buffer::empty(area);
    for (y, line) in lines.iter().enumerate() {
        buffer.set_string(0, y as u16, line, ratatui::style::Style::default());
    }
    buffer
}

#[test]
fn screen_text_preserves_spaces_borders_and_wide_chars() {
    let buf = buffer(&["a你 b", "└─ x "], 6);
    let screen = ScreenText::from_buffer(&buf);
    assert_eq!(screen.row_text(0).as_deref(), Some("a你 b "));
    assert_eq!(screen.row_text(1).as_deref(), Some("└─ x  "));

    let cells = screen.selection_cells(ScreenPoint::new(0, 0), ScreenPoint::new(1, 5));
    assert_eq!(cells.text(), "a你 b \n└─ x  ");
    assert_eq!(
        cells.rows[0]
            .cells
            .iter()
            .map(|c| c.width)
            .collect::<Vec<_>>(),
        vec![1, 2, 1, 1, 1]
    );
}

#[test]
fn wide_character_is_copied_once_even_when_selecting_its_tail() {
    let screen = ScreenText::from_buffer(&buffer(&["你a"], 4));
    let cells = screen.selection_cells(ScreenPoint::new(0, 1), ScreenPoint::new(0, 2));
    assert_eq!(cells.text(), "你a");
}

#[test]
fn selection_freezes_visible_text_only() {
    let screen = ScreenText::from_buffer(&buffer(&["visible"], 10));
    let cells = screen.selection_cells(ScreenPoint::new(0, 0), ScreenPoint::new(0, 6));
    assert_eq!(cells.text(), "visible");
}

#[test]
fn reanchor_finds_scrolled_rows() {
    let before = ScreenText::from_buffer(&buffer(&["head", "alpha", "beta", "tail"], 8));
    let cells = before.selection_cells(ScreenPoint::new(1, 0), ScreenPoint::new(2, 3));
    assert_eq!(cells.text(), "alpha   \nbeta");

    let after = ScreenText::from_buffer(&buffer(&["new", "head", "alpha", "beta", "tail"], 8));
    let (start, end) = after.reanchor(&cells).expect("rows still visible");
    assert_eq!(start, ScreenPoint::new(2, 0));
    assert_eq!(end, ScreenPoint::new(3, 3));
}

#[test]
fn word_range_uses_unicode_word_boundaries() {
    let screen = ScreenText::from_buffer(&buffer(&["hello-world 中文"], 20));
    assert_eq!(
        screen.word_range(ScreenPoint::new(0, 2)),
        (ScreenPoint::new(0, 0), ScreenPoint::new(0, 4))
    );
    assert_eq!(
        screen.word_range(ScreenPoint::new(0, 14)),
        (ScreenPoint::new(0, 12), ScreenPoint::new(0, 15))
    );
}

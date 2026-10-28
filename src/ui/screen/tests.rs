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
    assert_eq!(cells.text(), "a你 b\n└─ x  ");
    assert_eq!(
        cells.rows[0]
            .cells
            .iter()
            .map(|c| c.width)
            .collect::<Vec<_>>(),
        vec![1, 2, 1, 1]
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
    assert_eq!(cells.text(), "alpha\nbeta");

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

#[test]
fn line_range_stops_at_vertical_borders_between_panes() {
    let screen =
        ScreenText::from_buffer(&buffer(&["┌────┬────┐", "│left│right│", "└────┴────┘"], 12));
    assert_eq!(
        screen.line_range(ScreenPoint::new(1, 2)),
        (ScreenPoint::new(1, 1), ScreenPoint::new(1, 4))
    );
    assert_eq!(
        screen.line_range(ScreenPoint::new(1, 7)),
        (ScreenPoint::new(1, 6), ScreenPoint::new(1, 10))
    );
    let left = screen.selection_cells(ScreenPoint::new(1, 1), ScreenPoint::new(1, 4));
    assert_eq!(left.text(), "left");
}

#[test]
fn line_range_keeps_border_titles_but_drops_border_only_rows() {
    let line = "╭─ Prompt ─ Todo ─────╮";
    let screen = ScreenText::from_buffer(&buffer(&[line], 24));
    let prompt = line[..line.find("Prompt").expect("prompt title")]
        .chars()
        .count() as u16
        + 2;
    let todo = line[..line.find("Todo").expect("todo title")]
        .chars()
        .count() as u16
        + 1;
    let prompt_range = screen.line_range(ScreenPoint::new(0, prompt));
    let todo_range = screen.line_range(ScreenPoint::new(0, todo));
    assert_eq!(
        screen
            .selection_cells(prompt_range.0, prompt_range.1)
            .text(),
        "Prompt"
    );
    assert_eq!(
        screen.selection_cells(todo_range.0, todo_range.1).text(),
        "Todo"
    );

    let border = ScreenText::from_buffer(&buffer(&["├──────────┤"], 12));
    let point = ScreenPoint::new(0, 5);
    assert_eq!(border.line_range(point), (point, point));
}

#[test]
fn line_range_keeps_leading_indent_and_trims_trailing_padding() {
    let screen = ScreenText::from_buffer(&buffer(&["│  x  │"], 8));
    assert_eq!(
        screen.line_range(ScreenPoint::new(0, 3)),
        (ScreenPoint::new(0, 1), ScreenPoint::new(0, 3))
    );
    let cells = screen.selection_cells(ScreenPoint::new(0, 1), ScreenPoint::new(0, 3));
    assert_eq!(cells.text(), "  x");
}

#[test]
fn line_range_without_right_border_stops_at_last_non_space() {
    // 后台任务输出/弹窗正文没有右侧竖边框：不能一直选到容器右缘。
    let screen = ScreenText::from_buffer(&buffer(&["❯ Read /src   "], 24));
    assert_eq!(
        screen.line_range(ScreenPoint::new(0, 4)),
        (ScreenPoint::new(0, 0), ScreenPoint::new(0, 10))
    );
    assert_eq!(
        screen
            .selection_cells(ScreenPoint::new(0, 0), ScreenPoint::new(0, 10))
            .text(),
        "❯ Read /src"
    );
}

#[test]
fn multi_row_selection_is_clipped_to_container_columns() {
    let line = "│side│left │right│";
    let screen = ScreenText::from_buffer(&buffer(&[line, line, line], 18));
    assert_eq!(
        screen.line_range(ScreenPoint::new(1, 7)),
        (ScreenPoint::new(1, 6), ScreenPoint::new(1, 9))
    );
    let cells = screen.selection_cells(ScreenPoint::new(0, 7), ScreenPoint::new(2, 9));
    assert_eq!(cells.text(), "eft\nleft\nleft");
    for row in &cells.rows {
        assert!(row.start_col >= 6, "不得越过左侧容器边界：{row:?}");
        assert!(row.end_col <= 10, "不得越过右侧容器边界：{row:?}");
    }
}

#[test]
fn line_range_stops_before_right_scrollbar_thumb() {
    let screen = ScreenText::from_buffer(&buffer(&["Read /src   ▐ "], 16));
    assert_eq!(
        screen.line_range(ScreenPoint::new(0, 5)),
        (ScreenPoint::new(0, 0), ScreenPoint::new(0, 8))
    );
    let cells = screen.selection_cells(ScreenPoint::new(0, 0), ScreenPoint::new(0, 8));
    assert_eq!(cells.text(), "Read /src");
}

#[test]
fn line_range_on_blank_row_makes_no_selection() {
    let screen = ScreenText::from_buffer(&buffer(&["        "], 8));
    let point = ScreenPoint::new(0, 3);
    assert_eq!(screen.line_range(point), (point, point));
}

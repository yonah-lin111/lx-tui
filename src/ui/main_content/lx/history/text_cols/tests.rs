use ratatui::style::Style;

use super::*;
use crate::ui::main_content::lx::history::tool::tree::{self, Node, Section};

#[test]
fn trims_leading_and_trailing_whitespace() {
    assert_eq!(cols_from(0, ["  hello  "]), Some((2, 7)));
}

#[test]
fn empty_and_whitespace_lines_are_not_clickable() {
    assert_eq!(cols_from(0, [""]), None);
    assert_eq!(cols_from(0, ["   "]), None);
    assert_eq!(line_cols("\t  "), None);
}

#[test]
fn wide_chars_count_display_columns() {
    assert_eq!(cols_from(0, ["  中文"]), Some((2, 6)));
}

#[test]
fn tree_only_lines_are_not_clickable() {
    assert_eq!(cols_from(0, ["  │"]), None);
    assert_eq!(cols_from(0, ["├───┼───┤"]), None);
    assert_eq!(
        segment_cols(&[Segment::new("  └─ ", Style::default())]),
        None
    );
}

#[test]
fn tree_prefix_with_body_starts_at_prefix() {
    assert_eq!(cols_from(0, ["  ├─ cmd: ls"]), Some((2, 12)));
}

#[test]
fn md_line_cols_includes_indent() {
    let line = MdLine::plain(2, "hello", Style::default());
    assert_eq!(md_line_cols(&line), Some((2, 7)));
}

#[test]
fn layout_row_cols_matches_rows() {
    let sections = [Section::fixed(vec![
        Node::text("first", Style::default()),
        Node::text("  ", Style::default()),
    ])];
    let layout = tree::layout(&sections, 40);
    let cols = layout.row_cols();
    assert_eq!(cols.len(), layout.rows.len());
    assert!(cols[0].is_some());
    assert_eq!(cols[1], None);
}

//! 单元测试；仅测试构建编译。

use super::*;

fn prompt(cols: u16, rows: u16) -> Prompt {
    let mut prompt = Prompt::new(PaneId::from_raw_for_test(1));
    prompt.resize(cols, rows);
    prompt
}

#[test]
fn new_prompt_is_empty_with_single_row() {
    let prompt = prompt(10, 3);
    assert_eq!(prompt.text(), "");
    assert_eq!(
        prompt.visual_rows(),
        vec![VisualRow {
            line: 0,
            start: 0,
            end: 0
        }]
    );
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
}

#[test]
fn inserts_unicode_and_moves_by_character() {
    let mut prompt = prompt(10, 3);
    prompt.insert_str("a你");
    assert_eq!(prompt.text(), "a你");
    assert_eq!(prompt.cursor_cell(), Some((0, 3)));
    prompt.move_left();
    assert_eq!(prompt.cursor_cell(), Some((0, 1)));
    prompt.move_left();
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    prompt.move_left();
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    prompt.move_right();
    prompt.move_right();
    prompt.move_right();
    assert_eq!(prompt.cursor_cell(), Some((0, 3)));
}

#[test]
fn newline_backspace_and_delete_edit_text() {
    let mut prompt = prompt(10, 3);
    prompt.insert_str("ab");
    prompt.newline();
    prompt.insert_char('c');
    assert_eq!(prompt.text(), "ab\nc");
    prompt.backspace();
    assert_eq!(prompt.text(), "ab\n");
    prompt.backspace();
    assert_eq!(prompt.text(), "ab");
    prompt.move_home();
    prompt.delete();
    assert_eq!(prompt.text(), "b");
    prompt.move_end();
    prompt.delete();
    assert_eq!(prompt.text(), "b");
    prompt.backspace();
    prompt.backspace();
    assert_eq!(prompt.text(), "");
}

#[test]
fn insert_char_expands_tab_and_ignores_control() {
    let mut prompt = prompt(20, 3);
    prompt.insert_char('\t');
    assert_eq!(prompt.text(), "    ");
    prompt.insert_char('\x07');
    assert_eq!(prompt.text(), "    ");
    prompt.insert_char('\n');
    assert_eq!(prompt.text(), "    \n");
}

#[test]
fn insert_str_normalizes_line_endings_and_controls() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("a\r\nb\tc\x07d");
    assert_eq!(prompt.text(), "a\nb    cd");
}

#[test]
fn wraps_long_lines_at_content_width() {
    let mut prompt = prompt(4, 3);
    prompt.insert_str("hello");
    assert_eq!(
        prompt.visual_rows(),
        vec![
            VisualRow {
                line: 0,
                start: 0,
                end: 4
            },
            VisualRow {
                line: 0,
                start: 4,
                end: 5
            },
        ]
    );
}

#[test]
fn wide_chars_are_never_split_across_rows() {
    let mut prompt = prompt(4, 3);
    prompt.insert_str("你好你");
    assert_eq!(
        prompt.visual_rows(),
        vec![
            VisualRow {
                line: 0,
                start: 0,
                end: 6
            },
            VisualRow {
                line: 0,
                start: 6,
                end: 9
            },
        ]
    );
}

#[test]
fn cursor_moves_across_wrapped_rows_keeping_column() {
    let mut prompt = prompt(4, 3);
    prompt.insert_str("abcdefg");
    prompt.move_home();
    prompt.move_up();
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    prompt.move_right();
    prompt.move_right();
    assert_eq!(prompt.cursor_cell(), Some((0, 2)));
    prompt.move_down();
    assert_eq!(prompt.cursor_cell(), Some((1, 2)));
    prompt.move_up();
    prompt.move_up();
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    // 行宽恰好填满时，行尾与下一视觉行行首是同一位置，显示归入下一行。
    prompt.move_end();
    assert_eq!(prompt.cursor_cell(), Some((1, 0)));
    prompt.move_down();
    assert_eq!(prompt.cursor_cell(), Some((1, 3)));
    prompt.move_down();
    assert_eq!(prompt.cursor_cell(), Some((1, 3)));
}

#[test]
fn home_and_end_follow_visual_rows() {
    let mut prompt = prompt(4, 3);
    prompt.insert_str("abc\ndef");
    prompt.move_home();
    assert_eq!(prompt.cursor_cell(), Some((1, 0)));
    prompt.move_end();
    assert_eq!(prompt.cursor_cell(), Some((1, 3)));
    prompt.move_up();
    assert_eq!(prompt.cursor_cell(), Some((0, 3)));
    prompt.move_home();
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    prompt.move_end();
    assert_eq!(prompt.cursor_cell(), Some((0, 3)));
    prompt.move_right();
    assert_eq!(prompt.cursor_cell(), Some((1, 0)));
}

#[test]
fn scroll_follows_cursor_and_viewport_cell_tracks_it() {
    let mut prompt = prompt(10, 2);
    prompt.insert_str("1\n2\n3\n4");
    assert_eq!(prompt.scroll(), 2);
    assert_eq!(prompt.cursor_cell(), Some((1, 1)));
    prompt.move_up();
    assert_eq!(prompt.cursor_cell(), Some((0, 1)));
    prompt.move_up();
    assert_eq!(prompt.scroll(), 1);
    assert_eq!(prompt.cursor_cell(), Some((0, 1)));
}

#[test]
fn selection_extracts_inclusive_range_and_normalizes_order() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("hello");
    assert_eq!(
        prompt.selection_text((0, 1), (0, 3)).as_deref(),
        Some("ell")
    );
    assert_eq!(
        prompt.selection_text((0, 3), (0, 1)).as_deref(),
        Some("ell")
    );
    assert_eq!(prompt.selection_text((0, 0), (0, 0)), None);
    prompt.insert_str("\nworld");
    assert_eq!(
        prompt.selection_text((0, 0), (1, 0)).as_deref(),
        Some("hello\nw")
    );
}

#[test]
fn selection_spans_wrapped_rows_and_wide_chars() {
    let mut wrapped = prompt(4, 3);
    wrapped.insert_str("abcdefg");
    assert_eq!(
        wrapped.selection_text((0, 2), (1, 1)).as_deref(),
        Some("cdef")
    );

    let mut wide = prompt(10, 3);
    wide.insert_str("你好");
    assert_eq!(wide.selection_text((0, 0), (0, 1)).as_deref(), Some("你"));
}

#[test]
fn selection_maps_through_scroll() {
    let mut prompt = prompt(10, 2);
    prompt.insert_str("1\n2\n3\n4");
    assert_eq!(prompt.scroll(), 2);
    assert_eq!(
        prompt.selection_text((0, 0), (1, 0)).as_deref(),
        Some("3\n4")
    );
}

#[test]
fn resize_keeps_cursor_visible_and_guards_zero_size() {
    let mut prompt = prompt(20, 2);
    prompt.insert_str("0123456789");
    prompt.resize(4, 2);
    assert!(prompt.cursor_cell().is_some());
    prompt.resize(0, 0);
    assert_eq!(prompt.size(), (1, 1));
    assert!(prompt.cursor_cell().is_some());
}

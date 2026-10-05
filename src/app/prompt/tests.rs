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
    assert_eq!(prompt.text(), "  ");
    prompt.insert_char('\x07');
    assert_eq!(prompt.text(), "  ");
    prompt.insert_char('\n');
    assert_eq!(prompt.text(), "  \n");
}

#[test]
fn insert_str_normalizes_line_endings_and_controls() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("a\r\nb\tc\x07d");
    assert_eq!(prompt.text(), "a\nb  cd");
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
fn replace_range_swaps_selection_and_undo_restores_it() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("hello world");
    let bounds = prompt.selection_bounds((0, 6), (0, 10));
    assert_eq!(bounds, Some((6, 11)));
    assert!(prompt.replace_range(6, 11, "rust"));
    assert_eq!(prompt.text(), "hello rust");
    assert_eq!(prompt.cursor_cell(), Some((0, 10)));
    prompt.undo();
    assert_eq!(prompt.text(), "hello world");

    assert!(!prompt.replace_range(3, 3, "x"));
    assert!(!prompt.replace_range(0, 100, "x"));
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
fn click_maps_viewport_cells_to_cursor() {
    let mut editor = prompt(4, 3);
    editor.insert_str("abcd\nef");
    editor.set_cursor_from_cell(0, 2);
    assert_eq!(editor.cursor_cell(), Some((0, 2)));
    editor.set_cursor_from_cell(1, 1);
    assert_eq!(editor.cursor_cell(), Some((1, 1)));
    // 点击文本以下空白钳到最近行行尾
    editor.set_cursor_from_cell(2, 3);
    assert_eq!(editor.cursor_cell(), Some((1, 2)));
    // 宽字符任一格都落在字符前
    let mut wide = prompt(10, 3);
    wide.insert_str("你好");
    wide.set_cursor_from_cell(0, 1);
    assert_eq!(wide.cursor_cell(), Some((0, 0)));
}

#[test]
fn scroll_by_moves_viewport_without_cursor_and_reattaches_on_edit() {
    let mut prompt = prompt(10, 2);
    prompt.insert_str("1\n2\n3\n4");
    assert_eq!(prompt.scroll(), 2);
    prompt.scroll_by(-1);
    assert_eq!(prompt.scroll(), 1);
    assert_eq!(prompt.cursor_cell(), None);
    prompt.scroll_by(-5);
    assert_eq!(prompt.scroll(), 0);
    prompt.scroll_by(1);
    assert_eq!(prompt.scroll(), 1);
    prompt.scroll_by(5);
    assert_eq!(prompt.scroll(), 2);
    prompt.insert_char('x');
    assert_eq!(prompt.scroll(), 2);
    assert!(prompt.cursor_cell().is_some());
}

#[test]
fn scroll_by_keeps_viewport_fixed_within_short_text() {
    let mut prompt = prompt(10, 5);
    prompt.insert_str("1\n2");
    assert_eq!(prompt.scroll(), 0);
    prompt.scroll_by(3);
    assert_eq!(prompt.scroll(), 0);
}

#[test]
fn word_movement_skips_separators_and_cjk_counts_as_word() {
    let mut prompt = prompt(40, 3);
    prompt.insert_str("foo bar_2 中文 baz");
    prompt.move_word_backward();
    assert_eq!(prompt.cursor_cell(), Some((0, 15)));
    prompt.move_word_backward();
    assert_eq!(prompt.cursor_cell(), Some((0, 10)));
    prompt.move_word_backward();
    assert_eq!(prompt.cursor_cell(), Some((0, 4)));
    prompt.move_word_backward();
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    prompt.move_word_backward();
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    prompt.move_word_forward();
    assert_eq!(prompt.cursor_cell(), Some((0, 3)));
    prompt.move_word_forward();
    assert_eq!(prompt.cursor_cell(), Some((0, 9)));
    prompt.move_word_forward();
    assert_eq!(prompt.cursor_cell(), Some((0, 14)));
    prompt.move_word_forward();
    assert_eq!(prompt.cursor_cell(), Some((0, 18)));
    prompt.move_word_forward();
    assert_eq!(prompt.cursor_cell(), Some((0, 18)));
}

#[test]
fn word_deletion_removes_adjacent_separators() {
    let mut prompt = prompt(40, 3);
    prompt.insert_str("foo bar_2");
    prompt.delete_word_backward();
    assert_eq!(prompt.text(), "foo ");
    prompt.delete_word_backward();
    assert_eq!(prompt.text(), "");
    prompt.insert_str("foo   ");
    prompt.delete_word_backward();
    assert_eq!(prompt.text(), "");
    prompt.insert_str("a b");
    prompt.move_left();
    prompt.delete_word_forward();
    assert_eq!(prompt.text(), "a ");
    prompt.delete_word_forward();
    assert_eq!(prompt.text(), "a ");
}

#[test]
fn line_commands_use_logical_lines_not_visual_rows() {
    let mut prompt = prompt(4, 4);
    prompt.insert_str("abcdef");
    prompt.set_cursor_from_cell(1, 1);
    prompt.move_line_start();
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    prompt.move_line_end();
    assert_eq!(prompt.cursor_cell(), Some((1, 2)));
    prompt.set_cursor_from_cell(0, 2);
    prompt.delete_to_line_end();
    assert_eq!(prompt.text(), "ab");
    prompt.insert_str("cde");
    prompt.set_cursor_from_cell(0, 2);
    prompt.delete_to_line_start();
    assert_eq!(prompt.text(), "cde");
    prompt.delete_to_line_start();
    assert_eq!(prompt.text(), "cde");
}

#[test]
fn line_deletion_stops_at_newline() {
    let mut prompt = prompt(10, 3);
    prompt.insert_str("ab\ncd");
    prompt.move_up();
    prompt.move_line_start();
    prompt.delete_to_line_end();
    assert_eq!(prompt.text(), "\ncd");
    prompt.move_down();
    assert_eq!(prompt.cursor_cell(), Some((1, 0)));
    prompt.move_line_start();
    prompt.delete_to_line_start();
    assert_eq!(prompt.text(), "\ncd");
}

#[test]
fn typing_runs_merge_into_single_undo_step() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("abc");
    prompt.undo();
    assert_eq!(prompt.text(), "");
    assert_eq!(prompt.cursor_cell(), Some((0, 0)));
    prompt.redo();
    assert_eq!(prompt.text(), "abc");
}

#[test]
fn cursor_movement_breaks_merge_group() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("ab");
    prompt.move_left();
    prompt.insert_char('c');
    assert_eq!(prompt.text(), "acb");
    prompt.undo();
    assert_eq!(prompt.text(), "ab");
    prompt.undo();
    assert_eq!(prompt.text(), "");
}

#[test]
fn backspace_runs_merge_and_new_edit_clears_redo() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("abcd");
    prompt.backspace();
    prompt.backspace();
    assert_eq!(prompt.text(), "ab");
    prompt.undo();
    assert_eq!(prompt.text(), "abcd");
    prompt.redo();
    assert_eq!(prompt.text(), "ab");
    prompt.insert_char('x');
    assert_eq!(prompt.text(), "abx");
    prompt.redo();
    assert_eq!(prompt.text(), "abx");
}

#[test]
fn plain_newline_merges_with_typing() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("a");
    prompt.newline();
    prompt.insert_str("b");
    prompt.undo();
    assert_eq!(prompt.text(), "");
    prompt.redo();
    assert_eq!(prompt.text(), "a\nb");
}

#[test]
fn newline_continues_bullet_and_ordered_lists() {
    let mut bullet = prompt(20, 3);
    bullet.insert_str("- a");
    bullet.newline();
    assert_eq!(bullet.text(), "- a\n- ");
    bullet.insert_str("b");
    bullet.newline();
    assert_eq!(bullet.text(), "- a\n- b\n- ");

    let mut ordered = prompt(20, 3);
    ordered.insert_str("3) item");
    ordered.newline();
    assert_eq!(ordered.text(), "3) item\n4) ");

    let mut dotted = prompt(20, 3);
    dotted.insert_str("9. item");
    dotted.newline();
    assert_eq!(dotted.text(), "9. item\n10. ");
}

#[test]
fn newline_continues_nested_and_task_items() {
    let mut nested = prompt(20, 4);
    nested.insert_str("  * a");
    nested.newline();
    assert_eq!(nested.text(), "  * a\n  * ");

    let mut task = prompt(20, 3);
    task.insert_str("- [x] done");
    task.newline();
    assert_eq!(task.text(), "- [x] done\n- [ ] ");
}

#[test]
fn newline_splits_content_and_keeps_marker() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("- abcd");
    prompt.move_left();
    prompt.move_left();
    prompt.newline();
    assert_eq!(prompt.text(), "- ab\n- cd");
}

#[test]
fn newline_below_inserts_at_line_end_keeping_indent() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("  - abcd");
    prompt.move_left();
    prompt.move_left();
    prompt.newline_below();
    assert_eq!(prompt.text(), "  - abcd\n  ");
    assert_eq!(prompt.cursor_cell(), Some((1, 2)));
}

#[test]
fn empty_item_exits_list_level() {
    let mut top = prompt(20, 3);
    top.insert_str("- ");
    top.newline();
    assert_eq!(top.text(), "");
    assert_eq!(top.cursor_cell(), Some((0, 0)));

    let mut nested = prompt(20, 4);
    nested.insert_str("- a\n  - ");
    nested.newline();
    assert_eq!(nested.text(), "- a\n- ");
}

#[test]
fn list_continuation_is_its_own_undo_step() {
    let mut prompt = prompt(20, 3);
    prompt.insert_str("- a");
    prompt.newline();
    assert_eq!(prompt.text(), "- a\n- ");
    prompt.undo();
    assert_eq!(prompt.text(), "- a");
    prompt.undo();
    assert_eq!(prompt.text(), "");
}

#[test]
fn indent_and_outdent_move_whole_logical_line() {
    let mut editor = prompt(20, 3);
    editor.insert_str("- a");
    editor.indent();
    assert_eq!(editor.text(), "  - a");
    assert_eq!(editor.cursor_cell(), Some((0, 5)));
    editor.outdent();
    assert_eq!(editor.text(), "- a");
    editor.outdent();
    assert_eq!(editor.text(), "- a");

    let mut empty = prompt(20, 3);
    empty.indent();
    assert_eq!(empty.text(), "  ");
    assert_eq!(empty.cursor_cell(), Some((0, 2)));
}

#[test]
fn indent_affects_current_logical_line_only() {
    let mut prompt = prompt(20, 4);
    prompt.insert_str("a\nb");
    prompt.move_up();
    prompt.indent();
    assert_eq!(prompt.text(), "  a\nb");
}

#[test]
fn backspace_replaces_list_marker_with_blanks() {
    let mut editor = prompt(20, 3);
    editor.insert_str("- ab");
    editor.move_left();
    editor.move_left();
    editor.backspace();
    assert_eq!(editor.text(), "  ab");
    assert_eq!(editor.cursor_cell(), Some((0, 2)));
    editor.backspace();
    assert_eq!(editor.text(), " ab");

    let mut ordered = prompt(20, 3);
    ordered.insert_str("1. ");
    ordered.backspace();
    assert_eq!(ordered.text(), "   ");
}

#[test]
fn fence_disables_list_and_markup_behavior() {
    let mut editor = prompt(20, 4);
    editor.insert_str("```\n- a");
    editor.newline();
    assert_eq!(editor.text(), "```\n- a\n");
    editor.insert_str("x");
    editor.move_home();
    editor.indent();
    assert_eq!(editor.text(), "```\n- a\n  x");
    let mut markup = prompt(20, 3);
    markup.insert_str("```\n- ");
    markup.backspace();
    assert_eq!(markup.text(), "```\n-");
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

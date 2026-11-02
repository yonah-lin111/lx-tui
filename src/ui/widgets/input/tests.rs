//! 单行视口、多行软折行/3 行截断与点击定位的单元测试。

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

#[test]
fn ascii_view_shows_all_and_cursor_at_end() {
    let view = view("abc", 3, 10);
    assert_eq!(view.visible, "abc");
    assert_eq!(view.cursor_col, 3);
}

#[test]
fn wide_chars_count_two_columns() {
    let view = view("中文名称", 4, 20);
    assert_eq!(view.visible, "中文名称");
    assert_eq!(view.cursor_col, 8);
}

#[test]
fn long_text_scrolls_to_keep_cursor_visible() {
    let text = "x".repeat(60);
    let view = view(&text, 60, 38);
    assert_eq!(view.visible.chars().count(), 37);
    assert_eq!(view.cursor_col, 37);
}

#[test]
fn wide_text_scrolls_by_display_width() {
    let text = "中".repeat(30);
    let view = view(&text, 30, 38);
    assert_eq!(view.visible.chars().count(), 18);
    assert_eq!(view.cursor_col, 36);
}

#[test]
fn cursor_beyond_text_clamps_to_end() {
    let view = view("ab", 99, 10);
    assert_eq!(view.visible, "ab");
    assert_eq!(view.cursor_col, 2);
}

#[test]
fn empty_width_returns_empty_view() {
    let view = view("abc", 1, 0);
    assert_eq!(view.visible, "");
    assert_eq!(view.cursor_col, 0);
}

#[test]
fn render_draws_text_and_returns_hardware_cursor() {
    let mut terminal = Terminal::new(TestBackend::new(20, 3)).expect("test backend is infallible");
    let mut cursor = None;
    if let Err(error) = terminal.draw(|frame| {
        cursor = render(frame, Rect::new(2, 1, 10, 1), "中文", 2, None);
    }) {
        panic!("draw failed: {error}");
    }
    assert_eq!(cursor, Some((6, 1)));
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(buffer[(2, 1)].symbol(), "中");
    assert_eq!(buffer[(4, 1)].symbol(), "文");
}

#[test]
fn empty_text_renders_placeholder_dimmed() {
    let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("test backend is infallible");
    let mut cursor = None;
    if let Err(error) = terminal.draw(|frame| {
        cursor = render(frame, Rect::new(0, 0, 20, 1), "", 0, Some("new name"));
    }) {
        panic!("draw failed: {error}");
    }
    assert_eq!(cursor, Some((0, 0)));
    let buffer = terminal.backend().buffer().clone();
    let line: String = (0..20).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(line.starts_with("new name"), "line={line:?}");
    assert!(buffer[(0, 0)].modifier.contains(Modifier::DIM));
}

#[test]
fn non_empty_text_suppresses_placeholder() {
    let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| {
        render(frame, Rect::new(0, 0, 20, 1), "abc", 3, Some("hint"));
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let line: String = (0..20).map(|x| buffer[(x, 0)].symbol()).collect();
    assert!(line.starts_with("abc"), "line={line:?}");
    assert!(!line.contains("hint"), "line={line:?}");
    assert!(!buffer[(0, 0)].modifier.contains(Modifier::DIM));
}

#[test]
fn char_index_at_col_computes_exact_char_index() {
    let text = "hello 中文 world";
    assert_eq!(char_index_at_col(text, 0, 30, 0), 0);
    assert_eq!(char_index_at_col(text, 0, 30, 4), 4);
    assert_eq!(char_index_at_col(text, 0, 30, 6), 6); // '中' 的位置
    assert_eq!(char_index_at_col(text, 0, 30, 7), 6); // '中' 的第二半列仍算 '中'
    assert_eq!(char_index_at_col(text, 0, 30, 8), 7); // '文'
}

#[test]
fn wrap_clamped_lines_keeps_short_text_on_one_line() {
    assert_eq!(wrap_clamped_lines("abc", 10, 3), vec!["abc".to_string()]);
    assert_eq!(wrap_clamped_lines("", 10, 3), vec![String::new()]);
}

#[test]
fn wrap_clamped_lines_wraps_by_display_width() {
    assert_eq!(
        wrap_clamped_lines("abcdef", 3, 3),
        vec!["abc".to_string(), "def".to_string()]
    );
    // 宽字符按两列计：宽 3 列只放得下一个 `中`。
    assert_eq!(
        wrap_clamped_lines("中文", 3, 3),
        vec!["中".to_string(), "文".to_string()]
    );
}

#[test]
fn wrap_clamped_lines_respects_explicit_newlines() {
    assert_eq!(
        wrap_clamped_lines("a\nbc", 5, 3),
        vec!["a".to_string(), "bc".to_string()]
    );
}

#[test]
fn wrap_clamped_lines_truncates_after_max_rows_with_ellipsis() {
    let lines = wrap_clamped_lines("abcdefghij", 3, 3);
    assert_eq!(
        lines,
        vec!["abc".to_string(), "def".to_string(), "gh…".to_string()],
        "超 3 行保留前 3 行，末行留 1 列给省略号"
    );
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(lines[2].as_str()),
        3,
        "末行显示宽度不得超过 width"
    );
}

#[test]
fn wrap_clamped_lines_ellipsis_keeps_wide_chars_whole() {
    let lines = wrap_clamped_lines("中文字", 4, 1);
    assert_eq!(lines, vec!["中…".to_string()], "不得切出半个宽字符");
    assert_eq!(unicode_width::UnicodeWidthStr::width(lines[0].as_str()), 3);
}

#[test]
fn multiline_view_places_cursor_at_wrap_boundary_on_next_line() {
    let view = multiline_view("abcdef", 5, 5, 3);
    assert_eq!(view.visible_lines.len(), 2);
    assert_eq!(view.cursor_row_offset, 1, "软折行边界归下一行");
    assert_eq!(view.cursor_col, 0);
    assert_eq!(view.visible_lines[1].text, "f");
    assert_eq!(view.visible_lines[1].start_char, 5);
}

#[test]
fn multiline_view_keeps_cursor_before_hard_break_on_current_line() {
    let view = multiline_view("abc\ndef", 3, 5, 3);
    assert_eq!(view.visible_lines.len(), 2);
    assert_eq!(view.cursor_row_offset, 0);
    assert_eq!(view.cursor_col, 3, "硬换行前停在行尾");
    assert_eq!(view.visible_lines[1].text, "def");
}

#[test]
fn multiline_view_scrolls_window_to_keep_cursor_visible() {
    let view = multiline_view("abcdefghij", 9, 3, 3);
    assert_eq!(view.visible_lines.len(), 3, "超过 3 行时视口固定 3 行");
    assert_eq!(view.cursor_row_offset, 2, "光标在可见窗口最后一行");
    assert_eq!(view.cursor_col, 0);
    assert_eq!(view.visible_lines[0].start_char, 3, "窗口从第 2 行开始");
}

#[test]
fn multiline_char_index_at_maps_rows_back_to_char_index() {
    let text = "abcdefghij";
    assert_eq!(multiline_char_index_at(text, 0, 3, 3, 0, 0), 0);
    assert_eq!(multiline_char_index_at(text, 0, 3, 3, 1, 1), 4);
    assert_eq!(multiline_char_index_at(text, 0, 3, 3, 2, 99), 9);
}

#[test]
fn multiline_char_index_at_uses_visible_window() {
    let text = "abcdefghij";
    // 光标在第 4 行 → 视口从第 2 行开始，行偏移按可见窗口换算。
    assert_eq!(multiline_char_index_at(text, 9, 3, 3, 0, 0), 3);
    assert_eq!(multiline_char_index_at(text, 9, 3, 3, 1, 2), 8);
}

#[test]
fn multiline_view_wraps_full_width_line_end_cursor_to_next_row() {
    // 满宽行末尾（ASCII 与 CJK）光标软折到下一视觉行首列，避免与末字符重叠。
    let ascii = multiline_view("abcde", 5, 5, 3);
    assert_eq!(ascii.visible_lines.len(), 2);
    assert_eq!(ascii.visible_lines[0].text, "abcde");
    assert_eq!(ascii.visible_lines[1].text, "");
    assert_eq!(ascii.cursor_row_offset, 1);
    assert_eq!(ascii.cursor_col, 0);

    let cjk = multiline_view("中文字", 3, 6, 3);
    assert_eq!(cjk.visible_lines.len(), 2);
    assert_eq!(cjk.cursor_row_offset, 1);
    assert_eq!(cjk.cursor_col, 0);

    // 拖选/点击越过满宽行右缘时可到达 `line.end_char` 覆盖末字符。
    assert_eq!(multiline_char_index_at("abcde", 0, 5, 3, 0, 5), 5);
    assert_eq!(multiline_char_index_at("中文字", 0, 6, 3, 0, 6), 3);
}

#[test]
fn multiline_view_scrolled_keeps_window_stable_when_clicking_top_visible_row() {
    // 4 行文本（每行 3 列），当 `scroll_row = 1` 显示第 1..=3 行时，
    // 点击可见第 0 行（全文第 1 行，cursor = 4）不应把 `start_row` 跳回 0。
    let text = "abcdefghij";
    let initial = multiline_view_scrolled(text, 9, 3, 3, 0);
    assert_eq!(initial.start_row, 1);
    assert_eq!(initial.cursor_row_offset, 2);

    let clicked_char = multiline_char_index_at_scrolled(text, 9, 3, 3, initial.start_row, 0, 1);
    assert_eq!(clicked_char, 4, "可见首行第 1 列对应 'e'");

    let after_click = multiline_view_scrolled(text, clicked_char, 3, 3, initial.start_row);
    assert_eq!(
        after_click.start_row, 1,
        "点击可见首行后视口起点保持在 1，不跳回 0"
    );
    assert_eq!(after_click.cursor_row_offset, 0);
    assert_eq!(after_click.cursor_col, 1);

    // 继续上移光标到第 0 行（cursor = 1）时才上滚回 0。
    let scrolled_up = multiline_view_scrolled(text, 1, 3, 3, after_click.start_row);
    assert_eq!(scrolled_up.start_row, 0);
    assert_eq!(scrolled_up.cursor_row_offset, 0);
}

#[test]
fn render_with_selection_highlights_reversed() {
    let mut terminal = Terminal::new(TestBackend::new(20, 1)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| {
        render_with_selection(
            frame,
            Rect::new(0, 0, 20, 1),
            "abcdef",
            3,
            Some((2, 4)), // "cd"
            None,
        );
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert!(!buffer[(0, 0)].modifier.contains(Modifier::REVERSED)); // 'a'
    assert!(!buffer[(1, 0)].modifier.contains(Modifier::REVERSED)); // 'b'
    assert!(buffer[(2, 0)].modifier.contains(Modifier::REVERSED)); // 'c'
    assert!(buffer[(3, 0)].modifier.contains(Modifier::REVERSED)); // 'd'
    assert!(!buffer[(4, 0)].modifier.contains(Modifier::REVERSED)); // 'e'
}

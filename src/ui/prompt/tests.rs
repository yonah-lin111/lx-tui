//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::markdown::MentionEntry;
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
    render(area, &mut buf, &prompt, None);

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
    render(area, &mut buf, &prompt, None);
    assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
    assert_eq!(buf[(0, 1)].symbol(), "x");
    assert_eq!(buf[(0, 1)].fg, Color::Reset);
    assert!(!buf[(0, 1)].modifier.contains(Modifier::DIM));
    assert!(buf[(0, 2)].modifier.contains(Modifier::DIM));
}

#[test]
fn renders_scrolled_viewport() {
    let area = Rect::new(0, 0, 4, 2);
    let prompt = prompt(4, 2, "abcdefghij");
    assert_eq!(prompt.scroll(), 1);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].symbol(), "e");
    assert_eq!(buf[(3, 0)].symbol(), "h");
    assert_eq!(buf[(0, 1)].symbol(), "i");
}

#[test]
fn wrapped_continuation_row_keeps_line_token_offsets() {
    let area = Rect::new(0, 0, 80, 3);
    let prompt = prompt(
        80,
        3,
        "- Reference: @[refer-folder](/Users/yonah/projects/agent/herdr-master) \
         @[refer-folder](/Users/yonah/projects/agent/lx-agent)",
    );
    assert_eq!(prompt.visual_rows().len(), 2);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 1)].symbol(), "o");
    assert_eq!(buf[(0, 1)].fg, Color::LightBlue);
    assert!(buf[(0, 1)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(buf[(7, 1)].symbol(), "/");
    assert_eq!(buf[(7, 1)].fg, Color::Cyan);
}

#[test]
fn scrolled_viewport_hides_cursor_when_caret_leaves_view() {
    let area = Rect::new(0, 0, 4, 2);
    let mut prompt = prompt(4, 2, "abcdefghijklm");
    assert_eq!(prompt.scroll(), 2);
    prompt.scroll_by(-1);
    assert_eq!(prompt.scroll(), 1);
    assert_eq!(prompt.cursor_cell(), None);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].symbol(), "e");
}

#[test]
fn selection_reverses_cells_including_empty_trailing() {
    let area = Rect::new(0, 0, 10, 3);
    let prompt = prompt(10, 3, "abc");
    let mut buf = Buffer::empty(area);
    let selection = selection(&prompt, (0, 0), (0, 5));
    render(area, &mut buf, &prompt, Some(&selection));
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
    render(area, &mut buf, &prompt, None);
    assert_eq!(buf[(0, 0)].symbol(), "你");
    assert_eq!(buf[(1, 0)].diff_option, CellDiffOption::Skip);
    assert_eq!(buf[(2, 0)].symbol(), "好");
    assert_eq!(buf[(3, 0)].diff_option, CellDiffOption::Skip);
}

#[test]
fn renders_block_command_panel_below_cursor() {
    let area = Rect::new(0, 0, 30, 10);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 10);
    editor.insert_str("#");
    assert!(editor.panel().is_some());
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    assert_eq!(buf[(0, 1)].symbol(), "╭");
    let row: String = (0..area.width).map(|x| buf[(x, 2)].symbol()).collect();
    assert!(row.contains("Heading 1"));
    assert!(buf[(2, 2)].modifier.contains(Modifier::REVERSED));

    let mut empty = Buffer::empty(area);
    let blank = prompt(10, 3, "plain");
    render(area, &mut empty, &blank, None);
    assert_eq!(empty[(0, 1)].symbol(), " ");
}

#[test]
fn renders_mention_panel_stacked_and_hit_tests_items() {
    let area = Rect::new(0, 0, 30, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 20);
    editor.set_mention_root(Some(std::path::PathBuf::from("/tmp/ws")));
    editor.insert_str("@");
    let (generation, _) = editor.take_mention_scan_request().expect("scan requested");
    editor.apply_mention_entries(
        generation,
        vec![
            MentionEntry {
                path: "src/app.rs".into(),
                is_directory: false,
            },
            MentionEntry {
                path: "src/lib.rs".into(),
                is_directory: false,
            },
        ],
    );
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &editor, None);

    assert_eq!(buf[(0, 1)].symbol(), "╭");
    let name_row: String = (0..area.width).map(|x| buf[(x, 2)].symbol()).collect();
    let dir_row: String = (0..area.width).map(|x| buf[(x, 3)].symbol()).collect();
    assert!(name_row.contains("app.rs"));
    assert!(dir_row.contains("src"));
    assert!(buf[(3, 3)].modifier.contains(Modifier::DIM));
    assert!(buf[(2, 2)].modifier.contains(Modifier::REVERSED));

    assert_eq!(mention_item_at(&editor, area, 2, 2), Some(0));
    assert_eq!(mention_item_at(&editor, area, 2, 3), Some(0));
    assert_eq!(mention_item_at(&editor, area, 2, 4), Some(1));
    assert_eq!(mention_item_at(&editor, area, 2, 5), Some(1));
    assert_eq!(mention_item_at(&editor, area, 2, 6), None);
    assert_eq!(mention_item_at(&editor, area, 2, 1), None);
}

#[test]
fn mention_panel_window_keeps_still_when_hovering_visible_item() {
    let area = Rect::new(0, 0, 30, 20);
    let mut editor = Prompt::new(PaneId::from_raw_for_test(1));
    editor.resize(30, 20);
    editor.set_mention_root(Some(std::path::PathBuf::from("/tmp/ws")));
    editor.insert_str("@");
    let (generation, _) = editor.take_mention_scan_request().expect("scan requested");
    let entries: Vec<MentionEntry> = (0..10)
        .map(|index| MentionEntry {
            path: format!("src/f{index}.rs"),
            is_directory: false,
        })
        .collect();
    editor.apply_mention_entries(generation, entries);

    editor.mention_move(9);
    let rect = mention_panel_rect(&editor, area).expect("panel visible");
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 1), Some(6));

    // 悬停窗口内条目：窗口保持不动，不因悬停向上滚动。
    editor.mention_set_active(8);
    assert_eq!(mention_panel_rect(&editor, area), Some(rect));
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 1), Some(6));
    assert_eq!(mention_item_at(&editor, area, 2, rect.y + 7), Some(9));
}

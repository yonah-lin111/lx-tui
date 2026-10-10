//! 提示词树渲染单元测试：搜索框行（模态输入对齐）、连接符、状态标签、省略号、空状态与命中。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::*;
use crate::app::prompt::{PromptFolder, PromptItem, PromptItemStatus};

fn panel_with(folders: Vec<PromptFolder>, items: Vec<PromptItem>) -> PromptPanel {
    let mut panel = PromptPanel::new();
    panel.tree.replace(folders, items);
    panel
}

fn folder(id: i64, parent_id: Option<i64>, name: &str) -> PromptFolder {
    PromptFolder {
        id,
        parent_id,
        name: name.to_string(),
        collapsed: false,
    }
}

fn item(id: i64, folder_id: Option<i64>, name: &str, status: PromptItemStatus) -> PromptItem {
    PromptItem {
        id,
        folder_id,
        name: name.to_string(),
        content: String::new(),
        status,
    }
}

fn search(panel: &mut PromptPanel, query: &str) {
    panel.search.clear();
    for ch in query.chars() {
        panel.search.insert_char(ch);
    }
}

fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

#[test]
fn first_row_is_search_box_with_placeholder_and_cursor() {
    let mut panel = PromptPanel::new();
    let area = Rect::new(0, 0, 40, 4);
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &panel);
    let search_row = row_text(&buf, area, 0);
    assert!(
        search_row.starts_with(&format!(
            "{} {}",
            text::PROMPT_TREE_SEARCH_ICON,
            text::PROMPT_TREE_SEARCH_PLACEHOLDER
        )),
        "{search_row}"
    );
    assert_eq!(
        cursor,
        Some((2, 0)),
        "空过滤词光标在输入起点（图标与空档之后）"
    );

    search(&mut panel, "ide");
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &panel);
    let search_row = row_text(&buf, area, 0);
    assert!(search_row.starts_with("⌕ ide"), "{search_row}");
    assert_eq!(cursor, Some((5, 0)), "光标在过滤词末尾");
}

#[test]
fn search_row_scrolls_viewport_for_long_query() {
    let mut panel = PromptPanel::new();
    search(&mut panel, "0123456789");
    let area = Rect::new(0, 0, 8, 3);
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &panel);
    let search_row = row_text(&buf, area, 0);
    assert_eq!(search_row.trim_end(), "⌕ 56789", "视口右移保证光标可见");
    assert_eq!(cursor, Some((7, 0)), "光标在输入区最右列");
}

#[test]
fn renders_folder_and_item_rows_with_status() {
    let mut panel = panel_with(
        vec![folder(1, None, "ideas"), folder(2, Some(1), "deep")],
        vec![
            item(10, None, "loose", PromptItemStatus::Todo),
            item(11, Some(2), "nested", PromptItemStatus::InProgress),
        ],
    );
    let area = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);

    // 行 0 搜索框，树行从行 1 开始。
    let first = row_text(&buf, area, 1);
    assert!(first.starts_with("▾ ideas"), "{first}");
    // 子层同级还有后续条目：非末项连接符 `├─`。
    let second = row_text(&buf, area, 2);
    assert!(second.starts_with("├─ ▾ deep"), "{second}");
    // 子层最后一个可见后代：末项连接符 `└─`。
    let third = row_text(&buf, area, 3);
    assert!(third.starts_with("└─ • nested"), "{third}");
    assert!(third.contains("[run]"), "{third}");
    let fourth = row_text(&buf, area, 4);
    assert!(fourth.starts_with("• loose"), "{fourth}");
    assert!(fourth.contains("[todo]"), "{fourth}");

    panel.tree.toggle_folder(1);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let first = row_text(&buf, area, 1);
    assert!(first.starts_with("▸ ideas"), "{first}");
    let second = row_text(&buf, area, 2);
    assert!(second.starts_with("• loose"), "{second}");
}

#[test]
fn selected_row_gets_selection_background() {
    let mut panel = panel_with(
        vec![],
        vec![item(10, None, "final", PromptItemStatus::Completed)],
    );
    panel.selected = 0;
    let area = Rect::new(0, 0, 30, 3);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let cell = &buf[(29, 1)];
    assert_eq!(cell.bg, Color::Indexed(240), "整行铺选中底色");
    let status_cell = &buf[(24, 1)];
    assert_eq!(status_cell.symbol(), "[");
}

#[test]
fn long_names_are_ellipsized_within_width() {
    let panel = panel_with(
        vec![],
        vec![item(
            10,
            None,
            "a very long prompt name that cannot fit",
            PromptItemStatus::Todo,
        )],
    );
    let area = Rect::new(0, 0, 18, 3);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let text = row_text(&buf, area, 1);
    assert!(text.contains('…'), "{text}");
    assert_eq!(text.chars().count(), 18);
    assert!(text.ends_with("[todo]"), "{text}");
}

#[test]
fn empty_tree_shows_placeholder_under_search_row() {
    let panel = PromptPanel::new();
    let area = Rect::new(0, 0, 30, 4);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let text = row_text(&buf, area, 1);
    assert!(text.starts_with(text::PROMPT_TREE_EMPTY), "{text}");
}

#[test]
fn filter_without_matches_shows_hint_placeholder() {
    let mut panel = panel_with(
        vec![],
        vec![item(10, None, "alpha", PromptItemStatus::Todo)],
    );
    search(&mut panel, "zzz");
    let area = Rect::new(0, 0, 30, 4);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let text = row_text(&buf, area, 1);
    assert!(text.starts_with(text::PROMPT_TREE_NO_MATCHES), "{text}");
}

#[test]
fn filter_shows_ancestors_and_expands_collapsed_folders() {
    let mut panel = panel_with(
        vec![folder(1, None, "ideas"), folder(2, Some(1), "deep")],
        vec![
            item(10, None, "loose", PromptItemStatus::Todo),
            item(11, Some(2), "nested", PromptItemStatus::InProgress),
        ],
    );
    panel.tree.toggle_folder(1);
    panel.tree.toggle_folder(2);
    search(&mut panel, "nested");
    let area = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let first = row_text(&buf, area, 1);
    assert!(first.starts_with("▾ ideas"), "{first}");
    // deep 之后还有同深度行（其子项 nested）：非末项 `├─`；折叠被忽略按展开绘制。
    let second = row_text(&buf, area, 2);
    assert!(second.starts_with("├─ ▾ deep"), "{second}");
    let third = row_text(&buf, area, 3);
    assert!(third.starts_with("└─ • nested"), "{third}");
}

#[test]
fn row_at_maps_viewport_and_respects_scroll_and_search_row() {
    let mut panel = panel_with(
        vec![],
        vec![
            item(10, None, "a", PromptItemStatus::Todo),
            item(11, None, "b", PromptItemStatus::Todo),
            item(12, None, "c", PromptItemStatus::Todo),
        ],
    );
    let area = Rect::new(2, 1, 20, 4);
    assert_eq!(row_at(area, &panel, 2, 1), None, "搜索框行不可命中");
    assert_eq!(row_at(area, &panel, 2, 2), Some(0));
    assert_eq!(row_at(area, &panel, 2, 3), Some(1));
    assert_eq!(row_at(area, &panel, 2, 4), Some(2));
    assert_eq!(row_at(area, &panel, 2, 5), None, "超出高度");
    assert_eq!(row_at(area, &panel, 1, 2), None, "区域外");
    panel.scroll = 1;
    assert_eq!(row_at(area, &panel, 2, 2), Some(1));
    assert_eq!(row_at(area, &panel, 2, 3), Some(2));
    assert_eq!(row_at(area, &panel, 2, 4), None, "滚动后超出末行");
    panel.scroll = 3;
    assert_eq!(row_at(area, &panel, 2, 2), None, "滚动超过末行");
}

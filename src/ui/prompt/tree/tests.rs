//! 提示词树渲染单元测试：行渲染、状态标签、省略号、空状态与命中。

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

fn row_text(buf: &Buffer, area: Rect, y: u16) -> String {
    (area.x..area.right())
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
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

    let first = row_text(&buf, area, 0);
    assert!(first.starts_with("▾ ideas"), "{first}");
    let second = row_text(&buf, area, 1);
    assert!(second.starts_with("└─ ▾ deep"), "{second}");
    let third = row_text(&buf, area, 2);
    assert!(third.starts_with("└─ • nested"), "{third}");
    assert!(third.contains("[run]"), "{third}");
    let fourth = row_text(&buf, area, 3);
    assert!(fourth.starts_with("• loose"), "{fourth}");
    assert!(fourth.contains("[todo]"), "{fourth}");

    panel.tree.toggle_folder(1);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let first = row_text(&buf, area, 0);
    assert!(first.starts_with("▸ ideas"), "{first}");
    let second = row_text(&buf, area, 1);
    assert!(second.starts_with("• loose"), "{second}");
}

#[test]
fn selected_row_gets_selection_background() {
    let mut panel = panel_with(
        vec![],
        vec![item(10, None, "draft", PromptItemStatus::Completed)],
    );
    panel.selected = 0;
    let area = Rect::new(0, 0, 30, 3);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let cell = &buf[(29, 0)];
    assert_eq!(cell.bg, Color::Indexed(240), "整行铺选中底色");
    let status_cell = &buf[(24, 0)];
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
    let text = row_text(&buf, area, 0);
    assert!(text.contains('…'), "{text}");
    assert_eq!(text.chars().count(), 18);
    assert!(text.ends_with("[todo]"), "{text}");
}

#[test]
fn empty_tree_shows_placeholder() {
    let panel = PromptPanel::new();
    let area = Rect::new(0, 0, 30, 3);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel);
    let text = row_text(&buf, area, 0);
    assert!(
        text.starts_with(crate::ui::text::PROMPT_TREE_EMPTY),
        "{text}"
    );
}

#[test]
fn row_at_maps_viewport_and_respects_scroll() {
    let mut panel = panel_with(
        vec![],
        vec![
            item(10, None, "a", PromptItemStatus::Todo),
            item(11, None, "b", PromptItemStatus::Todo),
            item(12, None, "c", PromptItemStatus::Todo),
        ],
    );
    let area = Rect::new(2, 1, 20, 2);
    assert_eq!(row_at(area, &panel, 2, 1), Some(0));
    assert_eq!(row_at(area, &panel, 2, 2), Some(1));
    assert_eq!(row_at(area, &panel, 2, 3), None, "超出高度");
    assert_eq!(row_at(area, &panel, 1, 1), None, "区域外");
    panel.scroll = 1;
    assert_eq!(row_at(area, &panel, 2, 1), Some(1));
    assert_eq!(row_at(area, &panel, 2, 2), Some(2));
    panel.scroll = 2;
    assert_eq!(row_at(area, &panel, 2, 2), None, "滚动后超出末行");
}

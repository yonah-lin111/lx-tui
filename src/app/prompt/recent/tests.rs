//! 最近页面派生模型测试：排序、标题/状态、当前标记与数量上限。

use super::*;
use crate::app::prompt::{PromptItem, PromptPage, PromptPageId, PromptTree};

fn item(id: i64, name: &str, pages: Vec<PromptPage>) -> PromptItem {
    let active_page = pages
        .first()
        .map(|page| page.id)
        .unwrap_or(PromptPageId::Saved(0));
    PromptItem {
        id,
        folder_id: None,
        name: name.to_string(),
        pages,
        active_page,
    }
}

fn panel(items: Vec<PromptItem>) -> PromptPanel {
    let mut panel = PromptPanel::new();
    panel.tree = PromptTree::new();
    panel.tree.replace(Vec::new(), items);
    panel
}

#[test]
fn rows_sort_by_updated_at_desc_across_prompts() {
    let panel = panel(vec![
        item(
            1,
            "Alpha",
            vec![
                PromptPage::saved_at(10, "Page 1", "old", 100),
                PromptPage::saved_at(11, "Page 2", "newer", 300),
            ],
        ),
        item(
            2,
            "Beta",
            vec![PromptPage::saved_at(20, "Page 1", "middle", 200)],
        ),
    ]);

    let rows = recent_page_rows(&panel, None, 400);
    let ids: Vec<i64> = rows.iter().filter_map(|row| row.page_id.saved()).collect();
    assert_eq!(ids, vec![11, 20, 10], "跨条目全局按更新时间倒序");
    assert_eq!(rows[0].prompt_name, "Alpha");
    assert_eq!(rows[1].prompt_name, "Beta");
}

#[test]
fn same_timestamp_falls_back_to_page_id_desc() {
    let panel = panel(vec![item(
        1,
        "Alpha",
        vec![
            PromptPage::saved_at(10, "Page 1", "a", 100),
            PromptPage::saved_at(11, "Page 2", "b", 100),
        ],
    )]);
    let rows = recent_page_rows(&panel, None, 500);
    let ids: Vec<i64> = rows.iter().filter_map(|row| row.page_id.saved()).collect();
    assert_eq!(ids, vec![11, 10]);
}

#[test]
fn title_status_and_current_use_live_editor_text() {
    let content =
        "&&& refactorTemplate --start\n{title: Saved title}\n&&& refactorTemplate --end todo";
    let live =
        "&&& refactorTemplate --start\n{title: Live title}\n&&& refactorTemplate --end in_progress";
    let panel = panel(vec![item(
        1,
        "Alpha",
        vec![PromptPage::saved_at(10, "Page 1", content, 100)],
    )]);

    let rows = recent_page_rows(
        &panel,
        Some((PromptEditing::Item(1), PromptPageId::Saved(10), live)),
        200,
    );
    assert_eq!(rows[0].page_title, "Live title");
    assert_eq!(rows[0].status, Some(TemplateStatus::InProgress));
    assert!(rows[0].current);
    assert_eq!(rows[0].time_label, "[1m]");
}

#[test]
fn page_name_fallback_and_missing_template_has_no_status() {
    let panel = panel(vec![item(
        1,
        "Alpha",
        vec![PromptPage::saved_at(10, "  ", "plain text", 100)],
    )]);
    let rows = recent_page_rows(&panel, None, 100);
    assert_eq!(rows[0].page_title, "Page 1");
    assert_eq!(rows[0].status, None);
    assert!(!rows[0].current);
}

#[test]
fn rows_are_capped_at_twenty() {
    let pages = (0..25)
        .map(|index| PromptPage::saved_at(100 + index, "Page", "", 1_000 + index))
        .collect();
    let panel = panel(vec![item(1, "Alpha", pages)]);
    let rows = recent_page_rows(&panel, None, 2_000);
    assert_eq!(rows.len(), MAX_RECENT_PAGES);
    assert_eq!(rows[0].page_id.saved(), Some(124), "最新的排在最前");
}

#[test]
fn count_includes_only_saved_pages() {
    let panel = panel(vec![item(
        1,
        "Alpha",
        vec![
            PromptPage::saved_at(10, "Page 1", "", 100),
            PromptPage::temp(99, "Temp", ""),
        ],
    )]);
    assert_eq!(recent_page_count(&panel), 1);
}

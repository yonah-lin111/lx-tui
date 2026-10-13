//! 提示词树渲染单元测试：搜索框行（模态输入对齐）、连接符、状态标签、省略号、空状态与命中。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;

use super::*;
use crate::app::prompt::{PromptFolder, PromptItem, PromptItemStatus, PromptPage};

/// 树行渲染用的面板：去掉默认空白 temp，聚焦树行布局。
fn panel_with(folders: Vec<PromptFolder>, items: Vec<PromptItem>) -> PromptPanel {
    let mut panel = PromptPanel::new();
    if let Some(id) = panel.blank_temp() {
        panel.remove_temp(id);
    }
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
    PromptItem::single(id, folder_id, name, String::new(), status)
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
    let cursor = render(area, &mut buf, &panel, false);
    let search_row = row_text(&buf, area, 0);
    assert!(
        search_row.starts_with(&format!(
            "{} {}",
            text::INPUT_PREFIX_ICON,
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
    let cursor = render(area, &mut buf, &panel, false);
    let search_row = row_text(&buf, area, 0);
    assert!(
        search_row.starts_with(&format!("{} ide", text::INPUT_PREFIX_ICON)),
        "{search_row}"
    );
    assert_eq!(cursor, Some((5, 0)), "光标在过滤词末尾");
}

#[test]
fn search_row_scrolls_viewport_for_long_query() {
    let mut panel = PromptPanel::new();
    search(&mut panel, "0123456789");
    let area = Rect::new(0, 0, 8, 3);
    let mut buf = Buffer::empty(area);
    let cursor = render(area, &mut buf, &panel, false);
    let search_row = row_text(&buf, area, 0);
    assert_eq!(
        search_row.trim_end(),
        format!("{} 56789", text::INPUT_PREFIX_ICON),
        "视口右移保证光标可见"
    );
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
    render(area, &mut buf, &panel, false);

    // 行 0 搜索框、行 1 分割线，树行从行 2 开始。
    assert_eq!(row_text(&buf, area, 1), text::DIVIDER_MID.repeat(40));
    let first = row_text(&buf, area, 2);
    assert!(first.starts_with("▾ ideas"), "{first}");
    // 子层最后一个可见后代：末项连接符 `└─`。
    let second = row_text(&buf, area, 3);
    assert!(second.starts_with("└─ ▾ deep"), "{second}");
    // 嵌套文件夹内的条目深一层：deep 为末项，续接列留空。
    let third = row_text(&buf, area, 4);
    assert!(third.starts_with("   └─ • nested"), "{third}");
    assert!(third.contains("[run]"), "{third}");
    let fourth = row_text(&buf, area, 5);
    assert!(fourth.starts_with("• loose"), "{fourth}");
    assert!(fourth.contains("[todo]"), "{fourth}");

    panel.tree.toggle_folder(1);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let first = row_text(&buf, area, 2);
    assert!(first.starts_with("▸ ideas"), "{first}");
    let second = row_text(&buf, area, 3);
    assert!(second.starts_with("• loose"), "{second}");
}

#[test]
fn nested_items_keep_vertical_line_when_subfolder_has_later_siblings() {
    let panel = panel_with(
        vec![folder(1, None, "solo"), folder(2, Some(1), "deep")],
        vec![
            item(11, Some(2), "nested", PromptItemStatus::Todo),
            item(12, Some(1), "sibling", PromptItemStatus::Todo),
        ],
    );
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    // rows: solo、deep、nested、sibling；deep 之后仍有同层行（sibling）：续接列画 `│`。
    let deep = row_text(&buf, area, 3);
    assert!(deep.starts_with("├─ ▾ deep"), "{deep}");
    let nested = row_text(&buf, area, 4);
    assert!(nested.starts_with("│  └─ • nested"), "{nested}");
    let sibling = row_text(&buf, area, 5);
    assert!(sibling.starts_with("└─ • sibling"), "{sibling}");
}

#[test]
fn nested_items_use_spaces_when_root_folder_is_last() {
    let panel = panel_with(
        vec![folder(1, None, "solo"), folder(2, Some(1), "deep")],
        vec![item(11, Some(2), "nested", PromptItemStatus::Todo)],
    );
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    // solo 为唯一根层文件夹（末项）：续接列留空，嵌套条目以空格对齐。
    let deep = row_text(&buf, area, 3);
    assert!(deep.starts_with("└─ ▾ deep"), "{deep}");
    let nested = row_text(&buf, area, 4);
    assert!(nested.starts_with("   └─ • nested"), "{nested}");
}

#[test]
fn selected_row_gets_selection_background() {
    let mut panel = panel_with(
        vec![],
        vec![item(10, None, "final", PromptItemStatus::Completed)],
    );
    panel.selected = 0;
    let area = Rect::new(0, 0, 30, 4);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let cell = &buf[(29, 2)];
    assert_eq!(cell.bg, Color::Indexed(240), "整行铺选中底色");
    let status_cell = &buf[(24, 2)];
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
    let area = Rect::new(0, 0, 18, 4);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let text = row_text(&buf, area, 2);
    assert!(text.contains('…'), "{text}");
    assert_eq!(text.chars().count(), 18);
    assert!(text.ends_with("[todo]"), "{text}");
}

#[test]
fn empty_tree_shows_placeholder_under_search_row() {
    // 防御分支：无 temp 且无树行时显示空状态占位（进入列表恒有 default temp）。
    let panel = panel_with(vec![], vec![]);
    let area = Rect::new(0, 0, 30, 5);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let text = row_text(&buf, area, 2);
    assert!(text.starts_with(text::PROMPT_TREE_EMPTY), "{text}");
}

#[test]
fn default_temp_renders_placeholder_name_and_badge() {
    let panel = PromptPanel::new();
    let area = Rect::new(0, 0, 40, 5);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let tmp = row_text(&buf, area, 2);
    assert!(tmp.starts_with(text::PROMPT_SECTION_TMP), "{tmp}");
    let default = row_text(&buf, area, 3);
    assert!(default.starts_with("• default"), "{default}");
    assert!(default.ends_with(text::PROMPT_TEMP_BADGE), "{default}");
}

#[test]
fn filter_without_matches_shows_hint_placeholder() {
    let mut panel = panel_with(
        vec![],
        vec![item(10, None, "alpha", PromptItemStatus::Todo)],
    );
    search(&mut panel, "zzz");
    let area = Rect::new(0, 0, 30, 5);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let text = row_text(&buf, area, 2);
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
    render(area, &mut buf, &panel, false);
    let first = row_text(&buf, area, 2);
    assert!(first.starts_with("▾ ideas"), "{first}");
    // 过滤结果里 ideas 为唯一根层（末项）：子层续接列留空；折叠被忽略按展开绘制。
    let second = row_text(&buf, area, 3);
    assert!(second.starts_with("└─ ▾ deep"), "{second}");
    let third = row_text(&buf, area, 4);
    assert!(third.starts_with("   └─ • nested"), "{third}");
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
    let area = Rect::new(2, 1, 20, 5);
    assert_eq!(row_at(area, &panel, 2, 1), None, "搜索框行不可命中");
    assert_eq!(row_at(area, &panel, 2, 2), None, "分割线行不可命中");
    assert_eq!(row_at(area, &panel, 2, 3), Some(0));
    assert_eq!(row_at(area, &panel, 2, 4), Some(1));
    assert_eq!(row_at(area, &panel, 2, 5), Some(2));
    assert_eq!(row_at(area, &panel, 2, 6), None, "超出高度");
    assert_eq!(row_at(area, &panel, 1, 3), None, "区域外");
    panel.scroll = 1;
    assert_eq!(row_at(area, &panel, 2, 3), Some(1));
    assert_eq!(row_at(area, &panel, 2, 4), Some(2));
    assert_eq!(row_at(area, &panel, 2, 5), None, "滚动后超出末行");
    panel.scroll = 3;
    assert_eq!(row_at(area, &panel, 2, 3), None, "滚动超过末行");
}

#[test]
fn temp_rows_render_section_labels_summary_and_badge() {
    let mut panel = panel_with(
        vec![],
        vec![item(10, None, "alpha", PromptItemStatus::Todo)],
    );
    let temp_id = panel.new_temp();
    panel
        .temp_mut(temp_id)
        .expect("temp exists")
        .active_page_mut()
        .expect("temp has page")
        .content = "code review\nsecond line".into();
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);

    let tmp = row_text(&buf, area, 2);
    assert!(tmp.starts_with(text::PROMPT_SECTION_TMP), "{tmp}");
    let temp = row_text(&buf, area, 3);
    assert!(temp.starts_with("• code review"), "{temp}");
    assert!(temp.ends_with(text::PROMPT_TEMP_BADGE), "{temp}");
    assert_eq!(
        buf[(area.right() - 5, 3)].fg,
        Color::Reset,
        "tmp 徽标改用默认文字色，不再用主题强调色"
    );
    let saved = row_text(&buf, area, 4);
    assert!(saved.starts_with(text::PROMPT_SECTION_SAVED), "{saved}");
    let row = row_text(&buf, area, 5);
    assert!(row.starts_with("• alpha"), "{row}");
    assert!(row.contains("[todo]"), "{row}");
}

#[test]
fn temp_section_hides_saved_label_when_tree_filtered_out() {
    let mut panel = panel_with(
        vec![],
        vec![item(10, None, "alpha", PromptItemStatus::Todo)],
    );
    let temp_id = panel.new_temp();
    panel
        .temp_mut(temp_id)
        .expect("temp exists")
        .active_page_mut()
        .expect("temp has page")
        .content = "code review".into();
    search(&mut panel, "code");
    let area = Rect::new(0, 0, 40, 5);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let tmp = row_text(&buf, area, 2);
    assert!(tmp.starts_with(text::PROMPT_SECTION_TMP), "{tmp}");
    let temp = row_text(&buf, area, 3);
    assert!(temp.contains("code review"), "{temp}");
    let saved = row_text(&buf, area, 4);
    assert!(!saved.contains(text::PROMPT_SECTION_SAVED), "{saved}");
}

// ===== 列表逐页/汇总模板块计数 =====

/// 组装一个两页条目：第一页 1 个 todo 模板，第二页 1 个 run 模板。
fn paged_item() -> PromptItem {
    let mut item = item(10, None, "alpha", PromptItemStatus::Todo);
    item.pages[0].content = "&&& addTemplate --start\n&&& addTemplate --end".into();
    item.pages.push(PromptPage::saved(
        100,
        "Page 2",
        "&&& bugTemplate --start\n&&& bugTemplate --end in_progress",
    ));
    item
}

#[test]
fn item_row_summarizes_template_counts_across_pages() {
    let mut panel = PromptPanel::new();
    panel.tree.replace(vec![], vec![paged_item()]);
    let area = Rect::new(0, 0, 44, 6);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    // 行序（绝对行 = 搜索行 0 + 分割线 1 + 列表行）：tmp:、default、saved:、item。
    let row = row_text(&buf, area, 5);
    assert!(row.contains("[todo]"), "条目自身状态保留: {row}");
    assert!(row.contains("[todo 1]"), "第一页 todo 计数: {row}");
    assert!(row.contains("[run 1]"), "第二页 run 计数: {row}");
    assert!(row.starts_with("• alpha"), "{row}");
}

#[test]
fn item_row_omits_whole_count_labels_when_narrow() {
    let mut panel = PromptPanel::new();
    panel.tree.replace(vec![], vec![paged_item()]);
    let area = Rect::new(0, 0, 18, 6);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let row = row_text(&buf, area, 5);
    // 计数整体省略，不画半个标签；条目状态优先保留。
    assert!(!row.contains("run"), "尾部计数标签完整省略: {row}");
    assert!(!row.contains("[todo 1]") || row.contains("[todo]"), "{row}");
}

#[test]
fn temp_badge_uses_default_text_color_not_accent() {
    let mut panel = PromptPanel::new();
    let temp_id = panel.new_temp();
    panel
        .temp_mut(temp_id)
        .expect("temp")
        .active_page_mut()
        .expect("page")
        .content = "draft".into();
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    render(area, &mut buf, &panel, false);
    let row = row_text(&buf, area, 4);
    assert!(row.ends_with(text::PROMPT_TEMP_BADGE), "{row}");
    assert_eq!(
        buf[(area.right() - 5, 4)].fg,
        Color::Reset,
        "tmp 徽标为默认文字色"
    );
}

//! 提示词树模型单元测试：行构建、折叠、层级约束、级联删除与选中定位。

use super::*;

fn folder(id: i64, parent_id: Option<i64>, name: &str) -> PromptFolder {
    PromptFolder {
        id,
        parent_id,
        name: name.to_string(),
        collapsed: false,
    }
}

fn item(id: i64, folder_id: Option<i64>, name: &str) -> PromptItem {
    PromptItem {
        id,
        folder_id,
        name: name.to_string(),
        content: format!("content {id}"),
        status: PromptItemStatus::Todo,
    }
}

fn sample_tree() -> PromptTree {
    let mut tree = PromptTree::new();
    tree.replace(
        vec![
            folder(1, None, "root"),
            folder(2, Some(1), "sub"),
            folder(3, None, "other"),
        ],
        vec![
            item(10, None, "loose"),
            item(11, Some(1), "in-root"),
            item(12, Some(2), "in-sub"),
        ],
    );
    tree
}

fn row_kinds(tree: &PromptTree) -> Vec<(PromptRowKind, usize, usize)> {
    tree.rows()
        .into_iter()
        .map(|row| (row.kind, row.index, row.depth))
        .collect()
}

#[test]
fn rows_order_folders_before_root_items() {
    let tree = sample_tree();
    assert_eq!(
        row_kinds(&tree),
        vec![
            (PromptRowKind::Folder, 0, 0),
            (PromptRowKind::Folder, 1, 1),
            (PromptRowKind::Item, 2, 1),
            (PromptRowKind::Item, 1, 1),
            (PromptRowKind::Folder, 2, 0),
            (PromptRowKind::Item, 0, 0),
        ]
    );
}

#[test]
fn collapsed_folder_hides_descendants() {
    let mut tree = sample_tree();
    tree.toggle_folder(1);
    assert_eq!(
        row_kinds(&tree),
        vec![
            (PromptRowKind::Folder, 0, 0),
            (PromptRowKind::Folder, 2, 0),
            (PromptRowKind::Item, 0, 0),
        ]
    );
}

#[test]
fn folder_depth_and_subfolder_constraint() {
    let tree = sample_tree();
    assert_eq!(tree.folder_depth(1), Some(0));
    assert_eq!(tree.folder_depth(2), Some(1));
    assert_eq!(tree.folder_depth(99), None);
    assert!(tree.can_create_subfolder(1));
    assert!(!tree.can_create_subfolder(2));
}

#[test]
fn delete_folder_cascades_in_memory() {
    let mut tree = sample_tree();
    tree.delete_folder(1);
    assert!(tree.folder(1).is_none());
    assert!(tree.folder(2).is_none());
    assert!(tree.item(11).is_none());
    assert!(tree.item(12).is_none());
    assert!(tree.item(10).is_some(), "root item untouched");
}

#[test]
fn delete_item_removes_only_target() {
    let mut tree = sample_tree();
    tree.delete_item(12);
    assert!(tree.item(12).is_none());
    assert!(tree.item(11).is_some());
}

#[test]
fn replace_keeps_collapsed_state_and_sorts_by_id() {
    let mut tree = sample_tree();
    tree.toggle_folder(2);
    let mut reloaded = vec![
        folder(3, Some(1), "sub"),
        folder(1, None, "root"),
        folder(2, None, "other"),
    ];
    reloaded.sort_by_key(|folder| -folder.id);
    let mut items = vec![item(12, Some(3), "in-sub"), item(11, Some(1), "in-root")];
    items.sort_by_key(|item| -item.id);
    tree.replace(reloaded, items);
    assert!(tree.folder(2).is_some_and(|folder| folder.collapsed));
    assert_eq!(tree.folders().first().map(|folder| folder.id), Some(1));
    assert_eq!(tree.items().first().map(|item| item.id), Some(11));
}

#[test]
fn status_roundtrip_and_cycle() {
    for status in [
        PromptItemStatus::Todo,
        PromptItemStatus::InProgress,
        PromptItemStatus::Completed,
    ] {
        assert_eq!(PromptItemStatus::parse(status.as_str()), status);
    }
    assert_eq!(PromptItemStatus::parse("unknown"), PromptItemStatus::Todo);
    assert_eq!(PromptItemStatus::Todo.next(), PromptItemStatus::InProgress);
    assert_eq!(PromptItemStatus::Completed.next(), PromptItemStatus::Todo);
}

#[test]
fn panel_selection_clamps_and_locates_prompt() {
    let mut panel = PromptPanel::new();
    panel.tree = sample_tree();
    // 行序：root(0), sub(1), in-sub(2), in-root(3), other(4), loose(5)。
    panel.selected = 2;
    let row = panel.selected_row().expect("row exists");
    assert_eq!(panel.tree.item_of_row(row).map(|item| item.id), Some(12));
    assert_eq!(panel.selected_prompt_id(), Some(12));
    panel.selected = 99;
    panel.scroll = 99;
    panel.clamp_selection(3);
    assert_eq!(panel.selected, 5);
    assert_eq!(panel.scroll, 3);
}

fn search(panel: &mut PromptPanel, query: &str) {
    panel.search.clear();
    for ch in query.chars() {
        panel.search.insert_char(ch);
    }
}

#[test]
fn search_filters_folders_items_and_ancestors() {
    let mut panel = PromptPanel::new();
    panel.tree = sample_tree();
    search(&mut panel, "in-");
    let rows = panel.rows();
    // root(folder) + sub(folder) + in-sub + in-root。
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert_eq!(
        rows.iter().map(|row| row.kind).collect::<Vec<_>>(),
        vec![
            PromptRowKind::Folder,
            PromptRowKind::Folder,
            PromptRowKind::Item,
            PromptRowKind::Item
        ]
    );

    search(&mut panel, "other");
    let rows = panel.rows();
    assert_eq!(rows.len(), 1, "只保留命中文件夹：{rows:?}");
    assert_eq!(rows[0].kind, PromptRowKind::Folder);

    search(&mut panel, "zzz");
    assert!(panel.rows().is_empty(), "无命中为空列表");
}

#[test]
fn search_matching_folder_shows_whole_subtree() {
    let mut panel = PromptPanel::new();
    panel.tree = sample_tree();
    search(&mut panel, "root");
    let rows = panel.rows();
    // root + sub + in-sub + in-root（命中文件夹展示整棵子树）。
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows.iter().all(|row| row.depth <= 1));
}

#[test]
fn search_ignores_collapsed_state() {
    let mut panel = PromptPanel::new();
    panel.tree = sample_tree();
    panel.tree.toggle_folder(1);
    search(&mut panel, "in-sub");
    let rows = panel.rows();
    // 折叠被忽略：root → sub → in-sub 仍可见。
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert_eq!(rows[2].kind, PromptRowKind::Item);
}

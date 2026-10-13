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
        attachments: Vec::new(),
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
            (PromptRowKind::Item, 2, 2),
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
    // 行序：tmp:(0)、default(1)、saved:(2)、root(3)、sub(4)、in-sub(5)、in-root(6)、other(7)、loose(8)。
    panel.selected = 5;
    let row = panel.selected_row().expect("row exists");
    assert_eq!(panel.tree.item_of_row(row).map(|item| item.id), Some(12));
    assert_eq!(panel.selected_prompt_id(), Some(12));
    panel.selected = 99;
    panel.scroll = 99;
    panel.clamp_selection(3);
    assert_eq!(panel.selected, 8);
    assert_eq!(panel.scroll, 6);
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
    assert!(rows.iter().all(|row| row.depth <= 2));
    assert!(rows.iter().any(|row| row.depth == 2), "子文件夹条目深一层");
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

#[test]
fn temp_summary_uses_first_non_empty_line() {
    let temp = TempPrompt {
        id: 0,
        content: "\n  代码审查专家  \n严格执行".into(),
        attachments: Vec::new(),
    };
    assert_eq!(temp.summary(), "代码审查专家");
    assert!(!temp.is_blank());
    let blank = TempPrompt {
        id: 1,
        content: "  \n\t ".into(),
        attachments: Vec::new(),
    };
    assert_eq!(blank.summary(), "");
    assert!(blank.is_blank());
}

/// 追加一个内容非空的临时提示词并返回 id。
fn push_temp(panel: &mut PromptPanel, content: &str) -> u64 {
    let id = panel.new_temp();
    panel.temp_mut(id).expect("new temp exists").content = content.to_string();
    id
}

#[test]
fn rows_pin_temps_before_tree_with_section_labels() {
    let mut panel = PromptPanel::new();
    panel.tree = sample_tree();
    let temp_id = push_temp(&mut panel, "审查这段代码");
    let rows = panel.rows();
    let kinds: Vec<PromptRowKind> = rows.iter().map(|row| row.kind).collect();
    let mut expected = vec![
        PromptRowKind::Section(PromptSection::Temp),
        PromptRowKind::Temp,
        PromptRowKind::Temp,
        PromptRowKind::Section(PromptSection::Saved),
    ];
    expected.extend(sample_tree().rows().into_iter().map(|row| row.kind));
    assert_eq!(kinds, expected, "{rows:?}");
    assert_eq!(
        panel.temp_of_row(rows[1]).map(TempPrompt::display_name),
        Some(text::PROMPT_TEMP_DEFAULT_NAME),
        "空白 default temp 置顶"
    );
    let row = rows[2];
    assert_eq!(panel.temp_of_row(row).map(|temp| temp.id), Some(temp_id));
    assert_eq!(panel.tree.folder_of_row(rows[0]), None, "标签行不映射数据");

    // 无 temp 时退化为纯树行（不出现任何标签）。
    let mut bare = PromptPanel::new();
    bare.remove_temp(0);
    bare.tree = sample_tree();
    assert_eq!(bare.rows(), sample_tree().rows());
}

#[test]
fn blank_temp_shows_default_placeholder_name() {
    let mut panel = PromptPanel::new();
    panel.tree = sample_tree();
    let blank = panel.blank_temp().expect("new panel has blank temp");
    let visible = push_temp(&mut panel, "有内容");
    let rows = panel.rows();
    let temps: Vec<u64> = rows
        .iter()
        .filter_map(|row| panel.temp_of_row(*row).map(|temp| temp.id))
        .collect();
    assert_eq!(temps, vec![blank, visible], "空白 default temp 恒在列表");
    assert_eq!(
        panel.temp(blank).map(TempPrompt::summary),
        Some(""),
        "空白项摘要为空串"
    );
    assert_eq!(
        panel.temp(blank).map(TempPrompt::display_name),
        Some(text::PROMPT_TEMP_DEFAULT_NAME),
        "空白项展示占位名"
    );
    assert_eq!(
        panel.temp(visible).map(TempPrompt::display_name),
        Some("有内容")
    );
}

#[test]
fn search_filters_temps_and_keeps_sections_per_visibility() {
    let mut panel = PromptPanel::new();
    panel.tree = sample_tree();
    push_temp(&mut panel, "review 审查");
    push_temp(&mut panel, "test 生成");

    // 只命中 temp：tmp: 标签保留，DB 侧连标签一起消失。
    search(&mut panel, "审");
    let kinds: Vec<PromptRowKind> = panel.rows().iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds,
        vec![
            PromptRowKind::Section(PromptSection::Temp),
            PromptRowKind::Temp
        ]
    );

    // 只命中 DB：temp 侧连标签一起消失，退化为纯树行。
    search(&mut panel, "in-sub");
    let kinds: Vec<PromptRowKind> = panel.rows().iter().map(|row| row.kind).collect();
    assert!(!kinds.contains(&PromptRowKind::Section(PromptSection::Temp)));
    assert!(!kinds.contains(&PromptRowKind::Section(PromptSection::Saved)));

    // 两侧都有命中（`e` 命中 review/test 与 loose/nested）：标签成对出现。
    search(&mut panel, "e");
    let kinds: Vec<PromptRowKind> = panel.rows().iter().map(|row| row.kind).collect();
    assert!(kinds.contains(&PromptRowKind::Section(PromptSection::Temp)));
    assert!(kinds.contains(&PromptRowKind::Section(PromptSection::Saved)));
}

#[test]
fn clamp_selection_snaps_off_section_rows() {
    let mut panel = PromptPanel::new();
    panel.tree = sample_tree();
    push_temp(&mut panel, "有内容");
    // 行序：tmp:(0)、default(1)、temp(2)、saved:(3)、root(4)…
    panel.selected = 3;
    panel.clamp_selection(4);
    assert_eq!(panel.selected, 4, "选中吸附到标签之后的树行");
    assert_eq!(
        panel.selected_row().map(|row| row.kind),
        Some(PromptRowKind::Folder)
    );

    // 选中落在 tmp: 标签上：吸附向下到 default temp。
    panel.selected = 0;
    panel.clamp_selection(4);
    assert_eq!(panel.selected, 1, "tmp: 标签吸附到 temp 行");

    // 只有 temp 的短列表：越界钳制后吸附到末行。
    let mut short = PromptPanel::new();
    push_temp(&mut short, "唯一");
    short.selected = 99;
    short.clamp_selection(4);
    assert_eq!(short.selected, 2);
}

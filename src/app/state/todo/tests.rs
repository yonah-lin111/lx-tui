//! `TodoPanel` 状态机单元测试：列表不变式（`position == 下标`）、选中钳制、
//! 视口跟随与撤销槽；不涉及 `AppState` 与 IO。

use super::*;

/// 造一批条目，位置按给定顺序 0 基。
fn records(texts: &[&str]) -> Vec<TodoRecord> {
    texts
        .iter()
        .enumerate()
        .map(|(index, text)| TodoRecord::new(*text, TodoStatus::Todo, index as i64))
        .collect()
}

/// 列表不变式：`position == 下标`。
fn assert_dense(panel: &TodoPanel) {
    for (index, item) in panel.items().iter().enumerate() {
        assert_eq!(item.position, index as i64, "第 {index} 条位置错位");
    }
}

#[test]
fn replace_resets_selection_scroll_and_undo() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "b", "c"]));
    panel.set_viewport_rows(1);
    assert!(panel.scroll_by(2));
    assert_eq!(panel.scroll(), 2, "前提：视口已偏移");
    panel.set_undo(vec![TodoRecord::new("gone", TodoStatus::Done, 1)]);
    assert!(panel.can_undo());

    panel.replace(records(&["x", "y"]));

    assert_eq!(panel.len(), 2);
    assert_eq!(panel.selected(), Some(0), "整体替换后选中回到首项");
    assert_eq!(panel.scroll(), 0, "整体替换后视口贴顶");
    assert!(!panel.can_undo(), "整体替换清空撤销槽");
    assert!(!panel.is_editing());
    assert_dense(&panel);
}

#[test]
fn insert_keeps_positions_dense() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "b", "c"]));

    let at = panel.insert(1, TodoRecord::new("new", TodoStatus::Todo, 99));

    assert_eq!(at, 1);
    let texts: Vec<&str> = panel
        .items()
        .iter()
        .map(|item| item.text.as_str())
        .collect();
    assert_eq!(texts, ["a", "new", "b", "c"]);
    assert_dense(&panel);

    // 越界插入钳到末尾。
    let at = panel.insert(usize::MAX, TodoRecord::new("tail", TodoStatus::Todo, 0));
    assert_eq!(at, 4);
    assert_dense(&panel);
}

#[test]
fn clamp_selection_after_removing_last() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "b", "c"]));
    assert!(panel.select(2));

    let removed = panel.remove(2).expect("末项存在");
    assert_eq!(removed.text, "c");
    panel.clamp_selection();

    assert_eq!(panel.selected(), Some(1), "选中落到新的末项");
    assert_dense(&panel);

    // 删空后选中清空、视口归零。
    panel.remove(0);
    panel.remove(0);
    panel.clamp_selection();
    assert_eq!(panel.selected(), None);
    assert_eq!(panel.scroll(), 0);
}

#[test]
fn move_selection_clamps_and_supports_jump() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "b", "c", "d", "e"]));
    assert_eq!(panel.selected(), Some(0));

    assert!(!panel.move_selection(isize::MIN), "已在首项，无变化");
    assert_eq!(panel.selected(), Some(0));

    assert!(panel.move_selection(isize::MAX));
    assert_eq!(panel.selected(), Some(4), "跳末");

    assert!(!panel.move_selection(isize::MAX), "在末项再次跳末无变化");
    assert!(panel.move_selection(isize::MIN));
    assert_eq!(panel.selected(), Some(0), "跳首");

    assert!(!panel.move_selection(-1), "首项上移饱和");
    assert_eq!(panel.selected(), Some(0));
    assert_eq!(panel.len(), 5, "移动不改列表");
}

#[test]
fn scroll_into_view_follows_selection() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]));
    panel.set_viewport_rows(3);
    assert_eq!(panel.viewport_rows(), 3);

    assert!(panel.move_selection(isize::MAX));

    assert_eq!(panel.selected(), Some(9));
    assert_eq!(panel.scroll(), 7, "末项在 3 行视口里的首行下标");
}

#[test]
fn scroll_by_moves_viewport_only() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]));
    panel.set_viewport_rows(3);
    let selected = panel.selected();

    assert!(panel.scroll_by(3));
    assert_eq!(panel.scroll(), 3);
    assert_eq!(panel.selected(), selected, "滚轮不动高亮");

    assert!(panel.scroll_by(-10), "从 3 向 10 行滚到顶，确实产生变化");
    assert_eq!(panel.scroll(), 0);
    assert!(!panel.scroll_by(-1), "已在顶部，再向上无变化");

    assert!(panel.scroll_by(100));
    assert_eq!(panel.scroll(), 7, "向下钳到 len - rows");
    assert_eq!(panel.selected(), selected);
}

#[test]
fn take_with_status_removes_only_matching() {
    let mut panel = TodoPanel::new();
    panel.replace(vec![
        TodoRecord::new("open", TodoStatus::Todo, 0),
        TodoRecord::new("done-1", TodoStatus::Done, 1),
        TodoRecord::new("active", TodoStatus::InProgress, 2),
        TodoRecord::new("done-2", TodoStatus::Done, 3),
    ]);

    let removed = panel.take_with_status(TodoStatus::Done);

    let removed_texts: Vec<&str> = removed.iter().map(|item| item.text.as_str()).collect();
    assert_eq!(removed_texts, ["done-1", "done-2"], "保持原相对顺序");
    let kept: Vec<&str> = panel
        .items()
        .iter()
        .map(|item| item.text.as_str())
        .collect();
    assert_eq!(kept, ["open", "active"]);
    assert_dense(&panel);

    // 原位置（1 与 3）插回后回到原来的下标。
    panel.restore(removed);
    let texts: Vec<&str> = panel
        .items()
        .iter()
        .map(|item| item.text.as_str())
        .collect();
    assert_eq!(texts, ["open", "done-1", "active", "done-2"]);
    assert_dense(&panel);
}

#[test]
fn restore_reinserts_by_position() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "c"]));
    // `c` 占着位置 1、`a` 位置 0；插回 `b`（原位置 1）落在两者之间。
    panel.restore(vec![
        TodoRecord::new("z", TodoStatus::Done, 2),
        TodoRecord::new("b", TodoStatus::Todo, 1),
    ]);

    let texts: Vec<&str> = panel
        .items()
        .iter()
        .map(|item| item.text.as_str())
        .collect();
    assert_eq!(texts, ["a", "b", "z", "c"]);
    assert_dense(&panel);

    // 重复恢复同一批（id 已在列表里）：跳过，不造重复条目。
    let existing = panel.items()[1].clone();
    panel.restore(vec![existing]);
    assert_eq!(panel.len(), 4, "重复 id 不重复插入");

    // 撤销槽一次取走即失效。
    panel.set_undo(vec![TodoRecord::new("x", TodoStatus::Todo, 0)]);
    assert!(panel.take_undo().is_some());
    assert!(!panel.can_undo());
}

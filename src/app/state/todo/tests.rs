//! `TodoPanel` 状态机单元测试：列表不变式（`position == 下标`）、选中钳制、
//! 视口跟随与撤销槽；不涉及 `AppState` 与 IO。

use crate::db::todo::TodoPriority;

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

/// 编辑行滚入视口：列表溢出时在末尾新建，幽灵行槽位也必须落进视口。
#[test]
fn scroll_to_edit_reaches_ghost_slot() {
    let mut panel = TodoPanel::new();
    panel.set_viewport_rows(3);
    panel.replace(
        (0..10)
            .map(|index| TodoRecord::new(format!("t{index}"), TodoStatus::Todo, index as i64))
            .collect(),
    );
    assert_eq!(panel.scroll(), 0, "前提：视口贴顶");

    // 追加（`at == len`）：幽灵行在列表之后，视口必须滚到能看见它。
    panel.begin_edit(TodoEdit::new(None, String::new(), panel.len()));
    assert!(panel.scroll_to_edit(), "必须发生位移");
    assert_eq!(panel.scroll(), 8, "10 条 + 幽灵 1 行 - 视口 3 行");

    // 视口变大时上限收缩：幽灵槽位仍有效，scroll 不该被钳到「无幽灵」的那种更小上限。
    panel.set_viewport_rows(5);
    assert_eq!(panel.scroll(), 6, "10 条 + 幽灵 1 行 - 视口 5 行");

    // 取消编辑后幽灵槽位消失：再改可见行数时上限退回 len - rows。
    assert!(panel.take_edit().is_some());
    panel.set_viewport_rows(5);
    assert_eq!(panel.scroll(), 6, "未钳制（6 仍在 len - rows 之内）");
    panel.set_viewport_rows(9);
    assert_eq!(panel.scroll(), 1, "无幽灵行时上限是 len - rows = 1");
}

/// 边界：给 1 行视口改可见行数，若编辑行已滚到最末，幽灵槽位仍要算进上限。
#[test]
fn set_viewport_rows_keeps_ghost_slot_in_range() {
    let mut panel = TodoPanel::new();
    panel.replace(
        (0..4)
            .map(|index| TodoRecord::new(format!("t{index}"), TodoStatus::Todo, index as i64))
            .collect(),
    );
    panel.set_viewport_rows(4);
    panel.begin_edit(TodoEdit::new(None, String::new(), 4));
    assert!(panel.scroll_to_edit());
    assert_eq!(panel.scroll(), 1, "4 条 + 幽灵 - 4 行 = 1");

    // 视口从 4 缩到 5：幽灵槽位仍存在，scroll 不该被钳回去。
    panel.set_viewport_rows(5);
    assert_eq!(panel.scroll(), 0, "5 行能装下 5 个槽位");
}

#[test]
fn set_scroll_and_replace_and_today_rollover_invariants() {
    let mut panel = TodoPanel::new();
    panel.set_today_date("2026-10-31");
    // 乱序 position 注入当前选中日期：replace 必须按 position 升序重排后再归一。
    panel.replace(vec![
        TodoRecord::new_on(
            "2026-10-31",
            "second",
            TodoStatus::Todo,
            TodoPriority::P1,
            5,
        ),
        TodoRecord::new_on("2026-10-31", "first", TodoStatus::Todo, TodoPriority::P1, 0),
        TodoRecord::new_on("2026-10-30", "past", TodoStatus::Todo, TodoPriority::P0, 0),
    ]);
    assert_eq!(
        panel
            .items()
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );

    // 末尾打开幽灵行时，set_scroll 向下滚动不能把幽灵行钳出视口。
    panel.set_viewport_rows(2);
    panel.begin_edit(TodoEdit::new(None, String::new(), panel.len()));
    assert!(panel.scroll_to_edit());
    assert_eq!(panel.scroll(), 1, "2 条 + 1 幽灵 - 2 行视口 = 1");
    assert!(
        !panel.set_scroll(5),
        "已处于含幽灵行的最大偏移 1，不应倒退回 0"
    );
    assert_eq!(panel.scroll(), 1);
    panel.take_edit();

    // 关闭顺延横幅后跨日更新 today_date，必须重新允许显示新一天的顺延横幅。
    assert!(panel.dismiss_rollover());
    assert!(!panel.has_rollover_banner());
    panel.set_today_date("2026-11-01");
    assert!(
        panel.has_rollover_banner(),
        "跨日推进后 rollover_dismissed 必须复位"
    );
}

#[test]
fn merge_with_next_groups_two_singles_and_aligns_status() {
    let mut panel = TodoPanel::new();
    panel.replace(vec![
        TodoRecord::new("a", TodoStatus::InProgress, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
        TodoRecord::new("c", TodoStatus::Done, 2),
    ]);

    assert!(panel.has_next_to_group(0));
    assert!(panel.merge_with_next(0));
    assert_dense(&panel);

    let gid = panel.items()[0].group_id.clone().expect("已分配 group_id");
    assert_eq!(panel.items()[1].group_id.as_deref(), Some(gid.as_str()));
    assert_eq!(panel.items()[2].group_id, None);
    assert_eq!(panel.items()[0].status, TodoStatus::InProgress);
    assert_eq!(panel.items()[1].status, TodoStatus::InProgress);

    assert_eq!(
        panel.group_role(0),
        TodoGroupRole::Member {
            group_id: gid.clone(),
            offset: 0,
            count: 2,
        }
    );
    assert!(panel.group_role(0).is_status_anchor());
    assert!(!panel.group_role(1).is_status_anchor());

    // 再次向下合并将第 3 项并入同一组，锚点移至中项 offset == 1。
    assert!(panel.merge_with_next(0));
    assert_eq!(panel.group_range(0), 0..3);
    assert!(!panel.has_next_to_group(0));
    assert!(!panel.merge_with_next(0), "末尾组无法继续向下合并");
    assert!(!panel.group_role(0).is_status_anchor());
    assert!(panel.group_role(1).is_status_anchor());
    assert!(!panel.group_role(2).is_status_anchor());
    assert_eq!(panel.items()[2].status, TodoStatus::InProgress);
    assert_eq!(panel.items()[2].completed_date, None);
}

#[test]
fn merge_item_into_moves_non_adjacent_item_into_target_group() {
    let mut panel = TodoPanel::new();
    panel.replace(vec![
        TodoRecord::new("a", TodoStatus::Done, 0),
        TodoRecord::new("b", TodoStatus::Todo, 1),
        TodoRecord::new("c", TodoStatus::InProgress, 2),
    ]);

    // 将末项 c 并入首项 a：c 移动到 a 紧邻下方，且状态对齐 a（Done）。
    assert!(panel.merge_item_into(2, 0));
    assert_dense(&panel);

    let texts: Vec<&str> = panel.items().iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts, vec!["a", "c", "b"]);
    assert_eq!(panel.selected(), Some(1), "选中跟随至移动后的 c");
    assert!(panel.is_grouped(0));
    assert!(panel.is_grouped(1));
    assert!(!panel.is_grouped(2));
    assert_eq!(panel.items()[0].group_id, panel.items()[1].group_id);
    assert_eq!(panel.items()[1].status, TodoStatus::Done);
    assert!(panel.items()[1].completed_date.is_some());

    // 同组内再次并入返回 false。
    assert!(!panel.merge_item_into(0, 1));
}

#[test]
fn split_from_group_detaches_item_and_degenerates_singleton_group() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "b", "c"]));
    assert!(panel.merge_with_next(0));
    assert!(panel.merge_with_next(0));
    assert_eq!(panel.group_range(0), 0..3);

    // 拆出首项 a：a 挪到原组末尾（下标 2）并变为 Single，剩余 b、c 保持 2 人组。
    assert!(panel.split_from_group(0));
    assert_dense(&panel);
    let texts: Vec<&str> = panel.items().iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts, vec!["b", "c", "a"]);
    assert_eq!(panel.selected(), Some(2));
    assert!(panel.is_grouped(0));
    assert!(panel.is_grouped(1));
    assert!(!panel.is_grouped(2));

    // 再从 2 人组拆出 b：两边各剩 1 条，双双自动退化为 Single。
    assert!(panel.split_from_group(0));
    assert!(!panel.is_grouped(0));
    assert!(!panel.is_grouped(1));
    assert!(panel.items().iter().all(|i| i.group_id.is_none()));
}

#[test]
fn ungroup_at_dissolves_entire_group_in_place() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "b", "c", "d"]));
    assert!(panel.merge_with_next(0));
    assert!(panel.merge_with_next(0));
    assert_eq!(panel.group_range(1), 0..3);

    assert!(panel.ungroup_at(1));
    assert_dense(&panel);
    let texts: Vec<&str> = panel.items().iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts, vec!["a", "b", "c", "d"], "原地解散不改变顺序");
    assert!(panel.items().iter().all(|i| i.group_id.is_none()));
    assert!(!panel.ungroup_at(1), "独立项再次解散返回 false");
}

#[test]
fn cycle_status_updates_all_members_of_group() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "b", "c"]));
    assert!(panel.merge_with_next(0));

    let changed = panel.cycle_status(1);
    assert_eq!(changed.len(), 2);
    assert_eq!(panel.items()[0].status, TodoStatus::InProgress);
    assert_eq!(panel.items()[1].status, TodoStatus::InProgress);
    assert_eq!(panel.items()[2].status, TodoStatus::Todo);

    let changed = panel.cycle_status(0);
    assert_eq!(changed.len(), 2);
    assert_eq!(panel.items()[0].status, TodoStatus::Done);
    assert_eq!(panel.items()[1].status, TodoStatus::Done);
    assert!(panel.items()[0].completed_date.is_some());
    assert!(panel.items()[1].completed_date.is_some());
}

#[test]
fn remove_in_two_item_group_degenerates_survivor_to_single() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["a", "b"]));
    assert!(panel.merge_with_next(0));
    assert!(panel.is_grouped(0));

    panel.remove(0);
    assert_eq!(panel.len(), 1);
    assert_eq!(panel.items()[0].text, "b");
    assert_eq!(
        panel.items()[0].group_id,
        None,
        "孤儿组必须自动退化为 Single"
    );
    assert_eq!(panel.group_role(0), TodoGroupRole::Single);
}

#[test]
fn sort_by_priority_keeps_group_contiguous_and_orders_by_highest_member_priority() {
    let mut panel = TodoPanel::new();
    let today = panel.today_date().to_string();
    panel.replace(vec![
        TodoRecord::new_on(&today, "single-p1", TodoStatus::Todo, TodoPriority::P1, 0),
        TodoRecord::new_on(&today, "grp-p2", TodoStatus::Todo, TodoPriority::P2, 1)
            .with_group_id(Some("tgrp-1".to_string())),
        TodoRecord::new_on(&today, "grp-p0", TodoStatus::Todo, TodoPriority::P0, 2)
            .with_group_id(Some("tgrp-1".to_string())),
        TodoRecord::new_on(&today, "done-p0", TodoStatus::Done, TodoPriority::P0, 3),
    ]);

    // 聚合组内含 P2 与 P0，最高优先级为 P0，故整组排在 single-p1 之前，且组内重排为 [grp-p0, grp-p2]。
    let ordered = panel.sort_by_priority().expect("应发生重排");
    assert_eq!(ordered.len(), 4);
    let texts: Vec<&str> = panel.items().iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts, vec!["grp-p0", "grp-p2", "single-p1", "done-p0"]);
    assert_eq!(panel.group_range(0), 0..2);
    assert_dense(&panel);
}

#[test]
fn normalize_compacts_scattered_group_and_rollover_degenerates_singleton() {
    let mut panel = TodoPanel::new();
    panel.set_today_date("2026-10-31");
    panel.replace(vec![
        // 历史日 2026-10-30 有一组 2 条：其中 1 条 Done、1 条 Todo。
        TodoRecord::new_on(
            "2026-10-30",
            "past-todo",
            TodoStatus::Todo,
            TodoPriority::P1,
            0,
        )
        .with_group_id(Some("tgrp-past".to_string())),
        TodoRecord::new_on(
            "2026-10-30",
            "past-done",
            TodoStatus::Done,
            TodoPriority::P1,
            1,
        )
        .with_group_id(Some("tgrp-past".to_string())),
        // 今日被隔开的同组项：replace 必须自动归拢连续。
        TodoRecord::new_on(
            "2026-10-31",
            "g1",
            TodoStatus::InProgress,
            TodoPriority::P1,
            0,
        )
        .with_group_id(Some("tgrp-today".to_string())),
        TodoRecord::new_on("2026-10-31", "mid", TodoStatus::Todo, TodoPriority::P2, 1),
        TodoRecord::new_on("2026-10-31", "g2", TodoStatus::Todo, TodoPriority::P1, 2)
            .with_group_id(Some("tgrp-today".to_string())),
    ]);

    let texts: Vec<&str> = panel.items().iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts, vec!["g1", "g2", "mid"], "同组成员被自动归拢");
    assert_eq!(
        panel.items()[1].status,
        TodoStatus::InProgress,
        "同组状态自动对齐首项"
    );

    // 顺延前在 2026-10-30 由于同组状态对齐，past-done 已被对齐为首项的 Todo。
    // 若单独构造 1 条孤儿组顺延，顺延到今日后孤儿 group_id 应自动退化为 None。
    panel.select_date("2026-10-29");
    panel.insert(
        0,
        TodoRecord::new_on(
            "2026-10-29",
            "orphan",
            TodoStatus::Todo,
            TodoPriority::P1,
            0,
        )
        .with_group_id(Some("tgrp-orphan".to_string())),
    );
    panel.select_date("2026-10-31");
    let moved = panel.rollover_past_incomplete_to_today();
    assert!(!moved.is_empty());
    let orphan = panel
        .items()
        .iter()
        .find(|i| i.text == "orphan")
        .expect("orphan moved");
    assert_eq!(orphan.group_id, None);
}

#[test]
fn merge_with_next_and_ungroup_re_enable_follow_selected() {
    let mut panel = TodoPanel::new();
    panel.replace(records(&["0", "1", "2", "3", "4", "5"]));
    panel.set_viewport_rows(2);
    assert_eq!(panel.selected(), Some(0));

    // 手动滚轮滚离选中项（follow_selected 置为 false）。
    assert!(panel.scroll_by(3));
    assert_eq!(panel.scroll(), 3);

    // 向下合并时重新开启 follow_selected 并将选中项滚回视口。
    assert!(panel.merge_with_next(0));
    assert_eq!(panel.scroll(), 0);

    // 再次滚离后解散整组，同样重新开启 follow_selected 并将选中项滚回视口。
    assert!(panel.scroll_by(3));
    assert_eq!(panel.scroll(), 3);
    assert!(panel.ungroup_at(0));
    assert_eq!(panel.scroll(), 0);
}

use std::time::{Duration, Instant};

use super::*;

#[test]
fn todo_edit_word_operations_and_cut() {
    let mut state = todo_state(&["hello world"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));

    assert!(apply_todo_key(&mut state, TodoKey::DeleteWordBackward));
    assert_eq!(state.todo.editing().unwrap().input.text(), "hello ");
    assert!(apply_todo_key(&mut state, TodoKey::WordLeft));
    assert_eq!(state.todo.editing().unwrap().input.cursor(), 0);
    assert!(apply_todo_key(&mut state, TodoKey::WordRight));
    assert_eq!(state.todo.editing().unwrap().input.cursor(), 5);

    state.todo.edit_input_mut().unwrap().clear();
    state
        .todo
        .edit_input_mut()
        .unwrap()
        .insert_str("hello world");
    state.todo.edit_input_mut().unwrap().set_selection(5, 5);
    assert!(apply_todo_key(&mut state, TodoKey::DeleteWordForward));
    assert_eq!(state.todo.editing().unwrap().input.text(), "hello");

    state.todo.edit_input_mut().unwrap().clear();
    state
        .todo
        .edit_input_mut()
        .unwrap()
        .insert_str("hello world");
    state.todo.edit_input_mut().unwrap().set_selection(0, 5);
    let anchor = Some(state.prompt.id());
    assert!(apply_todo_key(&mut state, TodoKey::Cut));
    assert_eq!(state.todo.editing().unwrap().input.text(), " world");
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::CopyToClipboard { text, anchor: effect_anchor }
            if text == "hello" && *effect_anchor == anchor
    )));
}

#[test]
fn todo_edit_up_down_moves_across_wrapped_rows() {
    let mut state = todo_state(&["abcdefghij"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));
    assert!(state.todo.sync_viewport(5, vec![1], Some(0), 5));
    assert_eq!(state.todo.edit_text_width(), 5);

    assert!(apply_todo_key(&mut state, TodoKey::Up));
    assert_eq!(state.todo.editing().unwrap().input.cursor(), 5);
    assert!(apply_todo_key(&mut state, TodoKey::Up));
    assert_eq!(state.todo.editing().unwrap().input.cursor(), 0);
    assert!(apply_todo_key(&mut state, TodoKey::Down));
    assert_eq!(state.todo.editing().unwrap().input.cursor(), 5);
    assert!(apply_todo_key(&mut state, TodoKey::Down));
    assert_eq!(state.todo.editing().unwrap().input.cursor(), 10);
    assert!(state.todo.is_editing(), "上下移动不得退出编辑态");
}

#[test]
fn todo_edit_up_down_respects_wider_continuation_rows() {
    // 首行宽 3 ("abc", 0..3)，续行宽 6 ("defghi", 3..9)，第 3 行 "jk" (9..11)。
    let mut state = todo_state(&["abcdefghijk"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));
    assert!(state.todo.sync_viewport_indented(5, vec![3], Some(0), 3, 6));
    assert_eq!(state.todo.edit_text_width(), 3);
    assert_eq!(state.todo.edit_cont_width(), 6);

    assert!(apply_todo_key(&mut state, TodoKey::Up));
    assert_eq!(state.todo.editing().unwrap().input.cursor(), 5);
    assert!(apply_todo_key(&mut state, TodoKey::Up));
    assert_eq!(state.todo.editing().unwrap().input.cursor(), 2);
}

#[test]
fn todo_edit_select_all_and_replace() {
    let mut state = todo_state(&["abc"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));
    assert!(apply_todo_key(&mut state, TodoKey::SelectAll));
    assert_eq!(
        state
            .todo
            .editing()
            .unwrap()
            .input
            .selected_text()
            .as_deref(),
        Some("abc")
    );
    assert!(apply_todo_key(&mut state, TodoKey::Char('x')));
    assert_eq!(state.todo.editing().unwrap().input.text(), "x");
}

#[test]
fn key_m_u_upper_u_merge_split_and_ungroup_todos() {
    let mut state = todo_state(&["a", "b", "c"]);
    state.effects.clear();

    // 按 m 将 a 与 b 合并为 2 人组，再按 m 将 c 并入为 3 人组。
    assert!(apply_todo_key(&mut state, TodoKey::Char('m')));
    assert_eq!(state.todo.group_range(0), 0..2);
    assert!(
        state
            .effects
            .iter()
            .any(|e| matches!(e, Effect::SyncDateTodos { .. }))
    );

    assert!(apply_todo_key(&mut state, TodoKey::Char('m')));
    assert_eq!(state.todo.group_range(0), 0..3);
    assert!(
        !apply_todo_key(&mut state, TodoKey::Char('m')),
        "已无下一项可并"
    );

    // 按 u 将首项 a 拆出到组末（下标 2），剩余 b、c 仍为 2 人组。
    state.effects.clear();
    assert!(apply_todo_key(&mut state, TodoKey::Char('u')));
    assert_eq!(texts(&state), vec!["b", "c", "a"]);
    assert_eq!(state.todo.selected(), Some(2));
    assert!(!state.todo.is_grouped(2));
    assert_eq!(state.todo.group_range(0), 0..2);
    assert!(
        state
            .effects
            .iter()
            .any(|e| matches!(e, Effect::SyncDateTodos { .. }))
    );

    // 选中组内项 0（b），按 U 解散整组。
    assert!(state.todo.select(0));
    state.effects.clear();
    assert!(apply_todo_key(&mut state, TodoKey::Char('U')));
    assert!(!state.todo.is_grouped(0));
    assert!(!state.todo.is_grouped(1));
    assert!(
        state
            .effects
            .iter()
            .any(|e| matches!(e, Effect::SyncDateTodos { .. }))
    );
}

#[test]
fn key_o_inside_group_inserts_new_member_inheriting_group_and_status() {
    let mut state = todo_state(&["a", "b"]);
    assert!(apply_todo_key(&mut state, TodoKey::Char('m')));
    assert!(apply_todo_key(&mut state, TodoKey::Space));
    assert_eq!(state.todo.items()[0].status, TodoStatus::InProgress);
    let gid = state.todo.items()[0].group_id.clone().expect("grouped");

    // 在组内第 0 项按 o 新建：幽灵行携带该组 group_id 与 InProgress 状态。
    assert!(apply_todo_key(&mut state, TodoKey::Char('o')));
    let edit = state.todo.editing().expect("editing");
    assert_eq!(edit.group_id.as_deref(), Some(gid.as_str()));
    assert_eq!(edit.initial_status, TodoStatus::InProgress);

    type_text(&mut state, "mid");
    state.effects.clear();
    assert!(apply_todo_key(&mut state, TodoKey::Enter));

    assert_eq!(texts(&state), vec!["a", "mid", "b"]);
    assert_eq!(state.todo.group_range(0), 0..3);
    assert_eq!(
        state.todo.items()[1].group_id.as_deref(),
        Some(gid.as_str())
    );
    assert_eq!(state.todo.items()[1].status, TodoStatus::InProgress);
    assert!(
        state
            .effects
            .iter()
            .any(|e| matches!(e, Effect::SyncDateTodos { .. }))
    );
}

#[test]
fn mouse_drag_todo_onto_another_row_merges_into_group_without_breaking_double_click() {
    let mut state = todo_state(&["a", "b", "c"]);
    let t0 = Instant::now();

    // 1. 未跨行按下并松开：不触发合并，且保留双击序列供第二次点击进入编辑。
    assert!(begin_todo_row_press(&mut state, 1, t0));
    assert!(!drag_todo_to(&mut state, Some(1)));
    assert!(!finish_todo_drag(&mut state));
    assert!(begin_todo_row_press(
        &mut state,
        1,
        t0 + Duration::from_millis(50)
    ));
    assert!(state.todo.is_editing(), "未跨行松开不得破坏双击编辑");
    assert!(cancel_todo_edit(&mut state));

    // 2. 按住第 2 行 c，跨行拖到第 0 行 a 并松开：c 并入 a 所在聚合组。
    let t1 = t0 + Duration::from_secs(1);
    assert!(begin_todo_row_press(&mut state, 2, t1));
    assert!(drag_todo_to(&mut state, Some(0)));
    assert_eq!(
        state.todo_drag.as_ref().and_then(|d| d.target_index),
        Some(0)
    );
    state.effects.clear();
    assert!(finish_todo_drag(&mut state));
    assert_eq!(texts(&state), vec!["a", "c", "b"]);
    assert_eq!(state.todo.group_range(0), 0..2);
    assert!(
        state
            .effects
            .iter()
            .any(|e| matches!(e, Effect::SyncDateTodos { .. }))
    );

    // 3. 点击状态框整组切换状态。
    state.effects.clear();
    assert!(click_todo_status(&mut state, 0));
    assert_eq!(state.todo.items()[0].status, TodoStatus::InProgress);
    assert_eq!(state.todo.items()[1].status, TodoStatus::InProgress);
    assert_eq!(take_saved(&mut state).len(), 2);
}

#[test]
fn right_click_grouped_todo_exposes_split_and_ungroup_commands() {
    use crate::app::overlay::{MenuCommand, Overlay};

    let mut state = todo_state(&["a", "b", "c"]);
    let id_a = state.todo.items()[0].id.clone();

    // 先通过右键菜单 GroupTodoWithNext 将 a 与 b 合并。
    open_todo_item_menu(&mut state, &id_a, (10, 10));
    assert!(crate::app::update::set_menu_selection(&mut state, 1));
    assert!(crate::app::update::activate_menu(&mut state).is_none());
    assert_eq!(state.todo.group_range(0), 0..2);

    // 对已在组内的 a 再次打开右键菜单：因下方还有 c，且自身在组内，同时暴露 GroupWithNext / Split / UngroupAll。
    open_todo_item_menu(&mut state, &id_a, (10, 10));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu opened");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::EditTodo,
            MenuCommand::GroupTodoWithNext,
            MenuCommand::SplitTodoFromGroup,
            MenuCommand::UngroupTodoAll,
            MenuCommand::CycleTodoPriority,
            MenuCommand::DeleteTodo,
        ]
    );

    // 执行 UngroupTodoAll 解散整组。
    assert!(crate::app::update::set_menu_selection(&mut state, 3));
    assert!(crate::app::update::activate_menu(&mut state).is_none());
    assert!(!state.todo.is_grouped(0));
    assert!(!state.todo.is_grouped(1));
}

#[test]
fn resolve_display_index_rejects_ghost_slot_and_press_during_edit_does_not_arm_stale_drag() {
    let mut state = todo_state(&["a", "b", "c"]);
    let now = Instant::now();

    // 在第 0 项后打开空白幽灵行：显示序列为 a(0), 幽灵(1), b(2), c(3)。
    assert!(begin_todo_edit(&mut state, TodoEditTarget::After(0)));
    assert_eq!(resolve_display_index(&state, 0), Some(0));
    assert_eq!(
        resolve_display_index(&state, 1),
        None,
        "幽灵行槽位不得误映射为下方实体条目 b"
    );
    assert_eq!(resolve_display_index(&state, 2), Some(1));
    assert_eq!(resolve_display_index(&state, 3), Some(2));

    // 对空白独立幽灵行自身调用优先级/状态点击或拖拽目标解析，不得误改下方条目 b。
    assert!(!click_todo_priority(&mut state, 1));
    assert!(!click_todo_status(&mut state, 1));
    assert_eq!(state.todo.items()[1].priority, TodoPriority::P1);
    assert_eq!(state.todo.items()[1].status, TodoStatus::Todo);
    assert!(state.todo.is_editing());

    // 在幽灵行打开期间按下 display = 2（即条目 b）：
    // 提交会销毁空白幽灵行并使 b 上移到 display = 1，此次按下仅完成提交与选中，不得登记陈旧的 from_display = 2。
    assert!(begin_todo_row_press(&mut state, 2, now));
    assert!(!state.todo.is_editing());
    assert_eq!(state.todo.selected(), Some(1));
    assert!(
        state.todo_drag.is_none(),
        "消费在途编辑提交的按下不得登记陈旧拖拽起点"
    );
    assert!(!drag_todo_to(&mut state, Some(1)));
    assert!(!finish_todo_drag(&mut state));
    assert!(!state.todo.is_grouped(0));
    assert!(!state.todo.is_grouped(1));
    assert!(!state.todo.is_grouped(2));
}

#[test]
fn undo_delete_in_two_item_group_restores_group_in_memory_and_db() {
    let mut state = todo_state(&["a", "b", "c"]);
    assert!(apply_todo_key(&mut state, TodoKey::Char('m')));
    assert!(apply_todo_key(&mut state, TodoKey::Space));
    assert_eq!(state.todo.group_range(0), 0..2);
    let gid = state.todo.items()[0].group_id.clone().expect("grouped");

    let db = crate::db::Database::open_in_memory().expect("in-memory db");
    for item in state.todo.items() {
        db.save_todo(item).expect("seed todo in db");
    }
    db.sync_date_todos(state.todo.selected_date(), state.todo.items())
        .expect("sync initial group");

    // 删除 2 人组中的首项 a：幸存项 b 自动退化为 Single。
    let removed_id = state.todo.items()[0].id.clone();
    assert!(delete_todo_selected(&mut state));
    assert_eq!(texts(&state), vec!["b", "c"]);
    assert!(!state.todo.is_grouped(0));
    assert_eq!(state.todo.items()[0].group_id, None);
    db.delete_todo(&removed_id).expect("delete a in db");
    db.sync_date_todos(state.todo.selected_date(), state.todo.items())
        .expect("sync degenerated survivor");

    // 撤销删除：内存与数据库中的 2 人组 [a, b] 必须完整恢复原 group_id 与状态。
    state.effects.clear();
    assert!(undo_todo_delete(&mut state));
    assert_eq!(texts(&state), vec!["a", "b", "c"]);
    assert_eq!(state.todo.group_range(0), 0..2);
    assert!(state.todo.is_grouped(0));
    assert!(state.todo.is_grouped(1));
    assert_eq!(
        state.todo.items()[0].group_id.as_deref(),
        Some(gid.as_str())
    );
    assert_eq!(
        state.todo.items()[1].group_id.as_deref(),
        Some(gid.as_str())
    );
    assert_eq!(state.todo.items()[0].status, TodoStatus::InProgress);
    assert_eq!(state.todo.items()[1].status, TodoStatus::InProgress);

    for effect in &state.effects {
        match effect {
            Effect::RestoreTodos { records } => {
                db.restore_todos(records).expect("restore todos in db");
            }
            Effect::SyncDateTodos {
                entry_date,
                records,
            } => {
                db.sync_date_todos(entry_date, records)
                    .expect("sync restored group in db");
            }
            _ => {}
        }
    }
    let db_items = db
        .list_todos_by_date(state.todo.selected_date())
        .expect("list todos");
    assert_eq!(db_items.len(), 3);
    assert_eq!(db_items[0].group_id.as_deref(), Some(gid.as_str()));
    assert_eq!(db_items[1].group_id.as_deref(), Some(gid.as_str()));
    assert_eq!(db_items[2].group_id, None);
}

#[test]
fn status_click_and_tab_work_on_editing_anchor_and_in_group_ghost_row() {
    let mut state = todo_state(&["a", "b"]);
    assert!(apply_todo_key(&mut state, TodoKey::Char('m')));
    assert_eq!(state.todo.group_range(0), 0..2);

    // 1. 双击/进入锚点项 a 的行内编辑，直接点击 display = 0 的状态框：提交编辑并切换整组状态。
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));
    type_text(&mut state, "-edit");
    state.effects.clear();
    assert!(click_todo_status(&mut state, 0));
    assert!(!state.todo.is_editing());
    assert_eq!(texts(&state), vec!["a-edit", "b"]);
    assert_eq!(state.todo.items()[0].status, TodoStatus::InProgress);
    assert_eq!(state.todo.items()[1].status, TodoStatus::InProgress);

    // 2. 在组内第 0 项后按 o 打开组内新建幽灵行（显示槽位 1，承载 3 槽位组的唯一状态框）：
    // 按 Tab 循环切换整组状态（InProgress -> Done），且不退出幽灵行编辑。
    assert_eq!(state.todo.selected(), Some(0));
    assert!(apply_todo_key(&mut state, TodoKey::Char('o')));
    state.effects.clear();
    assert!(apply_todo_key(&mut state, TodoKey::Tab));
    assert!(state.todo.is_editing());
    assert_eq!(
        state.todo.editing().expect("editing").initial_status,
        TodoStatus::Done
    );
    assert_eq!(state.todo.items()[0].status, TodoStatus::Done);
    assert_eq!(state.todo.items()[1].status, TodoStatus::Done);
    assert_eq!(take_saved(&mut state).len(), 2);

    // 3. 在组内幽灵行输入文本后点击幽灵行（display = 1）上的状态框：
    // 先提交新成员入组，再将整组 3 条成员状态循环切换回 Todo。
    type_text(&mut state, "mid");
    state.effects.clear();
    assert!(click_todo_status(&mut state, 1));
    assert!(!state.todo.is_editing());
    assert_eq!(texts(&state), vec!["a-edit", "mid", "b"]);
    assert_eq!(state.todo.group_range(0), 0..3);
    assert!(
        state
            .todo
            .items()
            .iter()
            .all(|item| item.status == TodoStatus::Todo)
    );
}

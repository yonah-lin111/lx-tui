//! todo 行为层测试：三态循环、编辑提交/取消、删除撤销、工具栏与粘贴分派。
//!
//! 全部走 `AppState` 公开入口，断言内存列表与登记的 [`Effect`]，不触库。

use super::*;

/// 造一条列表并切到 Todo 面板、聚焦（键盘动作的前提）。
fn todo_state(texts: &[&str]) -> AppState {
    let mut state = AppState::demo();
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let records = texts
        .iter()
        .enumerate()
        .map(|(index, text)| TodoRecord::new(*text, TodoStatus::Todo, index as i64))
        .collect();
    apply_todo_list(&mut state, records);
    assert!(focus_todo(&mut state));
    state
}

/// 逐字输入。
fn type_text(state: &mut AppState, text: &str) {
    for ch in text.chars() {
        apply_todo_key(state, TodoKey::Char(ch));
    }
}

/// 当前列表文本快照。
fn texts(state: &AppState) -> Vec<&str> {
    state
        .todo
        .items()
        .iter()
        .map(|item| item.text.as_str())
        .collect()
}

/// 取走全部 `SaveTodo` 效应并清空效应队列，返回各次写入的状态序列。
fn take_saved(state: &mut AppState) -> Vec<TodoStatus> {
    let statuses = state
        .effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::SaveTodo { record } => Some(record.status),
            _ => None,
        })
        .collect();
    state.effects.clear();
    statuses
}

#[test]
fn space_cycles_todo_status_and_emits_save() {
    let mut state = todo_state(&["task"]);

    assert!(apply_todo_key(&mut state, TodoKey::Space));
    assert_eq!(state.todo.items()[0].status, TodoStatus::InProgress);
    assert_eq!(take_saved(&mut state), [TodoStatus::InProgress]);

    assert!(apply_todo_key(&mut state, TodoKey::Space));
    assert_eq!(state.todo.items()[0].status, TodoStatus::Done);
    assert_eq!(take_saved(&mut state), [TodoStatus::Done]);

    assert!(apply_todo_key(&mut state, TodoKey::Space));
    assert_eq!(state.todo.items()[0].status, TodoStatus::Todo);
    assert_eq!(take_saved(&mut state), [TodoStatus::Todo]);

    assert_eq!(state.todo.len(), 1, "循环不改条数");
}

#[test]
fn new_todo_on_empty_enter_creates_nothing() {
    let mut state = todo_state(&[]);

    assert!(apply_todo_key(&mut state, TodoKey::Char('a')));
    assert!(state.todo.is_editing(), "前提：进入新建编辑");

    assert!(apply_todo_key(&mut state, TodoKey::Enter));

    assert!(state.todo.is_empty(), "空输入不创建条目");
    assert!(!state.todo.is_editing(), "提交后退出编辑态");
    assert!(state.effects.is_empty(), "不登记写库效应");
}

#[test]
fn commit_with_text_inserts_item_and_emits_save() {
    let mut state = todo_state(&["first"]);

    assert!(apply_todo_key(&mut state, TodoKey::Char('a')));
    type_text(&mut state, "second");
    assert!(apply_todo_key(&mut state, TodoKey::Enter));

    assert_eq!(texts(&state), ["first", "second"]);
    assert_eq!(state.todo.items()[1].position, 1, "位置归一为下标");
    assert_eq!(state.todo.selected(), Some(1), "新建后选中新条目");
    assert!(!state.todo.is_editing());
    let saved = take_saved(&mut state);
    assert_eq!(saved, [TodoStatus::Todo], "登记一次写入");
}

#[test]
fn editing_cleared_text_deletes_item() {
    let mut state = todo_state(&["keep", "drop"]);
    assert!(state.todo.select(1));
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(1)));

    assert!(apply_todo_key(&mut state, TodoKey::Clear));
    assert_eq!(state.todo.editing().expect("editing").input.text(), "");
    assert!(apply_todo_key(&mut state, TodoKey::Enter));

    assert_eq!(texts(&state), ["keep"], "清空文本提交即删除该条");
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::DeleteTodo { .. })),
        "登记删除效应"
    );
    assert!(state.todo.can_undo(), "删除进入撤销槽");
}

#[test]
fn esc_cancels_edit_restoring_original_text() {
    let mut state = todo_state(&["origin"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));
    type_text(&mut state, "junk");

    assert!(apply_todo_key(&mut state, TodoKey::Esc));

    assert!(!state.todo.is_editing());
    assert_eq!(texts(&state), ["origin"], "取消不改原文");
    assert!(state.effects.is_empty(), "取消不写库");
}

#[test]
fn switch_tab_commits_edit() {
    let mut state = todo_state(&[]);
    assert!(apply_todo_key(&mut state, TodoKey::Char('a')));
    type_text(&mut state, "committed");

    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Prompt
    ));

    assert!(!state.todo.is_editing(), "切 tab 提交编辑");
    assert_eq!(texts(&state), ["committed"]);
    assert_eq!(take_saved(&mut state), [TodoStatus::Todo]);
}

#[test]
fn switch_workspace_commits_edit_and_preserves_list() {
    let mut state = todo_state(&["a", "b", "c"]);
    crate::app::update::create_workspace(&mut state);
    assert_eq!(state.active_workspace, 1, "前提：第二个工作区已激活");
    state.todo.set_viewport_rows(1);
    assert!(state.todo.select(2));
    // `select` 已把末项滚入视口（scroll = 2）；降到 1 才是一次真实位移。
    assert!(set_todo_scroll(&mut state, 1));
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(2)));
    type_text(&mut state, "-edited");

    crate::app::update::switch_workspace(&mut state, 0);

    assert!(!state.todo.is_editing(), "切工作区提交编辑");
    assert_eq!(texts(&state), ["a", "b", "c-edited"]);
    assert_eq!(state.todo.selected(), Some(2), "选中不随工作区变化");
    assert_eq!(state.todo.scroll(), 1, "视口不随工作区变化");
    assert!(state.todo_focused, "焦点不随工作区变化");
}

#[test]
fn delete_then_undo_restores_position() {
    let mut state = todo_state(&["a", "b", "c"]);
    assert!(state.todo.select(1));

    assert!(delete_todo_selected(&mut state));
    assert_eq!(texts(&state), ["a", "c"]);
    assert!(state.todo.can_undo());
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::DeleteTodo { .. }))
    );

    assert!(undo_todo_delete(&mut state));

    assert_eq!(texts(&state), ["a", "b", "c"], "插回原位置");
    assert_eq!(state.todo.items()[1].text, "b");
    assert!(!state.todo.can_undo(), "撤销槽一次即失效");
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::RestoreTodos { .. })),
        "登记恢复效应"
    );
}

#[test]
fn clear_done_removes_only_done_and_can_be_undone() {
    let mut state = todo_state(&["open", "done-1", "active", "done-2"]);
    for index in [1, 3] {
        let text = state.todo.items()[index].text.clone();
        state.todo.update(index, text, TodoStatus::Done);
    }

    assert!(clear_done_todos(&mut state));

    assert_eq!(texts(&state), ["open", "active"], "只删完成项");
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        Effect::DeleteTodosWithStatus {
            status: TodoStatus::Done
        }
    )));

    assert!(undo_todo_delete(&mut state));
    assert_eq!(texts(&state), ["open", "done-1", "active", "done-2"]);
}

#[test]
fn write_failure_keeps_items_and_toasts() {
    let mut state = todo_state(&["a", "b"]);
    let before: Vec<String> = state
        .todo
        .items()
        .iter()
        .map(|item| item.text.clone())
        .collect();

    fail_todo_write(&mut state);

    assert_eq!(
        state
            .todo
            .items()
            .iter()
            .map(|item| item.text.clone())
            .collect::<Vec<_>>(),
        before,
        "写库失败不改内存列表"
    );
    assert!(state.toast.is_some(), "失败必须 toast");
}

#[test]
fn toolbar_button_disabled_states() {
    let mut state = todo_state(&[]);
    let snapshot = (
        state.todo.len(),
        state.todo.selected(),
        state.todo.is_editing(),
        state.prompt_focused,
    );

    assert!(
        !apply_todo_button(&mut state, TodoButton::Edit),
        "空列表不可编辑"
    );
    assert!(
        !apply_todo_button(&mut state, TodoButton::Delete),
        "空列表无可删"
    );
    assert!(
        !apply_todo_button(&mut state, TodoButton::ClearDone),
        "无完成项"
    );
    assert_eq!(
        (
            state.todo.len(),
            state.todo.selected(),
            state.todo.is_editing(),
            state.prompt_focused,
        ),
        snapshot,
        "无效按钮不产生任何状态变化"
    );

    assert!(
        apply_todo_button(&mut state, TodoButton::New),
        "新建始终可用"
    );
    assert!(state.todo.is_editing());
}

#[test]
fn paste_in_list_mode_is_swallowed() {
    let mut state = todo_state(&["a"]);

    assert!(!apply_todo_paste(&mut state, "pasted"));
    assert!(!state.todo.is_editing());
    assert_eq!(texts(&state), ["a"], "列表态粘贴不建条目");
    assert!(state.effects.is_empty());
}

#[test]
fn paste_while_editing_inserts() {
    let mut state = todo_state(&[]);
    assert!(apply_todo_key(&mut state, TodoKey::Char('a')));

    assert!(apply_todo_paste(&mut state, "pasted text"));

    assert_eq!(
        state.todo.editing().expect("editing").input.text(),
        "pasted text",
        "编辑态粘贴写入输入框"
    );
    assert!(state.todo.items().is_empty(), "提交前不建条目");
}

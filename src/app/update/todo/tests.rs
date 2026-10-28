//! todo 行为层测试：三态循环、编辑提交/取消、删除撤销、工具栏与粘贴分派。
//!
//! 全部走 `AppState` 公开入口，断言内存列表与登记的 [`Effect`]，不触库。

use std::time::Duration;

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
    // 切到 Todo tab 已经同时把焦点交给右栏（见 `select_right_tab`），这里只做断言。
    assert!(state.todo_focused(), "切到 Todo tab 后右栏应持焦点");
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
    // 进入编辑会把编辑行滚入视口（新口径），此处以**切换前**的视口为准。
    let scroll = state.todo.scroll();

    crate::app::update::switch_workspace(&mut state, 0);

    assert!(!state.todo.is_editing(), "切工作区提交编辑");
    assert_eq!(texts(&state), ["a", "b", "c-edited"]);
    assert_eq!(state.todo.selected(), Some(2), "选中不随工作区变化");
    assert_eq!(state.todo.scroll(), scroll, "视口不随工作区变化");
    assert!(state.todo_focused(), "焦点不随工作区变化");
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
        state.prompt_focused(),
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
            state.prompt_focused(),
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

#[test]
fn tab_in_list_mode_cycles_status() {
    let mut state = todo_state(&["a", "b"]);

    assert!(apply_todo_key(&mut state, TodoKey::Tab));
    assert_eq!(state.todo.items()[0].status, TodoStatus::InProgress);
    assert_eq!(take_saved(&mut state), [TodoStatus::InProgress]);
    assert!(!state.todo.is_editing(), "列表态 Tab 只切状态，不进编辑");

    assert!(apply_todo_key(&mut state, TodoKey::Tab));
    assert_eq!(state.todo.items()[0].status, TodoStatus::Done);
}

#[test]
fn tab_while_editing_cycles_status_and_keeps_editing() {
    let mut state = todo_state(&["item"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));
    type_text(&mut state, "-edited");

    assert!(apply_todo_key(&mut state, TodoKey::Tab));

    assert!(state.todo.is_editing(), "Tab 不结束编辑");
    assert_eq!(
        state.todo.editing().expect("editing").input.text(),
        "item-edited",
        "Tab 不写进输入框"
    );
    assert_eq!(state.todo.items()[0].status, TodoStatus::InProgress);
    assert_eq!(take_saved(&mut state), [TodoStatus::InProgress]);
}

#[test]
fn tab_while_editing_new_todo_is_swallowed() {
    let mut state = todo_state(&[]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::New));

    assert!(
        !apply_todo_key(&mut state, TodoKey::Tab),
        "新建态没有记录可切状态"
    );
    assert!(state.todo.is_editing());
    assert!(state.effects.is_empty());
}

#[test]
fn click_row_twice_within_window_enters_edit() {
    let mut state = todo_state(&["a", "b"]);
    let t0 = Instant::now();

    assert!(click_todo_row(&mut state, 1, t0));
    assert_eq!(state.todo.selected(), Some(1));
    assert!(!state.todo.is_editing(), "单击不进入编辑");

    assert!(click_todo_row(
        &mut state,
        1,
        t0 + Duration::from_millis(50)
    ));
    assert!(state.todo.is_editing(), "双击进入编辑");
    assert_eq!(
        state.todo.editing().expect("editing").input.text(),
        "b",
        "编辑现场带原文"
    );
}

#[test]
fn click_different_row_resets_double_click() {
    let mut state = todo_state(&["a", "b", "c"]);
    let t0 = Instant::now();

    // 首行本来就是选中并聚焦的：这一击没有可见变化，返回值只能是 false。
    assert!(!click_todo_row(&mut state, 0, t0), "已选中行单击无变化");
    click_todo_row(&mut state, 1, t0 + Duration::from_millis(50));
    assert!(!state.todo.is_editing(), "换行后不算双击");
    // 同一次点击序列被换行重置；紧接着再点第二行仍是一次单击。
    click_todo_row(&mut state, 1, t0 + Duration::from_millis(80));
    assert!(state.todo.is_editing(), "同行为双击");
}

#[test]
fn click_outside_window_is_single_click() {
    let mut state = todo_state(&["a"]);
    let t0 = Instant::now();

    // 唯一一行已被选中，单击无变化。
    assert!(!click_todo_row(&mut state, 0, t0));
    click_todo_row(
        &mut state,
        0,
        t0 + crate::app::update::PROMPT_DOUBLE_CLICK_WINDOW + Duration::from_millis(1),
    );
    assert!(!state.todo.is_editing(), "超出窗口只算单击");
}

#[test]
fn double_click_editing_row_does_not_restart_edit() {
    let mut state = todo_state(&["a", "b"]);
    let t0 = Instant::now();
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(1)));
    type_text(&mut state, "-draft");

    click_todo_row(&mut state, 1, t0);
    click_todo_row(&mut state, 1, t0 + Duration::from_millis(30));

    assert!(state.todo.is_editing(), "仍是编辑态（未被打断）");
    assert_eq!(
        state.todo.editing().expect("editing").input.text(),
        "b-draft",
        "草稿不得被原文覆盖"
    );
}

/// 幽灵行占一个显示槽位：它下方的行在显示序列里整体后移一位，点击必须换算回列表下标。
#[test]
fn click_below_ghost_row_maps_display_to_list_index() {
    let mut state = todo_state(&["a", "b", "c"]);
    // 在 a 之后插入：显示序列 a(0) / 幽灵(1) / b(2) / c(3)。
    assert!(begin_todo_edit(&mut state, TodoEditTarget::After(0)));

    assert!(click_todo_row(&mut state, 2, Instant::now()));
    assert_eq!(
        state.todo.selected_item().map(|item| item.text.as_str()),
        Some("b"),
        "点幽灵行下方第一行必须选中 b，不是 c"
    );
}

/// 幽灵行打开时点它下方的行：选中按**条目**而不是行号落位；
/// 提交让幽灵行消失后，同一屏幕行的内容已经变了，点击序列随之作废（第二击只选中、不编辑）。
#[test]
fn click_below_ghost_selects_by_item_then_resets_sequence() {
    let mut state = todo_state(&["a", "b", "c"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::After(0)));
    let t0 = Instant::now();

    assert!(click_todo_row(&mut state, 2, t0));
    assert_eq!(
        state.todo.selected_item().map(|item| item.text.as_str()),
        Some("b"),
        "第一击必须选中 b（按条目，不按行号）"
    );
    assert!(!state.todo.is_editing(), "空白幽灵行提交后不创建条目");
    assert!(state.todo_row_click.is_none(), "幽灵行消失后点击序列作废");

    click_todo_row(&mut state, 2, t0 + Duration::from_millis(20));
    assert_eq!(
        state.todo.selected_item().map(|item| item.text.as_str()),
        Some("c"),
        "行号已整体上移，同一屏幕行现在是 c"
    );
    assert!(!state.todo.is_editing(), "序列已作废，不得编辑任何条目");
}

/// 幽灵行打开时双击幽灵行本身：不提交、不重开——草稿保留。
#[test]
fn double_click_ghost_row_keeps_draft() {
    let mut state = todo_state(&["a", "b"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::After(0)));
    type_text(&mut state, "draft");
    let t0 = Instant::now();

    click_todo_row(&mut state, 1, t0);
    click_todo_row(&mut state, 1, t0 + Duration::from_millis(20));

    let edit = state.todo.editing().expect("仍在编辑");
    assert!(edit.target.is_none(), "仍是新建");
    assert_eq!(edit.input.text(), "draft", "草稿保留");
}

#[test]
fn todo_edit_shortcuts_and_selection_lifecycle() {
    let mut state = todo_state(&["hello world"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));

    // Ctrl+A / LineStart 移至句首
    assert!(apply_todo_key(&mut state, TodoKey::LineStart));
    assert_eq!(state.todo.edit_input_mut().unwrap().cursor(), 0);

    // Ctrl+E / LineEnd 移至句末
    assert!(apply_todo_key(&mut state, TodoKey::LineEnd));
    assert_eq!(state.todo.edit_input_mut().unwrap().cursor(), 11);

    // 选区与退格
    state.todo.edit_input_mut().unwrap().begin_selection(6);
    state.todo.edit_input_mut().unwrap().drag_selection(11);
    assert_eq!(
        state
            .todo
            .edit_input_mut()
            .unwrap()
            .selected_text()
            .as_deref(),
        Some("world")
    );
    assert!(apply_todo_key(&mut state, TodoKey::Backspace));
    assert_eq!(state.todo.editing().unwrap().input.text(), "hello ");

    // Ctrl+U 删至行首
    assert!(apply_todo_key(&mut state, TodoKey::DeleteToLineStart));
    assert_eq!(state.todo.editing().unwrap().input.text(), "");
}

#[test]
fn todo_undo_and_redo_editing_and_list_mode_swallowed() {
    let mut state = todo_state(&["item"]);

    // 列表态：按 Undo/Redo 直接吞掉，不再恢复已删待办项
    assert!(!apply_todo_key(&mut state, TodoKey::Undo));
    assert!(!apply_todo_key(&mut state, TodoKey::Redo));

    // 编辑态：撤销打字与重做
    assert!(begin_todo_edit(&mut state, TodoEditTarget::Current(0)));
    apply_todo_key(&mut state, TodoKey::Char('!'));
    assert_eq!(state.todo.editing().unwrap().input.text(), "item!");

    // 撤销
    assert!(apply_todo_key(&mut state, TodoKey::Undo));
    assert_eq!(state.todo.editing().unwrap().input.text(), "item");

    // 重做
    assert!(apply_todo_key(&mut state, TodoKey::Redo));
    assert_eq!(state.todo.editing().unwrap().input.text(), "item!");
}

#[test]
fn todo_replace_at_max_limit_with_selection() {
    let mut state = todo_state(&[]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::New));
    let max_text = "a".repeat(crate::db::todo::MAX_TODO_TEXT_CHARS);
    assert!(apply_todo_paste(&mut state, &max_text));

    // 选中最后 5 个字符，插入 1 个字符替换
    state.todo.edit_input_mut().unwrap().set_selection(
        crate::db::todo::MAX_TODO_TEXT_CHARS - 5,
        crate::db::todo::MAX_TODO_TEXT_CHARS,
    );
    assert!(apply_todo_key(&mut state, TodoKey::Char('b')));
    assert_eq!(
        state.todo.editing().unwrap().input.text().chars().count(),
        crate::db::todo::MAX_TODO_TEXT_CHARS - 4
    );
    assert!(state.todo.editing().unwrap().input.text().ends_with('b'));
}

/// 幽灵行打开时点未选中行之外的位置不再造成越界选中。
#[test]
fn click_beyond_list_with_ghost_is_ignored() {
    let mut state = todo_state(&["a"]);
    assert!(begin_todo_edit(&mut state, TodoEditTarget::After(0)));
    // 显示序列：a(0) / 幽灵(1)，长度 2。点显示下标 2 越界。
    assert!(!click_todo_row(&mut state, 2, Instant::now()));
    assert_eq!(state.todo.len(), 1);
}

/// 列表态 `Enter` 编辑当前项（不是新建）；`Shift+Enter` 才是新建。
#[test]
fn enter_edits_current_row_and_shift_enter_creates() {
    let mut state = todo_state(&["a", "b"]);
    assert!(state.todo.select(1));

    assert!(apply_todo_key(&mut state, TodoKey::Enter));
    let edit = state.todo.editing().expect("Enter 进入编辑");
    assert_eq!(
        edit.target.as_deref(),
        Some(state.todo.items()[1].id.as_str()),
        "编辑的是当前选中项"
    );
    assert_eq!(edit.input.text(), "b", "输入框带原文");
    assert!(apply_todo_key(&mut state, TodoKey::Esc));

    assert!(apply_todo_key(&mut state, TodoKey::ShiftEnter));
    let edit = state.todo.editing().expect("Shift+Enter 新建");
    assert!(edit.target.is_none(), "新建现场没有目标 id");
    assert_eq!(edit.at, 2, "插在当前项之后");
    assert_eq!(edit.input.text(), "", "新建输入框为空");
}

/// 列表为空时 `Enter` 退化为新建（没有当前项可编辑）。
#[test]
fn enter_on_empty_list_starts_new_todo() {
    let mut state = todo_state(&[]);
    assert!(apply_todo_key(&mut state, TodoKey::Enter));
    assert!(
        state
            .todo
            .editing()
            .is_some_and(|edit| edit.target.is_none())
    );
}

/// 编辑态 `Shift+Enter` 与 `Enter` 同义（单行输入没有换行）。
#[test]
fn shift_enter_while_editing_commits() {
    let mut state = todo_state(&["seed"]);
    assert!(apply_todo_key(&mut state, TodoKey::Enter));
    type_text(&mut state, "-x");

    assert!(apply_todo_key(&mut state, TodoKey::ShiftEnter));

    assert!(!state.todo.is_editing(), "Shift+Enter 也提交");
    assert_eq!(texts(&state), ["seed-x"]);
}

/// 切顶部标签栏：编辑现场必须被提交，不能悬空。
#[test]
fn switching_top_tab_commits_todo_edit() {
    let mut state = todo_state(&["seed"]);
    assert!(apply_todo_key(&mut state, TodoKey::Enter));
    type_text(&mut state, "-x");

    crate::app::update::switch_tab(&mut state, 0);

    assert!(!state.todo.is_editing(), "切标签必须提交编辑");
    assert_eq!(texts(&state), ["seed-x"]);
    assert_eq!(take_saved(&mut state), [TodoStatus::Todo], "提交要写库");
    assert!(state.panes_focused(), "焦点回到主区");
}

/// 新建标签同样提交：`focus_panes` 是所有「交回主区」路径的唯一收口。
#[test]
fn creating_tab_commits_todo_edit() {
    let mut state = todo_state(&["seed"]);
    assert!(apply_todo_key(&mut state, TodoKey::Enter));
    type_text(&mut state, "-y");

    crate::app::update::create_tab(&mut state);

    assert!(!state.todo.is_editing());
    assert_eq!(texts(&state), ["seed-y"]);
}

/// 列表溢出时在末尾新建：编辑行必须滚进视口（否则用户看不到输入位置）。
#[test]
fn begin_edit_scrolls_ghost_row_into_view() {
    let mut state = todo_state(&["a", "b", "c", "d", "e", "f", "g", "h"]);
    state.todo.set_viewport_rows(3);
    assert_eq!(state.todo.scroll(), 0, "前提：视口贴顶");

    assert!(begin_todo_edit(&mut state, TodoEditTarget::New));

    assert_eq!(state.todo.scroll(), 6, "8 条 + 幽灵 1 行 - 视口 3 行");
}

/// 输入边界即存储上限：超长不得进内存列表（否则写库被拒、重启即丢）。
#[test]
fn typing_stops_at_storage_limit() {
    let mut state = todo_state(&[]);
    assert!(apply_todo_key(&mut state, TodoKey::Char('a')));
    let limit = crate::db::todo::MAX_TODO_TEXT_CHARS;
    type_text(&mut state, &"字".repeat(limit + 50));

    let text = state
        .todo
        .editing()
        .expect("仍在编辑")
        .input
        .text()
        .to_string();
    assert_eq!(text.chars().count(), limit, "逐字输入到上限即停");

    assert!(commit_todo_edit(&mut state));
    assert_eq!(
        state.todo.items()[0].text.chars().count(),
        limit,
        "落库条目不会超过上限"
    );
}

/// 粘贴按剩余空间裁剪，同样不会超出上限。
#[test]
fn paste_is_clipped_to_storage_limit() {
    let mut state = todo_state(&[]);
    assert!(apply_todo_key(&mut state, TodoKey::Char('a')));
    let limit = crate::db::todo::MAX_TODO_TEXT_CHARS;
    assert!(apply_todo_paste(&mut state, &"字".repeat(limit + 50)));

    let text = state
        .todo
        .editing()
        .expect("仍在编辑")
        .input
        .text()
        .to_string();
    assert_eq!(text.chars().count(), limit, "粘贴按剩余空间裁剪");

    // 已经装满：再粘贴不产生变化，也不置脏。
    assert!(!apply_todo_paste(&mut state, "more"));
}

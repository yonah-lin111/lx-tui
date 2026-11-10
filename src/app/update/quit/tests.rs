//! 退出确认层状态测试：独立层、底层浮层/编辑保留、快捷键去抖与单次提交。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::actions::Action;
use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, Rename, RenameTarget, TextInput};
use crate::app::prompt::{PromptEditing, PromptItem, PromptMode};
use crate::app::selection::Selection;
use crate::app::state::{AppState, QuitConfirmState};
use crate::app::trust::TrustTrigger;
use crate::app::update::{self, apply};

/// 打开一个重命名浮层作为底层模态。
fn open_rename_overlay(state: &mut AppState) {
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new("name"),
    }));
}

/// 进入一条正式提示词的编辑视图并输入草稿。
fn open_item_editing(state: &mut AppState) {
    let item = PromptItem::single(1, None, "item", "body");
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    state.workspaces[0].prompt_panel.mode = PromptMode::Editor;
    assert!(update::edit_prompt_item(state, 0, 1), "进入条目编辑视图");
    state.prompt.insert_str(" draft");
}

/// 已登记的 `SavePromptContent` 效应数。
fn save_effects(state: &AppState) -> usize {
    state
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::SavePromptContent { .. }))
        .count()
}

#[test]
fn request_quit_leaves_underlying_overlay_untouched() {
    let mut state = AppState::demo();
    open_rename_overlay(&mut state);
    apply(Action::RequestQuit, &mut state);
    assert!(state.quit_confirm.is_some(), "确认层独立打开");
    assert!(
        matches!(state.overlay, Some(Overlay::Rename(_))),
        "底层浮层保持不变"
    );
    assert!(!state.should_quit);
}

#[test]
fn request_quit_clears_active_gestures() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.resizing_sidebar = true;
    state.resizing_prompt = true;
    state.overlay_input_drag = true;
    state.todo_input_drag = true;
    state.prompt_search_drag = true;
    state.lx_input_drag = Some(pane);
    state.ask_input_drag = Some(pane);
    state.selection_autoscroll = Some(crate::app::state::SelectionAutoscroll {
        pane,
        direction: crate::app::state::AutoscrollDirection::Down,
        mouse: (0, 0),
        inner: ratatui::layout::Rect::new(0, 0, 1, 1),
        next_at: std::time::Instant::now(),
    });
    state.terminal_scroll_drag = Some((pane, 0));
    state.todo_scroll_drag = Some(2);
    state.workspace_drag = Some(0);
    state.workspace_dragging = true;
    state.workspace_drag_moved = true;
    apply(Action::RequestQuit, &mut state);
    assert!(!state.resizing_sidebar);
    assert!(!state.resizing_prompt);
    assert!(!state.overlay_input_drag);
    assert!(!state.todo_input_drag);
    assert!(!state.prompt_search_drag);
    assert!(state.lx_input_drag.is_none());
    assert!(state.ask_input_drag.is_none());
    assert!(state.selection_autoscroll.is_none());
    assert!(state.terminal_scroll_drag.is_none());
    assert!(state.todo_scroll_drag.is_none());
    assert!(state.workspace_drag.is_none());
    assert!(!state.workspace_dragging);
    assert!(!state.workspace_drag_moved);
}

#[test]
fn request_quit_does_not_set_should_quit_or_commit() {
    let mut state = AppState::demo();
    assert!(update::open_temp_prompt(&mut state));
    state.prompt.insert_str("draft");
    apply(Action::RequestQuit, &mut state);
    assert!(!state.should_quit, "第一次请求不退出");
    assert!(state.quit_confirm.is_some());
    assert!(state.overlay.is_none(), "没有底层浮层时也不创建");
    let Some(PromptEditing::Temp(id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp prompt editing expected");
    };
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .temp(id)
            .expect("temp prompt")
            .active_content(),
        "",
        "请求退出不提交编辑器"
    );
}

#[test]
fn confirm_quit_sets_should_quit_and_commits() {
    let mut state = AppState::demo();
    assert!(update::open_temp_prompt(&mut state));
    state.prompt.insert_str("draft");
    apply(Action::RequestQuit, &mut state);
    apply(Action::ConfirmQuit, &mut state);
    assert!(state.should_quit);
    assert!(state.quit_confirm.is_none());
    assert!(state.prompt_committed_for_quit);
    let Some(PromptEditing::Temp(id)) = state.workspaces[0].prompt_panel.editing else {
        panic!("temp prompt editing expected");
    };
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .temp(id)
            .expect("temp prompt")
            .active_content(),
        "draft",
        "确认退出才提交编辑器"
    );
}

#[test]
fn cancel_quit_preserves_underlying_overlay() {
    let mut state = AppState::demo();
    open_rename_overlay(&mut state);
    let pane = state.prompt.id();
    let selection = Selection::full(pane, 1, 3);
    state.selection = Some(selection);
    state.lx_selection = Some(selection);
    apply(Action::RequestQuit, &mut state);
    assert!(state.quit_confirm.is_some());
    assert_eq!(state.selection, Some(selection), "请求退出不清选区");
    apply(Action::CancelQuit, &mut state);
    assert!(!state.should_quit);
    assert!(state.quit_confirm.is_none());
    assert!(
        matches!(state.overlay, Some(Overlay::Rename(_))),
        "取消退出不恢复、也不清理底层浮层"
    );
    assert_eq!(state.selection, Some(selection), "取消后选区仍在");
    assert_eq!(state.lx_selection, Some(selection), "取消后 lx 选区仍在");
    assert!(state.quit_cancel_latch.is_some(), "取消后登记短窗口 latch");
}

#[test]
fn quit_confirm_survives_trust_resolution() {
    let mut state = AppState::demo();
    apply(Action::RequestQuit, &mut state);
    let path = PathBuf::from("/tmp/ws");
    update::open_trust_dialog(
        &mut state,
        path.clone(),
        TrustTrigger::CreateWorkspace { cwd: path },
    );
    assert!(
        matches!(state.overlay, Some(Overlay::TrustWorkspace(_))),
        "异步信任弹窗照常在底层打开"
    );
    assert!(state.quit_confirm.is_some(), "信任弹窗不得顶掉退出确认层");
}

#[test]
fn quit_confirm_accepts_shortcut_only_after_quiet_window() {
    let now = Instant::now();
    let mut confirm = QuitConfirmState {
        last_shortcut_press: now,
    };
    assert!(!confirm.accepts_shortcut(now), "刚打开时吞掉自动重复");
    assert!(
        !confirm.accepts_shortcut(now + Duration::from_millis(299)),
        "静默窗口内仍吞掉"
    );
    // 模拟按住时的连续 Press：每次重复都刷新窗口，永远不满足确认条件。
    confirm.note_shortcut_press(now + Duration::from_millis(299));
    assert!(
        !confirm.accepts_shortcut(now + Duration::from_millis(598)),
        "自动重复刷新后仍需再等满窗口"
    );
    assert!(
        confirm.accepts_shortcut(now + Duration::from_millis(599)),
        "停止重复并静默满窗口后才允许确认"
    );
}

#[test]
fn confirm_quit_commits_prompt_once() {
    let mut state = AppState::demo();
    open_item_editing(&mut state);
    apply(Action::RequestQuit, &mut state);
    assert_eq!(save_effects(&state), 0, "请求退出不登记保存");
    apply(Action::ConfirmQuit, &mut state);
    assert_eq!(save_effects(&state), 1, "确认退出只登记一次保存");
    assert!(!crate::db::service::Persistence::shutdown_commits_prompt(
        &state
    ));

    // 取出已登记的保存效应，模拟主循环退出时它尚未被执行。
    let queued = std::mem::take(&mut state.effects);
    assert!(matches!(
        queued.as_slice(),
        [Effect::SavePromptContent { .. }]
    ));
    // 拨回页更新时间：若 shutdown 又提交一次，`page.touch` 会把它改成当前时间。
    if let Some(page) = state.workspaces[0]
        .prompt_panel
        .tree
        .item_mut(1)
        .and_then(|item| item.active_page_mut())
    {
        page.updated_at = 1;
    }
    let mut persistence = crate::db::service::Persistence::without_database();
    persistence.shutdown(&mut state);
    let updated_at = state.workspaces[0]
        .prompt_panel
        .tree
        .item(1)
        .and_then(|item| item.active_page())
        .map(|page| page.updated_at);
    assert_eq!(updated_at, Some(1), "shutdown 不得再次提交编辑器");

    let fresh = AppState::demo();
    assert!(
        crate::db::service::Persistence::shutdown_commits_prompt(&fresh),
        "普通（非确认退出）收尾仍要提交编辑器"
    );
}

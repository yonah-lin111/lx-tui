//! 退出二次确认状态测试：全局 toast、底层浮层/编辑保留、快捷键去抖与单次提交。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::actions::Action;
use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, Rename, RenameTarget, TextInput};
use crate::app::prompt::{PromptEditing, PromptItem, PromptMode};
use crate::app::selection::Selection;
use crate::app::state::{AppState, QUIT_CONFIRM_WINDOW};
use crate::app::trust::TrustTrigger;
use crate::app::update::{self, apply};

/// 打开一个重命名浮层作为底层模态。
fn open_rename_overlay(state: &mut AppState) {
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new("name"),
    }));
}

/// 断言退出提示落在全局 toast，而不是窗格内 toast 或模态层。
fn assert_quit_toast(state: &AppState) {
    let toast = state.toast.as_ref().expect("quit toast");
    assert_eq!(toast.message, crate::ui::text::EXIT_CONFIRM_TOAST);
    assert_eq!(toast.kind, crate::app::toast::ToastKind::Warning);
    assert_eq!(
        toast.title.as_deref(),
        Some(crate::ui::text::EXIT_CONFIRM_TITLE)
    );
    assert_eq!(toast.anchor, None, "退出提示必须是全局 toast");
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
fn request_quit_shows_global_toast_and_leaves_underlying_overlay_untouched() {
    let mut state = AppState::demo();
    open_rename_overlay(&mut state);
    apply(Action::RequestQuit, &mut state);
    assert!(state.quit_requested_at.is_some(), "记录退出请求");
    assert_quit_toast(&state);
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
    assert!(state.quit_requested_at.is_some());
    assert_quit_toast(&state);
    assert!(state.overlay.is_none(), "toast 不创建模态浮层");
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
    update::confirm_quit(&mut state);
    assert!(state.should_quit);
    assert!(state.quit_requested_at.is_none());
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
fn request_after_window_rearms_without_quitting() {
    let mut state = AppState::demo();
    open_rename_overlay(&mut state);
    let pane = state.prompt.id();
    let selection = Selection::full(pane, 1, 3);
    state.selection = Some(selection);
    state.lx_selection = Some(selection);
    apply(Action::RequestQuit, &mut state);
    state.quit_requested_at = Some(Instant::now() - QUIT_CONFIRM_WINDOW - Duration::from_millis(1));
    apply(Action::RequestQuit, &mut state);
    assert!(!state.should_quit, "窗口过期后再次请求只重新开始");
    assert!(state.quit_requested_at.is_some());
    assert_quit_toast(&state);
    assert!(
        matches!(state.overlay, Some(Overlay::Rename(_))),
        "底层浮层原样保留"
    );
    assert_eq!(state.selection, Some(selection), "请求退出不清选区");
    assert_eq!(state.lx_selection, Some(selection), "请求退出不清 lx 选区");
}

#[test]
fn quit_request_survives_trust_resolution() {
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
    assert!(
        state.quit_requested_at.is_some(),
        "信任弹窗不得清掉退出请求"
    );
    assert_quit_toast(&state);
}

#[test]
fn dismissed_toast_restarts_confirmation() {
    let mut state = AppState::demo();
    apply(Action::RequestQuit, &mut state);
    state.toast = None;
    state.quit_requested_at = Some(Instant::now() - Duration::from_millis(301));
    apply(Action::RequestQuit, &mut state);
    assert!(!state.should_quit, "提示已关闭时旧请求不得直接确认退出");
    assert_quit_toast(&state);
}

#[test]
fn second_request_confirms_only_after_guard_and_within_window() {
    let mut state = AppState::demo();
    apply(Action::RequestQuit, &mut state);
    apply(Action::RequestQuit, &mut state);
    assert!(!state.should_quit, "连续自动重复不得退出");
    assert!(state.quit_requested_at.is_some());

    state.quit_requested_at = Some(Instant::now() - Duration::from_millis(299));
    apply(Action::RequestQuit, &mut state);
    assert!(!state.should_quit, "静默窗口内仍吞掉");

    state.quit_requested_at = Some(Instant::now() - Duration::from_millis(301));
    apply(Action::RequestQuit, &mut state);
    assert!(state.should_quit, "静默满窗口后的第二次请求确认退出");
}

#[test]
fn confirm_quit_commits_prompt_once() {
    let mut state = AppState::demo();
    open_item_editing(&mut state);
    apply(Action::RequestQuit, &mut state);
    assert_eq!(save_effects(&state), 0, "请求退出不登记保存");
    update::confirm_quit(&mut state);
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

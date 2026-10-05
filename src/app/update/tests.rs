//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::toast::{TOAST_DURATION, ToastKind};
use crate::terminal::GridSize;
use std::time::Duration;

#[test]
fn quit_sets_flag() {
    let mut state = AppState::demo();
    apply(Action::Quit, &mut state);
    assert!(state.should_quit);
}

#[test]
fn toggle_sidebar_flips_flag() {
    let mut state = AppState::demo();
    apply(Action::ToggleSidebar, &mut state);
    assert!(state.sidebar_collapsed);
    apply(Action::ToggleSidebar, &mut state);
    assert!(!state.sidebar_collapsed);
}

#[test]
fn toggle_prompt_flips_flag() {
    let mut state = AppState::demo();
    apply(Action::TogglePrompt, &mut state);
    assert!(state.prompt_collapsed);
    apply(Action::TogglePrompt, &mut state);
    assert!(!state.prompt_collapsed);
}

#[test]
fn collapsing_prompt_clears_focus() {
    let mut state = AppState::demo();
    focus_prompt(&mut state);
    assert!(state.prompt_focused);
    apply(Action::TogglePrompt, &mut state);
    assert!(!state.prompt_focused);
    apply(Action::TogglePrompt, &mut state);
    assert!(!state.prompt_focused);
}

#[test]
fn focus_switches_between_prompt_and_pane() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    focus_prompt(&mut state);
    assert!(state.prompt_focused);
    focus_pane(&mut state, pane);
    assert!(!state.prompt_focused);
    assert_eq!(state.active_tab().layout.focus(), pane);
}

#[test]
fn editor_commands_edit_prompt_text() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    apply_editor(&mut state, EditorCommand::InsertText("ab".into()));
    apply_editor(&mut state, EditorCommand::InsertChar('你'));
    apply_editor(&mut state, EditorCommand::Newline);
    apply_editor(&mut state, EditorCommand::InsertChar('c'));
    assert_eq!(state.prompt.text(), "ab你\nc");
    apply_editor(&mut state, EditorCommand::Backspace);
    apply_editor(&mut state, EditorCommand::Up);
    apply_editor(&mut state, EditorCommand::Home);
    apply_editor(&mut state, EditorCommand::Delete);
    assert_eq!(state.prompt.text(), "b你\n");
}

#[test]
fn readline_commands_apply_to_prompt() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    apply_editor(&mut state, EditorCommand::InsertText("foo bar baz".into()));
    apply_editor(&mut state, EditorCommand::DeleteWordBackward);
    assert_eq!(state.prompt.text(), "foo bar ");
    apply_editor(&mut state, EditorCommand::DeleteToLineStart);
    assert_eq!(state.prompt.text(), "");
    apply_editor(&mut state, EditorCommand::InsertText("foo bar".into()));
    apply_editor(&mut state, EditorCommand::WordLeft);
    assert_eq!(state.prompt.cursor_cell(), Some((0, 4)));
    apply_editor(&mut state, EditorCommand::WordRight);
    assert_eq!(state.prompt.cursor_cell(), Some((0, 7)));
    apply_editor(&mut state, EditorCommand::DeleteToLineEnd);
    assert_eq!(state.prompt.text(), "foo bar");
}

#[test]
fn scroll_prompt_moves_viewport_only() {
    let mut state = AppState::demo();
    state.prompt.resize(10, 2);
    apply_editor(&mut state, EditorCommand::InsertText("1\n2\n3\n4".into()));
    assert_eq!(state.prompt.scroll(), 2);
    scroll_prompt(&mut state, -1);
    assert_eq!(state.prompt.scroll(), 0);
    assert_eq!(state.prompt.cursor_cell(), None);
    scroll_prompt(&mut state, 1);
    assert_eq!(state.prompt.scroll(), 2);
    assert!(state.prompt.cursor_cell().is_some());
}

#[test]
fn place_prompt_cursor_maps_viewport_cell() {
    let mut state = AppState::demo();
    state.prompt.resize(10, 3);
    apply_editor(&mut state, EditorCommand::InsertText("ab\ncd".into()));
    place_prompt_cursor(&mut state, 0, 1);
    assert_eq!(state.prompt.cursor_cell(), Some((0, 1)));
    place_prompt_cursor(&mut state, 1, 1);
    assert_eq!(state.prompt.cursor_cell(), Some((1, 1)));
}

#[test]
fn toggle_agents_flips_flag() {
    let mut state = AppState::demo();
    apply(Action::ToggleAgents, &mut state);
    assert!(state.agents_collapsed);
    apply(Action::ToggleAgents, &mut state);
    assert!(!state.agents_collapsed);
}

#[test]
fn resize_syncs_terminal_size() {
    let mut state = AppState::demo();
    let id = state.active_tab().layout.focus();
    resize_panes(&mut state, &[(id, Rect::new(0, 1, 12, 6))]);
    let pane = state.active_tab().pane(id);
    assert_eq!(
        pane.map(|pane| pane.terminal.size()),
        Some(GridSize { cols: 10, rows: 4 })
    );
}

#[test]
fn resize_syncs_prompt_editor_size() {
    let mut state = AppState::demo();
    let id = state.prompt.id();
    resize_panes(&mut state, &[(id, Rect::new(70, 0, 30, 20))]);
    assert_eq!(state.prompt.size(), (28, 18));
}

#[test]
fn feed_and_exit_mark_pane() {
    let mut state = AppState::demo();
    let id = state.active_tab().layout.focus();
    let responses = feed_pane(&mut state, id, b"\x1b[6n");
    assert_eq!(String::from_utf8_lossy(&responses), "\x1b[1;1R");
    mark_pane_exited(&mut state, id);
    assert_eq!(
        state.active_tab().pane(id).map(|pane| pane.exited),
        Some(true)
    );
}

#[test]
fn selection_finish_extracts_terminal_text() {
    let mut state = AppState::demo();
    let logs = state.workspaces[0].tabs[1].layout.pane_ids()[0];
    feed_pane(&mut state, logs, b"hello");
    begin_selection(&mut state, logs, 0, 0);
    drag_selection(&mut state, logs, 0, 4);
    assert_eq!(finish_selection(&mut state).as_deref(), Some("hello"));
    assert!(state.selection.is_some());
}

#[test]
fn selection_ignores_non_terminal_pane() {
    let mut state = AppState::demo();
    let focus = state.active_tab().layout.focus();
    if let Some(pane) = state.active_tab_mut().pane_mut(focus) {
        pane.kind = PaneKind::Placeholder;
    }
    begin_selection(&mut state, focus, 0, 0);
    drag_selection(&mut state, focus, 0, 3);
    assert_eq!(finish_selection(&mut state), None);
    assert!(state.selection.is_some());
}

#[test]
fn selection_extracts_prompt_text() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    let prompt = state.prompt.id();
    apply_editor(&mut state, EditorCommand::InsertText("Drag".into()));
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 3);
    assert_eq!(finish_selection(&mut state).as_deref(), Some("Drag"));
}

#[test]
fn empty_prompt_selection_yields_nothing() {
    let mut state = AppState::demo();
    let prompt = state.prompt.id();
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 3);
    assert_eq!(finish_selection(&mut state), None);
}

#[test]
fn selection_drag_ignores_other_pane() {
    let mut state = AppState::demo();
    let focus = state.active_tab().layout.focus();
    let logs = state.workspaces[0].tabs[1].layout.pane_ids()[0];
    begin_selection(&mut state, logs, 0, 0);
    drag_selection(&mut state, focus, 2, 2);
    assert_eq!(finish_selection(&mut state), None);
}

#[test]
fn begin_prompt_resize_clears_selection() {
    let mut state = AppState::demo();
    let focus = state.active_tab().layout.focus();
    begin_selection(&mut state, focus, 0, 0);
    begin_prompt_resize(&mut state);
    assert!(state.selection.is_none());
    assert!(state.resizing_prompt);
}

#[test]
fn begin_selection_clears_prompt_resize() {
    let mut state = AppState::demo();
    let focus = state.active_tab().layout.focus();
    begin_prompt_resize(&mut state);
    begin_selection(&mut state, focus, 0, 0);
    assert!(!state.resizing_prompt);
    assert!(state.selection.is_some());
}

#[test]
fn drag_prompt_only_applies_while_resizing() {
    let mut state = AppState::demo();
    let before = state.prompt_width;
    drag_prompt(&mut state, 40);
    assert_eq!(state.prompt_width, before);
    begin_prompt_resize(&mut state);
    drag_prompt(&mut state, 40);
    assert_eq!(state.prompt_width, 40);
    end_prompt_resize(&mut state);
    assert!(!state.resizing_prompt);
}

#[test]
fn set_prompt_hover_reports_changes() {
    let mut state = AppState::demo();
    assert!(set_prompt_hover(&mut state, true));
    assert!(!set_prompt_hover(&mut state, true));
    assert!(set_prompt_hover(&mut state, false));
    assert!(!state.prompt_hover);
}

#[test]
fn show_toast_replaces_previous_and_resets_deadline() {
    let mut state = AppState::demo();
    let now = Instant::now();
    show_toast(&mut state, Toast::new(ToastKind::Info, "first", None, now));
    show_toast(&mut state, Toast::new(ToastKind::Info, "second", None, now));
    let toast = state.toast.as_ref().expect("toast is shown");
    assert_eq!(toast.message, "second");
    assert_eq!(toast.next_deadline(), Some(now + TOAST_DURATION));
}

#[test]
fn tick_clears_expired_toast_only() {
    let mut state = AppState::demo();
    let now = Instant::now();
    show_toast(&mut state, Toast::new(ToastKind::Info, "hello", None, now));
    assert!(!tick(
        &mut state,
        now + TOAST_DURATION - Duration::from_millis(1)
    ));
    assert!(state.toast.is_some());
    assert!(tick(&mut state, now + TOAST_DURATION));
    assert!(state.toast.is_none());
    assert!(!tick(&mut state, now + TOAST_DURATION));
}

#[test]
fn toast_hover_suspends_expiry_until_pointer_leaves() {
    let mut state = AppState::demo();
    let now = Instant::now();
    show_toast(&mut state, Toast::new(ToastKind::Info, "hello", None, now));
    assert!(set_toast_hover(&mut state, true));
    assert!(!set_toast_hover(&mut state, true));
    assert!(!tick(&mut state, now + Duration::from_secs(60)));
    assert!(state.toast.is_some());
    assert_eq!(next_deadline(&state), None);
    assert!(set_toast_hover(&mut state, false));
    assert!(tick(&mut state, now + Duration::from_secs(60)));
    assert!(state.toast.is_none());
}

#[test]
fn hover_without_toast_is_noop() {
    let mut state = AppState::demo();
    assert!(!set_toast_hover(&mut state, true));
    assert_eq!(next_deadline(&state), None);
    dismiss_toast(&mut state);
    assert!(state.toast.is_none());
}

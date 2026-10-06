//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::state::workspace_name;
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
fn editor_commands_apply_undo_redo_indent_and_list_newline() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    apply_editor(&mut state, EditorCommand::InsertText("- a".into()));
    apply_editor(&mut state, EditorCommand::NewlineBelow);
    assert_eq!(state.prompt.text(), "- a\n");
    apply_editor(&mut state, EditorCommand::Undo);
    apply_editor(&mut state, EditorCommand::Newline);
    assert_eq!(state.prompt.text(), "- a\n- ");
    apply_editor(&mut state, EditorCommand::Indent);
    assert_eq!(state.prompt.text(), "- a\n  - ");
    apply_editor(&mut state, EditorCommand::Undo);
    assert_eq!(state.prompt.text(), "- a\n- ");
    apply_editor(&mut state, EditorCommand::Redo);
    assert_eq!(state.prompt.text(), "- a\n  - ");
    apply_editor(&mut state, EditorCommand::Outdent);
    assert_eq!(state.prompt.text(), "- a\n- ");
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
    // 30 列面板：28 列内容区，最右 1 列预留滚动条槽。
    resize_panes(&mut state, &[(id, Rect::new(70, 0, 30, 20))]);
    assert_eq!(state.prompt.size(), (27, 18));
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
fn typing_replaces_prompt_selection() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    apply_editor(&mut state, EditorCommand::InsertText("hello world".into()));
    let prompt = state.prompt.id();
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 4);
    assert_eq!(prompt_selection_text(&state).as_deref(), Some("hello"));
    apply_editor(&mut state, EditorCommand::InsertChar('X'));
    assert_eq!(state.prompt.text(), "X world");
    assert!(state.selection.is_none());
}

#[test]
fn delete_keys_remove_prompt_selection() {
    for command in [EditorCommand::Backspace, EditorCommand::Delete] {
        let mut state = AppState::demo();
        state.prompt.resize(20, 5);
        apply_editor(&mut state, EditorCommand::InsertText("hello world".into()));
        let prompt = state.prompt.id();
        begin_selection(&mut state, prompt, 0, 0);
        drag_selection(&mut state, prompt, 0, 4);
        apply_editor(&mut state, command.clone());
        assert_eq!(state.prompt.text(), " world", "{command:?}");
        assert!(state.selection.is_none(), "{command:?}");
    }
}

#[test]
fn navigation_clears_prompt_selection_without_editing() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    apply_editor(&mut state, EditorCommand::InsertText("hello".into()));
    let prompt = state.prompt.id();
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 4);
    apply_editor(&mut state, EditorCommand::Left);
    assert_eq!(state.prompt.text(), "hello");
    assert!(state.selection.is_none());
}

#[test]
fn end_selection_drag_keeps_only_nonempty_selection() {
    let mut state = AppState::demo();
    let prompt = state.prompt.id();
    begin_selection(&mut state, prompt, 0, 0);
    end_selection_drag(&mut state);
    assert!(state.selection.is_none());

    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 4);
    end_selection_drag(&mut state);
    let selection = state.selection.expect("selection kept");
    assert!(!selection.is_dragging());
    assert_eq!(selection.range(), Some(((0, 0), (0, 4))));
}

#[test]
fn collapsing_prompt_clears_selection() {
    let mut state = AppState::demo();
    let prompt = state.prompt.id();
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 4);
    apply(Action::TogglePrompt, &mut state);
    assert!(state.selection.is_none());
}

#[test]
fn prompt_resize_clears_selection() {
    let mut state = AppState::demo();
    let prompt = state.prompt.id();
    state.prompt.resize(20, 5);
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 4);
    resize_panes(&mut state, &[(prompt, Rect::new(0, 0, 30, 8))]);
    assert!(state.selection.is_none());
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

#[test]
fn losing_prompt_focus_clears_block_panel() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('#'));
    assert!(state.prompt.panel().is_some());
    let pane = state.active_tab().layout.focus();
    focus_pane(&mut state, pane);
    assert!(state.prompt.panel().is_none());
}

#[test]
fn collapsing_prompt_clears_block_panel() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('-'));
    assert!(state.prompt.panel().is_some());
    apply(Action::TogglePrompt, &mut state);
    assert!(state.prompt.panel().is_none());
}

#[test]
fn panel_keys_are_consumed_before_editing() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('-'));
    apply_editor(&mut state, EditorCommand::Down);
    assert_eq!(state.prompt.panel().map(|panel| panel.active()), Some(1));
    apply_editor(&mut state, EditorCommand::Newline);
    assert_eq!(state.prompt.text(), "- [ ] ");
    assert!(state.prompt.panel().is_none());
    apply_editor(&mut state, EditorCommand::Escape);
    assert_eq!(state.prompt.text(), "- [ ] ");
}

#[test]
fn create_workspace_activates_deduped_cwd_workspace() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(state.active_workspace, 1);
    let workspace = state.active_workspace();
    assert_eq!(workspace.name, format!("{} 2", workspace_name()));
    assert_eq!(workspace.tabs.len(), 1);
    assert_eq!(workspace.tabs[0].title, "shell");
    assert_eq!(workspace.tabs[0].layout.pane_ids().len(), 1);
    assert!(!workspace.name_is_manual);
    assert!(!workspace.is_initial);
    assert!(!state.prompt_focused);
}

#[test]
fn update_workspace_cwd_renames_auto_named_workspace() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    let cwd = std::path::Path::new("/tmp/other-project");
    assert!(update_workspace_cwd(&mut state, 1, cwd));
    assert_eq!(state.workspaces[1].name, "other-project");
    assert_eq!(state.workspaces[1].cwd.as_deref(), Some(cwd));
    assert!(!update_workspace_cwd(&mut state, 1, cwd));
}

#[test]
fn update_workspace_cwd_ignores_manually_named_workspace() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "api".to_string();
    state.workspaces[0].name_is_manual = true;
    assert!(!update_workspace_cwd(
        &mut state,
        0,
        std::path::Path::new("/tmp/other-project")
    ));
    assert_eq!(state.workspaces[0].name, "api");
}

#[test]
fn update_workspace_cwd_dedupes_against_other_workspaces() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    state.workspaces[1].name = "api".to_string();
    state.workspaces[1].name_is_manual = true;
    assert!(update_workspace_cwd(
        &mut state,
        0,
        std::path::Path::new("/tmp/api")
    ));
    assert_eq!(state.workspaces[0].name, "api 2");
}

#[test]
fn workspace_scroll_clamps_and_follows_active() {
    let mut state = AppState::demo();
    for _ in 0..15 {
        create_workspace(&mut state);
    }
    assert_eq!(state.workspaces.len(), 16);
    assert!(scroll_workspace_list(&mut state, 3, 10));
    assert_eq!(state.workspace_scroll, 3);
    assert!(ensure_workspace_visible(&mut state, 10));
    assert_eq!(state.workspace_scroll, 6);
    assert!(!ensure_workspace_visible(&mut state, 10));
    assert!(set_workspace_scroll(&mut state, 4, 10));
    assert!(set_workspace_scroll(&mut state, 99, 10));
    assert_eq!(state.workspace_scroll, 6);
    assert!(!scroll_workspace_list(&mut state, 1, 10));
    assert_eq!(workspace_scroll_max(&state, 10), 6);
    assert!(scroll_workspace_list(&mut state, -99, 10));
    assert_eq!(state.workspace_scroll, 0);
    assert!(!clamp_workspace_scroll(&mut state, 16));
    assert!(!clamp_workspace_scroll(&mut state, 1));
}

#[test]
fn switch_workspace_clears_selection_and_prompt_focus() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    let pane = state.active_tab().layout.focus();
    focus_prompt(&mut state);
    begin_selection(&mut state, pane, 0, 0);
    switch_workspace(&mut state, 0);
    assert_eq!(state.active_workspace, 0);
    assert!(!state.prompt_focused);
    assert!(state.selection.is_none());
    switch_workspace(&mut state, 5);
    assert_eq!(state.active_workspace, 0);
}

#[test]
fn open_workspace_menu_offers_rename_and_close() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    open_workspace_menu(&mut state, 0, (10, 5));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(menu.target, MenuTarget::Workspace(0));
    assert_eq!(
        menu.commands,
        vec![MenuCommand::RenameWorkspace, MenuCommand::CloseWorkspace]
    );
    assert_eq!(menu.selected, 0);
}

#[test]
fn open_workspace_menu_hides_close_for_last_workspace() {
    let mut state = AppState::demo();
    open_workspace_menu(&mut state, 0, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(menu.commands, vec![MenuCommand::RenameWorkspace]);
    open_workspace_menu(&mut state, 9, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(menu.target, MenuTarget::Workspace(0));
}

#[test]
fn menu_selection_wraps_and_reports_changes() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    open_workspace_menu(&mut state, 0, (0, 0));
    move_menu_selection(&mut state, -1);
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(menu.selected, 1);
    move_menu_selection(&mut state, 1);
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(menu.selected, 0);
    assert!(set_menu_selection(&mut state, 1));
    assert!(!set_menu_selection(&mut state, 1));
    assert!(!set_menu_selection(&mut state, 9));
}

#[test]
fn activate_menu_rename_opens_prefilled_input() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    let expected = state.workspaces[1].name.clone();
    open_workspace_menu(&mut state, 1, (0, 0));
    activate_menu(&mut state);
    let Some(Overlay::Rename(rename)) = state.overlay.as_ref() else {
        panic!("rename overlay expected");
    };
    assert_eq!(rename.target, 1);
    assert_eq!(rename.input.text(), expected);
    assert_eq!(rename.input.cursor(), expected.chars().count());
}

#[test]
fn activate_menu_close_opens_confirmation() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    open_workspace_menu(&mut state, 1, (0, 0));
    set_menu_selection(&mut state, 1);
    activate_menu(&mut state);
    assert_eq!(
        state.overlay.as_ref().map(Overlay::kind),
        Some(OverlayKind::ConfirmClose)
    );
    assert_eq!(state.workspaces.len(), 2);
}

#[test]
fn overlay_esc_closes_and_enter_dispatches() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    open_workspace_menu(&mut state, 0, (0, 0));
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert!(state.overlay.is_none());

    open_workspace_menu(&mut state, 0, (0, 0));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(
        state.overlay.as_ref().map(Overlay::kind),
        Some(OverlayKind::Rename)
    );
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert!(state.overlay.is_none());

    open_workspace_menu(&mut state, 0, (0, 0));
    set_menu_selection(&mut state, 1);
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(
        state.overlay.as_ref().map(Overlay::kind),
        Some(OverlayKind::ConfirmClose)
    );
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert!(state.overlay.is_none());
}

#[test]
fn rename_overlay_edits_and_commits_trimmed_name() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    open_workspace_menu(&mut state, 1, (0, 0));
    activate_menu(&mut state);
    for key in [
        OverlayKey::Backspace,
        OverlayKey::Backspace,
        OverlayKey::Char(' '),
        OverlayKey::Char('a'),
        OverlayKey::Char('p'),
        OverlayKey::Char('i'),
    ] {
        apply_overlay_key(&mut state, key);
    }
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(state.overlay.is_none());
    assert_eq!(
        state.workspaces[1].name,
        format!("{} api", workspace_name())
    );
}

#[test]
fn rename_overlay_rejects_empty_name() {
    let mut state = AppState::demo();
    let original = state.workspaces[0].name.clone();
    open_workspace_menu(&mut state, 0, (0, 0));
    activate_menu(&mut state);
    apply_overlay_key(&mut state, OverlayKey::Home);
    for _ in 0..200 {
        apply_overlay_key(&mut state, OverlayKey::Delete);
    }
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(
        state.overlay.as_ref().map(Overlay::kind),
        Some(OverlayKind::Rename)
    );
    assert_eq!(state.workspaces[0].name, original);
}

#[test]
fn confirm_close_removes_workspace_and_keeps_active_identity() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    create_workspace(&mut state);
    let active_name = state.active_workspace().name.clone();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose { target: 0 }));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(state.active_workspace().name, active_name);
    assert_eq!(state.active_workspace, 1);

    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose { target: 1 }));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.workspaces.len(), 1);
    assert_eq!(state.active_workspace, 0);
}

#[test]
fn confirm_close_refuses_last_workspace() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose { target: 0 }));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.workspaces.len(), 1);
    assert!(state.overlay.is_none());
}

#[test]
fn sidebar_resize_only_applies_while_resizing() {
    let mut state = AppState::demo();
    let before = state.sidebar_width;
    drag_sidebar(&mut state, 32);
    assert_eq!(state.sidebar_width, before);
    begin_sidebar_resize(&mut state);
    assert!(state.resizing_sidebar);
    drag_sidebar(&mut state, 32);
    assert_eq!(state.sidebar_width, 32);
    end_sidebar_resize(&mut state);
    assert!(!state.resizing_sidebar);
}

#[test]
fn begin_sidebar_resize_clears_selection() {
    let mut state = AppState::demo();
    let focus = state.active_tab().layout.focus();
    begin_selection(&mut state, focus, 0, 0);
    begin_sidebar_resize(&mut state);
    assert!(state.selection.is_none());
}

#[test]
fn set_sidebar_hover_reports_changes() {
    let mut state = AppState::demo();
    assert!(set_sidebar_hover(&mut state, true));
    assert!(!set_sidebar_hover(&mut state, true));
    assert!(set_sidebar_hover(&mut state, false));
    assert!(!state.sidebar_hover);
}

#[test]
fn drag_workspace_reorders_and_follows_active() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    create_workspace(&mut state);
    let names: Vec<String> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.clone())
        .collect();

    switch_workspace(&mut state, 0);
    begin_workspace_drag(&mut state, 0);
    assert!(drag_workspace_to(&mut state, 2));
    assert_eq!(state.workspace_drag, Some(2));
    assert_eq!(state.active_workspace, 2);
    let order: Vec<&str> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(
        order,
        vec![names[1].as_str(), names[2].as_str(), names[0].as_str()]
    );
    assert!(state.workspaces[2].is_initial);

    end_workspace_drag(&mut state);
    assert!(state.workspace_drag.is_none());
}

#[test]
fn drag_workspace_clamps_and_ignores_noops() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    switch_workspace(&mut state, 0);
    begin_workspace_drag(&mut state, 0);
    assert!(!drag_workspace_to(&mut state, 0));
    assert!(drag_workspace_to(&mut state, 99));
    assert_eq!(state.workspace_drag, Some(1));
    assert_eq!(state.active_workspace, 1);

    end_workspace_drag(&mut state);
    assert!(!drag_workspace_to(&mut state, 0));
}

#[test]
fn drag_workspace_shifts_other_active_workspace() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    create_workspace(&mut state);
    switch_workspace(&mut state, 1);
    begin_workspace_drag(&mut state, 0);
    assert!(drag_workspace_to(&mut state, 2));
    assert_eq!(state.active_workspace, 0);
}

#[test]
fn mention_panel_keys_are_consumed_before_editing() {
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    apply_mention_entries(
        &mut state,
        generation,
        vec![MentionEntry {
            path: "app.rs".into(),
            is_directory: false,
        }],
    );
    assert!(state.prompt.mention().is_some());
    apply_editor(&mut state, EditorCommand::Down);
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(0));
    apply_editor(&mut state, EditorCommand::Newline);
    assert_eq!(state.prompt.text(), "@app.rs ");
    assert!(state.prompt.mention().is_none());
}

#[test]
fn switching_workspace_root_invalidates_mention_cache() {
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, first_root) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    apply_mention_entries(
        &mut state,
        generation,
        vec![MentionEntry {
            path: "a.rs".into(),
            is_directory: false,
        }],
    );
    assert!(state.prompt.mention().is_some());

    state.workspaces[0].cwd = Some(std::path::PathBuf::from("/tmp/other"));
    apply_editor(&mut state, EditorCommand::InsertChar('a'));
    assert!(state.prompt.mention().is_none());
    let (_, next_root) = state
        .prompt
        .take_mention_scan_request()
        .expect("rescan requested");
    assert_ne!(next_root, first_root);
    assert_eq!(next_root, std::path::PathBuf::from("/tmp/other"));
}

#[test]
fn clear_panel_hides_mention_panel_on_focus_loss() {
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    apply_mention_entries(
        &mut state,
        generation,
        vec![MentionEntry {
            path: "a.rs".into(),
            is_directory: false,
        }],
    );
    assert!(state.prompt.mention().is_some());
    let pane = state.active_tab().layout.focus();
    focus_pane(&mut state, pane);
    assert!(state.prompt.mention().is_none());
}

#[test]
fn mouse_mention_hover_select_and_wheel_flow() {
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    apply_mention_entries(
        &mut state,
        generation,
        vec![
            MentionEntry {
                path: "a.rs".into(),
                is_directory: false,
            },
            MentionEntry {
                path: "b.rs".into(),
                is_directory: false,
            },
        ],
    );

    assert!(hover_mention(&mut state, 1));
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(1));
    assert!(!hover_mention(&mut state, 1));
    assert!(!scroll_mention(&mut state, 1));
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(1));
    assert!(scroll_mention(&mut state, -1));
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(0));
    assert!(!scroll_mention(&mut state, -1));
    apply_editor(&mut state, EditorCommand::Up);
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(1));
    apply_editor(&mut state, EditorCommand::Down);
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(0));
    select_mention(&mut state, 1);
    assert_eq!(state.prompt.text(), "@b.rs ");
    assert!(state.prompt.mention().is_none());
}

//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::overlay::WorktreeStatus;
use crate::app::state::{WorkspaceGit, workspace_name};
use crate::app::toast::{TOAST_DURATION, ToastKind};
use crate::terminal::GridSize;
use std::path::PathBuf;
use std::time::Duration;

/// demo 状态并把活动窗格置为终端视图：避免 lx 动画干扰定时类断言。
fn demo_terminal() -> AppState {
    let mut state = AppState::demo();
    let focus = state.active_tab().layout.focus();
    if let Some(pane) = state.active_tab_mut().pane_mut(focus) {
        pane.view = PaneView::Terminal;
    }
    state
}

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
    // 30 列面板：28 列内容区，顶部 2 行工具栏表头，最右 1 列预留滚动条槽。
    resize_panes(&mut state, &[(id, Rect::new(70, 0, 30, 20))]);
    assert_eq!(state.prompt.size(), (27, 16));
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
fn terminal_selection_finish_extracts_text() {
    let mut state = AppState::demo();
    create_tab(&mut state);
    let logs = state.workspaces[0].tabs[1].layout.pane_ids()[0];
    feed_pane(&mut state, logs, b"hello");
    begin_terminal_selection(&mut state, logs, 0, 0);
    drag_terminal_selection(&mut state, logs, 0, 4);
    assert_eq!(
        finish_terminal_selection(&mut state, logs).as_deref(),
        Some("hello")
    );
    assert!(state.terminal_selection.is_none());
}

#[test]
fn terminal_selection_finish_ignores_whitespace_only() {
    let mut state = AppState::demo();
    create_tab(&mut state);
    let logs = state.workspaces[0].tabs[1].layout.pane_ids()[0];
    // 空终端只有空格：拖一块空白选区不应产生可复制文本。
    begin_terminal_selection(&mut state, logs, 0, 0);
    drag_terminal_selection(&mut state, logs, 0, 4);
    assert_eq!(finish_terminal_selection(&mut state, logs), None);
    assert!(state.terminal_selection.is_none());
}

/// 填满终端回滚历史。
fn fill_scrollback(state: &mut AppState, pane: PaneId, lines: usize) {
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    for i in 0..lines {
        target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
    }
}

#[test]
fn arm_selection_autoscroll_beyond_edge_scrolls_immediately() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    fill_scrollback(&mut state, pane, 80);
    let inner = Rect::new(0, 5, 40, 10);
    begin_terminal_selection(&mut state, pane, 5, 0);
    drag_terminal_selection(&mut state, pane, 5, 2);

    // 鼠标在内容区上方 3 行：立即滚动 9 行并登记计划。
    arm_selection_autoscroll(&mut state, pane, inner, (2, inner.y - 3), Instant::now());
    let autoscroll = state.selection_autoscroll.expect("autoscroll armed");
    assert_eq!(autoscroll.direction, AutoscrollDirection::Up);
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 9);

    let before = target.terminal.display_offset();
    assert!(tick(&mut state, Instant::now() + Duration::from_millis(31)));
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), before + 1);
}

#[test]
fn edge_autoscroll_stops_at_scrollback_top() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    fill_scrollback(&mut state, pane, 20);
    let inner = Rect::new(0, 0, 40, 10);
    begin_terminal_selection(&mut state, pane, 5, 0);
    drag_terminal_selection(&mut state, pane, 5, 2);
    let now = Instant::now();
    arm_selection_autoscroll(&mut state, pane, inner, (2, 0), now);
    assert!(state.selection_autoscroll.is_some());

    let mut at = now;
    let mut guard = 0;
    while state.selection_autoscroll.is_some() && guard < 100 {
        at += Duration::from_millis(31);
        tick(&mut state, at);
        guard += 1;
    }
    assert!(guard < 100, "autoscroll must stop by itself");
    assert!(state.selection_autoscroll.is_none());
}

#[test]
fn prompt_selection_wheel_keeps_anchor_and_moves_cursor() {
    let mut state = AppState::demo();
    state.prompt.resize(10, 2);
    apply_editor(
        &mut state,
        EditorCommand::InsertText("1\n2\n3\n4\n5\n6".into()),
    );
    set_prompt_scroll(&mut state, 0);
    let prompt = state.prompt.id();
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 1, 0);

    assert!(wheel_prompt_selection(&mut state, 1, 1, 0));
    let scroll = state.prompt.scroll();
    assert_eq!(scroll, WHEEL_LINES as usize);
    let range = state
        .selection
        .and_then(|selection| selection.range())
        .expect("selection range");
    // 选区是内容行：锚点钉在第 1 行不动，终点跟到鼠标所在视口行（滚动量 + 1）。
    assert_eq!(range.0, (0, 0));
    assert_eq!(range.1.0, scroll as i32 + 1);
    assert_eq!(
        prompt_selection_text(&state).as_deref(),
        Some("1\n2\n3\n4\n5")
    );
}

#[test]
fn terminal_selection_ignores_non_terminal_pane() {
    let mut state = AppState::demo();
    let focus = state.active_tab().layout.focus();
    if let Some(pane) = state.active_tab_mut().pane_mut(focus) {
        pane.kind = PaneKind::Placeholder;
    }
    begin_terminal_selection(&mut state, focus, 0, 0);
    drag_terminal_selection(&mut state, focus, 0, 3);
    assert!(state.terminal_selection.is_none());
    assert_eq!(finish_terminal_selection(&mut state, focus), None);
}

#[test]
fn selection_extracts_prompt_text() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 5);
    let prompt = state.prompt.id();
    apply_editor(&mut state, EditorCommand::InsertText("Drag".into()));
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 3);
    assert_eq!(prompt_selection_text(&state).as_deref(), Some("Drag"));
}

#[test]
fn empty_prompt_selection_yields_nothing() {
    let mut state = AppState::demo();
    let prompt = state.prompt.id();
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 0, 3);
    assert_eq!(prompt_selection_text(&state), None);
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
fn scrolling_keeps_prompt_selection_anchor_in_content() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 3);
    let prompt = state.prompt.id();
    apply_editor(
        &mut state,
        EditorCommand::InsertText("a\nb\nc\nd\ne".into()),
    );
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 2, 0);
    assert_eq!(prompt_selection_text(&state).as_deref(), Some("c\nd\ne"));

    scroll_prompt(&mut state, -1);
    assert_eq!(state.prompt.scroll(), 0);
    assert_eq!(prompt_selection_text(&state).as_deref(), Some("c\nd\ne"));
}

#[test]
fn dragging_to_prompt_edge_autoscrolls_and_extends_selection() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 4);
    let prompt = state.prompt.id();
    apply_editor(
        &mut state,
        EditorCommand::InsertText("l0\nl1\nl2\nl3\nl4\nl5\nl6\nl7".into()),
    );
    set_prompt_scroll(&mut state, 1);
    assert_eq!(state.prompt.scroll(), 1);

    let inner = Rect::new(0, 0, 20, 4);
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, prompt, 3, 1);
    arm_selection_autoscroll(&mut state, prompt, inner, (1, 3), Instant::now());
    let autoscroll = state.selection_autoscroll.expect("edge scroll armed");
    assert_eq!(autoscroll.direction, AutoscrollDirection::Down);
    assert_eq!(
        prompt_selection_text(&state).as_deref(),
        Some("l1\nl2\nl3\nl4")
    );

    let deadline = next_deadline(&state).expect("deadline");
    assert!(tick(&mut state, deadline));
    assert_eq!(state.prompt.scroll(), 2);
    assert_eq!(
        prompt_selection_text(&state).as_deref(),
        Some("l1\nl2\nl3\nl4\nl5")
    );

    // 滚到边界后停止并解除自动滚动。
    state.prompt.scroll_to(4);
    let deadline = next_deadline(&state).expect("deadline");
    assert!(!tick(&mut state, deadline));
    assert!(state.selection_autoscroll.is_none());
}

#[test]
fn autoscroll_arms_only_near_prompt_edges() {
    let mut state = AppState::demo();
    state.prompt.resize(20, 10);
    let prompt = state.prompt.id();
    apply_editor(
        &mut state,
        EditorCommand::InsertText(
            "l0\nl1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nla\nlb\nlc\nld\nle".into(),
        ),
    );
    let inner = Rect::new(0, 0, 20, 10);
    let now = Instant::now();
    begin_selection(&mut state, prompt, 2, 0);
    drag_selection(&mut state, prompt, 5, 0);
    arm_selection_autoscroll(&mut state, prompt, inner, (0, 5), now);
    assert!(state.selection_autoscroll.is_none(), "中部拖动不自动滚动");

    drag_selection(&mut state, prompt, 9, 0);
    arm_selection_autoscroll(&mut state, prompt, inner, (0, 9), now);
    assert!(state.selection_autoscroll.is_some(), "底边拖动自动滚动");
    drag_selection(&mut state, prompt, 5, 0);
    arm_selection_autoscroll(&mut state, prompt, inner, (0, 5), now);
    assert!(state.selection_autoscroll.is_none(), "离开边缘停止");

    // 终端选区同样支持边缘自动滚动（对齐 herdr）。
    let terminal = state.active_tab().layout.focus();
    begin_terminal_selection(&mut state, terminal, 0, 0);
    drag_terminal_selection(&mut state, terminal, 0, 0);
    arm_selection_autoscroll(&mut state, terminal, inner, (0, 9), now);
    assert!(state.selection_autoscroll.is_some(), "终端底边拖动自动滚动");

    clear_selection(&mut state);
    assert!(state.selection_autoscroll.is_none(), "清除选区停止自动滚动");
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
    create_tab(&mut state);
    let focus = state.workspaces[0].tabs[0].layout.focus();
    let prompt = state.prompt.id();
    begin_selection(&mut state, prompt, 0, 0);
    drag_selection(&mut state, focus, 2, 2);
    assert_eq!(prompt_selection_text(&state), None);
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
    let mut state = demo_terminal();
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
    let mut state = demo_terminal();
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
    let mut state = demo_terminal();
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
    assert_eq!(workspace.tabs[0].name, None);
    assert_eq!(workspace.tabs[0].layout.pane_ids().len(), 1);
    assert!(!workspace.name_is_manual);
    assert!(!workspace.is_initial);
    assert!(!state.prompt_focused);
}

#[test]
fn create_workspace_uses_focused_terminal_cwd() {
    let mut state = AppState::demo();
    let id = state.active_tab().layout.focus();
    let cwd = std::path::Path::new("/tmp/focused");
    assert!(update_pane_cwd(&mut state, id, cwd, "focused".to_string()));
    create_workspace(&mut state);
    let workspace = state.active_workspace();
    assert_eq!(workspace.cwd.as_deref(), Some(cwd));
    assert_eq!(workspace.name, "focused");
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

/// 在当前菜单浮层中高亮指定命令项。
fn select_menu_command(state: &mut AppState, command: MenuCommand) {
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    let index = menu
        .commands
        .iter()
        .position(|candidate| *candidate == command)
        .expect("command offered by menu");
    set_menu_selection(state, index);
}

#[test]
fn open_workspace_menu_offers_terminal_prompt_rename_and_close() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    open_workspace_menu(&mut state, 0, (10, 5));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(menu.target, OverlayTarget::Workspace(0));
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::NewTerminal,
            MenuCommand::OpenPrompt,
            MenuCommand::RenameWorkspace,
            MenuCommand::CloseWorkspace
        ]
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
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::NewTerminal,
            MenuCommand::OpenPrompt,
            MenuCommand::RenameWorkspace
        ]
    );
    open_workspace_menu(&mut state, 9, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(menu.target, OverlayTarget::Workspace(0));
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
    assert_eq!(menu.selected, 3);
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
    select_menu_command(&mut state, MenuCommand::RenameWorkspace);
    activate_menu(&mut state);
    let Some(Overlay::Rename(rename)) = state.overlay.as_ref() else {
        panic!("rename overlay expected");
    };
    assert_eq!(rename.target, RenameTarget::Workspace(1));
    assert_eq!(rename.input.text(), expected);
    assert_eq!(rename.input.cursor(), expected.chars().count());
}

#[test]
fn activate_menu_close_opens_confirmation() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    open_workspace_menu(&mut state, 1, (0, 0));
    select_menu_command(&mut state, MenuCommand::CloseWorkspace);
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
    select_menu_command(&mut state, MenuCommand::RenameWorkspace);
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(
        state.overlay.as_ref().map(Overlay::kind),
        Some(OverlayKind::Rename)
    );
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert!(state.overlay.is_none());

    open_workspace_menu(&mut state, 0, (0, 0));
    select_menu_command(&mut state, MenuCommand::CloseWorkspace);
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
    select_menu_command(&mut state, MenuCommand::RenameWorkspace);
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
    select_menu_command(&mut state, MenuCommand::RenameWorkspace);
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
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Workspace(0),
    }));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(state.active_workspace().name, active_name);
    assert_eq!(state.active_workspace, 1);

    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Workspace(1),
    }));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.workspaces.len(), 1);
    assert_eq!(state.active_workspace, 0);
}

#[test]
fn confirm_close_refuses_last_workspace() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Workspace(0),
    }));
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
fn drag_workspace_moves_collapsed_group_as_block() {
    let mut state = AppState::demo();
    let demo = state.workspaces[0].name.clone();
    push_git_workspace(
        &mut state,
        "repo",
        "/repo",
        git_info("/repo", "/repo", false, Some("main")),
    );
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.wt/feat",
        git_info("/repo", "/repo/.wt/feat", true, Some("feature/x")),
    );
    state
        .workspaces
        .push(Workspace::single_terminal("tmp".to_string(), None));
    state.collapsed_groups.push(PathBuf::from("/repo"));

    begin_workspace_drag(&mut state, 1);
    assert!(drag_workspace_to(&mut state, 3));
    assert_eq!(state.workspace_drag, Some(2));
    let order: Vec<&str> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(order, vec![demo.as_str(), "tmp", "repo", "feat"]);
}

#[test]
fn drag_workspace_child_moves_whole_group() {
    let mut state = AppState::demo();
    let demo = state.workspaces[0].name.clone();
    push_git_workspace(
        &mut state,
        "repo",
        "/repo",
        git_info("/repo", "/repo", false, Some("main")),
    );
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.wt/feat",
        git_info("/repo", "/repo/.wt/feat", true, Some("feature/x")),
    );
    state
        .workspaces
        .push(Workspace::single_terminal("tmp".to_string(), None));

    begin_workspace_drag(&mut state, 2);
    assert!(drag_workspace_to(&mut state, 0));
    let order: Vec<&str> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(order, vec!["repo", "feat", demo.as_str(), "tmp"]);
    assert_eq!(state.workspace_drag, Some(1));
}

#[test]
fn drag_workspace_keeps_duplicate_main_independent() {
    let mut state = AppState::demo();
    let demo = state.workspaces[0].name.clone();
    push_git_workspace(
        &mut state,
        "repo",
        "/repo",
        git_info("/repo", "/repo", false, Some("main")),
    );
    push_git_workspace(
        &mut state,
        "repo 2",
        "/repo",
        git_info("/repo", "/repo", false, Some("main")),
    );
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.wt/feat",
        git_info("/repo", "/repo/.wt/feat", true, Some("feature/x")),
    );

    // 拖重复主 checkout 到顶部：只移动它自己，树不动。
    begin_workspace_drag(&mut state, 2);
    assert!(drag_workspace_to(&mut state, 0));
    let order: Vec<&str> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(order, vec!["repo 2", demo.as_str(), "repo", "feat"]);
    let rows = state.workspace_rows();
    assert!(rows[2].parent, "树根身份按创建序稳定");
    assert!(rows[3].child);

    // 拖树（父项）越过重复项：整树移动，重复项相对位置不变。
    begin_workspace_drag(&mut state, 2);
    assert!(drag_workspace_to(&mut state, 0));
    let order: Vec<&str> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(order, vec!["repo", "feat", "repo 2", demo.as_str()]);
    let rows = state.workspace_rows();
    assert!(rows[0].parent);
    assert!(rows[1].child);
    assert!(!rows[2].parent && !rows[2].child, "重复项保持独立顶层行");
}

#[test]
fn workspace_drag_highlight_waits_for_pointer_move() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    begin_workspace_drag(&mut state, 0);
    assert!(!state.workspace_dragging, "按下未移动不整块反显");
    assert!(!drag_workspace_to(&mut state, 0));
    assert!(state.workspace_dragging, "指针移动后进入拖动高亮");
    end_workspace_drag(&mut state);
    assert!(state.workspace_drag.is_none() && !state.workspace_dragging);
}

#[test]
fn drag_workspace_snaps_to_other_group_boundary() {
    let mut state = AppState::demo();
    let demo = state.workspaces[0].name.clone();
    push_git_workspace(
        &mut state,
        "a",
        "/a",
        git_info("/a", "/a", false, Some("main")),
    );
    push_git_workspace(
        &mut state,
        "a-feat",
        "/a/.wt/feat",
        git_info("/a", "/a/.wt/feat", true, Some("feature/x")),
    );
    push_git_workspace(
        &mut state,
        "b",
        "/b",
        git_info("/b", "/b", false, Some("main")),
    );
    push_git_workspace(
        &mut state,
        "b-feat",
        "/b/.wt/feat",
        git_info("/b", "/b/.wt/feat", true, Some("feature/x")),
    );

    begin_workspace_drag(&mut state, 1);
    assert!(drag_workspace_to(&mut state, 4));
    let order: Vec<&str> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(order, vec![demo.as_str(), "b", "b-feat", "a", "a-feat"]);
}

#[test]
fn create_tab_appends_and_activates_auto_named_tab() {
    let mut state = AppState::demo();
    create_tab(&mut state);
    assert_eq!(state.active_workspace().tabs.len(), 2);
    assert_eq!(state.active_workspace().active_tab, 1);
    assert_eq!(state.active_tab().name, None);
    assert_eq!(tab_label(1, state.active_tab().name.as_deref()), "tab 2");
    assert!(!state.prompt_focused);
    assert!(state.selection.is_none());
}

#[test]
fn switch_tab_switches_and_ignores_out_of_range() {
    let mut state = AppState::demo();
    create_tab(&mut state);
    focus_prompt(&mut state);
    switch_tab(&mut state, 0);
    assert_eq!(state.active_workspace().active_tab, 0);
    assert!(!state.prompt_focused);
    switch_tab(&mut state, 9);
    assert_eq!(state.active_workspace().active_tab, 0);
}

#[test]
fn open_tab_menu_offers_new_rename_and_close() {
    let mut state = AppState::demo();
    create_tab(&mut state);
    open_tab_menu(&mut state, 0, (10, 5));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(
        menu.target,
        OverlayTarget::Tab {
            workspace: 0,
            tab: 0
        }
    );
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::NewTab,
            MenuCommand::RenameTab,
            MenuCommand::CloseTab
        ]
    );
    assert_eq!(menu.selected, 0);
    open_tab_menu(&mut state, 9, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(
        menu.target,
        OverlayTarget::Tab {
            workspace: 0,
            tab: 0
        }
    );
}

#[test]
fn open_tab_menu_hides_close_for_last_tab() {
    let mut state = AppState::demo();
    open_tab_menu(&mut state, 0, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(
        menu.commands,
        vec![MenuCommand::NewTab, MenuCommand::RenameTab]
    );
}

#[test]
fn activate_menu_new_tab_creates_immediately() {
    let mut state = AppState::demo();
    open_tab_menu(&mut state, 0, (0, 0));
    activate_menu(&mut state);
    assert!(state.overlay.is_none());
    assert_eq!(state.active_workspace().tabs.len(), 2);
    assert_eq!(state.active_workspace().active_tab, 1);
}

#[test]
fn activate_menu_rename_tab_prefills_auto_title() {
    let mut state = AppState::demo();
    create_tab(&mut state);
    open_tab_menu(&mut state, 1, (0, 0));
    set_menu_selection(&mut state, 1);
    activate_menu(&mut state);
    let Some(Overlay::Rename(rename)) = state.overlay.as_ref() else {
        panic!("rename overlay expected");
    };
    assert_eq!(
        rename.target,
        RenameTarget::Tab {
            workspace: 0,
            tab: 1
        }
    );
    assert_eq!(rename.input.text(), "tab 2");
}

#[test]
fn rename_tab_overlay_commits_custom_name() {
    let mut state = AppState::demo();
    create_tab(&mut state);
    open_tab_menu(&mut state, 1, (0, 0));
    set_menu_selection(&mut state, 1);
    activate_menu(&mut state);
    apply_overlay_key(&mut state, OverlayKey::Clear);
    for ch in "dev".chars() {
        apply_overlay_key(&mut state, OverlayKey::Char(ch));
    }
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(state.overlay.is_none());
    assert_eq!(state.workspaces[0].tabs[1].name.as_deref(), Some("dev"));
    assert_eq!(
        tab_label(1, state.workspaces[0].tabs[1].name.as_deref()),
        "dev"
    );
}

#[test]
fn confirm_close_tab_removes_and_keeps_active_identity() {
    let mut state = AppState::demo();
    create_tab(&mut state);
    create_tab(&mut state);
    assert_eq!(state.active_workspace().active_tab, 2);
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Tab {
            workspace: 0,
            tab: 0,
        },
    }));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.active_workspace().tabs.len(), 2);
    assert_eq!(state.active_workspace().active_tab, 1);

    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Tab {
            workspace: 0,
            tab: 1,
        },
    }));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.active_workspace().tabs.len(), 1);
    assert_eq!(state.active_workspace().active_tab, 0);
}

#[test]
fn confirm_close_refuses_last_tab() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Tab {
            workspace: 0,
            tab: 0,
        },
    }));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.active_workspace().tabs.len(), 1);
    assert!(state.overlay.is_none());
}

/// demo 状态并把活动标签拆成左右两个窗格；焦点在新窗格。
fn demo_two_panes() -> (AppState, PaneId, PaneId) {
    let mut state = AppState::demo();
    let first = state.active_tab().layout.focus();
    let second = state
        .active_tab_mut()
        .split_pane(first, Direction::Horizontal)
        .expect("split succeeds");
    (state, first, second)
}

#[test]
fn open_pane_menu_offers_split_switch_prompt_path_and_close() {
    let (mut state, _, pane) = demo_two_panes();
    state.workspaces[0].cwd = Some(PathBuf::from("/tmp/lx-tui"));
    open_pane_menu(&mut state, pane, (7, 3));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("pane menu overlay expected");
    };
    assert_eq!(menu.anchor, (7, 3));
    assert_eq!(
        menu.target,
        OverlayTarget::Pane {
            workspace: 0,
            tab: 0,
            pane
        }
    );
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::SplitRight,
            MenuCommand::SplitDown,
            MenuCommand::SwitchToTerminal,
            MenuCommand::OpenPrompt,
            MenuCommand::SwitchToWorkspaceCwd,
            MenuCommand::ClosePane
        ]
    );

    state.overlay = None;
    open_pane_menu(&mut state, PaneId::alloc(), (0, 0));
    assert!(state.overlay.is_none());
}

#[test]
fn open_pane_menu_hides_close_and_path_for_last_pane_without_cwd() {
    let mut state = AppState::demo();
    state.workspaces[0].cwd = None;
    let only = state.active_tab().layout.focus();
    open_pane_menu(&mut state, only, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("pane menu overlay expected");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::SplitRight,
            MenuCommand::SplitDown,
            MenuCommand::SwitchToTerminal,
            MenuCommand::OpenPrompt
        ]
    );
}

#[test]
fn open_pane_menu_switch_command_follows_target_view() {
    let (mut state, _, pane) = demo_two_panes();
    state.pane_mut_anywhere(pane).expect("pane exists").view = PaneView::Terminal;
    open_pane_menu(&mut state, pane, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("pane menu overlay expected");
    };
    assert!(menu.commands.contains(&MenuCommand::SwitchToLx));
}

#[test]
fn activate_menu_split_right_creates_pane_inheriting_view() {
    let (mut state, first, _) = demo_two_panes();
    state.pane_mut_anywhere(first).expect("pane exists").view = PaneView::Terminal;
    open_pane_menu(&mut state, first, (0, 0));
    activate_menu(&mut state);

    let tab = state.active_tab();
    assert_eq!(tab.layout.pane_ids().len(), 3);
    let focus = tab.layout.focus();
    assert_ne!(focus, first);
    assert_eq!(tab.pane(focus).expect("new pane").view, PaneView::Terminal);
    assert_eq!(tab.pane(first).expect("source").view, PaneView::Terminal);
}

#[test]
fn activate_menu_split_down_places_pane_below() {
    let mut state = AppState::demo();
    let source = state.active_tab().layout.focus();
    open_pane_menu(&mut state, source, (0, 0));
    set_menu_selection(&mut state, 1);
    activate_menu(&mut state);

    let tab = state.active_tab();
    assert_eq!(tab.layout.pane_ids().len(), 2);
    let new_id = tab.layout.focus();
    let rects = crate::layout::pane_rects(&tab.layout, Rect::new(0, 0, 80, 20), 10, 3);
    let rect_of = |id: PaneId| rects.iter().find(|(pane, _)| *pane == id).map(|(_, r)| *r);
    assert_eq!(rect_of(source).map(|rect| rect.y), Some(0));
    assert_eq!(rect_of(new_id).map(|rect| rect.y), Some(10));
}

#[test]
fn activate_menu_switch_toggles_only_target_pane() {
    let (mut state, first, second) = demo_two_panes();
    focus_pane(&mut state, first);
    open_pane_menu(&mut state, second, (0, 0));
    set_menu_selection(&mut state, 2);
    activate_menu(&mut state);

    assert_eq!(
        state.active_tab().pane(second).expect("pane exists").view,
        PaneView::Terminal
    );
    assert_eq!(
        state.active_tab().pane(first).expect("pane exists").view,
        PaneView::Lx
    );
    assert_eq!(state.active_tab().layout.focus(), first);
}

#[test]
fn activate_menu_close_pane_opens_confirmation() {
    let (mut state, _, second) = demo_two_panes();
    open_pane_menu(&mut state, second, (0, 0));
    select_menu_command(&mut state, MenuCommand::ClosePane);
    activate_menu(&mut state);

    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm close overlay expected");
    };
    assert_eq!(
        confirm.target,
        OverlayTarget::Pane {
            workspace: 0,
            tab: 0,
            pane: second
        }
    );
    assert_eq!(state.active_tab().layout.pane_ids().len(), 2);
}

#[test]
fn confirm_close_pane_removes_and_focuses_sibling() {
    let (mut state, first, second) = demo_two_panes();
    assert_eq!(state.active_tab().layout.focus(), second);
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Pane {
            workspace: 0,
            tab: 0,
            pane: second,
        },
    }));
    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    assert_eq!(state.active_tab().layout.pane_ids(), vec![first]);
    assert_eq!(state.active_tab().layout.focus(), first);
    assert!(state.active_tab().pane(second).is_none());
}

#[test]
fn confirm_close_pane_refuses_last_pane() {
    let mut state = AppState::demo();
    let only = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Pane {
            workspace: 0,
            tab: 0,
            pane: only,
        },
    }));
    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    assert_eq!(state.active_tab().layout.pane_ids(), vec![only]);
}

#[test]
fn confirm_close_pane_clears_transient_state_for_removed_pane() {
    let (mut state, _, second) = demo_two_panes();
    state.terminal_selection = Some(second);
    state.terminal_scroll_drag = Some((second, 3));
    state.selection_autoscroll = Some(SelectionAutoscroll {
        pane: second,
        direction: AutoscrollDirection::Down,
        mouse: (1, 1),
        inner: Rect::new(0, 0, 10, 5),
        next_at: Instant::now(),
    });
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Pane {
            workspace: 0,
            tab: 0,
            pane: second,
        },
    }));
    apply_overlay_key(&mut state, OverlayKey::Enter);

    assert!(state.terminal_selection.is_none());
    assert!(state.terminal_scroll_drag.is_none());
    assert!(state.selection_autoscroll.is_none());
}

#[test]
fn create_terminal_in_workspace_appends_terminal_tab_and_activates() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    let target_cwd = state.workspaces[1].cwd.clone();
    state.active_workspace = 0;
    state.prompt_focused = true;

    create_terminal_in_workspace(&mut state, 1);

    assert_eq!(state.active_workspace, 1);
    let workspace = state.active_workspace();
    assert_eq!(workspace.tabs.len(), 2);
    assert_eq!(workspace.active_tab, 1);
    let pane = workspace.tabs[1].root_pane().expect("root pane exists");
    assert_eq!(
        workspace.tabs[1].pane(pane).expect("pane exists").view,
        PaneView::Terminal
    );
    assert_eq!(state.workspace_cwd_for_pane(pane), target_cwd);
    assert!(!state.prompt_focused);

    create_terminal_in_workspace(&mut state, 9);
    assert_eq!(state.active_workspace().tabs.len(), 2);
}

#[test]
fn activate_menu_new_terminal_targets_clicked_workspace() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    create_workspace(&mut state);
    state.active_workspace = 0;
    open_workspace_menu(&mut state, 1, (0, 0));
    activate_menu(&mut state);

    assert_eq!(state.active_workspace, 1);
    assert_eq!(state.workspaces[1].tabs.len(), 2);
    assert_eq!(state.workspaces[2].tabs.len(), 1);
}

#[test]
fn open_prompt_for_workspace_switches_binds_and_focuses() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    state.workspaces[1].cwd = Some(PathBuf::from("/tmp/target"));
    state.active_workspace = 0;
    state.prompt_collapsed = true;

    open_prompt_for_workspace(&mut state, 1);

    assert_eq!(state.active_workspace, 1);
    assert!(state.prompt_focused);
    assert!(!state.prompt_collapsed);
    assert_eq!(
        state.prompt_root.as_deref(),
        Some(std::path::Path::new("/tmp/target"))
    );
}

#[test]
fn activate_menu_open_prompt_from_workspace_menu_binds_workspace_cwd() {
    let mut state = AppState::demo();
    create_workspace(&mut state);
    state.workspaces[1].cwd = Some(PathBuf::from("/tmp/ws-target"));
    state.active_workspace = 0;
    open_workspace_menu(&mut state, 1, (0, 0));
    select_menu_command(&mut state, MenuCommand::OpenPrompt);
    activate_menu(&mut state);

    assert_eq!(state.active_workspace, 1);
    assert!(state.prompt_focused);
    assert_eq!(
        state.prompt_root.as_deref(),
        Some(std::path::Path::new("/tmp/ws-target"))
    );
}

#[test]
fn open_prompt_for_pane_binds_pane_cwd_and_keeps_workspace() {
    let (mut state, _, pane) = demo_two_panes();
    state.prompt_collapsed = true;
    update_pane_cwd(
        &mut state,
        pane,
        std::path::Path::new("/tmp/pane-dir"),
        "pane-dir".to_string(),
    );

    open_prompt_for_pane(&mut state, pane);

    assert_eq!(state.active_workspace, 0);
    assert!(state.prompt_focused);
    assert!(!state.prompt_collapsed);
    assert_eq!(
        state.prompt_root.as_deref(),
        Some(std::path::Path::new("/tmp/pane-dir"))
    );
}

#[test]
fn open_prompt_for_pane_falls_back_to_workspace_cwd() {
    let (mut state, _, pane) = demo_two_panes();
    state.workspaces[0].cwd = Some(PathBuf::from("/tmp/ws-root"));

    open_prompt_for_pane(&mut state, pane);

    assert_eq!(
        state.prompt_root.as_deref(),
        Some(std::path::Path::new("/tmp/ws-root"))
    );
}

#[test]
fn activate_menu_open_prompt_from_pane_menu_binds_pane_cwd() {
    let (mut state, _, pane) = demo_two_panes();
    update_pane_cwd(
        &mut state,
        pane,
        std::path::Path::new("/tmp/pane-dir"),
        "pane-dir".to_string(),
    );
    open_pane_menu(&mut state, pane, (0, 0));
    select_menu_command(&mut state, MenuCommand::OpenPrompt);
    activate_menu(&mut state);

    assert!(state.prompt_focused);
    assert_eq!(
        state.prompt_root.as_deref(),
        Some(std::path::Path::new("/tmp/pane-dir"))
    );
}

#[test]
fn switch_to_ws_path_opens_confirm_and_returns_cd_command() {
    let (mut state, _, pane) = demo_two_panes();
    state.workspaces[0].cwd = Some(PathBuf::from("/tmp/ws path"));
    open_pane_menu(&mut state, pane, (0, 0));
    select_menu_command(&mut state, MenuCommand::SwitchToWorkspaceCwd);
    activate_menu(&mut state);

    let Some(Overlay::ConfirmSwitchCwd(confirm)) = state.overlay.as_ref() else {
        panic!("confirm switch cwd overlay expected");
    };
    assert_eq!(confirm.workspace, 0);
    assert_eq!(confirm.tab, 0);
    assert_eq!(confirm.pane, pane);
    assert_eq!(confirm.path, PathBuf::from("/tmp/ws path"));

    let action = apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(action, Some((pane, b"cd \"/tmp/ws path\"\n".to_vec())));
    assert!(state.overlay.is_none());
}

#[test]
fn switch_to_ws_path_cancel_returns_no_command() {
    let (mut state, _, pane) = demo_two_panes();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: PathBuf::from("/tmp/ws"),
    }));

    assert_eq!(apply_overlay_key(&mut state, OverlayKey::Esc), None);
    assert!(state.overlay.is_none());
}

#[test]
fn switch_cwd_command_escapes_shell_specials() {
    let (mut state, _, pane) = demo_two_panes();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: PathBuf::from("/tmp/a\"b$c\\d`e"),
    }));

    let Some((target, bytes)) = apply_overlay_key(&mut state, OverlayKey::Enter) else {
        panic!("cd command expected");
    };
    assert_eq!(target, pane);
    let command = String::from_utf8(bytes).expect("command is utf-8");
    assert_eq!(command, "cd \"/tmp/a\\\"b\\$c\\\\d\\`e\"\n");
}

#[test]
fn switch_cwd_confirm_with_missing_pane_returns_none() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 9,
        pane,
        path: PathBuf::from("/tmp/ws"),
    }));

    assert_eq!(apply_overlay_key(&mut state, OverlayKey::Enter), None);
    assert!(state.overlay.is_none());
}

#[test]
fn tab_scroll_clamps_and_reports_changes() {
    let mut state = AppState::demo();
    assert!(!set_tab_scroll(&mut state, 0, 5));
    assert!(set_tab_scroll(&mut state, 3, 5));
    assert!(scroll_tab_bar(&mut state, 1, 5));
    assert_eq!(state.tab_scroll, 4);
    assert!(scroll_tab_bar(&mut state, 99, 5));
    assert_eq!(state.tab_scroll, 5);
    assert!(!scroll_tab_bar(&mut state, 1, 5));
    assert!(scroll_tab_bar(&mut state, -99, 5));
    assert_eq!(state.tab_scroll, 0);
    assert!(set_tab_scroll(&mut state, 99, 2));
    assert_eq!(state.tab_scroll, 2);
}

#[test]
fn update_pane_cwd_reports_changes() {
    let mut state = AppState::demo();
    let id = state.active_tab().layout.focus();
    let cwd = std::path::Path::new("/tmp/lx-tui");
    assert!(update_pane_cwd(&mut state, id, cwd, "lx-tui".to_string()));
    assert!(!update_pane_cwd(&mut state, id, cwd, "lx-tui".to_string()));
    assert!(update_pane_cwd(
        &mut state,
        id,
        std::path::Path::new("/tmp/herdr"),
        "herdr".to_string()
    ));
    assert!(state.active_tab().pane(id).is_some_and(|pane| {
        pane.cwd_label.as_deref() == Some("herdr")
            && pane.cwd.as_deref() == Some(std::path::Path::new("/tmp/herdr"))
    }));
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

/// 构造 `@` 已触发且候选已写入的提及场景。
fn mention_state(entries: Vec<MentionEntry>) -> AppState {
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('@'));
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    apply_mention_entries(&mut state, generation, entries);
    state
}

#[test]
fn enter_folder_key_scopes_panel_and_file_highlight_is_swallowed() {
    let mut state = mention_state(vec![
        MentionEntry {
            path: "src".into(),
            is_directory: true,
        },
        MentionEntry {
            path: "src/app.rs".into(),
            is_directory: false,
        },
    ]);
    apply_editor(&mut state, EditorCommand::EnterFolder);
    assert_eq!(state.prompt.text(), "@src/");
    let panel = state.prompt.mention().expect("scoped panel");
    assert_eq!(panel.scope_name(), Some("src"));
    assert_eq!(panel.items().len(), 1);

    // 高亮文件时 Shift+Enter 被吞掉：不插入换行、不改文本。
    apply_editor(&mut state, EditorCommand::EnterFolder);
    assert_eq!(state.prompt.text(), "@src/");
}

#[test]
fn enter_folder_falls_back_to_newline_when_panel_closed() {
    let mut state = mention_state(vec![MentionEntry {
        path: "src".into(),
        is_directory: true,
    }]);
    apply_editor(&mut state, EditorCommand::Escape);
    assert!(state.prompt.mention().is_none());
    assert!(state.prompt.text().starts_with('@'));
    apply_editor(&mut state, EditorCommand::EnterFolder);
    assert_eq!(state.prompt.text(), "@\n", "面板关闭时回落行尾另起一行");
}

#[test]
fn undo_key_steps_back_folder_levels() {
    let mut state = mention_state(vec![
        MentionEntry {
            path: "src".into(),
            is_directory: true,
        },
        MentionEntry {
            path: "src/ui".into(),
            is_directory: true,
        },
        MentionEntry {
            path: "src/ui/mod.rs".into(),
            is_directory: false,
        },
    ]);
    apply_editor(&mut state, EditorCommand::EnterFolder);
    assert_eq!(state.prompt.text(), "@src/");
    apply_editor(&mut state, EditorCommand::EnterFolder);
    assert_eq!(state.prompt.text(), "@src/ui/");

    // 回退上一级走编辑器撤销。
    apply_editor(&mut state, EditorCommand::Undo);
    assert_eq!(state.prompt.text(), "@src/");
    assert_eq!(
        state.prompt.mention().map(|panel| panel.scope_name()),
        Some(Some("src"))
    );
    apply_editor(&mut state, EditorCommand::Undo);
    assert_eq!(state.prompt.text(), "@");
    assert_eq!(
        state.prompt.mention().map(|panel| panel.scope_name()),
        Some(None)
    );
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
    // 滚轮只滚视口：高亮不动，视口从当前窗口起点偏移并钳制。
    assert!(scroll_mention(&mut state, 1, 0));
    assert_eq!(
        state.prompt.mention().map(|panel| panel.viewport()),
        Some(Some(1))
    );
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(1));
    assert!(!scroll_mention(&mut state, 1, 1), "到底后钳制不循环");
    assert!(scroll_mention(&mut state, -1, 1));
    assert_eq!(
        state.prompt.mention().map(|panel| panel.viewport()),
        Some(Some(0))
    );
    apply_editor(&mut state, EditorCommand::Up);
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(0));
    apply_editor(&mut state, EditorCommand::Down);
    assert_eq!(state.prompt.mention().map(|panel| panel.active()), Some(1));
    select_mention(&mut state, 1);
    assert_eq!(state.prompt.text(), "@b.rs ");
    assert!(state.prompt.mention().is_none());
}

#[test]
fn mouse_block_panel_hover_select_and_wheel_flow() {
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    focus_prompt(&mut state);
    apply_editor(&mut state, EditorCommand::InsertChar('#'));
    assert!(state.prompt.panel().is_some(), "块命令面板打开");

    assert!(hover_panel(&mut state, 2));
    assert_eq!(state.prompt.panel().map(|panel| panel.active()), Some(2));
    assert_eq!(
        state.prompt.panel().map(|panel| panel.anchor()),
        Some(0),
        "悬停只改高亮，不动窗口锚点"
    );
    assert!(!hover_panel(&mut state, 2));

    // 滚轮只滚视口：高亮与锚点不动。
    assert!(scroll_panel(&mut state, 1, 0));
    assert_eq!(
        state.prompt.panel().map(|panel| panel.viewport()),
        Some(Some(1))
    );
    assert_eq!(state.prompt.panel().map(|panel| panel.active()), Some(2));
    assert!(scroll_panel(&mut state, 99, 1));
    assert_eq!(
        state.prompt.panel().map(|panel| panel.viewport()),
        Some(Some(5))
    );
    assert!(!scroll_panel(&mut state, 1, 5), "到底后钳制不循环");
    assert!(scroll_panel(&mut state, -99, 5));
    assert_eq!(
        state.prompt.panel().map(|panel| panel.viewport()),
        Some(Some(0))
    );

    apply_editor(&mut state, EditorCommand::Up);
    assert_eq!(state.prompt.panel().map(|panel| panel.active()), Some(1));
    apply_editor(&mut state, EditorCommand::Down);
    assert_eq!(state.prompt.panel().map(|panel| panel.active()), Some(2));

    select_panel(&mut state, 1);
    assert_eq!(state.prompt.text(), "## ");
    assert!(state.prompt.panel().is_none(), "点选后关闭面板");
}

#[test]
fn toggle_pane_view_flips_target_and_hides_terminal_state() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    assert_eq!(view(&state, pane), PaneView::Lx);

    assert!(toggle_pane_view(&mut state, pane));
    assert_eq!(view(&state, pane), PaneView::Terminal);
    begin_terminal_selection(&mut state, pane, 0, 0);
    assert_eq!(state.terminal_selection, Some(pane));
    state.terminal_scroll_drag = Some((pane, 0));

    assert!(toggle_pane_view(&mut state, pane));
    assert_eq!(view(&state, pane), PaneView::Lx);
    assert!(state.terminal_selection.is_none(), "切回 lx 清理终端选区");
    assert!(
        state.terminal_scroll_drag.is_none(),
        "切回 lx 清理滚动条拖拽"
    );
    assert!(!toggle_pane_view(&mut state, PaneId::alloc()));
}

#[test]
fn lx_animation_advances_only_while_visible() {
    let mut state = AppState::demo();
    let now = Instant::now();
    state.lx_last_tick = now;
    assert!(tick(&mut state, now + LX_FRAME_INTERVAL));
    assert_eq!(state.lx_phase, 1);
    assert!(!tick(&mut state, now + LX_FRAME_INTERVAL), "同刻只推进一步");

    let pane = state.active_tab().layout.focus();
    assert!(toggle_pane_view(&mut state, pane));
    assert!(!state.lx_visible());
    assert!(!tick(&mut state, now + LX_FRAME_INTERVAL * 10));
    assert_eq!(state.lx_phase, 1, "无 lx 视图时动画冻结");
    assert_eq!(next_deadline(&state), None);
}

#[test]
fn lx_animation_registers_frame_deadline_when_visible() {
    let mut state = AppState::demo();
    let now = Instant::now();
    state.lx_last_tick = now;
    assert_eq!(next_deadline(&state), Some(now + LX_FRAME_INTERVAL));

    let pane = state.active_tab().layout.focus();
    toggle_pane_view(&mut state, pane);
    assert_eq!(next_deadline(&state), None);
}

/// 窗格视图（测试断言用）。
fn view(state: &AppState, pane: PaneId) -> PaneView {
    state.pane_anywhere(pane).expect("pane exists").view
}

/// 构造工作区 git 元数据。
fn git_info(repo_root: &str, checkout: &str, linked: bool, branch: Option<&str>) -> WorkspaceGit {
    WorkspaceGit {
        repo_root: PathBuf::from(repo_root),
        checkout_path: PathBuf::from(checkout),
        is_linked: linked,
        branch: branch.map(str::to_string),
        main_branch: branch.map(str::to_string),
    }
}

/// 追加一个带 git 元数据的自动命名工作区。
fn push_git_workspace(state: &mut AppState, name: &str, cwd: &str, git: WorkspaceGit) -> usize {
    let mut workspace = Workspace::single_terminal(name.to_string(), Some(PathBuf::from(cwd)));
    workspace.git = Some(git);
    state.workspaces.push(workspace);
    state.workspaces.len() - 1
}

/// 构造一条 git worktree 记录。
fn worktree_entry(path: &str, branch: Option<&str>, bare: bool) -> crate::git::WorktreeEntry {
    crate::git::WorktreeEntry {
        path: PathBuf::from(path),
        branch: branch.map(str::to_string),
        is_bare: bare,
    }
}

#[test]
fn workspace_menu_offers_open_worktree_only_with_git_metadata() {
    let mut state = AppState::demo();
    open_workspace_menu(&mut state, 0, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("workspace menu opens");
    };
    assert!(!menu.commands.contains(&MenuCommand::OpenWorktree));

    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    open_workspace_menu(&mut state, 0, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("workspace menu opens");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::NewTerminal,
            MenuCommand::OpenPrompt,
            MenuCommand::RenameWorkspace,
            MenuCommand::OpenWorktree
        ]
    );

    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    open_workspace_menu(&mut state, 0, (0, 0));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("workspace menu opens");
    };
    assert_eq!(
        menu.commands,
        vec![
            MenuCommand::NewTerminal,
            MenuCommand::OpenPrompt,
            MenuCommand::RenameWorkspace,
            MenuCommand::OpenWorktree,
            MenuCommand::CloseWorkspace
        ]
    );
}

#[test]
fn open_worktree_dialog_requires_git_metadata_and_starts_loading() {
    let mut state = AppState::demo();
    open_worktree_dialog(&mut state, 0);
    assert!(state.overlay.is_none());

    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    open_worktree_dialog(&mut state, 0);
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("worktree dialog opens");
    };
    assert_eq!(dialog.source, 0);
    assert_eq!(dialog.repo_root, PathBuf::from("/repo"));
    assert!(dialog.loading);
    assert!(dialog.entries.is_empty());
}

#[test]
fn apply_worktree_list_fills_entries_and_marks_open_checkouts() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    let linked = push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    open_worktree_dialog(&mut state, 0);
    apply_worktree_list(
        &mut state,
        Path::new("/repo"),
        vec![
            worktree_entry("/repo", Some("main"), false),
            worktree_entry("/repo/.worktrees/feat", Some("feature/x"), false),
            worktree_entry("/repo/.worktrees/notes", None, false),
        ],
        false,
    );
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("worktree dialog stays open");
    };
    assert!(!dialog.loading);
    assert!(!dialog.failed);
    assert_eq!(dialog.entries.len(), 3);
    assert_eq!(dialog.entries[0].already_open, Some(0));
    assert_eq!(dialog.entries[1].already_open, Some(linked));
    assert_eq!(dialog.entries[2].already_open, None);
    assert!(dialog.entries[2].is_linked);
    assert_eq!(dialog.entries[0].status(), WorktreeStatus::Open);
}

#[test]
fn apply_worktree_list_ignores_results_for_other_repo_or_closed_dialog() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    open_worktree_dialog(&mut state, 0);
    apply_worktree_list(
        &mut state,
        Path::new("/other"),
        vec![worktree_entry("/other", Some("main"), false)],
        false,
    );
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open");
    };
    assert!(dialog.entries.is_empty());
    assert!(dialog.loading);

    close_overlay(&mut state);
    apply_worktree_list(
        &mut state,
        Path::new("/repo"),
        vec![worktree_entry("/repo", Some("main"), false)],
        false,
    );
    assert!(state.overlay.is_none());
}

#[test]
fn apply_worktree_list_marks_failure_without_entries() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    open_worktree_dialog(&mut state, 0);
    apply_worktree_list(&mut state, Path::new("/repo"), Vec::new(), true);
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open");
    };
    assert!(dialog.failed);
    assert!(!dialog.loading);
    assert!(dialog.entries.is_empty());
}

#[test]
fn commit_worktree_open_creates_workspace_with_checkout_cwd_and_git() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    open_worktree_dialog(&mut state, 0);
    apply_worktree_list(
        &mut state,
        Path::new("/repo"),
        vec![
            worktree_entry("/repo", Some("main"), false),
            worktree_entry("/repo/.worktrees/feat", Some("feature/x"), false),
        ],
        false,
    );
    apply_worktree_open_key(&mut state, OverlayKey::Down);
    apply_worktree_open_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(state.active_workspace, 1);
    let opened = state.active_workspace();
    assert_eq!(opened.name, "feat");
    assert_eq!(opened.cwd, Some(PathBuf::from("/repo/.worktrees/feat")));
    let git = opened.git.as_ref().expect("git metadata is carried over");
    assert_eq!(git.repo_root, PathBuf::from("/repo"));
    assert_eq!(git.checkout_path, PathBuf::from("/repo/.worktrees/feat"));
    assert!(git.is_linked);
    assert_eq!(git.branch.as_deref(), Some("feature/x"));
}

#[test]
fn commit_worktree_open_switches_to_existing_workspace_without_duplicate() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    let existing = push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    open_worktree_dialog(&mut state, 0);
    apply_worktree_list(
        &mut state,
        Path::new("/repo"),
        vec![
            worktree_entry("/repo", Some("main"), false),
            worktree_entry("/repo/.worktrees/feat", Some("feature/x"), false),
        ],
        false,
    );
    apply_worktree_open_key(&mut state, OverlayKey::Down);
    apply_worktree_open_key(&mut state, OverlayKey::Enter);

    assert!(state.overlay.is_none());
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(state.active_workspace, existing);
}

#[test]
fn commit_worktree_open_ignores_empty_dialog() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    open_worktree_dialog(&mut state, 0);
    commit_worktree_open(&mut state);
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("empty dialog stays open");
    };
    assert!(dialog.loading);
    assert_eq!(state.workspaces.len(), 1);
}

#[test]
fn apply_worktree_open_key_edits_query_and_closes_on_escape() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    open_worktree_dialog(&mut state, 0);
    apply_worktree_list(
        &mut state,
        Path::new("/repo"),
        vec![
            worktree_entry("/repo", Some("main"), false),
            worktree_entry("/repo/.worktrees/feat", Some("feature/x"), false),
        ],
        false,
    );
    apply_worktree_open_key(&mut state, OverlayKey::Char('f'));
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open");
    };
    assert_eq!(dialog.query.text(), "f");
    assert_eq!(dialog.selected_entry_index(), Some(1));

    apply_worktree_open_key(&mut state, OverlayKey::Backspace);
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open");
    };
    assert_eq!(dialog.query.text(), "");

    apply_worktree_open_key(&mut state, OverlayKey::Char('f'));
    apply_worktree_open_key(&mut state, OverlayKey::Clear);
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open");
    };
    assert_eq!(dialog.query.text(), "");

    apply_worktree_open_key(&mut state, OverlayKey::Esc);
    assert!(state.overlay.is_none());
}

#[test]
fn set_worktree_open_selection_ignores_out_of_range() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    open_worktree_dialog(&mut state, 0);
    apply_worktree_list(
        &mut state,
        Path::new("/repo"),
        vec![
            worktree_entry("/repo", Some("main"), false),
            worktree_entry("/repo/.worktrees/feat", Some("feature/x"), false),
        ],
        false,
    );
    assert!(set_worktree_open_selection(&mut state, 1));
    assert!(!set_worktree_open_selection(&mut state, 1));
    assert!(!set_worktree_open_selection(&mut state, 5));
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        panic!("dialog stays open");
    };
    assert_eq!(dialog.selected, 1);
}

#[test]
fn apply_git_refresh_updates_workspaces_with_matching_cwd_only() {
    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(PathBuf::from("/repo"));
    let other = push_git_workspace(
        &mut state,
        "other",
        "/elsewhere",
        git_info("/x", "/x", false, None),
    );

    apply_git_refresh(
        &mut state,
        Path::new("/repo"),
        Some(git_info("/repo", "/repo", false, Some("main"))),
    );
    assert!(state.workspaces[0].git.is_some());
    assert!(state.workspaces[other].git.is_some());

    apply_git_refresh(&mut state, Path::new("/repo"), None);
    assert!(state.workspaces[0].git.is_none());
    assert!(state.workspaces[other].git.is_some());
}

#[test]
fn request_git_refresh_dedups_pending_cwds_and_take_drains() {
    let mut state = AppState::demo();
    request_git_refresh(&mut state, Path::new("/a"));
    request_git_refresh(&mut state, Path::new("/a"));
    request_git_refresh(&mut state, Path::new("/b"));
    assert_eq!(
        state.git_requests,
        vec![PathBuf::from("/a"), PathBuf::from("/b")]
    );
    let taken = take_git_requests(&mut state);
    assert_eq!(taken, vec![PathBuf::from("/a"), PathBuf::from("/b")]);
    assert!(state.git_requests.is_empty());
}

#[test]
fn toggle_workspace_group_flips_repo_key_and_ignores_plain_workspaces() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    toggle_workspace_group(&mut state, 0);
    assert_eq!(state.collapsed_groups, vec![PathBuf::from("/repo")]);
    toggle_workspace_group(&mut state, 0);
    assert!(state.collapsed_groups.is_empty());

    state
        .workspaces
        .push(Workspace::single_terminal("plain".to_string(), None));
    toggle_workspace_group(&mut state, 1);
    assert!(state.collapsed_groups.is_empty());
}

#[test]
fn collapsed_group_scroll_and_visibility_follow_visible_rows() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    push_git_workspace(
        &mut state,
        "notes",
        "/repo/.worktrees/notes",
        git_info("/repo", "/repo/.worktrees/notes", true, Some("notes")),
    );

    assert_eq!(workspace_scroll_max(&state, 2), 1);
    state.collapsed_groups.push(PathBuf::from("/repo"));
    assert_eq!(workspace_scroll_max(&state, 2), 0, "折叠后仅剩父项一行");

    state.active_workspace = 2;
    assert_eq!(workspace_scroll_max(&state, 1), 1);
    state.workspace_scroll = 5;
    assert!(ensure_workspace_visible(&mut state, 1));
    assert_eq!(state.workspace_scroll, 1);
}

#[test]
fn apply_git_refresh_auto_opens_parent_for_first_linked_workspace() {
    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(PathBuf::from("/repo/.worktrees/feat"));
    apply_git_refresh(
        &mut state,
        Path::new("/repo/.worktrees/feat"),
        Some(git_info(
            "/repo",
            "/repo/.worktrees/feat",
            true,
            Some("feature/x"),
        )),
    );

    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(state.workspaces[0].cwd, Some(PathBuf::from("/repo")));
    let parent_git = state.workspaces[0].git.as_ref().expect("parent metadata");
    assert!(!parent_git.is_linked);
    assert_eq!(parent_git.checkout_path, PathBuf::from("/repo"));
    assert_eq!(
        state.workspaces[1].cwd,
        Some(PathBuf::from("/repo/.worktrees/feat"))
    );
    assert_eq!(state.active_workspace, 1, "激活跟随原工作区");

    apply_git_refresh(
        &mut state,
        Path::new("/repo/.worktrees/feat"),
        Some(git_info(
            "/repo",
            "/repo/.worktrees/feat",
            true,
            Some("feature/x"),
        )),
    );
    assert_eq!(state.workspaces.len(), 2, "已有元数据刷新不重复补开");
}

#[test]
fn ensure_main_workspace_skips_when_parent_is_open() {
    let mut state = AppState::demo();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, Some("main")));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    assert_eq!(worktree::ensure_main_workspace(&mut state, 1), None);
    assert_eq!(state.workspaces.len(), 2);
}

#[test]
fn commit_worktree_open_auto_opens_parent_before_linked_source() {
    let mut state = AppState::demo();
    state.workspaces[0].cwd = Some(PathBuf::from("/repo/.worktrees/source"));
    state.workspaces[0].git = Some(git_info(
        "/repo",
        "/repo/.worktrees/source",
        true,
        Some("source"),
    ));
    open_worktree_dialog(&mut state, 0);
    apply_worktree_list(
        &mut state,
        Path::new("/repo"),
        vec![
            worktree_entry("/repo", Some("main"), false),
            worktree_entry("/repo/.worktrees/feat", Some("feature/x"), false),
        ],
        false,
    );
    apply_worktree_open_key(&mut state, OverlayKey::Down);
    apply_worktree_open_key(&mut state, OverlayKey::Enter);

    assert_eq!(state.workspaces.len(), 3);
    assert_eq!(
        state.workspaces[0].cwd,
        Some(PathBuf::from("/repo")),
        "父项插到仓库首个成员之前"
    );
    assert!(
        state.workspaces[0]
            .git
            .as_ref()
            .is_some_and(|git| !git.is_linked)
    );
    assert_eq!(
        state.workspaces[1].cwd,
        Some(PathBuf::from("/repo/.worktrees/source"))
    );
    assert_eq!(
        state.workspaces[2].cwd,
        Some(PathBuf::from("/repo/.worktrees/feat"))
    );
    assert_eq!(state.active_workspace, 2, "激活跟随新打开的 worktree");

    let rows = state.workspace_rows();
    assert!(rows[0].parent);
    assert!(rows[1].child && rows[2].child, "树形结构始终成立");
}

#[test]
fn create_workspace_requests_git_metadata_for_cwd() {
    let mut state = AppState::demo();
    assert!(state.git_requests.is_empty());
    create_workspace(&mut state);
    assert_eq!(state.git_requests.len(), 1);
}

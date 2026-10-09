//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::overlay::{OverlayTarget, RenameTarget, TextInput};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 80,
    height: 24,
};

fn draw_overlay(state: &AppState) -> (Vec<String>, Option<(u16, u16)>) {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    let mut cursor = None;
    if let Err(error) = terminal.draw(|frame| {
        cursor = render(frame, frame.area(), state);
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let lines = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect();
    (lines, cursor)
}

fn menu_state(commands: Vec<MenuCommand>, selected: usize) -> AppState {
    let mut state = AppState::demo();
    state.workspaces[0].name = "demo".into();
    state.overlay = Some(Overlay::Menu(Menu {
        anchor: (10, 5),
        target: OverlayTarget::Workspace(0),
        commands,
        selected,
    }));
    state
}

fn rename_state(name: &str) -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new(name),
    }));
    state
}

#[test]
fn menu_layout_maps_commands_to_text_labels() {
    let state = menu_state(
        vec![MenuCommand::RenameWorkspace, MenuCommand::CloseWorkspace],
        0,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    let layout = menu_layout(&state, SCREEN, menu);
    assert_eq!(layout.area.x, 10);
    assert_eq!(layout.area.y, 5);
    assert_eq!(layout.item_rects.len(), 2);
}

#[test]
fn menu_renders_target_name_in_border_title() {
    let state = menu_state(
        vec![MenuCommand::RenameWorkspace, MenuCommand::CloseWorkspace],
        0,
    );
    let (lines, _) = draw_overlay(&state);
    assert!(lines[5].contains("demo"), "title row={:?}", lines[5]);
    assert!(lines[5].contains('╭'), "title row={:?}", lines[5]);
}

#[test]
fn menu_falls_back_to_target_kind_title() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Menu(Menu {
        anchor: (10, 5),
        target: OverlayTarget::Workspace(99),
        commands: vec![MenuCommand::CloseWorkspace],
        selected: 0,
    }));
    let (lines, _) = draw_overlay(&state);
    assert!(
        lines[5].contains(text::MENU_TITLE_WORKSPACE),
        "title row={:?}",
        lines[5]
    );
}

#[test]
fn menu_renders_labels_and_overlay_selection() {
    let state = menu_state(
        vec![MenuCommand::RenameWorkspace, MenuCommand::CloseWorkspace],
        1,
    );
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(lines[6].contains(text::MENU_RENAME_WORKSPACE));
    assert!(lines[7].contains(text::MENU_CLOSE_WORKSPACE));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| {
        render(frame, frame.area(), &state);
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(buffer[(11, 7)].bg, Color::Indexed(240));
    assert_eq!(buffer[(11, 7)].fg, Color::Indexed(255));
    assert!(!buffer[(11, 7)].modifier.contains(Modifier::REVERSED));
}

#[test]
fn modal_uses_overlay_panel_background() {
    let state = rename_state("demo");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| {
        render(frame, frame.area(), &state);
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let shell = rename_shell(SCREEN).expect("rename modal fits");
    assert_eq!(buffer[(shell.area.x, shell.area.y)].bg, Color::Indexed(236));
    assert_eq!(
        buffer[(shell.inner.x, shell.inner.y)].bg,
        Color::Indexed(236)
    );
}

#[test]
fn rename_modal_renders_input_buttons_and_cursor() {
    let state = rename_state("workspace 2");
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, Some((32, 10)));
    assert!(lines[9].contains(text::RENAME_WORKSPACE_TITLE));
    assert!(lines[10].contains("workspace 2"));
    let buttons = lines[12].clone();
    assert!(buttons.contains(text::BUTTON_SAVE), "{buttons}");
    assert!(buttons.contains(text::BUTTON_CLEAR), "{buttons}");
    assert!(buttons.contains(text::BUTTON_CANCEL), "{buttons}");
}

#[test]
fn rename_modal_keeps_cursor_visible_for_long_names() {
    let state = rename_state(&"x".repeat(60));
    let (lines, cursor) = draw_overlay(&state);
    let cursor = cursor.expect("cursor is set");
    assert!(cursor.0 < SCREEN.right());
    assert_eq!(cursor.0, 58);
    assert_eq!(cursor.1, 10);
    assert_eq!(lines[10].chars().filter(|ch| *ch == 'x').count(), 37);
}

#[test]
fn rename_modal_cursor_uses_display_width_for_wide_chars() {
    let state = rename_state("中文名称");
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, Some((29, 10)));
    let text: String = lines[10].chars().filter(|ch| !ch.is_whitespace()).collect();
    assert!(text.contains("中文名称"), "{text}");
}

#[test]
fn rename_modal_scrolls_by_display_width_for_long_wide_names() {
    let state = rename_state(&"中".repeat(30));
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, Some((57, 10)));
    assert_eq!(lines[10].matches('中').count(), 18);
}

#[test]
fn rename_buttons_hit_their_cells() {
    let shell = rename_shell(SCREEN).expect("rename modal fits");
    assert_eq!(rename_button_at(&shell, 21, 12), Some(RenameButton::Save));
    assert_eq!(rename_button_at(&shell, 35, 12), Some(RenameButton::Clear));
    assert_eq!(rename_button_at(&shell, 47, 12), Some(RenameButton::Cancel));
    assert_eq!(rename_button_at(&shell, 20, 12), None);
    assert_eq!(rename_button_at(&shell, 21, 11), None);
}

#[test]
fn confirm_modal_renders_question_and_buttons() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Workspace(0),
    }));
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(lines[10].contains(text::CONFIRM_CLOSE_TITLE));
    assert!(lines[11].contains("close \""));
    let buttons = lines[12].clone();
    assert!(buttons.contains(text::BUTTON_CONFIRM), "{buttons}");
    assert!(buttons.contains(text::BUTTON_CANCEL), "{buttons}");
}

#[test]
fn confirm_buttons_hit_their_cells() {
    let shell = confirm_shell(SCREEN).expect("confirm modal fits");
    assert_eq!(
        confirm_button_at(&shell, 25, 12),
        Some(ConfirmButton::Confirm)
    );
    assert_eq!(
        confirm_button_at(&shell, 42, 12),
        Some(ConfirmButton::Cancel)
    );
    assert_eq!(confirm_button_at(&shell, 24, 12), None);
}

#[test]
fn no_overlay_renders_nothing() {
    let state = AppState::demo();
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(!lines.iter().any(|line| line.contains("Rename")));
}

#[test]
fn tab_menu_renders_new_rename_close_labels() {
    let state = menu_state(
        vec![
            MenuCommand::NewTab,
            MenuCommand::RenameTab,
            MenuCommand::CloseTab,
        ],
        0,
    );
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(lines[6].contains(text::MENU_NEW_TAB));
    assert!(lines[7].contains(text::MENU_RENAME_TAB));
    assert!(lines[8].contains(text::MENU_CLOSE_TAB));
}

#[test]
fn pane_menu_renders_split_switch_and_close_labels() {
    let state = menu_state(
        vec![
            MenuCommand::SplitRight,
            MenuCommand::SplitDown,
            MenuCommand::SwitchToTerminal,
            MenuCommand::ClosePane,
        ],
        0,
    );
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(lines[6].contains(text::MENU_SPLIT_RIGHT));
    assert!(lines[7].contains(text::MENU_SPLIT_DOWN));
    assert!(lines[8].contains(text::MENU_SWITCH_TO_TERMINAL));
    assert!(lines[9].contains(text::MENU_CLOSE_PANE));
}

#[test]
fn confirm_modal_uses_pane_title_for_pane_target() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .active_tab_mut()
        .pane_mut(pane)
        .expect("pane exists")
        .cwd_label = Some("demo-cwd".into());
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Pane {
            workspace: 0,
            tab: 0,
            pane,
        },
    }));
    let (lines, _) = draw_overlay(&state);
    assert!(lines[10].contains(text::CONFIRM_CLOSE_PANE_TITLE));
    assert!(lines[11].contains("demo-cwd"));
}

#[test]
fn rename_modal_uses_tab_title_for_tab_target() {
    let mut state = AppState::demo();
    crate::app::update::create_tab(&mut state);
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Tab {
            workspace: 0,
            tab: 1,
        },
        input: TextInput::new("tab 2"),
    }));
    let (lines, _) = draw_overlay(&state);
    assert!(lines[9].contains(text::RENAME_TAB_TITLE));
    assert!(lines[10].contains("tab 2"));
}

#[test]
fn confirm_modal_uses_tab_label_for_tab_target() {
    let mut state = AppState::demo();
    crate::app::update::create_tab(&mut state);
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::Tab {
            workspace: 0,
            tab: 1,
        },
    }));
    let (lines, _) = draw_overlay(&state);
    assert!(lines[10].contains(text::CONFIRM_CLOSE_TAB_TITLE));
    assert!(lines[11].contains("tab 2"));
}

#[test]
fn menu_labels_cover_terminal_prompt_and_workspace_path_commands() {
    let state = menu_state(
        vec![
            MenuCommand::NewTerminal,
            MenuCommand::OpenPrompt,
            MenuCommand::SwitchToWorkspaceCwd,
            MenuCommand::SyncWorkspaceToTerminalCwd,
        ],
        0,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu overlay expected");
    };
    assert_eq!(
        menu_labels(menu),
        vec![
            text::MENU_NEW_TERMINAL,
            text::MENU_OPEN_PROMPT,
            text::MENU_SWITCH_TO_WORKSPACE_CWD,
            text::MENU_SYNC_WS_TO_TERMINAL_CWD,
        ]
    );
}

#[test]
fn workspace_menu_renders_all_labels_without_ellipsis() {
    let state = menu_state(
        vec![
            MenuCommand::NewTerminal,
            MenuCommand::OpenPrompt,
            MenuCommand::RenameWorkspace,
            MenuCommand::OpenWorktree,
            MenuCommand::CloseWorkspace,
        ],
        0,
    );
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    for label in [
        text::MENU_NEW_TERMINAL,
        text::MENU_OPEN_PROMPT,
        text::MENU_RENAME_WORKSPACE,
        text::MENU_OPEN_WORKTREE,
        text::MENU_CLOSE_WORKSPACE,
    ] {
        assert!(joined.contains(label), "缺少 {label}: {joined}");
    }
    assert!(!joined.contains('…'), "菜单文案不得出现省略: {joined}");
}

#[test]
fn switch_cwd_confirm_renders_title_and_full_path() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let path = PathBuf::from("/tmp/workspace path");
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: path.clone(),
    }));
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    let joined = lines.join("\n");
    assert!(joined.contains(text::CONFIRM_SWITCH_CWD_TITLE), "{joined}");
    assert!(joined.contains("/tmp/workspace path"), "{joined}");
}

#[test]
fn switch_cwd_confirm_shell_widens_for_long_paths_and_clamps() {
    let long = PathBuf::from(format!("/tmp/{}", "d".repeat(60)));
    let wide_screen = Rect::new(0, 0, 100, 24);
    let shell = confirm_switch_cwd_shell(wide_screen, &long).expect("dialog fits");
    assert!(shell.area.width > CONFIRM_WIDTH);
    let question = text::confirm_switch_cwd_question(&long.display().to_string());
    assert!(usize::from(shell.inner.width) >= question.width());

    let narrow = Rect::new(0, 0, 30, 24);
    let clamped = confirm_switch_cwd_shell(narrow, &long).expect("dialog fits");
    assert_eq!(clamped.area.width, 30);
}

#[test]
fn switch_cwd_confirm_buttons_hit_their_cells() {
    let path = PathBuf::from("/tmp/ws");
    let shell = confirm_switch_cwd_shell(SCREEN, &path).expect("dialog fits");
    let buttons = widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_CONFIRM, text::BUTTON_CANCEL],
        BUTTON_GAP,
        CONFIRM_BUTTON_ROW,
    );
    assert_eq!(
        confirm_button_at(&shell, buttons[0].x, buttons[0].y),
        Some(ConfirmButton::Confirm)
    );
    assert_eq!(
        confirm_button_at(&shell, buttons[1].x, buttons[1].y),
        Some(ConfirmButton::Cancel)
    );
}

#[test]
fn sync_ws_cwd_confirm_renders_title_and_path() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let path = PathBuf::from("/tmp/terminal-target");
    state.overlay = Some(Overlay::ConfirmSyncWorkspaceCwd(ConfirmSyncWorkspaceCwd {
        workspace: 0,
        pane,
        path: path.clone(),
    }));
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    let joined = lines.join("\n");
    assert!(joined.contains(text::CONFIRM_SYNC_WS_CWD_TITLE), "{joined}");
    assert!(joined.contains("/tmp/terminal-target"), "{joined}");
}

#[test]
fn sync_ws_cwd_confirm_shell_widens_for_long_paths_and_clamps() {
    let long = PathBuf::from(format!("/tmp/{}", "d".repeat(60)));
    let wide_screen = Rect::new(0, 0, 100, 24);
    let shell = confirm_sync_workspace_cwd_shell(wide_screen, &long).expect("dialog fits");
    assert!(shell.area.width > CONFIRM_WIDTH);

    let narrow = Rect::new(0, 0, 30, 24);
    let clamped = confirm_sync_workspace_cwd_shell(narrow, &long).expect("dialog fits");
    assert_eq!(clamped.area.width, 30);
}

#[test]
fn sync_ws_cwd_confirm_buttons_hit_their_cells() {
    let path = PathBuf::from("/tmp/terminal-target");
    let shell = confirm_sync_workspace_cwd_shell(SCREEN, &path).expect("dialog fits");
    let buttons = widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_CONFIRM, text::BUTTON_CANCEL],
        BUTTON_GAP,
        CONFIRM_BUTTON_ROW,
    );
    assert_eq!(
        confirm_button_at(&shell, buttons[0].x, buttons[0].y),
        Some(ConfirmButton::Confirm)
    );
    assert_eq!(
        confirm_button_at(&shell, buttons[1].x, buttons[1].y),
        Some(ConfirmButton::Cancel)
    );
}

#[test]
fn new_workspace_renders_title_buttons_and_cursor() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
        input: TextInput::new("/test/path"),
    }));
    let (lines, cursor) = draw_overlay(&state);
    assert!(cursor.is_some(), "text input must place cursor");
    let joined = lines.join("\n");
    assert!(joined.contains(text::NEW_WORKSPACE_TITLE), "{joined}");
    assert!(joined.contains("/test/path"), "{joined}");
    assert!(joined.contains(text::BUTTON_CREATE), "{joined}");
    assert!(joined.contains(text::BUTTON_CLEAR), "{joined}");
    assert!(joined.contains(text::BUTTON_CANCEL), "{joined}");
}

#[test]
fn new_workspace_buttons_hit_their_cells() {
    let shell = new_workspace_shell(SCREEN).expect("dialog fits");
    let buttons = widgets::modal::button_row(
        shell.inner,
        &NEW_WORKSPACE_BUTTONS,
        BUTTON_GAP,
        NEW_WORKSPACE_BUTTON_ROW,
    );
    assert_eq!(
        new_workspace_button_at(&shell, buttons[0].x, buttons[0].y),
        Some(NewWorkspaceButton::Create)
    );
    assert_eq!(
        new_workspace_button_at(&shell, buttons[1].x, buttons[1].y),
        Some(NewWorkspaceButton::Clear)
    );
    assert_eq!(
        new_workspace_button_at(&shell, buttons[2].x, buttons[2].y),
        Some(NewWorkspaceButton::Cancel)
    );
}

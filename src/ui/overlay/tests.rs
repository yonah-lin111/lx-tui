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

/// 模态输入行的 `> ` 前缀宽度（图标 + 空档）。
fn input_prefix_width() -> u16 {
    text::INPUT_PREFIX_ICON.chars().count() as u16 + 1
}

#[test]
fn trust_overlay_renders_question_path_and_buttons() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::TrustWorkspace(TrustWorkspace {
        path: PathBuf::from("/tmp/trusted-workspace"),
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace {
            cwd: PathBuf::from("/tmp/trusted-workspace"),
        },
    }));
    let (lines, cursor) = draw_overlay(&state);
    let body = lines.join("\n");
    assert!(body.contains(text::TRUST_WORKSPACE_TITLE), "{body}");
    assert!(body.contains(text::TRUST_WORKSPACE_QUESTION), "{body}");
    assert!(body.contains("/tmp/trusted-workspace"), "{body}");
    assert!(body.contains(text::BUTTON_TRUST), "{body}");
    assert!(body.contains(text::BUTTON_DONT_TRUST), "{body}");
    assert!(cursor.is_none(), "信任浮层不占用硬件光标");
}

#[test]
fn trust_overlay_buttons_hit_expected_cells() {
    let shell = trust_shell(SCREEN).expect("trust shell fits");
    let rects =
        widgets::modal::button_row(shell.inner, &TRUST_BUTTONS, BUTTON_GAP, TRUST_BUTTON_ROW);
    assert_eq!(
        trust_button_at(&shell, rects[0].x, rects[0].y),
        Some(TrustButton::Trust)
    );
    assert_eq!(
        trust_button_at(&shell, rects[1].x, rects[1].y),
        Some(TrustButton::Refuse)
    );
    assert_eq!(trust_button_at(&shell, shell.inner.x, shell.inner.y), None);
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
fn menu_workspace_list_shows_list_title_and_new_workspace_label() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Menu(Menu {
        anchor: (10, 5),
        target: OverlayTarget::WorkspaceList,
        commands: vec![MenuCommand::NewWorkspace],
        selected: 0,
    }));
    let (lines, _) = draw_overlay(&state);
    assert!(
        lines[5].contains(text::MENU_TITLE_WORKSPACES),
        "title row={:?}",
        lines[5]
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains(text::MENU_NEW_WORKSPACE)),
        "item label missing: {lines:?}"
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
    assert_eq!(cursor, Some((34, 10)));
    assert!(lines[9].contains(text::RENAME_WORKSPACE_TITLE));
    assert!(
        lines[10].contains(&format!("{} workspace 2", text::INPUT_PREFIX_ICON)),
        "输入行带 > 前缀: {:?}",
        lines[10]
    );
    assert!(!lines[10].contains(text::RENAME_PLACEHOLDER));
    let buttons = lines[12].clone();
    assert!(buttons.contains(text::BUTTON_SAVE), "{buttons}");
    assert!(buttons.contains(text::BUTTON_CLEAR), "{buttons}");
    assert!(buttons.contains(text::BUTTON_CANCEL), "{buttons}");
}

#[test]
fn rename_modal_renders_placeholder_when_empty() {
    let state = rename_state("");
    let (lines, _) = draw_overlay(&state);
    assert!(
        lines[10].contains(text::RENAME_PLACEHOLDER),
        "row={:?}",
        lines[10]
    );
}

#[test]
fn rename_modal_keeps_cursor_visible_for_long_names() {
    let state = rename_state(&"x".repeat(60));
    let (lines, cursor) = draw_overlay(&state);
    let cursor = cursor.expect("cursor is set");
    assert!(cursor.0 < SCREEN.right());
    assert_eq!(cursor.0, 58);
    assert_eq!(cursor.1, 10);
    assert_eq!(lines[10].chars().filter(|ch| *ch == 'x').count(), 35);
}

#[test]
fn rename_modal_cursor_uses_display_width_for_wide_chars() {
    let state = rename_state("中文名称");
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, Some((31, 10)));
    let text: String = lines[10].chars().filter(|ch| !ch.is_whitespace()).collect();
    assert!(text.contains("中文名称"), "{text}");
}

#[test]
fn rename_modal_scrolls_by_display_width_for_long_wide_names() {
    let state = rename_state(&"中".repeat(30));
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, Some((57, 10)));
    assert_eq!(lines[10].matches('中').count(), 17);
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
    let shell = confirm_shell(SCREEN, "close \"x\"?").expect("confirm modal fits");
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
fn confirm_question_wraps_and_dialog_grows() {
    let mut state = AppState::demo();
    let name = "New folder11111 with a very long trailing name that cannot fit on one line";
    state.workspaces[0].prompt_panel.tree.replace(
        vec![crate::app::prompt::PromptFolder {
            id: 1,
            parent_id: None,
            name: name.to_string(),
            collapsed: false,
        }],
        vec![],
    );
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose {
        target: OverlayTarget::PromptFolder {
            workspace: 0,
            folder_id: 1,
        },
    }));
    let question = confirm_question(
        &state,
        OverlayTarget::PromptFolder {
            workspace: 0,
            folder_id: 1,
        },
    );
    let shell = confirm_shell(SCREEN, &question).expect("dialog fits");
    assert!(shell.area.height > 4, "高度随换行增长: {:?}", shell.area);
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(
        joined.contains("delete folder \"New folder11111"),
        "{joined}"
    );
    assert!(
        joined.contains("one line\" and its contents?"),
        "整段问题可见（自动换行）: {joined}"
    );
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
            MenuCommand::OpenPrompt,
            MenuCommand::SwitchToTerminal,
            MenuCommand::ClosePane,
        ],
        0,
    );
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, None);
    assert!(lines[6].contains(text::MENU_SPLIT_RIGHT));
    assert!(lines[7].contains(text::MENU_SPLIT_DOWN));
    assert!(lines[8].contains(text::MENU_OPEN_PROMPT));
    assert!(lines[9].contains(text::MENU_SWITCH_TO_TERMINAL));
    assert!(lines[10].contains(text::MENU_CLOSE_PANE));
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
            MenuCommand::OpenWorktree,
            MenuCommand::OpenPrompt,
            MenuCommand::RenameWorkspace,
            MenuCommand::CloseWorkspace,
        ],
        0,
    );
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    for label in [
        text::MENU_NEW_TERMINAL,
        text::MENU_OPEN_WORKTREE,
        text::MENU_OPEN_PROMPT,
        text::MENU_RENAME_WORKSPACE,
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
        shell.inner.height.saturating_sub(1),
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
        shell.inner.height.saturating_sub(1),
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
    assert!(
        !joined.contains(text::NEW_WORKSPACE_PLACEHOLDER),
        "{joined}"
    );
    assert!(joined.contains(text::BUTTON_CREATE), "{joined}");
    assert!(joined.contains(text::BUTTON_CLEAR), "{joined}");
    assert!(joined.contains(text::BUTTON_CANCEL), "{joined}");
}

#[test]
fn new_workspace_renders_placeholder_when_empty() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
        input: TextInput::new(""),
    }));
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::NEW_WORKSPACE_PLACEHOLDER), "{joined}");
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

fn save_draft_state(entries: Vec<SaveDraftEntry>, selected: usize) -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::SaveDraft(SaveDraft {
        workspace: 0,
        name: TextInput::new(text::PROMPT_DEFAULT_NAME),
        entries,
        selected,
    }));
    state
}

#[test]
fn save_draft_picker_renders_name_tree_entries_and_buttons() {
    let entries = vec![
        SaveDraftEntry::Root,
        SaveDraftEntry::Folder {
            id: 1,
            depth: 0,
            name: "ideas".into(),
        },
        SaveDraftEntry::Folder {
            id: 2,
            depth: 1,
            name: "deep".into(),
        },
        SaveDraftEntry::AddFolder,
    ];
    let state = save_draft_state(entries, 1);
    let (lines, cursor) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::SAVE_DRAFT_TITLE), "{joined}");
    assert!(
        joined.contains(&format!(
            "{} {}",
            text::INPUT_PREFIX_ICON,
            text::PROMPT_DEFAULT_NAME
        )),
        "名称输入行带 > 前缀: {joined}"
    );
    assert!(
        joined.contains(text::PROMPT_DEFAULT_NAME),
        "名称预填: {joined}"
    );
    assert!(joined.contains(text::SAVE_DRAFT_ROOT), "{joined}");
    assert!(joined.contains("└─ deep"), "子层树形连接符: {joined}");
    assert!(joined.contains(text::SAVE_DRAFT_NEW_FOLDER), "{joined}");
    assert!(joined.contains(text::BUTTON_SAVE), "{joined}");
    assert!(joined.contains(text::BUTTON_CANCEL), "{joined}");
    // 选中行非内联输入：硬件光标在名称输入。
    let shell = save_draft_shell(SCREEN, 4).expect("shell fits");
    let name_x = shell.inner.x + input_prefix_width();
    assert_eq!(
        cursor,
        Some((
            name_x + text::PROMPT_DEFAULT_NAME.chars().count() as u16,
            shell.inner.y
        )),
        "硬件光标在名称输入末尾"
    );
    // 选中行整行铺选中底色（Indexed 240）。
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    terminal
        .draw(|frame| {
            render(frame, frame.area(), &state);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    let list_y = shell.inner.y + 2;
    assert_eq!(buffer[(shell.inner.x, list_y + 1)].bg, Color::Indexed(240));
    assert_eq!(buffer[(shell.inner.x, list_y)].bg, Color::Indexed(236));
}

#[test]
fn new_prompt_dialog_renders_title_and_default_name() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::NewNode(NewNode {
        kind: NewNodeKind::Prompt { folder_id: None },
        name: TextInput::new(""),
        resume: None,
    }));
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::NEW_PROMPT_TITLE), "{joined}");
    assert!(joined.contains(text::PROMPT_DEFAULT_NAME), "{joined}");
    assert!(joined.contains(text::BUTTON_CREATE), "{joined}");
}

#[test]
fn new_folder_dialog_renders_input_buttons_and_hit() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::NewNode(NewNode {
        kind: NewNodeKind::Folder { parent_id: None },
        name: TextInput::new(""),
        resume: None,
    }));
    let (lines, cursor) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::NEW_FOLDER_TITLE), "{joined}");
    assert!(
        joined.contains(text::PROMPT_FOLDER_DEFAULT_NAME),
        "{joined}"
    );
    assert!(joined.contains(text::BUTTON_CREATE), "{joined}");
    assert!(joined.contains(text::BUTTON_CLEAR), "{joined}");
    assert!(joined.contains(text::BUTTON_CANCEL), "{joined}");
    let shell = new_node_shell(SCREEN).expect("shell fits");
    assert_eq!(
        cursor,
        Some((shell.inner.x + input_prefix_width(), shell.inner.y)),
        "空输入时光标在 > 前缀后的名称起点"
    );
    let buttons = widgets::modal::button_row(
        shell.inner,
        &NEW_NODE_BUTTONS,
        BUTTON_GAP,
        RENAME_BUTTON_ROW,
    );
    assert_eq!(
        new_node_button_at(&shell, buttons[0].x, buttons[0].y),
        Some(NewNodeButton::Create)
    );
    assert_eq!(
        new_node_button_at(&shell, buttons[1].x, buttons[1].y),
        Some(NewNodeButton::Clear)
    );
    assert_eq!(
        new_node_button_at(&shell, buttons[2].x, buttons[2].y),
        Some(NewNodeButton::Cancel)
    );
}

#[test]
fn save_draft_entry_hit_maps_rows_and_buttons() {
    let dialog = SaveDraft {
        workspace: 0,
        name: TextInput::new("x"),
        entries: vec![
            SaveDraftEntry::Root,
            SaveDraftEntry::Folder {
                id: 1,
                depth: 0,
                name: "a".into(),
            },
            SaveDraftEntry::AddFolder,
        ],
        selected: 0,
    };
    let shell = save_draft_shell(SCREEN, dialog.entries.len()).expect("shell");
    let list_y = shell.inner.y + 2;
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, shell.inner.y),
        None,
        "名称行不可命中条目"
    );
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, list_y),
        Some(0)
    );
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, list_y + 2),
        Some(2)
    );
    assert_eq!(
        save_draft_entry_at(
            &shell,
            &dialog,
            shell.inner.x,
            shell.inner.bottom().saturating_sub(1)
        ),
        None,
        "按钮行不可命中条目"
    );
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x.saturating_sub(1), list_y),
        None,
        "横向区域外"
    );
    let buttons = widgets::modal::button_row(
        shell.inner,
        &SAVE_DRAFT_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    assert_eq!(
        save_draft_button_at(&shell, buttons[0].x, buttons[0].y),
        Some(SaveDraftButton::Save)
    );
    assert_eq!(
        save_draft_button_at(&shell, buttons[1].x, buttons[1].y),
        Some(SaveDraftButton::Cancel)
    );
}

#[test]
fn save_draft_picker_windows_to_selected_entry() {
    let mut entries = Vec::new();
    for index in 0..30 {
        entries.push(SaveDraftEntry::Folder {
            id: index,
            depth: 0,
            name: format!("f{index}"),
        });
    }
    entries.push(SaveDraftEntry::AddFolder);
    let dialog = SaveDraft {
        workspace: 0,
        name: TextInput::new("x"),
        entries,
        selected: 30,
    };
    let shell = save_draft_shell(SCREEN, dialog.entries.len()).expect("shell");
    let max_rows = save_draft_max_rows(&shell);
    let start = save_draft_visible_start(&dialog, max_rows);
    assert!(start > 0, "选中项在窗口内需要上移窗口");
    let last_row = shell.inner.y + 2 + (max_rows as u16).saturating_sub(1);
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, last_row),
        Some(30)
    );
}

// ===== 页码菜单 =====

/// 绘制整帧并返回缓冲区（页码菜单底色/高亮断言用）。
fn draw_overlay_buffer(state: &AppState) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend");
    terminal
        .draw(|frame| {
            render(frame, frame.area(), state);
        })
        .expect("draw");
    terminal.backend().buffer().clone()
}

#[test]
fn page_menu_renders_split_buttons_two_line_pages_and_current_marker() {
    use crate::app::prompt::{PromptItem, PromptPage};
    let mut state = AppState::demo();
    let mut item = PromptItem::single(7, None, "paged", "");
    item.pages[0].content = "&&& addTemplate --start\n&&& addTemplate --end".into();
    item.pages.push(PromptPage::saved(
        70,
        "Notes",
        "&&& bugTemplate --start\n&&& bugTemplate --end done",
    ));
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, 7));
    assert!(crate::app::update::open_prompt_page_menu(
        &mut state,
        (10, 4)
    ));
    let buffer = draw_overlay_buffer(&state);
    let lines: Vec<String> = (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let body = lines.join("\n");
    assert!(body.contains(text::MENU_PAGE_NEW), "{body}");
    assert!(body.contains(text::MENU_PAGE_DELETE), "{body}");
    assert!(body.contains("Page 1"), "{body}");
    assert!(body.contains("Notes"), "{body}");
    assert!(body.contains("[todo 1]"), "逐页 todo 计数: {body}");
    assert!(body.contains("[done 1]"), "逐页 done 计数: {body}");

    let Some(Overlay::PageMenu(menu)) = &state.overlay else {
        panic!("page menu open");
    };
    let layout = page_menu_layout(&state, SCREEN, menu);
    assert_eq!(menu.selected, 1, "打开时光标停在当前页");
    // 顶部按钮平分菜单宽度，命中分别映射到首行与末行。
    assert_eq!(
        layout.new_rect.width + layout.delete_rect.width,
        layout.area.width - 2
    );
    assert!(layout.new_rect.width.abs_diff(layout.delete_rect.width) <= 1);
    assert_eq!(
        page_menu_item_at(&state, SCREEN, menu, layout.new_rect.x, layout.new_rect.y),
        Some(0)
    );
    assert_eq!(
        page_menu_item_at(
            &state,
            SCREEN,
            menu,
            layout.delete_rect.x,
            layout.delete_rect.y
        ),
        Some(menu.rows.len() - 1)
    );

    // 当前页首行带 `❯` 标记；光标移到下一页后当前页行无颜色高亮。
    let current = layout.page_rects[menu.selected - 1];
    let row: String = (current.x..current.right())
        .map(|x| buffer[(x, current.y)].symbol())
        .collect();
    assert!(
        row.starts_with(&format!("{} ", text::AGENT_SELECTED_MARKER)),
        "当前页行首标记: {row:?}"
    );
    assert!(crate::app::update::set_page_menu_selection(&mut state, 2));
    let Some(Overlay::PageMenu(menu)) = &state.overlay else {
        panic!("page menu open");
    };
    let buffer = draw_overlay_buffer(&state);
    let current = layout.page_rects[0];
    assert_eq!(
        Some(buffer[(current.x, current.y)].bg),
        style::overlay_panel().bg,
        "当前页行不铺颜色高亮"
    );
    let row: String = (current.x..current.right())
        .map(|x| buffer[(x, current.y)].symbol())
        .collect();
    assert!(row.starts_with(text::AGENT_SELECTED_MARKER), "{row:?}");
    assert_eq!(
        Some(buffer[(layout.page_rects[1].x, layout.page_rects[1].y)].bg),
        style::overlay_selection().bg,
        "光标行保留菜单反馈底色"
    );
    // 两行页项：两行都可命中同一行快照。
    let page = layout.page_rects[1];
    assert_eq!(
        page_menu_item_at(&state, SCREEN, menu, page.x + 1, page.y),
        Some(menu.selected)
    );
    assert_eq!(
        page_menu_item_at(&state, SCREEN, menu, page.x + 1, page.y + 1),
        Some(menu.selected)
    );
}

#[test]
fn page_delete_confirm_uses_page_name_question() {
    use crate::app::prompt::{PromptItem, PromptPage, PromptPageId};
    let mut state = AppState::demo();
    let mut item = PromptItem::single(7, None, "paged", "body");
    item.pages.push(PromptPage::saved(70, "Notes", ""));
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, 7));
    let question = confirm_question(
        &state,
        OverlayTarget::PromptPage {
            workspace: 0,
            owner: crate::app::prompt::PromptEditing::Item(7),
            page: PromptPageId::Saved(70),
        },
    );
    assert_eq!(question, "delete page \"Notes\"?");
}

#[test]
fn page_menu_windows_around_selected_row_when_tall() {
    use crate::app::prompt::{PromptItem, PromptPage};
    let mut state = AppState::demo();
    let mut item = PromptItem::single(7, None, "paged", "");
    for index in 2..=30 {
        item.pages
            .push(PromptPage::saved(60 + index, format!("Page {index}"), ""));
    }
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, 7));
    assert!(crate::app::update::open_prompt_page_menu(
        &mut state,
        (10, 4)
    ));
    let visible = page_menu_visible_pages(SCREEN);
    assert!(visible >= 2, "测试屏幕需能显示多页");
    let pages = 30;
    assert!(pages > visible, "页数需超过视口");
    // 光标移到中部页行：窗口跟随，该页可见。
    let target = 15;
    assert!(crate::app::update::set_page_menu_selection(
        &mut state, target
    ));
    let Some(Overlay::PageMenu(menu)) = &state.overlay else {
        panic!("page menu open");
    };
    let layout = page_menu_layout(&state, SCREEN, menu);
    assert_eq!(layout.page_rects.len(), visible);
    assert_eq!(
        layout.area.height,
        u16::try_from(2 + 1 + visible * 2).expect("height"),
        "菜单高度钳到屏幕内"
    );
    let selected_rect = layout.page_rects[target - layout.start - 1];
    assert_eq!(
        page_menu_item_at(&state, SCREEN, menu, selected_rect.x + 1, selected_rect.y),
        Some(target),
        "高亮页行在窗口内可命中"
    );
    // 光标移到末行（Delete page）：窗口贴底。
    let last = menu.rows.len() - 1;
    assert!(crate::app::update::set_page_menu_selection(
        &mut state, last
    ));
    let Some(Overlay::PageMenu(menu)) = &state.overlay else {
        panic!("page menu open");
    };
    let layout = page_menu_layout(&state, SCREEN, menu);
    assert_eq!(layout.start, pages - visible, "窗口滚到尾部：最后一页可见");
    let bottom = layout.page_rects[visible - 1];
    assert_eq!(
        page_menu_item_at(&state, SCREEN, menu, bottom.x, bottom.y + 1),
        Some(pages),
        "末页（两行）可命中"
    );
}

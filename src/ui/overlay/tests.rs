//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::overlay::{
    ConfirmClose, ConfirmSwitchCwd, ConfirmSyncWorkspaceCwd, Menu, MenuCommand, NewNode,
    NewNodeKind, NewWorkspace, Overlay, OverlayTarget, ReferencedFolderMenuRow, Rename,
    RenameTarget, SaveDraft, SaveDraftEntry, TextInput,
};
use crate::app::referenced::ReferencedEntry;
use crate::app::trust::TrustWorkspace;
use crate::ui::style;
use crate::ui::text;
use crate::ui::widgets;

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
    let mut state = menu_state(
        vec![MenuCommand::RenameWorkspace, MenuCommand::CloseWorkspace],
        1,
    );
    state.menu_hover = true;
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

    // 指针离开容器：高亮反馈整体移除。
    state.menu_hover = false;
    if let Err(error) = terminal.draw(|frame| {
        render(frame, frame.area(), &state);
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(
        buffer[(11, 7)].bg,
        style::overlay_panel().bg.expect("panel bg"),
        "离开容器后无悬停高亮"
    );
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

fn referenced_menu_state(rows: Vec<ReferencedFolderMenuRow>, selected: usize) -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderMenu(
        crate::app::overlay::ReferencedFolderMenu {
            anchor: (4, 3),
            key: Some(PathBuf::from("/repo")),
            rows,
            selected,
        },
    ));
    state
}

#[test]
fn referenced_menu_renders_modal_rows_addresses_and_footer() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows = vec![
        ReferencedFolderMenuRow::Folder {
            id: 1,
            path: PathBuf::from("/tmp/alpha"),
        },
        ReferencedFolderMenuRow::Folder {
            id: 2,
            path: PathBuf::from("/tmp/work/beta"),
        },
        ReferencedFolderMenuRow::Add,
    ];
    let state = referenced_menu_state(rows, 0);
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(
        joined.contains(text::MENU_TITLE_REFERENCED_FOLDERS),
        "{joined}"
    );
    assert!(joined.contains(text::REFERENCED_MENU_ADD_ROW), "{joined}");
    assert!(joined.contains(text::REFERENCED_MENU_FOOTER), "{joined}");
    assert!(joined.contains("alpha"), "{joined}");
    assert!(joined.contains("beta"), "{joined}");
    // 记录行右端显示绝对路径（prompt 列表式右对齐地址）。
    assert!(joined.contains("/tmp/work"), "{joined}");
}

#[test]
fn referenced_menu_keeps_fixed_size_and_centers() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows = vec![
        ReferencedFolderMenuRow::Folder {
            id: 1,
            path: PathBuf::from("/tmp/alpha"),
        },
        ReferencedFolderMenuRow::Add,
    ];
    let few = referenced_menu_state(rows.clone(), 0);
    let many_rows: Vec<ReferencedFolderMenuRow> = (0..40)
        .map(|id| ReferencedFolderMenuRow::Folder {
            id,
            path: PathBuf::from(format!("/tmp/alpha-{id}")),
        })
        .chain(std::iter::once(ReferencedFolderMenuRow::Add))
        .collect();
    let many = referenced_menu_state(many_rows, 0);
    let layout_for = |state: &AppState| {
        let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
            panic!("menu");
        };
        referenced_menu_layout(SCREEN, menu)
    };
    let few = layout_for(&few);
    let many = layout_for(&many);
    assert_eq!(few.area, many.area, "弹窗几何不随记录数变化");
    assert!(few.area.x > SCREEN.x && few.area.right() < SCREEN.right());
    assert!(few.area.y > SCREEN.y && few.area.bottom() < SCREEN.bottom());
    assert_eq!(few.total, 2);
    assert_eq!(few.rows.len(), 2);
    assert_eq!(many.rows.len(), 16, "满窗口显示（扣除上下边框）");
}

#[test]
fn referenced_menu_row_hit_maps_rows_and_outside() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let rows = vec![
        ReferencedFolderMenuRow::Folder {
            id: 7,
            path: PathBuf::from("/tmp/alpha"),
        },
        ReferencedFolderMenuRow::Add,
    ];
    let state = referenced_menu_state(rows, 0);
    let menu = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderMenu(menu)) => menu,
        _ => panic!("menu"),
    };
    let layout = referenced_menu_layout(SCREEN, menu);
    let first = layout.rows[0];
    assert_eq!(
        referenced_menu_row_at(SCREEN, menu, first.x, first.y),
        Some(0)
    );
    assert_eq!(
        referenced_menu_row_at(SCREEN, menu, first.right() - 1, first.y),
        Some(0)
    );
    let add = layout.rows[1];
    assert_eq!(referenced_menu_row_at(SCREEN, menu, add.x, add.y), Some(1));
    // 列表下方空白与弹窗之外都不命中。
    let blank = Rect::new(first.x, add.y + 1, first.width, 1);
    assert_eq!(referenced_menu_row_at(SCREEN, menu, blank.x, blank.y), None);
    assert_eq!(referenced_menu_row_at(SCREEN, menu, 0, 0), None);
}

fn referenced_search_state(items: Vec<ReferencedEntry>, selected: usize) -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderSearch(
        crate::app::overlay::ReferencedFolderSearch {
            anchor: (0, 0),
            folder: 1,
            key: Some(PathBuf::from("/repo")),
            root: PathBuf::from("/tmp/alpha"),
            initial_root: PathBuf::from("/tmp/alpha"),
            stack: Vec::new(),
            query: TextInput::new(String::new()),
            entries: items.clone(),
            items,
            scan_truncated: false,
            truncated: false,
            selected,
            viewport: None,
            loading: false,
            failed: false,
            request: Some(1),
        },
    ));
    state
}

#[test]
fn referenced_search_renders_scope_query_items_and_footer() {
    let items = vec![
        ReferencedEntry {
            path: PathBuf::from("/tmp/alpha/docs/src"),
            relative: "docs/src".into(),
            name: "src".into(),
            is_directory: true,
        },
        ReferencedEntry {
            path: PathBuf::from("/tmp/alpha/docs/file.rs"),
            relative: "docs/file.rs".into(),
            name: "file.rs".into(),
            is_directory: false,
        },
    ];
    let state = referenced_search_state(items, 1);
    let (lines, cursor) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::REFERENCED_SEARCH_TITLE), "{joined}");
    assert!(joined.contains("/tmp/alpha"), "范围路径: {joined}");
    assert!(joined.contains("src/"), "目录行带尾斜杠: {joined}");
    assert!(joined.contains("file.rs"), "{joined}");
    // 两条候选在下一行显示地址（父目录绝对路径）。
    assert!(joined.contains("/tmp/alpha/docs"), "候选地址行: {joined}");
    assert!(
        joined.contains(text::WORKSPACE_TREE_LAST),
        "地址行带 └─ 前缀: {joined}"
    );
    assert!(
        joined.contains(text::REFERENCED_SEARCH_FOOTER),
        "底边快捷键: {joined}"
    );
    // 搜索框硬件光标可见。
    assert!(cursor.is_some());
}

#[test]
fn referenced_search_shows_entered_folder_top_right() {
    let items = vec![ReferencedEntry {
        path: PathBuf::from("/tmp/alpha/docs/file.rs"),
        relative: "docs/file.rs".into(),
        name: "file.rs".into(),
        is_directory: false,
    }];
    let mut state = referenced_search_state(items, 0);
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.root = PathBuf::from("/tmp/alpha/docs");
        search.stack = vec![PathBuf::from("/tmp/alpha")];
    }
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains("docs"), "顶边右端显示当前文件夹: {joined}");

    // 仍在初始根时不显示目录名后缀。
    let mut state = referenced_search_state(Vec::new(), 0);
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.initial_root = PathBuf::from("/tmp/alpha");
        search.root = PathBuf::from("/tmp/alpha");
    }
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::REFERENCED_SEARCH_TITLE));
}

#[test]
fn referenced_search_keeps_fixed_size_regardless_of_items() {
    let entry = |index: usize| ReferencedEntry {
        path: PathBuf::from(format!("/tmp/alpha/docs/file{index}.rs")),
        relative: format!("docs/file{index}.rs"),
        name: format!("file{index}.rs"),
        is_directory: false,
    };
    let layout_for = |count: usize| {
        let state = referenced_search_state((0..count).map(entry).collect(), 0);
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
            panic!("search");
        };
        referenced_search_layout(SCREEN, search).expect("layout")
    };
    let few = layout_for(1);
    let many = layout_for(60);
    assert_eq!(few.area, many.area, "弹窗几何不随候选数量变化");
    assert_eq!(few.visible(), 1);
    assert!(many.visible() > few.visible(), "候选多时窗口内条目更多");
    assert!(few.area.x > SCREEN.x && few.area.right() < SCREEN.right());
    assert!(few.area.y > SCREEN.y && few.area.bottom() < SCREEN.bottom());
}

#[test]
fn referenced_search_paints_solid_panel_background() {
    let items: Vec<ReferencedEntry> = (0..10)
        .map(|index| ReferencedEntry {
            path: PathBuf::from(format!("/tmp/alpha/docs/file{index}.rs")),
            relative: format!("docs/file{index}.rs"),
            name: format!("file{index}.rs"),
            is_directory: false,
        })
        .collect();
    let state = referenced_search_state(items, 0);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("backend");
    if let Err(error) = terminal.draw(|frame| {
        render(frame, frame.area(), &state);
    }) {
        panic!("draw failed: {error}");
    }
    let buf = terminal.backend().buffer().clone();
    let search = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search,
        _ => panic!("search"),
    };
    let layout = referenced_search_layout(SCREEN, search).expect("layout");
    let scrollbar = layout.scrollbar.expect("list overflows");
    // 范围路径文字、搜索框、非选中候选与滚动条都保持浮层底色，不透明。
    for (label, x, y) in [
        ("范围路径", layout.title.x, layout.title.y),
        ("范围路径第二个字符", layout.title.x + 1, layout.title.y),
        ("搜索框", layout.query.x, layout.query.y),
        ("非选中候选首行", layout.list.x, layout.list.y + 2),
        ("非选中候选地址行", layout.list.x + 2, layout.list.y + 3),
        ("滚动条轨道", scrollbar.track.x, scrollbar.track.y),
        ("滚动条 thumb", scrollbar.thumb.x, scrollbar.thumb.y),
    ] {
        assert_eq!(
            buf[(x, y)].bg,
            Color::Indexed(236),
            "{label} 应保持浮层底色"
        );
    }
    assert_eq!(
        buf[(layout.list.x, layout.list.y)].bg,
        Color::Indexed(240),
        "选中候选底色"
    );
    assert_eq!(
        buf[(layout.list.x + 2, layout.list.y + 1)].bg,
        Color::Indexed(240),
        "选中候选地址行底色"
    );
}

#[test]
fn referenced_search_hit_tests_copy_and_items() {
    let items = vec![
        ReferencedEntry {
            path: PathBuf::from("/tmp/alpha/file.rs"),
            relative: "file.rs".into(),
            name: "file.rs".into(),
            is_directory: false,
        },
        ReferencedEntry {
            path: PathBuf::from("/tmp/alpha/other.rs"),
            relative: "other.rs".into(),
            name: "other.rs".into(),
            is_directory: false,
        },
    ];
    let state = referenced_search_state(items, 0);
    let search = match state.overlay.as_ref() {
        Some(Overlay::ReferencedFolderSearch(search)) => search,
        _ => panic!("search"),
    };
    let layout = referenced_search_layout(SCREEN, search).expect("layout");
    assert!(referenced_search_copy_at(
        SCREEN,
        search,
        layout.copy_path.x,
        layout.copy_path.y
    ));
    // 每条候选占两行：首行与地址行都命中同一候选。
    assert_eq!(
        referenced_search_item_at(SCREEN, search, layout.list.x, layout.list.y),
        Some(0)
    );
    assert_eq!(
        referenced_search_item_at(SCREEN, search, layout.list.x, layout.list.y + 1),
        Some(0)
    );
    assert_eq!(
        referenced_search_item_at(SCREEN, search, layout.list.x, layout.list.y + 2),
        Some(1)
    );
    assert_eq!(
        referenced_search_item_at(SCREEN, search, layout.list.x, layout.area.bottom()),
        None
    );
}

#[test]
fn referenced_search_failed_and_empty_states_render_messages() {
    let mut state = referenced_search_state(Vec::new(), 0);
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.failed = true;
    }
    let (lines, _) = draw_overlay(&state);
    assert!(
        lines.join("\n").contains(text::REFERENCED_SEARCH_FAILED),
        "不可访问提示"
    );

    let state = referenced_search_state(Vec::new(), 0);
    let (lines, _) = draw_overlay(&state);
    assert!(lines.join("\n").contains(text::REFERENCED_SEARCH_EMPTY));
}

#[test]
fn referenced_input_renders_placeholder_and_error() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderInput(
        crate::app::overlay::ReferencedFolderInput {
            anchor: (0, 0),
            input: TextInput::new(""),
            key: Some(PathBuf::from("/repo")),
            validating: false,
            error: Some(text::REFERENCED_ERROR_DUPLICATE),
            request: None,
        },
    ));
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::REFERENCED_INPUT_TITLE), "{joined}");
    assert!(
        joined.contains(text::REFERENCED_INPUT_PLACEHOLDER),
        "{joined}"
    );
    assert!(
        joined.contains(text::REFERENCED_ERROR_DUPLICATE),
        "{joined}"
    );
}

#[test]
fn referenced_input_buttons_hit_their_cells() {
    let shell = referenced_input_shell(SCREEN).expect("input shell fits");
    let buttons = widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_CREATE, text::BUTTON_CLEAR, text::BUTTON_CANCEL],
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    assert_eq!(
        referenced_input_button_at(&shell, buttons[0].x, buttons[0].y),
        Some(ReferencedInputButton::Create)
    );
    assert_eq!(
        referenced_input_button_at(&shell, buttons[1].x, buttons[1].y),
        Some(ReferencedInputButton::Clear)
    );
    assert_eq!(
        referenced_input_button_at(&shell, buttons[2].x, buttons[2].y),
        Some(ReferencedInputButton::Cancel)
    );
    assert_eq!(
        referenced_input_button_at(&shell, shell.inner.x, shell.inner.y),
        None
    );
}

#[test]
fn referenced_input_renders_long_absolute_path_in_full() {
    let path = "/Users/yonah/projects/agent/lx-agent/src/ui/overlay/referenced";
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderInput(
        crate::app::overlay::ReferencedFolderInput {
            anchor: (0, 0),
            input: TextInput::new(path),
            key: Some(PathBuf::from("/repo")),
            validating: false,
            error: None,
            request: None,
        },
    ));
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(path), "长路径不被截断: {joined}");
}

#[test]
fn referenced_delete_dialog_renders_question_and_buttons() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ReferencedFolderDelete(
        crate::app::overlay::ReferencedFolderDelete {
            key: Some(PathBuf::from("/repo")),
            id: 1,
            path: PathBuf::from("/tmp/alpha"),
        },
    ));
    let (lines, _) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains(text::REFERENCED_DELETE_TITLE), "{joined}");
    assert!(joined.contains("alpha"), "{joined}");
    assert!(joined.contains(text::BUTTON_CONFIRM), "{joined}");
    assert!(joined.contains(text::BUTTON_CANCEL), "{joined}");
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
fn page_menu_renders_border_chips_single_line_pages_and_current_marker() {
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
    assert!(body.contains(text::MENU_TITLE_PAGES), "{body}");
    assert!(body.contains("Page 1"), "{body}");
    assert!(body.contains("Notes"), "{body}");
    // 单行页项：名称与 `n|m|x` 计数同行，数字带状态色。
    assert!(body.contains("1|0|0"), "逐页 todo 计数: {body}");
    assert!(body.contains("0|0|1"), "逐页 done 计数: {body}");
    assert!(!body.contains("todo 1"), "不再使用旧计数标签: {body}");

    let Some(Overlay::PageMenu(menu)) = &state.overlay else {
        panic!("page menu open");
    };
    let layout = page_menu_layout(&state, SCREEN, menu);
    assert_eq!(menu.selected, 1, "打开时光标停在当前页");
    // 按钮嵌在顶部边框右端：同行、贴右角、按钮之间有间隔。
    assert_eq!(layout.new_rect.y, layout.area.y);
    assert_eq!(layout.delete_rect.y, layout.area.y);
    assert_eq!(layout.delete_rect.right(), layout.area.right() - 1);
    assert!(layout.new_rect.right() < layout.delete_rect.x);
    let top: String = (layout.area.x..layout.area.right())
        .map(|x| buffer[(x, layout.area.y)].symbol())
        .collect();
    assert!(top.contains(text::MENU_PAGE_NEW), "{top:?}");
    assert!(top.contains(text::MENU_PAGE_DELETE), "{top:?}");
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

    // 每项一行：相邻页行 y 差 1，单行内可命中。
    let page = layout.page_rects[0];
    assert_eq!(page.height, 1);
    assert_eq!(layout.page_rects[1].y, page.y + 1);
    assert_eq!(
        page_menu_item_at(&state, SCREEN, menu, page.x + 1, page.y),
        Some(menu.selected)
    );

    // 计数数字用对应模板块状态色（非选中行：todo dim、run 黄、done 绿）。
    let notes = layout.page_rects[1];
    let counts_x = notes.right() - 5;
    let done_digit = &buffer[(counts_x + 4, notes.y)];
    assert_eq!(done_digit.symbol(), "1");
    assert_eq!(done_digit.fg, ratatui::style::Color::Green, "done 计数绿色");
    assert_eq!(buffer[(counts_x + 1, notes.y)].symbol(), "|", "计数分隔符");

    // 当前页行带 `❯` 标记；指针离开容器后整菜单无悬停底色。
    let current = layout.page_rects[menu.selected - 1];
    let row: String = (current.x..current.right())
        .map(|x| buffer[(x, current.y)].symbol())
        .collect();
    assert!(
        row.starts_with(&format!("{} ", text::AGENT_SELECTED_MARKER)),
        "当前页行首标记: {row:?}"
    );
    state.menu_hover = false;
    let buffer = draw_overlay_buffer(&state);
    assert_eq!(
        Some(buffer[(current.x, current.y)].bg),
        style::overlay_panel().bg,
        "指针离开容器：当前页行不铺颜色高亮"
    );
    let row: String = (current.x..current.right())
        .map(|x| buffer[(x, current.y)].symbol())
        .collect();
    assert!(row.starts_with(text::AGENT_SELECTED_MARKER), "{row:?}");
}

#[test]
fn page_menu_border_chips_never_highlight_and_gray_when_disabled() {
    use crate::app::prompt::PromptItem;
    let mut state = AppState::demo();
    let item = PromptItem::single(7, None, "single", "body");
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, 7));
    assert!(crate::app::update::open_prompt_page_menu(
        &mut state,
        (10, 4)
    ));
    // 悬停新建按钮、键盘光标也在其上：按钮不铺 hover 底色。
    state.menu_hover = true;
    let buffer = draw_overlay_buffer(&state);
    let Some(Overlay::PageMenu(menu)) = &state.overlay else {
        panic!("page menu open");
    };
    let layout = page_menu_layout(&state, SCREEN, menu);
    for rect in [layout.new_rect, layout.delete_rect] {
        assert_eq!(
            Some(buffer[(rect.x, rect.y)].bg),
            style::overlay_panel().bg,
            "边框按钮无 hover 底色"
        );
    }
    // 单页时删除按钮不可用：默认灰（前景 252 + dim），与工具栏撤销/重做一致。
    let disabled = &buffer[(layout.delete_rect.x, layout.delete_rect.y)];
    assert_eq!(
        disabled.fg,
        style::overlay_panel().fg.expect("panel fg"),
        "禁用按钮灰前景"
    );
    assert!(disabled.modifier.contains(ratatui::style::Modifier::DIM));
    let enabled = &buffer[(layout.new_rect.x, layout.new_rect.y)];
    assert_eq!(
        enabled.fg,
        style::accent().fg.expect("accent fg"),
        "可用按钮强调色"
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
        u16::try_from(2 + visible).expect("height"),
        "菜单高度钳到屏幕内"
    );
    let selected_rect = layout.page_rects[target - layout.start - 1];
    assert_eq!(
        page_menu_item_at(&state, SCREEN, menu, selected_rect.x + 1, selected_rect.y),
        Some(target),
        "高亮页行在窗口内可命中"
    );
    // 光标移到末行（删除按钮）：窗口贴底。
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
        page_menu_item_at(&state, SCREEN, menu, bottom.x, bottom.y),
        Some(pages),
        "末页可命中"
    );
}

#[test]
fn save_draft_empty_or_overflow_behavior() {
    let dialog = SaveDraft {
        workspace: 0,
        name: TextInput::new("test"),
        entries: Vec::new(),
        selected: 0,
    };

    let shell = save_draft_shell(SCREEN, 0).expect("shell exists for 0 entries");
    assert!(shell.inner.height >= 5);
    let max_rows = save_draft_max_rows(&shell);
    assert_eq!(save_draft_visible_start(&dialog, max_rows), 0);
    assert_eq!(
        save_draft_entry_at(&shell, &dialog, shell.inner.x, shell.inner.y + 2),
        None
    );

    let too_small = Rect::new(0, 0, 3, 2);
    assert!(save_draft_shell(too_small, 5).is_none());
}

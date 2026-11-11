//! `dialog` 浮层渲染与几何测试；仅测试构建编译。

use super::*;
use crate::app::overlay::{Overlay, TextInput};
use crate::app::state::{WorkspaceGit, WorkspaceHistoryEntry};
use crate::ui::style;
use crate::ui::text;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use std::path::PathBuf;

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
        cursor = crate::ui::overlay::render(frame, frame.area(), state);
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

fn rename_state(name: &str) -> AppState {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::Rename(Rename {
        target: RenameTarget::Workspace(0),
        input: TextInput::new(name),
    }));
    state
}

/// 工作区历史条目夹具；`git` 为所属仓库元数据（非 Git 目录为 None）。
fn history_entry(
    name: &str,
    path: &str,
    exists: bool,
    git: Option<WorkspaceGit>,
) -> WorkspaceHistoryEntry {
    WorkspaceHistoryEntry {
        name: name.into(),
        path: PathBuf::from(path),
        exists,
        name_is_manual: false,
        position: 0,
        trusted: true,
        git,
    }
}

/// Git 分组元数据：`repo_root` 同时作为 checkout 路径。
fn git_group(repo_root: &str, is_linked: bool) -> WorkspaceGit {
    WorkspaceGit {
        repo_root: PathBuf::from(repo_root),
        checkout_path: PathBuf::from(repo_root),
        is_linked,
        branch: Some("feature".into()),
        main_branch: Some("dev".into()),
    }
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
fn modal_uses_transparent_panel_and_surface_input_bar() {
    let state = rename_state("demo");
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| {
        crate::ui::overlay::render(frame, frame.area(), &state);
    }) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let shell = rename_shell(SCREEN).expect("rename modal fits");
    assert_eq!(buffer[(shell.area.x, shell.area.y)].bg, Color::Reset);
    assert_eq!(
        buffer[(shell.area.x + 1, shell.area.y + 1)].bg,
        Color::Reset
    );
    assert_eq!(
        buffer[(shell.inner.x, shell.inner.y)].bg,
        Color::Indexed(236),
        "输入行铺暗色 surface 底条"
    );
}

#[test]
fn rename_modal_renders_input_buttons_and_cursor() {
    let state = rename_state("workspace 2");
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, Some((34, 11)));
    assert!(lines[9].contains(text::RENAME_WORKSPACE_TITLE));
    assert!(
        lines[11].contains(&format!("{} workspace 2", text::INPUT_PREFIX_ICON)),
        "输入行带 > 前缀: {:?}",
        lines[11]
    );
    assert!(!lines[11].contains(text::RENAME_PLACEHOLDER));
    let buttons = lines[13].clone();
    assert!(buttons.contains(text::BUTTON_SAVE), "{buttons}");
    assert!(buttons.contains(text::BUTTON_CLEAR), "{buttons}");
    assert!(buttons.contains(text::BUTTON_CANCEL), "{buttons}");
}

#[test]
fn rename_modal_renders_placeholder_when_empty() {
    let state = rename_state("");
    let (lines, _) = draw_overlay(&state);
    assert!(
        lines[11].contains(text::RENAME_PLACEHOLDER),
        "row={:?}",
        lines[11]
    );
}

#[test]
fn rename_modal_keeps_cursor_visible_for_long_names() {
    let state = rename_state(&"x".repeat(60));
    let (lines, cursor) = draw_overlay(&state);
    let cursor = cursor.expect("cursor is set");
    assert!(cursor.0 < SCREEN.right());
    assert_eq!(cursor.0, 58);
    assert_eq!(cursor.1, 11);
    assert_eq!(lines[11].chars().filter(|ch| *ch == 'x').count(), 35);
}

#[test]
fn rename_modal_cursor_uses_display_width_for_wide_chars() {
    let state = rename_state("中文名称");
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, Some((31, 11)));
    let text: String = lines[11].chars().filter(|ch| !ch.is_whitespace()).collect();
    assert!(text.contains("中文名称"), "{text}");
}

#[test]
fn rename_modal_scrolls_by_display_width_for_long_wide_names() {
    let state = rename_state(&"中".repeat(30));
    let (lines, cursor) = draw_overlay(&state);
    assert_eq!(cursor, Some((57, 11)));
    assert_eq!(lines[11].matches('中').count(), 17);
}

#[test]
fn rename_buttons_hit_their_cells() {
    // 按真实按钮行几何取单元格：不写死列号，文案长度变化不会悄悄失效。
    let shell = rename_shell(SCREEN).expect("rename modal fits");
    let rects = crate::ui::widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_SAVE, text::BUTTON_CLEAR, text::BUTTON_CANCEL],
        crate::ui::overlay::BUTTON_GAP,
        crate::ui::overlay::RENAME_BUTTON_ROW,
    );
    let row = rects[0].y;
    assert_eq!(
        rename_button_at(&shell, rects[0].x, row),
        Some(RenameButton::Save)
    );
    assert_eq!(
        rename_button_at(&shell, rects[1].x, row),
        Some(RenameButton::Clear)
    );
    assert_eq!(
        rename_button_at(&shell, rects[2].x, row),
        Some(RenameButton::Cancel)
    );
    // 按钮行之外不命中。
    assert_eq!(rename_button_at(&shell, rects[0].x, row + 1), None);
}

#[test]
fn confirm_modal_renders_question_and_buttons() {
    let mut state = AppState::demo();
    let item = crate::app::prompt::PromptItem::single(1, None, "item", "body");
    let page_id = item.pages[0].id;
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::PromptPage {
            workspace: 0,
            owner: crate::app::prompt::PromptEditing::Item(1),
            page: page_id,
        },
    )));
    let (lines, cursor) = draw_overlay(&state);
    assert!(lines[9].contains(text::CONFIRM_DELETE_PAGE_TITLE));
    assert!(lines[11].contains("delete page \"Page 1\"?"));
    assert!(
        lines[12].contains(text::CONFIRM_DELETE_INPUT_PLACEHOLDER),
        "输入行显示口令占位: {:?}",
        lines[12]
    );
    assert_eq!(cursor, Some((15, 12)), "硬件光标落在口令输入行");
    let buttons = lines[14].clone();
    assert!(buttons.contains(text::BUTTON_CONFIRM), "{buttons}");
    assert!(buttons.contains(text::BUTTON_CANCEL), "{buttons}");
}

#[test]
fn confirm_buttons_hit_their_cells() {
    // 按真实按钮行几何取单元格：不写死列号。
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::Workspace(0),
    )));
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let shell = confirm_close_layout(&state, SCREEN, confirm)
        .map(|layout| layout.shell)
        .expect("confirm modal fits");
    // 确认浮层的按钮行在内容区最后一行（与重命名浮层的固定行号不同）。
    let offset = shell.inner.height.saturating_sub(1);
    let rects = crate::ui::widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_CONFIRM, text::BUTTON_CANCEL],
        crate::ui::overlay::BUTTON_GAP,
        offset,
    );
    let row = shell.inner.y + offset;
    assert_eq!(
        confirm_button_at(&shell, rects[0].x, row),
        Some(ConfirmButton::Confirm)
    );
    assert_eq!(
        confirm_button_at(&shell, rects[1].x, row),
        Some(ConfirmButton::Cancel)
    );
    assert_eq!(confirm_button_at(&shell, rects[0].x, row + 1), None);
}

#[test]
fn confirm_close_input_rect_sits_below_wrapped_question() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "demo with a very long name that wraps over multiple lines".into();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::Workspace(0),
    )));
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let question = confirm_question(&state, &confirm.target);
    let layout = confirm_close_layout(&state, SCREEN, confirm).expect("dialog fits");
    let shell = layout.shell;
    assert!(layout.list.is_none(), "工作区目标没有待删列表");
    let lines = text::wrap(&question, usize::from(shell.inner.width));
    assert!(lines.len() > 1, "问题必须换行以覆盖动态高度");
    let input = layout.input;
    assert_eq!(input.y, shell.inner.y + lines.len() as u16);
    assert_eq!(input.x, shell.inner.x);
    assert_eq!(input.width, shell.inner.width);
    assert!(
        input.y + 2 <= shell.inner.bottom(),
        "输入行下留空白与按钮行"
    );
}

#[test]
fn confirm_question_wraps_and_dialog_grows() {
    let mut state = AppState::demo();
    let name = "Page 11111 with a very long trailing name that cannot fit on one line";
    let page_id = crate::app::prompt::PromptPageId::Saved(1);
    state.workspaces[0].prompt_panel.tree.replace(
        vec![],
        vec![crate::app::prompt::PromptItem {
            id: 1,
            folder_id: None,
            name: "item".into(),
            pages: vec![crate::app::prompt::PromptPage {
                id: page_id,
                name: name.to_string(),
                content: "body".into(),
                updated_at: 0,
            }],
            active_page: page_id,
        }],
    );
    let target = OverlayTarget::PromptPage {
        workspace: 0,
        owner: crate::app::prompt::PromptEditing::Item(1),
        page: page_id,
    };
    let question = confirm_question(&state, &target);
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(target)));
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let shell = confirm_close_layout(&state, SCREEN, confirm)
        .map(|layout| layout.shell)
        .expect("dialog fits");
    assert_eq!(shell.area.width, 56, "确认弹窗固定 56 列");
    assert_eq!(
        text::wrap(&question, usize::from(shell.inner.width)).len(),
        2,
        "长工作区名在 54 列内容区仍换行成两行"
    );
    assert!(shell.area.height > 4, "高度随换行增长: {:?}", shell.area);
    let (lines, cursor) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(joined.contains("delete page \"Page 11111"), "{joined}");
    assert!(
        joined.contains("one line\"?"),
        "整段问题可见（自动换行）: {joined}"
    );
    assert!(
        joined.contains(text::CONFIRM_DELETE_INPUT_PLACEHOLDER),
        "问题换行后仍显示口令输入行: {joined}"
    );
    assert!(cursor.is_some(), "硬件光标在口令输入行");
}

#[test]
fn confirm_and_new_workspace_dialogs_width_is_56_on_wide_screen() {
    let screen = Rect::new(0, 0, 100, 24);
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::Workspace(0),
    )));
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let confirm = confirm_close_layout(&state, screen, confirm)
        .map(|layout| layout.shell)
        .expect("confirm fits");
    assert_eq!(confirm.area.width, 56);
    let workspace = new_workspace_shell(screen).expect("new workspace fits");
    assert_eq!(workspace.area.width, 56);
}

#[test]
fn confirm_button_switches_from_muted_to_danger_when_phrase_matches() {
    let mut state = AppState::demo();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::Workspace(0),
    )));
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let shell = confirm_close_layout(&state, SCREEN, confirm)
        .map(|layout| layout.shell)
        .expect("dialog fits");
    let rects = widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_CONFIRM, text::BUTTON_CANCEL],
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    let button_cell = |state: &AppState| {
        let mut terminal =
            Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
        if let Err(error) = terminal.draw(|frame| {
            crate::ui::overlay::render(frame, frame.area(), state);
        }) {
            panic!("draw failed: {error}");
        }
        let buffer = terminal.backend().buffer().clone();
        buffer[(rects[0].x, rects[0].y)].clone()
    };
    let idle = button_cell(&state);
    assert_eq!(idle.fg, Color::Reset, "未匹配时 Confirm 按钮为暗色禁用态");
    assert_eq!(idle.bg, Color::Reset, "未匹配时 Confirm 按钮无危险底色");
    assert!(idle.modifier.contains(Modifier::DIM));

    if let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_mut() {
        confirm.input = TextInput::new("  Confirm Delete ");
    }
    let confirmed = button_cell(&state);
    assert_eq!(confirmed.fg, style::button_danger().fg.unwrap());
    assert_eq!(confirmed.bg, style::button_danger().bg.unwrap());
    assert!(confirmed.modifier.contains(Modifier::BOLD));
}

#[test]
fn missing_workspaces_confirm_shows_bulk_question_and_count() {
    let mut state = AppState::demo();
    state.workspace_history.entries = (0..2)
        .map(|index| WorkspaceHistoryEntry {
            name: format!("gone-{index}"),
            path: PathBuf::from(format!("/tmp/gone-{index}")),
            exists: false,
            name_is_manual: false,
            position: index,
            trusted: false,
            git: None,
        })
        .collect();
    let target = OverlayTarget::MissingWorkspaces;
    assert_eq!(
        confirm_question(&state, &target),
        "delete 2 missing workspace(s) and all their data?"
    );
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(target)));
    let (lines, cursor) = draw_overlay(&state);
    let joined = lines.join("\n");
    assert!(
        joined.contains(text::CONFIRM_DELETE_MISSING_WORKSPACES_TITLE),
        "{joined}"
    );
    assert!(joined.contains("delete 2 missing workspace(s)"), "{joined}");
    assert!(joined.contains("their data?"), "{joined}");
    assert!(
        joined.contains(text::CONFIRM_DELETE_INPUT_PLACEHOLDER),
        "{joined}"
    );
    assert!(joined.contains(text::BUTTON_CONFIRM), "{joined}");
    assert!(joined.contains(text::BUTTON_CANCEL), "{joined}");
    assert!(cursor.is_some(), "批量清理确认弹窗同样接管硬件光标");
}

#[test]
fn confirm_delete_workspace_lists_target_and_linked_worktrees() {
    let mut state = AppState::demo();
    state.workspace_history.entries = vec![
        history_entry("demo", "/repo", true, Some(git_group("/repo", false))),
        history_entry("feature", "/repo/wt", true, Some(git_group("/repo", true))),
    ];
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::WorkspaceHistory(0),
    )));
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let question = confirm_question(&state, &confirm.target);
    let layout = confirm_close_layout(&state, SCREEN, confirm).expect("confirm modal fits");
    let list = layout.list.expect("cascade workspace list rendered");
    assert_eq!(layout.visible_rows, 2);
    assert_eq!(
        list.y,
        layout.shell.inner.y
            + text::wrap(&question, usize::from(layout.shell.inner.width)).len() as u16,
        "列表紧接问题文案下方"
    );
    assert!(layout.input.y > list.bottom(), "输入框在列表下方");
    assert!(layout.scrollbar.is_none(), "两行列表无需滚动条");

    let (lines, cursor) = draw_overlay(&state);
    assert!(
        lines[layout.shell.inner.y as usize].contains("delete workspace"),
        "问题位于列表上方: {lines:?}"
    );
    assert!(
        lines[list.y as usize].contains("• demo (/repo)"),
        "单个工作区行: {:?}",
        lines[list.y as usize]
    );
    assert!(
        lines[list.y as usize + 1].contains("• feature (/repo/wt)"),
        "级联 worktree 行: {:?}",
        lines[list.y as usize + 1]
    );
    assert!(
        lines[layout.input.y as usize].contains(text::CONFIRM_DELETE_INPUT_PLACEHOLDER),
        "列表与输入框互不覆盖: {:?}",
        lines[layout.input.y as usize]
    );
    assert!(cursor.is_some(), "硬件光标落在口令输入行");
}

#[test]
fn missing_workspaces_confirm_caps_list_scrolls_and_shows_scrollbar() {
    let mut state = AppState::demo();
    state.workspace_history.entries = (0..8)
        .map(|index| {
            let name = format!("gone-{index}");
            let path = format!("/tmp/gone-{index}");
            history_entry(&name, &path, false, None)
        })
        .collect();
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::MissingWorkspaces,
    )));
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let layout = confirm_close_layout(&state, SCREEN, confirm).expect("confirm modal fits");
    let list = layout.list.expect("missing workspace list rendered");
    assert_eq!(
        layout.visible_rows,
        crate::app::overlay::CONFIRM_DELETE_LIST_MAX_ROWS,
        "可见列表高度封顶 5 行"
    );
    assert_eq!(usize::from(list.height), layout.visible_rows);
    assert_eq!(layout.scroll, 0);
    assert!(layout.scrollbar.is_some(), "超过 5 行时显示滚动条");
    assert!(list.bottom() < layout.input.y, "输入框在列表下方");

    let (lines, _) = draw_overlay(&state);
    let first_view = lines[list.y as usize..list.bottom() as usize].join("\n");
    for index in 0..5 {
        assert!(
            first_view.contains(&format!("• gone-{index} ")),
            "第 {index} 个失效工作区可见: {first_view:?}"
        );
    }
    assert!(
        !first_view.contains("• gone-5 "),
        "第 6 行需滚动后才可见: {first_view:?}"
    );
    assert!(
        lines.join("\n").contains('▐'),
        "滚动条 thumb 已绘制: {lines:?}"
    );

    if let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_mut() {
        confirm.scroll = 3;
    }
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let layout = confirm_close_layout(&state, SCREEN, confirm).expect("confirm modal fits");
    assert_eq!(layout.scroll, 3, "滚动偏移传递到渲染几何");
    let (lines, _) = draw_overlay(&state);
    let scrolled_view = lines[list.y as usize..list.bottom() as usize].join("\n");
    assert!(
        scrolled_view.contains("• gone-3 "),
        "滚动后从第 4 个失效项开始: {scrolled_view:?}"
    );
    assert!(
        scrolled_view.contains("• gone-7 "),
        "最后一行进入视口: {scrolled_view:?}"
    );
    assert!(
        !scrolled_view.contains("• gone-2 "),
        "前 3 行已滚出视口: {scrolled_view:?}"
    );
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
    assert!(lines[11].contains("tab 2"));
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
        &OverlayTarget::PromptPage {
            workspace: 0,
            owner: crate::app::prompt::PromptEditing::Item(7),
            page: PromptPageId::Saved(70),
        },
    );
    assert_eq!(question, "delete page \"Notes\"?");
}

// ===== `/resume` 会话编辑模态 =====

/// 构造处于 `/resume` 编辑模态的状态。
fn resume_edit_state(confirming: bool, title: &str) -> AppState {
    use crate::app::overlay::ResumeSessionEdit;
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ResumeSessionEdit(ResumeSessionEdit {
        pane,
        workspace: std::path::PathBuf::from("/tmp/ws"),
        session_id: "sess-a".into(),
        input: TextInput::new(title),
        confirming_delete: confirming,
        hint: None,
    }));
    state
}

#[test]
fn resume_edit_modal_renders_input_and_buttons() {
    let state = resume_edit_state(false, "My session");
    let (lines, cursor) = draw_overlay(&state);
    let all = lines.join("\n");
    assert!(
        all.contains(crate::ui::text::RESUME_EDIT_MODAL_TITLE),
        "{all}"
    );
    assert!(all.contains("My session"), "显示标题正文: {all}");
    assert!(all.contains(crate::ui::text::BUTTON_SAVE), "{all}");
    assert!(all.contains(crate::ui::text::BUTTON_DELETE), "{all}");
    assert!(all.contains(crate::ui::text::BUTTON_CANCEL), "{all}");
    assert!(cursor.is_some(), "输入框有硬件光标");
}

#[test]
fn resume_edit_modal_confirm_state_swaps_buttons_and_shows_hint() {
    let state = resume_edit_state(true, "My session");
    let (lines, _) = draw_overlay(&state);
    let all = lines.join("\n");
    assert!(
        all.contains(crate::ui::text::BUTTON_CONFIRM_DELETE),
        "确认态主按钮: {all}"
    );
    assert!(
        !all.contains(crate::ui::text::BUTTON_DELETE),
        "不再显示编辑态删除按钮: {all}"
    );
    assert!(
        all.contains(crate::ui::text::RESUME_EDIT_CONFIRM_HINT),
        "显示二次确认提示: {all}"
    );
}

#[test]
fn resume_edit_modal_buttons_hit_primary_delete_and_cancel() {
    use crate::ui::overlay::ResumeEditButton;
    let shell = crate::ui::overlay::resume_edit_shell(SCREEN).expect("shell");
    // 编辑态：save / delete / cancel 三个按钮可命中。
    let row = shell.inner.y + crate::ui::overlay::RENAME_BUTTON_ROW;
    let rects = crate::ui::widgets::modal::button_row(
        shell.inner,
        &[
            crate::ui::text::BUTTON_SAVE,
            crate::ui::text::BUTTON_DELETE,
            crate::ui::text::BUTTON_CANCEL,
        ],
        crate::ui::overlay::BUTTON_GAP,
        crate::ui::overlay::RENAME_BUTTON_ROW,
    );
    assert_eq!(
        crate::ui::overlay::resume_edit_button_at(&shell, false, rects[0].x, row),
        Some(ResumeEditButton::Primary)
    );
    assert_eq!(
        crate::ui::overlay::resume_edit_button_at(&shell, false, rects[1].x, row),
        Some(ResumeEditButton::Delete)
    );
    assert_eq!(
        crate::ui::overlay::resume_edit_button_at(&shell, false, rects[2].x, row),
        Some(ResumeEditButton::Cancel)
    );
}

#[test]
fn resume_edit_modal_confirm_state_second_button_is_cancel() {
    use crate::ui::overlay::ResumeEditButton;
    let shell = crate::ui::overlay::resume_edit_shell(SCREEN).expect("shell");
    let row = shell.inner.y + crate::ui::overlay::RENAME_BUTTON_ROW;
    let rects = crate::ui::widgets::modal::button_row(
        shell.inner,
        &[
            crate::ui::text::BUTTON_CONFIRM_DELETE,
            crate::ui::text::BUTTON_CANCEL,
        ],
        crate::ui::overlay::BUTTON_GAP,
        crate::ui::overlay::RENAME_BUTTON_ROW,
    );
    assert_eq!(
        crate::ui::overlay::resume_edit_button_at(&shell, true, rects[0].x, row),
        Some(ResumeEditButton::Primary)
    );
    assert_eq!(
        crate::ui::overlay::resume_edit_button_at(&shell, true, rects[1].x, row),
        Some(ResumeEditButton::Cancel),
        "确认态第二个按钮是取消（退回编辑态）"
    );
}

#[test]
fn workspace_history_group_delete_question_lists_worktrees() {
    let mut state = AppState::demo();
    let repo = PathBuf::from("/repo");
    let child = repo.join(".worktrees/feat");
    state.workspace_history.entries = vec![
        WorkspaceHistoryEntry {
            name: "repo".into(),
            path: repo.clone(),
            exists: true,
            name_is_manual: false,
            position: 0,
            trusted: true,
            git: Some(WorkspaceGit {
                repo_root: repo.clone(),
                checkout_path: repo,
                is_linked: false,
                branch: Some("dev".into()),
                main_branch: Some("dev".into()),
            }),
        },
        WorkspaceHistoryEntry {
            name: "feat".into(),
            path: child.clone(),
            exists: true,
            name_is_manual: false,
            position: 1,
            trusted: true,
            git: Some(WorkspaceGit {
                repo_root: PathBuf::from("/repo"),
                checkout_path: child,
                is_linked: true,
                branch: Some("feat".into()),
                main_branch: Some("dev".into()),
            }),
        },
    ];

    let question = confirm_question(&state, &OverlayTarget::WorkspaceHistory(0));
    assert!(
        question.contains("1 linked worktree"),
        "父项删除必须明确提示同仓库子项: {question}"
    );
}

#[test]
fn confirm_modal_ellipsizes_wide_cjk_workspace_name_and_path_by_display_width() {
    let mut state = AppState::demo();
    let repo = format!("/tmp/{}", "非常长的仓库路径".repeat(8));
    let long_name = "超长工作区名称".repeat(8);
    let mut child_git = git_group(&repo, true);
    child_git.branch = None;
    state.workspace_history.entries = vec![
        history_entry("仓库", &repo, true, Some(git_group(&repo, false))),
        history_entry(&long_name, "/tmp/子工作树", true, Some(child_git)),
    ];
    state.overlay = Some(Overlay::ConfirmClose(ConfirmClose::new(
        OverlayTarget::WorkspaceHistory(0),
    )));
    let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_ref() else {
        panic!("confirm overlay expected");
    };
    let layout = confirm_close_layout(&state, SCREEN, confirm).expect("confirm modal fits");
    let list = layout.list.expect("cascade workspace list rendered");
    assert_eq!(layout.visible_rows, 2, "父项与 linked worktree 各占一行");
    assert!(layout.scrollbar.is_none(), "两行列表无需滚动条");

    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test backend is infallible");
    terminal
        .draw(|frame| {
            crate::ui::overlay::render(frame, frame.area(), &state);
        })
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer().clone();
    for row in 0..layout.visible_rows {
        let y = list.y + row as u16;
        // 取列表区内最后一个非空单元格：按字符数截断会把省略号推到右边界之外，
        // 被 Paragraph 硬裁后这里会退化成 CJK 字符；按列宽截断则以 `…` 收尾。
        let last = (list.x..list.right())
            .rev()
            .find(|&x| buffer[(x, y)].symbol() != " ")
            .expect("list row has rendered content");
        assert_eq!(
            buffer[(last, y)].symbol(),
            "…",
            "超出 54 列的 CJK 行必须保留省略号而非被硬裁: row={row}"
        );
    }
}

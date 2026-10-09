//! 浮层渲染：右键菜单、重命名与关闭确认；几何与命中供事件循环复用。

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::overlay::{
    ConfirmClose, ConfirmSwitchCwd, ConfirmSyncWorkspaceCwd, Menu, MenuCommand, NewWorkspace,
    Overlay, OverlayTarget, Rename, RenameTarget, WorktreeOpen, WorktreeOpenEntry, WorktreeStatus,
};
use crate::app::state::{AppState, tab_label};
use crate::ui::widgets;
use crate::ui::{style, text};

/// 重命名浮层尺寸（列，行）：输入行、空行与按钮行。
const RENAME_WIDTH: u16 = 40;
const RENAME_HEIGHT: u16 = 5;
/// 新建工作区浮层尺寸（列，行）：输入行、空行与按钮行。
const NEW_WORKSPACE_WIDTH: u16 = 44;
const NEW_WORKSPACE_HEIGHT: u16 = 5;
/// 关闭确认浮层尺寸（列，行）：问题行与按钮行。
const CONFIRM_WIDTH: u16 = 40;
const CONFIRM_HEIGHT: u16 = 4;
/// worktree 对话框宽度与高度范围（行）：条目两行一条 + 搜索/分隔/按钮与外框。
const WORKTREE_DIALOG_WIDTH: u16 = 56;
const WORKTREE_DIALOG_MIN_HEIGHT: u16 = 10;
const WORKTREE_DIALOG_MAX_HEIGHT: u16 = 20;
/// 按钮间距与所在内容行。
const BUTTON_GAP: u16 = 2;
const RENAME_BUTTON_ROW: u16 = 2;
const NEW_WORKSPACE_BUTTON_ROW: u16 = 2;
const CONFIRM_BUTTON_ROW: u16 = 1;
const RENAME_BUTTONS: [&str; 3] = [text::BUTTON_SAVE, text::BUTTON_CLEAR, text::BUTTON_CANCEL];
const NEW_WORKSPACE_BUTTONS: [&str; 3] =
    [text::BUTTON_CREATE, text::BUTTON_CLEAR, text::BUTTON_CANCEL];
const CONFIRM_BUTTONS: [&str; 2] = [text::BUTTON_CONFIRM, text::BUTTON_CANCEL];
const WORKTREE_DIALOG_BUTTONS: [&str; 2] = [text::BUTTON_OPEN, text::BUTTON_CANCEL];

/// 重命名浮层按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameButton {
    Save,
    Clear,
    Cancel,
}

/// 新建工作区浮层按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewWorkspaceButton {
    Create,
    Clear,
    Cancel,
}

/// 关闭确认浮层按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmButton {
    Confirm,
    Cancel,
}

/// worktree 对话框按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeDialogButton {
    Open,
    Cancel,
}

/// worktree 对话框几何；条目两行一条，高度随条目数夹取。
pub fn worktree_dialog_shell(
    screen: Rect,
    entry_count: usize,
) -> Option<widgets::modal::ModalShell> {
    let rows = entry_count.max(1).saturating_mul(2) as u16;
    let height = rows
        .saturating_add(6)
        .clamp(WORKTREE_DIALOG_MIN_HEIGHT, WORKTREE_DIALOG_MAX_HEIGHT);
    widgets::modal::layout(screen, WORKTREE_DIALOG_WIDTH, height)
}

/// 对话框列表可视条目数；每条两行。
pub fn worktree_dialog_max_rows(shell: &widgets::modal::ModalShell) -> usize {
    usize::from(shell.inner.height.saturating_sub(3)) / 2
}

/// 列表窗口起始条目位置：让选中项落在窗口内。
pub fn worktree_dialog_visible_start(dialog: &WorktreeOpen, max_rows: usize) -> usize {
    if max_rows == 0 {
        return 0;
    }
    let filtered = dialog.filtered_indices();
    let position = dialog
        .selected_entry_index()
        .and_then(|index| filtered.iter().position(|candidate| *candidate == index))
        .unwrap_or(0);
    position.saturating_add(1).saturating_sub(max_rows)
}

/// 命中对话框条目；返回条目索引（非过滤位置）。
pub fn worktree_dialog_entry_at(
    shell: &widgets::modal::ModalShell,
    dialog: &WorktreeOpen,
    column: u16,
    row: u16,
) -> Option<usize> {
    let list_y = shell.inner.y.saturating_add(2);
    if column < shell.inner.x || column >= shell.inner.right() || row < list_y {
        return None;
    }
    let max_rows = worktree_dialog_max_rows(shell);
    if max_rows == 0 {
        return None;
    }
    let offset = usize::from(row - list_y) / 2;
    if offset >= max_rows {
        return None;
    }
    let start = worktree_dialog_visible_start(dialog, max_rows);
    dialog.filtered_indices().get(start + offset).copied()
}

/// 命中对话框按钮。
pub fn worktree_dialog_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<WorktreeDialogButton> {
    let rects = widgets::modal::button_row(
        shell.inner,
        &WORKTREE_DIALOG_BUTTONS,
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(WorktreeDialogButton::Open),
        Some(1) => Some(WorktreeDialogButton::Cancel),
        _ => None,
    }
}

/// 菜单几何与文案；渲染与鼠标命中共用。
pub fn menu_layout(state: &AppState, screen: Rect, menu: &Menu) -> widgets::menu::MenuLayout {
    widgets::menu::layout(
        screen,
        menu.anchor,
        &menu_title(state, menu),
        &menu_labels(menu),
    )
}

/// 重命名浮层几何。
pub fn rename_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, RENAME_WIDTH, RENAME_HEIGHT)
}

/// 关闭确认浮层几何。
pub fn confirm_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, CONFIRM_WIDTH, CONFIRM_HEIGHT)
}

/// 切换工作区路径确认浮层几何：宽度随问题完整展开，仅在超出屏幕时夹取。
pub fn confirm_switch_cwd_shell(screen: Rect, path: &Path) -> Option<widgets::modal::ModalShell> {
    let question = text::confirm_switch_cwd_question(&path.display().to_string());
    let width = u16::try_from(question.width().saturating_add(2))
        .unwrap_or(u16::MAX)
        .max(CONFIRM_WIDTH)
        .min(screen.width);
    widgets::modal::layout(screen, width, CONFIRM_HEIGHT)
}

/// 同步当前工作区路径确认浮层几何：宽度随完整问题展开，最小 CONFIRM_WIDTH，仅在超出屏幕时夹取。
pub fn confirm_sync_workspace_cwd_shell(
    screen: Rect,
    path: &Path,
) -> Option<widgets::modal::ModalShell> {
    let question = format!("switch workspace path to \"{}\"?", path.display());
    let width = u16::try_from(question.width().saturating_add(2))
        .unwrap_or(u16::MAX)
        .max(CONFIRM_WIDTH)
        .min(screen.width);
    widgets::modal::layout(screen, width, CONFIRM_HEIGHT)
}

/// 命中重命名按钮。
pub fn rename_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<RenameButton> {
    let rects =
        widgets::modal::button_row(shell.inner, &RENAME_BUTTONS, BUTTON_GAP, RENAME_BUTTON_ROW);
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(RenameButton::Save),
        Some(1) => Some(RenameButton::Clear),
        Some(2) => Some(RenameButton::Cancel),
        _ => None,
    }
}

/// 新建工作区浮层几何。
pub fn new_workspace_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, NEW_WORKSPACE_WIDTH, NEW_WORKSPACE_HEIGHT)
}

/// 命中新建工作区按钮。
pub fn new_workspace_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<NewWorkspaceButton> {
    let rects = widgets::modal::button_row(
        shell.inner,
        &NEW_WORKSPACE_BUTTONS,
        BUTTON_GAP,
        RENAME_BUTTON_ROW,
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(NewWorkspaceButton::Create),
        Some(1) => Some(NewWorkspaceButton::Clear),
        Some(2) => Some(NewWorkspaceButton::Cancel),
        _ => None,
    }
}

/// 命中关闭确认按钮。
pub fn confirm_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<ConfirmButton> {
    let rects = widgets::modal::button_row(
        shell.inner,
        &CONFIRM_BUTTONS,
        BUTTON_GAP,
        CONFIRM_BUTTON_ROW,
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(ConfirmButton::Confirm),
        Some(1) => Some(ConfirmButton::Cancel),
        _ => None,
    }
}

/// 渲染当前浮层；返回需要同步的硬件光标位置（重命名输入或新建工作区路径输入）。
pub fn render(frame: &mut Frame<'_>, screen: Rect, state: &AppState) -> Option<(u16, u16)> {
    match state.overlay.as_ref()? {
        Overlay::Menu(menu) => {
            let labels = menu_labels(menu);
            let title = menu_title(state, menu);
            let layout = widgets::menu::layout(screen, menu.anchor, &title, &labels);
            widgets::menu::render(frame, &layout, &title, &labels, Some(menu.selected));
            None
        }
        Overlay::Rename(rename) => render_rename(frame, screen, rename),
        Overlay::NewWorkspace(new_ws) => render_new_workspace(frame, screen, new_ws),
        Overlay::ConfirmClose(confirm) => {
            render_confirm(frame, screen, state, confirm);
            None
        }
        Overlay::ConfirmSwitchCwd(confirm) => {
            render_confirm_switch_cwd(frame, screen, confirm);
            None
        }
        Overlay::ConfirmSyncWorkspaceCwd(confirm) => {
            render_confirm_sync_workspace_cwd(frame, screen, confirm);
            None
        }
        Overlay::WorktreeOpen(dialog) => render_worktree_open(frame, screen, dialog),
    }
}

/// 重命名浮层标题：按目标种类分派。
fn rename_title(target: RenameTarget) -> &'static str {
    match target {
        RenameTarget::Workspace(_) => text::RENAME_WORKSPACE_TITLE,
        RenameTarget::Tab { .. } => text::RENAME_TAB_TITLE,
    }
}

/// 关闭确认浮层标题与目标名：按目标种类分派。
fn confirm_text(state: &AppState, target: OverlayTarget) -> (&'static str, String) {
    match target {
        OverlayTarget::Workspace(index) => (
            text::CONFIRM_CLOSE_TITLE,
            state
                .workspaces
                .get(index)
                .map(|workspace| workspace.name.clone())
                .unwrap_or_default(),
        ),
        OverlayTarget::Tab { workspace, tab } => {
            let name = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.tabs.get(tab))
                .map(|tab_state| tab_label(tab, tab_state.name.as_deref()))
                .unwrap_or_default();
            (text::CONFIRM_CLOSE_TAB_TITLE, name)
        }
        OverlayTarget::Pane {
            workspace,
            tab,
            pane,
        } => {
            let name = state
                .workspaces
                .get(workspace)
                .and_then(|workspace| workspace.tabs.get(tab))
                .and_then(|tab_state| tab_state.pane(pane))
                .map(|pane_state| {
                    text::pane_title(
                        pane,
                        pane_state.terminal.title(),
                        pane_state.cwd_label.as_deref(),
                    )
                })
                .unwrap_or_default();
            (text::CONFIRM_CLOSE_PANE_TITLE, name)
        }
    }
}

/// 菜单命令到文案的映射；文案集中在 `ui/text.rs`。
fn menu_labels(menu: &Menu) -> Vec<&'static str> {
    menu.commands
        .iter()
        .map(|command| match command {
            MenuCommand::NewTab => text::MENU_NEW_TAB,
            MenuCommand::NewTerminal => text::MENU_NEW_TERMINAL,
            MenuCommand::RenameWorkspace => text::MENU_RENAME_WORKSPACE,
            MenuCommand::OpenWorktree => text::MENU_OPEN_WORKTREE,
            MenuCommand::OpenPrompt => text::MENU_OPEN_PROMPT,
            MenuCommand::CloseWorkspace => text::MENU_CLOSE_WORKSPACE,
            MenuCommand::RenameTab => text::MENU_RENAME_TAB,
            MenuCommand::CloseTab => text::MENU_CLOSE_TAB,
            MenuCommand::SplitRight => text::MENU_SPLIT_RIGHT,
            MenuCommand::SplitDown => text::MENU_SPLIT_DOWN,
            MenuCommand::SwitchToTerminal => text::MENU_SWITCH_TO_TERMINAL,
            MenuCommand::SwitchToLx => text::MENU_SWITCH_TO_LX,
            MenuCommand::SwitchToWorkspaceCwd => text::MENU_SWITCH_TO_WORKSPACE_CWD,
            MenuCommand::SyncWorkspaceToTerminalCwd => text::MENU_SYNC_WS_TO_TERMINAL_CWD,
            MenuCommand::ClosePane => text::MENU_CLOSE_PANE,
        })
        .collect()
}

/// 菜单边框标题：目标名优先，取不到时回退种类名。
fn menu_title(state: &AppState, menu: &Menu) -> String {
    match menu.target {
        OverlayTarget::Workspace(index) => state
            .workspaces
            .get(index)
            .map(|workspace| workspace.name.clone())
            .unwrap_or_else(|| text::MENU_TITLE_WORKSPACE.to_string()),
        OverlayTarget::Tab { workspace, tab } => state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.tabs.get(tab))
            .map(|tab_state| tab_label(tab, tab_state.name.as_deref()))
            .unwrap_or_else(|| text::MENU_TITLE_TAB.to_string()),
        OverlayTarget::Pane {
            workspace,
            tab,
            pane,
        } => state
            .workspaces
            .get(workspace)
            .and_then(|workspace| workspace.tabs.get(tab))
            .and_then(|tab_state| tab_state.pane(pane))
            .map(|pane_state| {
                text::pane_title(
                    pane,
                    pane_state.terminal.title(),
                    pane_state.cwd_label.as_deref(),
                )
            })
            .unwrap_or_else(|| text::MENU_TITLE_PANE.to_string()),
    }
}

/// 重命名浮层：单行输入 + 底部按钮；输入视口与光标由公共单行输入组件承担。
fn render_rename(frame: &mut Frame<'_>, screen: Rect, rename: &Rename) -> Option<(u16, u16)> {
    let shell = rename_shell(screen)?;
    widgets::modal::render(frame, &shell, rename_title(rename.target));
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    render_buttons(
        frame,
        &widgets::modal::button_row(shell.inner, &RENAME_BUTTONS, BUTTON_GAP, RENAME_BUTTON_ROW),
        &[
            (text::BUTTON_SAVE, style::accent()),
            (text::BUTTON_CLEAR, style::muted()),
            (text::BUTTON_CANCEL, style::muted()),
        ],
    );
    widgets::input::render(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        rename.input.text(),
        rename.input.cursor(),
    )
}

/// 新建工作区浮层：单行路径输入 + 底部按钮；输入视口与光标由公共单行输入组件承担。
fn render_new_workspace(
    frame: &mut Frame<'_>,
    screen: Rect,
    new_ws: &NewWorkspace,
) -> Option<(u16, u16)> {
    let shell = new_workspace_shell(screen)?;
    widgets::modal::render(frame, &shell, text::NEW_WORKSPACE_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &NEW_WORKSPACE_BUTTONS,
            BUTTON_GAP,
            NEW_WORKSPACE_BUTTON_ROW,
        ),
        &[
            (text::BUTTON_CREATE, style::accent()),
            (text::BUTTON_CLEAR, style::muted()),
            (text::BUTTON_CANCEL, style::muted()),
        ],
    );
    widgets::input::render(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        new_ws.input.text(),
        new_ws.input.cursor(),
    )
}

/// 关闭确认浮层：问题与底部按钮。
fn render_confirm(frame: &mut Frame<'_>, screen: Rect, state: &AppState, confirm: &ConfirmClose) {
    let Some(shell) = confirm_shell(screen) else {
        return;
    };
    let (title, name) = confirm_text(state, confirm.target);
    render_confirm_dialog(frame, &shell, title, &text::confirm_close_question(&name));
}

/// 切换工作区路径确认浮层：完整展示目标路径与底部按钮。
fn render_confirm_switch_cwd(frame: &mut Frame<'_>, screen: Rect, confirm: &ConfirmSwitchCwd) {
    let Some(shell) = confirm_switch_cwd_shell(screen, &confirm.path) else {
        return;
    };
    let question = text::confirm_switch_cwd_question(&confirm.path.display().to_string());
    render_confirm_dialog(frame, &shell, text::CONFIRM_SWITCH_CWD_TITLE, &question);
}

/// 同步当前工作区路径确认浮层：展示目标路径（超出截断）与底部按钮。
fn render_confirm_sync_workspace_cwd(
    frame: &mut Frame<'_>,
    screen: Rect,
    confirm: &ConfirmSyncWorkspaceCwd,
) {
    let Some(shell) = confirm_sync_workspace_cwd_shell(screen, &confirm.path) else {
        return;
    };
    let width = usize::from(shell.inner.width);
    let question = text::confirm_sync_ws_cwd_question(&confirm.path.display().to_string(), width);
    render_confirm_dialog(frame, &shell, text::CONFIRM_SYNC_WS_CWD_TITLE, &question);
}

/// 确认类浮层通用渲染：标题、问题行与底部按钮。
fn render_confirm_dialog(
    frame: &mut Frame<'_>,
    shell: &widgets::modal::ModalShell,
    title: &str,
    question: &str,
) {
    widgets::modal::render(frame, shell, title);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return;
    }
    let width = usize::from(shell.inner.width);
    let question_area = Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text::ellipsize(question, width),
            style::text(),
        ))),
        question_area,
    );
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &CONFIRM_BUTTONS,
            BUTTON_GAP,
            CONFIRM_BUTTON_ROW,
        ),
        &[
            (text::BUTTON_CONFIRM, style::accent()),
            (text::BUTTON_CANCEL, style::muted()),
        ],
    );
}

/// 渲染按钮行；矩形与文案一一对应。
fn render_buttons(
    frame: &mut Frame<'_>,
    rects: &[Rect],
    buttons: &[(&str, ratatui::style::Style)],
) {
    for (rect, (label, button_style)) in rects.iter().zip(buttons) {
        frame.render_widget(Paragraph::new(Span::styled(*label, *button_style)), *rect);
    }
}

/// worktree 对话框：搜索行、分隔线、两行条目与底部按钮；返回搜索光标位置。
fn render_worktree_open(
    frame: &mut Frame<'_>,
    screen: Rect,
    dialog: &WorktreeOpen,
) -> Option<(u16, u16)> {
    let shell = worktree_dialog_shell(screen, dialog.entries.len())?;
    widgets::modal::render(frame, &shell, text::WORKTREE_OPEN_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let search_area = Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1);
    if dialog.query.text().is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(text::WORKTREE_OPEN_FILTER, style::muted())),
            search_area,
        );
    }
    let cursor = widgets::input::render(
        frame,
        search_area,
        dialog.query.text(),
        dialog.query.cursor(),
    );
    let separator_area = Rect::new(
        shell.inner.x,
        shell.inner.y.saturating_add(1),
        shell.inner.width,
        1,
    );
    frame.render_widget(
        Paragraph::new(text::DIVIDER_MID.repeat(usize::from(shell.inner.width)))
            .style(style::border(false)),
        separator_area,
    );

    let list_y = shell.inner.y.saturating_add(2);
    let list_area = Rect::new(
        shell.inner.x,
        list_y,
        shell.inner.width,
        shell.inner.height.saturating_sub(3),
    );
    let message = if dialog.loading {
        Some(text::WORKTREE_OPEN_LOADING)
    } else if dialog.failed {
        Some(text::WORKTREE_OPEN_FAILED)
    } else if dialog.filtered_indices().is_empty() {
        Some(text::WORKTREE_OPEN_EMPTY)
    } else {
        None
    };
    if let Some(message) = message {
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {message}"), style::muted())),
            Rect {
                height: 1,
                ..list_area
            },
        );
    } else {
        let max_rows = worktree_dialog_max_rows(&shell);
        let start = worktree_dialog_visible_start(dialog, max_rows);
        let selected = dialog.selected_entry_index();
        for (visible, entry_index) in dialog
            .filtered_indices()
            .iter()
            .skip(start)
            .take(max_rows)
            .enumerate()
        {
            let Some(entry) = dialog.entries.get(*entry_index) else {
                continue;
            };
            let y = list_y.saturating_add((visible as u16).saturating_mul(2));
            render_worktree_entry(frame, shell.inner, y, entry, Some(*entry_index) == selected);
        }
    }

    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &WORKTREE_DIALOG_BUTTONS,
            BUTTON_GAP,
            shell.inner.height.saturating_sub(1),
        ),
        &[
            (text::BUTTON_OPEN, style::accent()),
            (text::BUTTON_CANCEL, style::muted()),
        ],
    );
    cursor
}

/// 单条 worktree：首行 `› 名字` 加右对齐状态，次行缩进路径。
fn render_worktree_entry(
    frame: &mut Frame<'_>,
    inner: Rect,
    y: u16,
    entry: &WorktreeOpenEntry,
    selected: bool,
) {
    let width = usize::from(inner.width);
    let marker = if selected {
        text::WORKTREE_OPEN_MARKER
    } else {
        " "
    };
    let name_style = if selected {
        style::accent()
    } else {
        style::text()
    };
    let status = match entry.status() {
        WorktreeStatus::Open => text::WORKTREE_STATUS_OPEN,
        WorktreeStatus::Detached => text::WORKTREE_STATUS_DETACHED,
        WorktreeStatus::Root => text::WORKTREE_STATUS_ROOT,
        WorktreeStatus::Branch => "",
    };
    let prefix = format!("{marker} ");
    let name_width = width
        .saturating_sub(prefix.chars().count())
        .saturating_sub(status.chars().count())
        .saturating_sub(1);
    let title = format!(
        "{prefix}{}",
        text::ellipsize(&entry.display_name(), name_width)
    );
    let pad = width
        .saturating_sub(title.chars().count())
        .saturating_sub(status.chars().count())
        .max(1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(title, name_style),
            Span::styled(" ".repeat(pad), name_style),
            Span::styled(status, style::muted()),
        ])),
        Rect::new(inner.x, y, inner.width, 1),
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::ellipsize(&format!("  {}", entry.path.display()), width),
            style::muted(),
        )),
        Rect::new(inner.x, y.saturating_add(1), inner.width, 1),
    );
}

#[cfg(test)]
mod tests;

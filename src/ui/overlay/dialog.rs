//! 模态输入与确认对话框几何、命中与渲染。

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::overlay::{
    ConfirmClose, ConfirmSwitchCwd, ConfirmSyncWorkspaceCwd, NewNode, NewNodeKind, NewWorkspace,
    OverlayTarget, Rename, RenameTarget,
};
use crate::app::state::{AppState, tab_label};
use crate::app::trust::TrustWorkspace;
use crate::ui::widgets;
use crate::ui::{style, text};

use super::menu::{prompt_folder_name, prompt_item_name, prompt_page_name};

/// 重命名浮层尺寸（列，行）：输入行、空行与按钮行。
const RENAME_WIDTH: u16 = 40;
const RENAME_HEIGHT: u16 = 5;
/// 新建节点浮层尺寸（列，行）：输入行、空行与按钮行（容纳三个按钮）。
const NEW_NODE_WIDTH: u16 = 42;
const NEW_NODE_HEIGHT: u16 = 5;
/// 新建工作区浮层尺寸（列，行）：输入行、空行与按钮行。
const NEW_WORKSPACE_WIDTH: u16 = 44;
const NEW_WORKSPACE_HEIGHT: u16 = 5;
/// 关闭确认浮层最小宽度（列）；高度 = 问题换行行数 + 按钮行 + 上下边框。
pub(crate) const CONFIRM_WIDTH: u16 = 40;
/// 信任浮层尺寸（列，行）：问题行、路径行、空行与按钮行。
const TRUST_WIDTH: u16 = 52;
const TRUST_HEIGHT: u16 = 6;
/// 按钮间距与所在内容行。
pub(crate) const BUTTON_GAP: u16 = 2;
pub(crate) const RENAME_BUTTON_ROW: u16 = 2;
pub(crate) const NEW_WORKSPACE_BUTTON_ROW: u16 = 2;

const RENAME_BUTTONS: [&str; 3] = [text::BUTTON_SAVE, text::BUTTON_CLEAR, text::BUTTON_CANCEL];
/// `/resume` 会话编辑模态尺寸与按钮组（编辑态 3 个、确认删除态 2 个）。
const RESUME_EDIT_WIDTH: u16 = 48;
const RESUME_EDIT_HEIGHT: u16 = 5;
const RESUME_EDIT_BUTTONS: [&str; 3] =
    [text::BUTTON_SAVE, text::BUTTON_DELETE, text::BUTTON_CANCEL];
const RESUME_EDIT_CONFIRM_BUTTONS: [&str; 2] = [text::BUTTON_CONFIRM_DELETE, text::BUTTON_CANCEL];
pub(crate) const NEW_NODE_BUTTONS: [&str; 3] =
    [text::BUTTON_CREATE, text::BUTTON_CLEAR, text::BUTTON_CANCEL];
pub(crate) const NEW_WORKSPACE_BUTTONS: [&str; 3] =
    [text::BUTTON_CREATE, text::BUTTON_CLEAR, text::BUTTON_CANCEL];
const CONFIRM_BUTTONS: [&str; 2] = [text::BUTTON_CONFIRM, text::BUTTON_CANCEL];
pub(crate) const TRUST_BUTTONS: [&str; 2] = [text::BUTTON_TRUST, text::BUTTON_DONT_TRUST];
/// 信任浮层按钮行号（相对内容区）。
pub(crate) const TRUST_BUTTON_ROW: u16 = 3;

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

/// 新建节点浮层按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewNodeButton {
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

/// 信任浮层按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustButton {
    Trust,
    Refuse,
}

/// 重命名浮层几何。
pub fn rename_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, RENAME_WIDTH, RENAME_HEIGHT)
}

/// 确认类浮层几何：宽度夹取到屏幕，高度随问题换行行数增长（按钮行 + 上下边框）。
fn confirm_shell_for(
    screen: Rect,
    question: &str,
    width: u16,
) -> Option<widgets::modal::ModalShell> {
    let width = width.min(screen.width);
    let inner_width = usize::from(width.saturating_sub(2)).max(1);
    let lines = text::wrap(question, inner_width).len().max(1);
    let height = u16::try_from(lines)
        .unwrap_or(u16::MAX)
        .saturating_add(3)
        .min(screen.height);
    widgets::modal::layout(screen, width, height)
}

/// 关闭/删除确认浮层几何：固定宽度，问题按内容自动换行。
pub fn confirm_shell(screen: Rect, question: &str) -> Option<widgets::modal::ModalShell> {
    confirm_shell_for(screen, question, CONFIRM_WIDTH)
}

/// 路径确认浮层宽度：随问题完整展开，超出屏幕时夹取（再由换行承接）。
fn path_confirm_width(screen: Rect, question: &str) -> u16 {
    u16::try_from(question.width().saturating_add(2))
        .unwrap_or(u16::MAX)
        .max(CONFIRM_WIDTH)
        .min(screen.width)
}

/// 切换工作区路径确认浮层几何：宽度随问题完整展开，超出屏幕时换行。
pub fn confirm_switch_cwd_shell(screen: Rect, path: &Path) -> Option<widgets::modal::ModalShell> {
    let question = text::confirm_switch_cwd_question(&path.display().to_string());
    confirm_shell_for(screen, &question, path_confirm_width(screen, &question))
}

/// 同步当前工作区路径确认浮层几何：宽度随完整问题展开，超出屏幕时换行。
pub fn confirm_sync_workspace_cwd_shell(
    screen: Rect,
    path: &Path,
) -> Option<widgets::modal::ModalShell> {
    let question = text::confirm_sync_ws_cwd_question(&path.display().to_string());
    confirm_shell_for(screen, &question, path_confirm_width(screen, &question))
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

/// 新建节点浮层几何。
pub fn new_node_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, NEW_NODE_WIDTH, NEW_NODE_HEIGHT)
}

/// 命中新建节点按钮。
pub fn new_node_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<NewNodeButton> {
    let rects = widgets::modal::button_row(
        shell.inner,
        &NEW_NODE_BUTTONS,
        BUTTON_GAP,
        RENAME_BUTTON_ROW,
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(NewNodeButton::Create),
        Some(1) => Some(NewNodeButton::Clear),
        Some(2) => Some(NewNodeButton::Cancel),
        _ => None,
    }
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
        shell.inner.height.saturating_sub(1),
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(ConfirmButton::Confirm),
        Some(1) => Some(ConfirmButton::Cancel),
        _ => None,
    }
}

/// 信任浮层几何。
pub fn trust_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, TRUST_WIDTH, TRUST_HEIGHT)
}

/// 命中信任浮层按钮。
pub fn trust_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<TrustButton> {
    let rects =
        widgets::modal::button_row(shell.inner, &TRUST_BUTTONS, BUTTON_GAP, TRUST_BUTTON_ROW);
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(TrustButton::Trust),
        Some(1) => Some(TrustButton::Refuse),
        _ => None,
    }
}

/// 重命名浮层标题：按目标种类分派。
fn rename_title(target: RenameTarget) -> &'static str {
    match target {
        RenameTarget::Workspace(_) => text::RENAME_WORKSPACE_TITLE,
        RenameTarget::Tab { .. } => text::RENAME_TAB_TITLE,
        RenameTarget::PromptFolder { .. } => text::RENAME_FOLDER_TITLE,
        RenameTarget::PromptItem { .. } => text::RENAME_PROMPT_TITLE,
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
        OverlayTarget::PromptFolder {
            workspace,
            folder_id,
        } => (
            text::CONFIRM_DELETE_FOLDER_TITLE,
            prompt_folder_name(state, workspace, folder_id).unwrap_or_default(),
        ),
        OverlayTarget::PromptItem { workspace, item_id } => (
            text::CONFIRM_DELETE_PROMPT_TITLE,
            prompt_item_name(state, workspace, item_id).unwrap_or_default(),
        ),
        OverlayTarget::PromptPage {
            workspace,
            owner,
            page,
        } => (
            text::CONFIRM_DELETE_PAGE_TITLE,
            prompt_page_name(state, workspace, owner, page).unwrap_or_default(),
        ),
        OverlayTarget::TempPrompt { .. } => (text::CONFIRM_CLOSE_TITLE, String::new()),
        OverlayTarget::PromptTree { .. } => (text::CONFIRM_CLOSE_TITLE, String::new()),
        OverlayTarget::WorkspaceList => (text::CONFIRM_CLOSE_TITLE, String::new()),
    }
}

/// 关闭/删除确认浮层的问题文案。
pub fn confirm_question(state: &AppState, target: OverlayTarget) -> String {
    let (_, name) = confirm_text(state, target);
    match target {
        OverlayTarget::PromptFolder { .. } => text::confirm_delete_folder_question(&name),
        OverlayTarget::PromptItem { .. } => text::confirm_delete_question(&name),
        OverlayTarget::PromptPage { .. } => text::confirm_delete_page_question(&name),
        _ => text::confirm_close_question(&name),
    }
}

/// 重命名浮层：单行输入 + 底部按钮；输入视口与光标由公共单行输入组件承担。
pub(crate) fn render_rename(
    frame: &mut Frame<'_>,
    screen: Rect,
    rename: &Rename,
) -> Option<(u16, u16)> {
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
    render_input(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        rename.input.text(),
        rename.input.cursor(),
        rename.input.selection(),
        Some(text::RENAME_PLACEHOLDER),
    )
}

/// 新建工作区浮层：单行路径输入 + 底部按钮；输入视口与光标由公共单行输入组件承担。
pub(crate) fn render_new_workspace(
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
    render_input(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        new_ws.input.text(),
        new_ws.input.cursor(),
        new_ws.input.selection(),
        Some(text::NEW_WORKSPACE_PLACEHOLDER),
    )
}

/// 新建节点浮层：单行名称输入 + 底部按钮；标题与占位按节点种类分派。
pub(crate) fn render_new_node(
    frame: &mut Frame<'_>,
    screen: Rect,
    node: &NewNode,
) -> Option<(u16, u16)> {
    let shell = new_node_shell(screen)?;
    let (title, placeholder) = match node.kind {
        NewNodeKind::Folder { .. } => (text::NEW_FOLDER_TITLE, text::PROMPT_FOLDER_DEFAULT_NAME),
        NewNodeKind::Prompt { .. } => (text::NEW_PROMPT_TITLE, text::PROMPT_DEFAULT_NAME),
    };
    widgets::modal::render(frame, &shell, title);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &NEW_NODE_BUTTONS,
            BUTTON_GAP,
            RENAME_BUTTON_ROW,
        ),
        &[
            (text::BUTTON_CREATE, style::accent()),
            (text::BUTTON_CLEAR, style::muted()),
            (text::BUTTON_CANCEL, style::muted()),
        ],
    );
    render_input(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        node.name.text(),
        node.name.cursor(),
        node.name.selection(),
        Some(placeholder),
    )
}

/// 关闭确认浮层：问题与底部按钮；提示词删除按目标种类使用对应问题文案。
pub(crate) fn render_confirm(
    frame: &mut Frame<'_>,
    screen: Rect,
    state: &AppState,
    confirm: &ConfirmClose,
) {
    let question = confirm_question(state, confirm.target);
    let Some(shell) = confirm_shell(screen, &question) else {
        return;
    };
    let (title, _) = confirm_text(state, confirm.target);
    render_confirm_dialog(frame, &shell, title, &question);
}

/// 切换工作区路径确认浮层：完整展示目标路径与底部按钮。
pub(crate) fn render_confirm_switch_cwd(
    frame: &mut Frame<'_>,
    screen: Rect,
    confirm: &ConfirmSwitchCwd,
) {
    let Some(shell) = confirm_switch_cwd_shell(screen, &confirm.path) else {
        return;
    };
    let question = text::confirm_switch_cwd_question(&confirm.path.display().to_string());
    render_confirm_dialog(frame, &shell, text::CONFIRM_SWITCH_CWD_TITLE, &question);
}

/// 同步当前工作区路径确认浮层：展示目标路径（超出截断）与底部按钮。
pub(crate) fn render_confirm_sync_workspace_cwd(
    frame: &mut Frame<'_>,
    screen: Rect,
    confirm: &ConfirmSyncWorkspaceCwd,
) {
    let question = text::confirm_sync_ws_cwd_question(&confirm.path.display().to_string());
    let Some(shell) = confirm_sync_workspace_cwd_shell(screen, &confirm.path) else {
        return;
    };
    render_confirm_dialog(frame, &shell, text::CONFIRM_SYNC_WS_CWD_TITLE, &question);
}

/// 引用文件夹删除确认浮层：展示待删目录名与底部按钮。
pub(crate) fn render_referenced_delete(
    frame: &mut Frame<'_>,
    screen: Rect,
    delete: &crate::app::overlay::ReferencedFolderDelete,
) {
    let name = delete
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| delete.path.display().to_string());
    let question = text::referenced_delete_question(&name);
    let Some(shell) = confirm_shell(screen, &question) else {
        return;
    };
    render_confirm_dialog(frame, &shell, text::REFERENCED_DELETE_TITLE, &question);
}

/// 渲染信任浮层：问题、路径高亮与底部按钮。
pub(crate) fn render_trust(frame: &mut Frame<'_>, screen: Rect, trust: &TrustWorkspace) {
    let Some(shell) = trust_shell(screen) else {
        return;
    };
    widgets::modal::render(frame, &shell, text::TRUST_WORKSPACE_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return;
    }
    let width = usize::from(shell.inner.width);
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::ellipsize(text::TRUST_WORKSPACE_QUESTION, width),
            style::text(),
        )),
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
    );
    let path = text::ellipsize_middle(&trust.path.display().to_string(), width);
    frame.render_widget(
        Paragraph::new(Span::styled(path, style::accent())),
        Rect::new(
            shell.inner.x,
            shell.inner.y.saturating_add(1),
            shell.inner.width,
            1,
        ),
    );
    render_buttons(
        frame,
        &widgets::modal::button_row(shell.inner, &TRUST_BUTTONS, BUTTON_GAP, TRUST_BUTTON_ROW),
        &[
            (text::BUTTON_TRUST, style::accent()),
            (text::BUTTON_DONT_TRUST, style::muted()),
        ],
    );
}

/// 确认类浮层通用渲染：标题、自动换行的问题与底部按钮；问题行数超出可用高度时
/// 末行以 `…` 截断提示。
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
    let lines = text::wrap(question, width);
    let max_rows = usize::from(shell.inner.height.saturating_sub(1));
    for (index, line) in lines.iter().take(max_rows).enumerate() {
        let clipped = lines.len() > max_rows && index + 1 == max_rows;
        let visible = if clipped {
            text::ellipsize(&format!("{line}…"), width)
        } else {
            line.clone()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(visible, style::text())),
            Rect::new(
                shell.inner.x,
                shell.inner.y.saturating_add(index as u16),
                shell.inner.width,
                1,
            ),
        );
    }
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &CONFIRM_BUTTONS,
            BUTTON_GAP,
            shell.inner.height.saturating_sub(1),
        ),
        &[
            (text::BUTTON_CONFIRM, style::accent()),
            (text::BUTTON_CANCEL, style::muted()),
        ],
    );
}

/// 渲染按钮行；矩形与文案一一对应。
pub(crate) fn render_buttons(
    frame: &mut Frame<'_>,
    rects: &[Rect],
    buttons: &[(&str, ratatui::style::Style)],
) {
    for (rect, (label, button_style)) in rects.iter().zip(buttons) {
        frame.render_widget(Paragraph::new(Span::styled(*label, *button_style)), *rect);
    }
}

/// `/resume` 会话编辑模态几何：标题输入行、提示行（仅确认态/错误时占位）与按钮行。
pub fn resume_edit_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, RESUME_EDIT_WIDTH, RESUME_EDIT_HEIGHT)
}

/// `/resume` 会话编辑模态的按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeEditButton {
    /// 主按钮：编辑态保存、确认删除态确认删除。
    Primary,
    /// 编辑态的 `[delete ^d]`。
    Delete,
    /// 编辑态 `[cancel esc]` 关闭、确认态退回编辑态。
    Cancel,
}

/// 命中 `/resume` 会话编辑模态的按钮行。
pub fn resume_edit_button_at(
    shell: &widgets::modal::ModalShell,
    confirming: bool,
    column: u16,
    row: u16,
) -> Option<ResumeEditButton> {
    let labels: &[&str] = if confirming {
        &RESUME_EDIT_CONFIRM_BUTTONS
    } else {
        &RESUME_EDIT_BUTTONS
    };
    let rects = widgets::modal::button_row(shell.inner, labels, BUTTON_GAP, RENAME_BUTTON_ROW);
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(ResumeEditButton::Primary),
        Some(1) if !confirming => Some(ResumeEditButton::Delete),
        Some(_) => Some(ResumeEditButton::Cancel),
        None => None,
    }
}

/// `/resume` 会话编辑模态：标题输入 + 提示行 + 按钮行；返回硬件光标位置。
///
/// 编辑态按钮是 `[save ↵] [delete ^d] [cancel esc]`，确认删除态换成
/// `[confirm del enter] [cancel esc]` 并在标题行右侧提示再按一次确认。
pub(crate) fn render_resume_session_edit(
    frame: &mut Frame<'_>,
    screen: Rect,
    edit: &crate::app::overlay::ResumeSessionEdit,
) -> Option<(u16, u16)> {
    let shell = resume_edit_shell(screen)?;
    widgets::modal::render(frame, &shell, text::RESUME_EDIT_MODAL_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let (labels, buttons): (&[&str], &[(&str, ratatui::style::Style)]) = if edit.confirming_delete {
        (
            &RESUME_EDIT_CONFIRM_BUTTONS,
            &[
                (text::BUTTON_CONFIRM_DELETE, style::accent()),
                (text::BUTTON_CANCEL, style::muted()),
            ],
        )
    } else {
        (
            &RESUME_EDIT_BUTTONS,
            &[
                (text::BUTTON_SAVE, style::accent()),
                (text::BUTTON_DELETE, style::error()),
                (text::BUTTON_CANCEL, style::muted()),
            ],
        )
    };
    render_buttons(
        frame,
        &widgets::modal::button_row(shell.inner, labels, BUTTON_GAP, RENAME_BUTTON_ROW),
        buttons,
    );
    // 提示行：确认态显示"再按一次"，或显示校验/占用错误（红字）。
    let hint = edit
        .hint
        .as_deref()
        .map(|hint| (hint, style::error()))
        .or_else(|| {
            edit.confirming_delete
                .then_some((text::RESUME_EDIT_CONFIRM_HINT, style::error()))
        });
    if let Some((hint, hint_style)) = hint
        && shell.inner.height > 1
    {
        frame.render_widget(
            Paragraph::new(Span::styled(
                text::ellipsize(hint, usize::from(shell.inner.width)),
                hint_style,
            )),
            Rect::new(
                shell.inner.x,
                shell.inner.y.saturating_add(1),
                shell.inner.width,
                1,
            ),
        );
    }
    render_input(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        edit.input.text(),
        edit.input.cursor(),
        edit.input.selection(),
        Some(text::RESUME_EDIT_PLACEHOLDER),
    )
}

/// 模态输入行：左侧 `> ` 前缀（与 prompt 列表搜索框同图标）+ 单行输入（支持选区反色）；
/// 返回硬件光标位置；宽度放不下前缀时输入区为空、不显示光标。
pub(crate) fn render_input(
    frame: &mut Frame<'_>,
    area: Rect,
    text: &str,
    cursor: usize,
    selection: Option<(usize, usize)>,
    placeholder: Option<&str>,
) -> Option<(u16, u16)> {
    let prefix = format!("{} ", text::INPUT_PREFIX_ICON);
    let prefix_width = prefix.chars().count() as u16;
    frame.render_widget(
        Paragraph::new(Span::styled(prefix, style::accent())),
        Rect {
            width: area.width.min(prefix_width),
            height: 1,
            ..area
        },
    );
    widgets::input::render_with_selection(
        frame,
        Rect {
            x: area.x.saturating_add(prefix_width),
            width: area.width.saturating_sub(prefix_width),
            height: 1,
            ..area
        },
        text,
        cursor,
        selection,
        placeholder,
    )
}

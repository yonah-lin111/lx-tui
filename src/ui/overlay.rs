//! 浮层渲染：右键菜单、重命名与关闭确认；几何与命中供事件循环复用。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::overlay::{
    ConfirmClose, Menu, MenuCommand, Overlay, OverlayTarget, Rename, RenameTarget,
};
use crate::app::state::{AppState, tab_label};
use crate::ui::widgets;
use crate::ui::{style, text};

/// 重命名浮层尺寸（列，行）：输入行、空行与按钮行。
const RENAME_WIDTH: u16 = 40;
const RENAME_HEIGHT: u16 = 5;
/// 关闭确认浮层尺寸（列，行）：问题行与按钮行。
const CONFIRM_WIDTH: u16 = 40;
const CONFIRM_HEIGHT: u16 = 4;
/// 按钮间距与所在内容行。
const BUTTON_GAP: u16 = 2;
const RENAME_BUTTON_ROW: u16 = 2;
const CONFIRM_BUTTON_ROW: u16 = 1;
const RENAME_BUTTONS: [&str; 3] = [text::BUTTON_SAVE, text::BUTTON_CLEAR, text::BUTTON_CANCEL];
const CONFIRM_BUTTONS: [&str; 2] = [text::BUTTON_CONFIRM, text::BUTTON_CANCEL];

/// 重命名浮层按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameButton {
    Save,
    Clear,
    Cancel,
}

/// 关闭确认浮层按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmButton {
    Confirm,
    Cancel,
}

/// 菜单几何与文案；渲染与鼠标命中共用。
pub fn menu_layout(screen: Rect, menu: &Menu) -> widgets::menu::MenuLayout {
    widgets::menu::layout(screen, menu.anchor, &menu_labels(menu))
}

/// 重命名浮层几何。
pub fn rename_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, RENAME_WIDTH, RENAME_HEIGHT)
}

/// 关闭确认浮层几何。
pub fn confirm_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, CONFIRM_WIDTH, CONFIRM_HEIGHT)
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

/// 渲染当前浮层；返回需要同步的硬件光标位置（仅重命名输入）。
pub fn render(frame: &mut Frame<'_>, screen: Rect, state: &AppState) -> Option<(u16, u16)> {
    match state.overlay.as_ref()? {
        Overlay::Menu(menu) => {
            let labels = menu_labels(menu);
            let layout = menu_layout(screen, menu);
            widgets::menu::render(frame, &layout, &labels, Some(menu.selected));
            None
        }
        Overlay::Rename(rename) => render_rename(frame, screen, rename),
        Overlay::ConfirmClose(confirm) => {
            render_confirm(frame, screen, state, confirm);
            None
        }
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
            MenuCommand::RenameWorkspace => text::MENU_RENAME_WORKSPACE,
            MenuCommand::CloseWorkspace => text::MENU_CLOSE_WORKSPACE,
            MenuCommand::RenameTab => text::MENU_RENAME_TAB,
            MenuCommand::CloseTab => text::MENU_CLOSE_TAB,
            MenuCommand::SplitRight => text::MENU_SPLIT_RIGHT,
            MenuCommand::SplitDown => text::MENU_SPLIT_DOWN,
            MenuCommand::SwitchToTerminal => text::MENU_SWITCH_TO_TERMINAL,
            MenuCommand::SwitchToLx => text::MENU_SWITCH_TO_LX,
            MenuCommand::ClosePane => text::MENU_CLOSE_PANE,
        })
        .collect()
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

/// 关闭确认浮层：问题与底部按钮。
fn render_confirm(frame: &mut Frame<'_>, screen: Rect, state: &AppState, confirm: &ConfirmClose) {
    let Some(shell) = confirm_shell(screen) else {
        return;
    };
    let (title, name) = confirm_text(state, confirm.target);
    widgets::modal::render(frame, &shell, title);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return;
    }
    let width = usize::from(shell.inner.width);
    let question = text::confirm_close_question(&name);
    let question_area = Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text::ellipsize(&question, width),
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

#[cfg(test)]
mod tests;

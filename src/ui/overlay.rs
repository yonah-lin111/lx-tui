//! 浮层渲染：右键菜单、重命名与关闭确认；几何与命中供事件循环复用。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::overlay::{ConfirmClose, Menu, MenuCommand, Overlay, Rename};
use crate::app::state::AppState;
use crate::ui::widgets;
use crate::ui::{style, text};

/// 重命名浮层尺寸（列，行）。
const RENAME_WIDTH: u16 = 40;
const RENAME_HEIGHT: u16 = 3;
/// 关闭确认浮层尺寸（列，行）：标题、问题与键位提示。
const CONFIRM_WIDTH: u16 = 40;
const CONFIRM_HEIGHT: u16 = 4;

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

/// 菜单命令到文案的映射；文案集中在 `ui/text.rs`。
fn menu_labels(menu: &Menu) -> Vec<&'static str> {
    menu.commands
        .iter()
        .map(|command| match command {
            MenuCommand::RenameWorkspace => text::MENU_RENAME_WORKSPACE,
            MenuCommand::CloseWorkspace => text::MENU_CLOSE_WORKSPACE,
        })
        .collect()
}

/// 重命名浮层：单行输入；视口跟随光标，保证光标可见。
fn render_rename(frame: &mut Frame<'_>, screen: Rect, rename: &Rename) -> Option<(u16, u16)> {
    let shell = rename_shell(screen)?;
    widgets::modal::render(frame, &shell, text::RENAME_WORKSPACE_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    let width = usize::from(shell.inner.width);
    let chars: Vec<char> = rename.input.text().chars().collect();
    let cursor = rename.input.cursor().min(chars.len());
    let offset = cursor.saturating_sub(width.saturating_sub(1));
    let visible: String = chars[offset..].iter().take(width).collect();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(visible, style::text()))),
        shell.inner,
    );
    let column = shell
        .inner
        .x
        .saturating_add(u16::try_from(cursor - offset).unwrap_or(u16::MAX))
        .min(shell.inner.right().saturating_sub(1));
    Some((column, shell.inner.y))
}

/// 关闭确认浮层：问题与键位提示。
fn render_confirm(frame: &mut Frame<'_>, screen: Rect, state: &AppState, confirm: &ConfirmClose) {
    let Some(shell) = confirm_shell(screen) else {
        return;
    };
    widgets::modal::render(frame, &shell, text::CONFIRM_CLOSE_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return;
    }
    let width = usize::from(shell.inner.width);
    let name = state
        .workspaces
        .get(confirm.target)
        .map(|workspace| workspace.name.as_str())
        .unwrap_or_default();
    let question = text::confirm_close_question(name);
    let lines = vec![
        Line::from(Span::styled(
            text::ellipsize(&question, width),
            style::text(),
        )),
        Line::from(Span::styled(
            text::ellipsize(text::CONFIRM_CLOSE_HINT, width),
            style::muted(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), shell.inner);
}

#[cfg(test)]
mod tests;

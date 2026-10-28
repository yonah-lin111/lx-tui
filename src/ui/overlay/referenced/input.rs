//! 引用文件夹路径输入弹窗：几何、命中与渲染。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::overlay::ReferencedFolderInput;
use crate::ui::widgets;
use crate::ui::{style, text};

use super::super::dialog::{BUTTON_GAP, render_buttons, render_input};
use super::paint_text;

/// 路径输入弹窗宽度（列）；容纳长绝对路径。
const INPUT_WIDTH: u16 = 72;
/// 路径输入弹窗高度（行）：输入行、校验/错误行与底部按钮。
const INPUT_HEIGHT: u16 = 8;

/// 路径输入弹窗按钮几何；不可用时 None（屏幕放不下）。
pub fn referenced_input_shell(screen: Rect) -> Option<widgets::modal::ModalShell> {
    widgets::modal::layout(screen, INPUT_WIDTH, INPUT_HEIGHT)
}

/// 路径输入弹窗按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferencedInputButton {
    Create,
    Clear,
    Cancel,
}

/// 命中路径输入弹窗底部按钮。
pub fn referenced_input_button_at(
    shell: &widgets::modal::ModalShell,
    column: u16,
    row: u16,
) -> Option<ReferencedInputButton> {
    let rects = widgets::modal::button_row(
        shell.inner,
        &[text::BUTTON_CREATE, text::BUTTON_CLEAR, text::BUTTON_CANCEL],
        BUTTON_GAP,
        shell.inner.height.saturating_sub(1),
    );
    match widgets::modal::button_at(&rects, column, row) {
        Some(0) => Some(ReferencedInputButton::Create),
        Some(1) => Some(ReferencedInputButton::Clear),
        Some(2) => Some(ReferencedInputButton::Cancel),
        _ => None,
    }
}

/// 渲染路径输入弹窗；返回输入硬件光标位置。
pub fn render_referenced_input(
    frame: &mut Frame<'_>,
    screen: Rect,
    input: &ReferencedFolderInput,
) -> Option<(u16, u16)> {
    let shell = referenced_input_shell(screen)?;
    widgets::modal::render(frame, &shell, text::REFERENCED_INPUT_TITLE);
    if shell.inner.width == 0 || shell.inner.height == 0 {
        return None;
    }
    render_buttons(
        frame,
        &widgets::modal::button_row(
            shell.inner,
            &[text::BUTTON_CREATE, text::BUTTON_CLEAR, text::BUTTON_CANCEL],
            BUTTON_GAP,
            shell.inner.height.saturating_sub(1),
        ),
        &[
            (text::BUTTON_CREATE, style::accent()),
            (text::BUTTON_CLEAR, style::muted()),
            (text::BUTTON_CANCEL, style::muted()),
        ],
    );
    let cursor = render_input(
        frame,
        Rect::new(shell.inner.x, shell.inner.y, shell.inner.width, 1),
        input.input.text(),
        input.input.cursor(),
        input.input.selection(),
        Some(text::REFERENCED_INPUT_PLACEHOLDER),
    );
    if let Some(error) = input.error {
        paint_text(
            frame,
            Rect::new(
                shell.inner.x,
                shell.inner.y.saturating_add(1),
                shell.inner.width,
                1,
            ),
            &text::ellipsize(error, usize::from(shell.inner.width)),
            style::error(),
        );
    } else if input.validating {
        paint_text(
            frame,
            Rect::new(
                shell.inner.x,
                shell.inner.y.saturating_add(1),
                shell.inner.width,
                1,
            ),
            text::REFERENCED_SEARCH_LOADING,
            style::muted(),
        );
    }
    cursor
}

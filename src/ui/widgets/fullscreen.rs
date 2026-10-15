//! 全屏覆盖组件：整屏覆盖、最外层蓝色边框，标题与快捷键提示内嵌在边框线上。
//!
//! 只接收几何与文案，不依赖应用领域数据；内容由调用方在 `content` 区自行绘制。
//! 打开时用 `Clear` 把整屏重置为终端默认底色（不铺自包含面板底色），
//! 因此透明终端下不会出现整块背景。
//! 标题内嵌在顶边框左侧、提示内嵌在底边框左侧（同 prompt 面板 `Prompt` / `b:` 的做法），
//! 不占用内容行。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::ui::{style, text};

/// 全屏覆盖几何：整屏（含最外层边框）与边框内内容区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullscreenShell {
    /// 整块覆盖区域（通常为整个屏幕，含最外层边框）。
    pub area: Rect,
    /// 边框内的内容区（标题/提示内嵌在边框上，不占内容行）。
    pub content: Rect,
}

/// 全屏覆盖最小可用宽度：边框 + 容纳右上角 `[✕]`。
const MIN_WIDTH: u16 = 10;
/// 全屏覆盖最小可用高度：边框 + 至少 1 行内容。
const MIN_HEIGHT: u16 = 3;

/// 覆盖整屏的几何；屏幕过小时返回 None。
pub fn layout(screen: Rect) -> Option<FullscreenShell> {
    if screen.width < MIN_WIDTH || screen.height < MIN_HEIGHT {
        return None;
    }
    let content = Block::bordered().inner(screen);
    if content.width == 0 || content.height == 0 {
        return None;
    }
    Some(FullscreenShell {
        area: screen,
        content,
    })
}

/// 右上角关闭按钮矩形（顶边框右端，3 列）；区域过窄时 None。
pub fn close_button_rect(shell: &FullscreenShell) -> Option<Rect> {
    (shell.area.width >= 8).then_some(Rect::new(
        shell.area.right().saturating_sub(4),
        shell.area.y,
        3,
        1,
    ))
}

/// 命中右上角关闭按钮。
pub fn close_button_at(shell: &FullscreenShell, column: u16, row: u16) -> bool {
    close_button_rect(shell).is_some_and(|rect| rect.contains((column, row).into()))
}

/// 绘制全屏覆盖：蓝色圆角边框、清底，标题内嵌顶边框左侧、提示内嵌底边框左侧、右上角 `[✕]`。
pub fn render(frame: &mut Frame<'_>, shell: &FullscreenShell, title: &str, hint: &str) {
    frame.render_widget(Clear, shell.area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .title_top(
            Line::from(Span::styled(format!(" {title} "), style::border_title()))
                .alignment(ratatui::layout::Alignment::Left),
        )
        .title_bottom(
            Line::from(Span::styled(format!(" {hint} "), style::muted()))
                .alignment(ratatui::layout::Alignment::Left),
        );
    frame.render_widget(block, shell.area);
    // 右上角关闭按钮覆盖在顶边框右端。
    if let Some(rect) = close_button_rect(shell) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, style::accent())),
            rect,
        );
    }
}

#[cfg(test)]
mod tests;

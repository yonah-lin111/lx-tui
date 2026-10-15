//! 全屏覆盖组件：整屏覆盖、最外层蓝色边框，含顶部标题栏与底部提示栏。
//!
//! 只接收几何与文案，不依赖应用领域数据；内容由调用方在 `content` 区自行绘制。
//! 打开时用 `Clear` 把整屏重置为终端默认底色（不铺自包含面板底色），
//! 因此透明终端下不会出现整块背景。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::ui::{style, text};

/// 全屏覆盖几何：整屏、边框内区、顶部标题栏、内容区与底部提示栏。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FullscreenShell {
    /// 整块覆盖区域（通常为整个屏幕，含最外层边框）。
    pub area: Rect,
    /// 边框内的整体内容区。
    pub inner: Rect,
    /// 顶部标题栏（高度 1）。
    pub header: Rect,
    /// 内容区（标题栏与提示栏之间，占用全部剩余高度）。
    pub content: Rect,
    /// 底部提示栏（高度 1）。
    pub footer: Rect,
}

/// 全屏覆盖最小可用宽度：边框 + 容纳右上角 `[✕]`。
const MIN_WIDTH: u16 = 10;
/// 全屏覆盖最小可用高度：边框 + 标题栏 + 至少 1 行内容 + 提示栏。
const MIN_HEIGHT: u16 = 5;

/// 覆盖整屏的几何；屏幕过小时返回 None。
pub fn layout(screen: Rect) -> Option<FullscreenShell> {
    if screen.width < MIN_WIDTH || screen.height < MIN_HEIGHT {
        return None;
    }
    let inner = Block::bordered().inner(screen);
    if inner.width == 0 || inner.height < 3 {
        return None;
    }
    let header = Rect::new(inner.x, inner.y, inner.width, 1);
    let content = Rect::new(
        inner.x,
        inner.y + 1,
        inner.width,
        inner.height.saturating_sub(2),
    );
    let footer = Rect::new(inner.x, inner.bottom().saturating_sub(1), inner.width, 1);
    Some(FullscreenShell {
        area: screen,
        inner,
        header,
        content,
        footer,
    })
}

/// 右上角关闭按钮矩形（标题栏右端，3 列）；标题栏过窄时 None。
pub fn close_button_rect(shell: &FullscreenShell) -> Option<Rect> {
    (shell.header.width >= 5).then_some(Rect::new(
        shell.header.right().saturating_sub(3),
        shell.header.y,
        3,
        1,
    ))
}

/// 命中右上角关闭按钮。
pub fn close_button_at(shell: &FullscreenShell, column: u16, row: u16) -> bool {
    close_button_rect(shell).is_some_and(|rect| rect.contains((column, row).into()))
}

/// 绘制全屏覆盖：最外层蓝色圆角边框、清底、标题栏标题与右上角 `[✕]`、底部提示。
pub fn render(frame: &mut Frame<'_>, shell: &FullscreenShell, title: &str, hint: &str) {
    frame.render_widget(Clear, shell.area);
    frame.render_widget(
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style::accent()),
        shell.area,
    );
    // 标题为右上角关闭按钮预留空间，避免重叠。
    let max_title = usize::from(shell.header.width.saturating_sub(4));
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::ellipsize(title, max_title),
            style::border_title(),
        )),
        shell.header,
    );
    if let Some(rect) = close_button_rect(shell) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, style::accent())),
            rect,
        );
    }
    frame.render_widget(
        Paragraph::new(Span::styled(
            text::ellipsize(hint, usize::from(shell.footer.width)),
            style::muted(),
        )),
        shell.footer,
    );
}

#[cfg(test)]
mod tests;

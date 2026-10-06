//! 模态容器组件：居中定位、清底、边框与标题；内容由调用方在 `inner` 绘制。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear};

use crate::ui::style;

/// 模态几何：整体区域与内容区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalShell {
    pub area: Rect,
    pub inner: Rect,
}

/// 居中模态几何；屏幕放不下（宽 < 4 或高 < 3）返回 None。
pub fn layout(screen: Rect, width: u16, height: u16) -> Option<ModalShell> {
    let width = width.min(screen.width);
    let height = height.min(screen.height);
    if width < 4 || height < 3 {
        return None;
    }
    let area = Rect::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );
    let inner = Block::bordered().inner(area);
    Some(ModalShell { area, inner })
}

/// 渲染清底、强调色边框与标题；内容由调用方绘制在 `shell.inner`。
pub fn render(frame: &mut Frame<'_>, shell: &ModalShell, title: &str) {
    frame.render_widget(Clear, shell.area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .title(Span::styled(format!(" {title} "), style::muted()));
    frame.render_widget(block, shell.area);
}

/// 按钮行：按标签宽度水平居中排布；`row_offset` 为相对内容区顶部的行号。
pub fn button_row(inner: Rect, labels: &[&str], gap: u16, row_offset: u16) -> Vec<Rect> {
    let widths: Vec<u16> = labels
        .iter()
        .map(|label| label.chars().count() as u16)
        .collect();
    let total = widths.iter().copied().sum::<u16>()
        + gap.saturating_mul(widths.len().saturating_sub(1) as u16);
    let mut x = inner.x + inner.width.saturating_sub(total) / 2;
    let y = inner.y + row_offset.min(inner.height.saturating_sub(1));
    widths
        .iter()
        .map(|width| {
            let rect = Rect::new(
                x,
                y,
                (*width).min(inner.width.saturating_sub(x.saturating_sub(inner.x))),
                1,
            );
            x = x.saturating_add(*width).saturating_add(gap);
            rect
        })
        .collect()
}

/// 命中按钮索引。
pub fn button_at(rects: &[Rect], column: u16, row: u16) -> Option<usize> {
    rects
        .iter()
        .position(|rect| rect.contains((column, row).into()))
}

#[cfg(test)]
mod tests;

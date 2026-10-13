//! 模态容器组件：居中定位、清底、边框与标题；内容由调用方在 `inner` 绘制。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::ui::{style, text};

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

/// 渲染清底、强调色边框、标题与右上角关闭按钮；内容由调用方绘制在 `shell.inner`。
pub fn render(frame: &mut Frame<'_>, shell: &ModalShell, title: &str) {
    frame.render_widget(Clear, shell.area);
    // 标题为关闭按钮预留右上角空间，避免重叠。
    let title = text::ellipsize(title, usize::from(shell.area.width.saturating_sub(7)));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent())
        .title(Span::styled(format!(" {title} "), style::border_title()))
        .style(style::overlay_panel());
    frame.render_widget(block, shell.area);
    if let Some(rect) = close_button_rect(shell.area) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, style::accent())),
            rect,
        );
    }
}

/// 右上角关闭按钮矩形（顶边框 `[✕]`，3 列）；区域过窄（< 10 列）时 None。
pub fn close_button_rect(area: Rect) -> Option<Rect> {
    (area.width >= 10).then_some(Rect::new(area.right().saturating_sub(4), area.y, 3, 1))
}

/// 命中右上角关闭按钮。
pub fn close_button_at(area: Rect, column: u16, row: u16) -> bool {
    close_button_rect(area).is_some_and(|rect| rect.contains((column, row).into()))
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

//! 模态容器组件：居中定位、清底、边框与标题；内容由调用方在 `inner` 绘制。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// 模态几何：整体区域与内容区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalShell {
    pub area: Rect,
    pub inner: Rect,
}

/// 居中模态几何；屏幕放不下（宽 < 4 或高 < 3）返回 None。
///
/// 当高度充裕（≥ 6 行）时，顶部预留 3 行顶栏框（`┌──┐` + `│ Title [✕] │` + `├──┤`），
/// `inner` 从 `area.y + 3` 起算。
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
    let inner = if area.height >= 6 {
        Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(3),
            area.width.saturating_sub(2),
            area.height.saturating_sub(4),
        )
    } else {
        Block::bordered().inner(area)
    };
    Some(ModalShell { area, inner })
}

/// 渲染清底、强调色边框、顶栏标题行与右上角关闭按钮；标题行左侧写标题，
/// 右侧用 `─` 连通到关闭按钮，形成 `│ Title ───── [✕] │`；内容由调用方绘制在 `shell.inner`。
pub fn render(frame: &mut Frame<'_>, shell: &ModalShell, title: &str) {
    clear::render(frame, shell.area);
    let block = Block::bordered().border_style(style::accent());
    frame.render_widget(block, shell.area);
    if shell.area.height >= 6 && shell.area.width >= 4 {
        let fitted = text::ellipsize(title, usize::from(shell.area.width.saturating_sub(7)));
        let title_w = u16::try_from(fitted.width()).unwrap_or(u16::MAX);
        frame.render_widget(
            Paragraph::new(Span::styled(fitted, style::border_title())),
            Rect::new(
                shell.area.x + 2,
                shell.area.y + 1,
                shell.area.width.saturating_sub(4),
                1,
            ),
        );
        let from = shell.area.x.saturating_add(2).saturating_add(title_w);
        let to = close_button_rect(shell.area).map_or_else(
            || shell.area.right().saturating_sub(2),
            |rect| rect.x.saturating_sub(1),
        );
        paint_title_rule(
            frame.buffer_mut(),
            shell.area,
            shell.area.y + 1,
            from.saturating_add(1),
            to,
            style::accent(),
        );
    }
    if let Some(rect) = close_button_rect(shell.area) {
        frame.render_widget(
            Paragraph::new(Span::styled(text::MODAL_CLOSE, style::accent())),
            rect,
        );
    }
}

/// 在标题行把标题与右侧元素之间的空白填成 `─`（外框两列与既有文字由调用方排除）。
pub(crate) fn paint_title_rule(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    from: u16,
    to: u16,
    border_style: Style,
) {
    if y <= area.y || y >= area.bottom().saturating_sub(1) {
        return;
    }
    let start = from.max(area.x.saturating_add(1));
    let end = to.min(area.right().saturating_sub(1));
    for x in start..end {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_char('─');
            cell.set_style(border_style);
        }
    }
}

/// 右上角关闭按钮矩形（`[✕]`，3 列，右侧留 1 列内边距）；高度 ≥ 6 时位于顶栏行（`area.y + 1`），否则位于 `area.y`。
pub fn close_button_rect(area: Rect) -> Option<Rect> {
    let y = if area.height >= 6 {
        area.y.saturating_add(1)
    } else {
        area.y
    };
    (area.width >= 10 && y < area.bottom()).then_some(Rect::new(
        area.right().saturating_sub(5),
        y,
        3,
        1,
    ))
}

/// 命中右上角关闭按钮。
pub fn close_button_at(area: Rect, column: u16, row: u16) -> bool {
    if area.width < 10 {
        return false;
    }
    let x = area.right().saturating_sub(5);
    Rect::new(x, area.y, 3, 1).contains((column, row).into())
        || (area.height >= 6
            && Rect::new(x, area.y.saturating_add(1), 3, 1).contains((column, row).into()))
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

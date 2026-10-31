//! 模态容器组件：背景压暗、居中定位、清底、边框与标题；内容由调用方在 `inner` 绘制。
//!
//! 布局对齐 herdr `render_modal_shell` + `render_modal_header`：
//! 底层屏幕施加 `DIM` 压暗，模态框为直角边框（普通强调色 / 危险操作红色），
//! 标题位于顶边框内首行加粗显示，标题与内容之间留 1 行空白呼吸区，无 `├───┤` 贯穿线与 `[✕]` 按钮。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Paragraph};

use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// 模态几何：整体区域与内容区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModalShell {
    pub area: Rect,
    pub inner: Rect,
}

/// 将底层区域所有单元格压暗（对齐 herdr `dim_background`）。
pub fn dim_background(frame: &mut Frame<'_>, area: Rect) {
    let buf = frame.buffer_mut();
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_style(cell.style().add_modifier(Modifier::DIM));
            }
        }
    }
}

/// 居中模态几何；屏幕放不下（宽 < 4 或高 < 3）返回 None。
///
/// 当高度充裕（≥ 6 行）时，顶部预留 3 行（`┌──┐` + 粗体标题行 + 1 行空白间隔），
/// `inner` 从 `area.y + 3` 起算并包含底部按钮行。
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

/// 渲染常规模态壳：底层压暗、清底、强调色边框与内框首行粗体标题。
pub fn render(frame: &mut Frame<'_>, shell: &ModalShell, title: &str) {
    render_with_style(frame, shell, title, style::accent(), style::strong());
}

/// 渲染危险确认模态壳：底层压暗、清底、红色边框与内框首行红色粗体标题（对齐 herdr 确认弹窗）。
pub fn render_danger(frame: &mut Frame<'_>, shell: &ModalShell, title: &str) {
    let danger = style::error().add_modifier(Modifier::BOLD);
    render_with_style(frame, shell, title, danger, danger);
}

fn render_with_style(
    frame: &mut Frame<'_>,
    shell: &ModalShell,
    title: &str,
    border_style: Style,
    title_style: Style,
) {
    dim_background(frame, frame.area());
    clear::render(frame, shell.area);
    let block = Block::bordered().border_style(border_style);
    frame.render_widget(block, shell.area);
    if shell.area.height >= 4 && shell.area.width >= 4 {
        let fitted = text::ellipsize(title, usize::from(shell.area.width.saturating_sub(4)));
        frame.render_widget(
            Paragraph::new(Span::styled(fitted, title_style)),
            Rect::new(
                shell.area.x + 2,
                shell.area.y + 1,
                shell.area.width.saturating_sub(4),
                1,
            ),
        );
    }
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

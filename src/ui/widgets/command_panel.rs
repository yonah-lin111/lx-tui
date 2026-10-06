//! 浮层命令面板：条目列表、高亮、窗口滚动与边框绘制；无业务语义、不读状态。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::ui::style;

/// 面板最小可用尺寸（含边框）。
const MIN_WIDTH: u16 = 3;
const MIN_HEIGHT: u16 = 3;

/// 标签与预览之间的间距（列）。
const ITEM_GAP: usize = 2;

/// 面板条目：主文案与格式预览；绘制时格式在前、名称在后。
pub struct CommandItem<'a> {
    pub label: &'a str,
    pub preview: &'a str,
}

/// 面板视图：条目、高亮索引与锚点行。
///
/// `anchor_row` 为内容区内的视觉行；面板绘制在其上方或下方，不覆盖该行。
pub struct CommandPanelView<'a> {
    pub items: &'a [CommandItem<'a>],
    pub active: usize,
    pub anchor_row: u16,
}

/// 在内容区内绘制命令面板；返回实际绘制矩形，空间不足时返回 None。
pub fn render(area: Rect, buf: &mut Buffer, view: &CommandPanelView<'_>) -> Option<Rect> {
    if view.items.is_empty() || area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return None;
    }
    let needed = (view.items.len() + 2) as u16;
    let below = area
        .height
        .saturating_sub(view.anchor_row.saturating_add(1));
    let above = view.anchor_row;
    let place_below = below >= needed || below >= above;
    let space = if place_below { below } else { above };
    let height = needed.min(space);
    if height < MIN_HEIGHT {
        return None;
    }
    let y = if place_below {
        area.y + view.anchor_row + 1
    } else {
        area.y + view.anchor_row - height
    };
    let rect = Rect::new(area.x, y, panel_width(area.width, view.items), height);
    Clear.render(rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::muted());
    let inner = block.inner(rect);
    block.render(rect, buf);
    render_items(buf, inner, view);
    Some(rect)
}

/// 面板宽度：最长条目的预览+间距+标签，加两边框与左侧内边距，钳制在内容区内。
fn panel_width(available: u16, items: &[CommandItem<'_>]) -> u16 {
    let content = items
        .iter()
        .map(|item| item.label.width() + ITEM_GAP + item.preview.width())
        .max()
        .unwrap_or(0)
        .min(usize::from(available));
    ((content + 4) as u16).min(available).max(MIN_WIDTH)
}

/// 绘制可见条目；窗口滚动保证高亮项可见。
fn render_items(buf: &mut Buffer, inner: Rect, view: &CommandPanelView<'_>) {
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let visible = usize::from(inner.height);
    let active = view.active.min(view.items.len().saturating_sub(1));
    let start = active
        .saturating_sub(visible.saturating_sub(1))
        .min(view.items.len().saturating_sub(visible));
    for (offset, item) in view.items.iter().skip(start).take(visible).enumerate() {
        let row = Rect::new(inner.x, inner.y + offset as u16, inner.width, 1);
        render_item(buf, row, item, start + offset == active);
    }
}

/// 绘制单条目：格式预览在前、名称在后；高亮项整行反显，超宽由 Paragraph 截断。
fn render_item(buf: &mut Buffer, area: Rect, item: &CommandItem<'_>, active: bool) {
    if area.width < 3 {
        return;
    }
    let line = Line::from(vec![
        Span::raw(" "),
        Span::styled(item.preview, style::muted()),
        Span::raw(" ".repeat(ITEM_GAP)),
        Span::styled(item.label, style::text()),
    ]);
    let row_style = if active {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    Paragraph::new(line)
        .style(row_style)
        .render(Rect::new(area.x + 1, area.y, area.width - 1, 1), buf);
}

#[cfg(test)]
mod tests;

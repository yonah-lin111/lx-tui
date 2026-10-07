//! 浮层命令面板：条目列表、高亮、窗口滚动与边框绘制；无业务语义、不读状态。

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::ui::style;
use crate::ui::widgets::scrollbar::{self, ScrollbarLayout};

/// 面板最小可用尺寸（含边框）。
const MIN_WIDTH: u16 = 3;
const MIN_HEIGHT: u16 = 3;

/// 名称列与预览列之间的间距（列）。
const COLUMN_GAP: usize = 1;

/// 面板条目：单行（名称 + 右侧预览）或双行（名称在上、明细在下）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandItem<'a> {
    Inline { label: &'a str, preview: &'a str },
    Stacked { label: &'a str, detail: &'a str },
}

/// 面板视图：条目、高亮索引、窗口锚点、锚点行与最大高度。
///
/// `anchor_row` 为内容区内的视觉行；面板绘制在其上方或下方，不覆盖该行。
/// `window_anchor` 为窗口底部锚定的条目索引：窗口以它为底向前回退填满预算；
/// `None` 表示锚定高亮。鼠标悬停只改 `active`、不改锚点，窗口因此不滚动。
pub struct CommandPanelView<'a> {
    pub items: &'a [CommandItem<'a>],
    pub active: usize,
    pub window_anchor: Option<usize>,
    pub anchor_row: u16,
    /// 面板最大高度（含边框）；None 表示仅受可用空间限制。
    pub max_height: Option<u16>,
}

/// 面板布局：面板矩形、条目内容区、滚动条、首个可见条目与可见条目数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelLayout {
    pub rect: Rect,
    pub inner: Rect,
    /// 条目绘制与命中区；滚动条出现时不含其占用列。
    pub content: Rect,
    pub scrollbar: Option<ScrollbarLayout>,
    pub start: usize,
    pub visible: usize,
}

/// 计算面板布局；空间不足或没有可见条目时返回 None。
pub fn layout(area: Rect, view: &CommandPanelView<'_>) -> Option<PanelLayout> {
    if view.items.is_empty() || area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        return None;
    }
    let content_rows: u16 = view.items.iter().map(item_height).sum();
    let needed = content_rows.saturating_add(2);
    let needed = view
        .max_height
        .map_or(needed, |max_height| needed.min(max_height));
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
    let budget = usize::from(height.saturating_sub(2));
    let active = view.active.min(view.items.len() - 1);
    let anchor = view
        .window_anchor
        .map_or(active, |anchor| anchor.min(view.items.len() - 1));
    let mut start = window_start(view.items, anchor, budget);
    let mut visible = visible_count(view.items, start, budget);
    // 锚点窗口不含高亮时回退到以高亮为底，保证高亮始终可见。
    if active < start || active >= start.saturating_add(visible) {
        start = window_start(view.items, active, budget);
        visible = visible_count(view.items, start, budget);
    }
    if visible == 0 {
        return None;
    }
    let overflow = view.items.len() > visible;
    let width = panel_width(area.width, view.items)
        .saturating_add(u16::from(overflow))
        .min(area.width);
    let rect = Rect::new(area.x, y, width, height);
    let inner = inner_rect(rect);
    let scrollbar = if overflow {
        scrollbar::layout(inner, view.items.len(), visible, start)
    } else {
        None
    };
    let content = match scrollbar {
        Some(_) => Rect::new(
            inner.x,
            inner.y,
            inner.width.saturating_sub(1),
            inner.height,
        ),
        None => inner,
    };
    Some(PanelLayout {
        rect,
        inner,
        content,
        scrollbar,
        start,
        visible,
    })
}

/// 在内容区内绘制命令面板；返回实际绘制矩形，空间不足时返回 None。
pub fn render(area: Rect, buf: &mut Buffer, view: &CommandPanelView<'_>) -> Option<Rect> {
    let layout = layout(area, view)?;
    Clear.render(layout.rect, buf);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::muted())
        .style(style::overlay_panel());
    block.render(layout.rect, buf);
    render_items(buf, &layout, view);
    if let Some(scrollbar) = layout.scrollbar.as_ref() {
        scrollbar::render_buffer(buf, scrollbar);
    }
    Some(layout.rect)
}

/// 命中面板内某条目：返回条目索引；未命中返回 None。
pub fn item_at(
    layout: &PanelLayout,
    items: &[CommandItem<'_>],
    column: u16,
    row: u16,
) -> Option<usize> {
    if !layout.content.contains(Position::new(column, row)) {
        return None;
    }
    let mut y = layout.content.y;
    for (offset, item) in items
        .iter()
        .skip(layout.start)
        .take(layout.visible)
        .enumerate()
    {
        if row < y.saturating_add(item_height(item)) {
            return Some(layout.start + offset);
        }
        y = y.saturating_add(item_height(item));
    }
    None
}

/// 条目占用的行数；双行条目明细为空时只占一行。
fn item_height(item: &CommandItem<'_>) -> u16 {
    match item {
        CommandItem::Inline { .. } => 1,
        CommandItem::Stacked { detail, .. } => {
            if detail.is_empty() {
                1
            } else {
                2
            }
        }
    }
}

/// 面板宽度：单行按最长标签 + 间距 + 最长预览，双行按标签与明细的较宽者，加两边框与左侧内边距。
fn panel_width(available: u16, items: &[CommandItem<'_>]) -> u16 {
    let label = items
        .iter()
        .map(|item| match item {
            CommandItem::Inline { label, .. } | CommandItem::Stacked { label, .. } => label.width(),
        })
        .max()
        .unwrap_or(0);
    let preview = items
        .iter()
        .filter_map(|item| match item {
            CommandItem::Inline { preview, .. } => Some(preview.width()),
            CommandItem::Stacked { .. } => None,
        })
        .max()
        .unwrap_or(0);
    let detail = items
        .iter()
        .filter_map(|item| match item {
            CommandItem::Inline { .. } => None,
            CommandItem::Stacked { detail, .. } => Some(detail.width()),
        })
        .max()
        .unwrap_or(0);
    let has_inline = items
        .iter()
        .any(|item| matches!(item, CommandItem::Inline { .. }));
    let has_stacked = items
        .iter()
        .any(|item| matches!(item, CommandItem::Stacked { .. }));
    let content = match (has_inline, has_stacked) {
        (true, false) => label + COLUMN_GAP + preview,
        (false, true) => label.max(detail),
        _ => (label + COLUMN_GAP + preview).max(label.max(detail)),
    }
    .min(usize::from(available));
    ((content + 4) as u16).min(available).max(MIN_WIDTH)
}

/// 窗口起点：从高亮项向前回退，直到行数预算装不下。
fn window_start(items: &[CommandItem<'_>], active: usize, budget: usize) -> usize {
    let mut used = 0;
    let mut start = active;
    for index in (0..=active).rev() {
        let height = usize::from(item_height(&items[index]));
        if used + height > budget {
            break;
        }
        used += height;
        start = index;
    }
    start
}

/// 自起点起、行数预算内可见的条目数。
fn visible_count(items: &[CommandItem<'_>], start: usize, budget: usize) -> usize {
    let mut used = 0;
    let mut count = 0;
    for item in items.iter().skip(start) {
        let height = usize::from(item_height(item));
        if used + height > budget {
            break;
        }
        used += height;
        count += 1;
    }
    count
}

/// 绘制可见条目；窗口滚动保证高亮项可见，单行预览列按最长标签对齐。
fn render_items(buf: &mut Buffer, layout: &PanelLayout, view: &CommandPanelView<'_>) {
    if layout.content.width == 0 || layout.content.height == 0 {
        return;
    }
    let label_width = view
        .items
        .iter()
        .map(|item| match item {
            CommandItem::Inline { label, .. } | CommandItem::Stacked { label, .. } => label.width(),
        })
        .max()
        .unwrap_or(0);
    let mut y = layout.content.y;
    for (offset, item) in view
        .items
        .iter()
        .skip(layout.start)
        .take(layout.visible)
        .enumerate()
    {
        let height = item_height(item);
        let row = Rect::new(layout.content.x, y, layout.content.width, height);
        render_item(
            buf,
            row,
            item,
            layout.start + offset == view.active,
            label_width,
        );
        y = y.saturating_add(height);
    }
}

/// 绘制单条目；高亮项整行反显，超宽由 Paragraph 截断。
fn render_item(
    buf: &mut Buffer,
    area: Rect,
    item: &CommandItem<'_>,
    active: bool,
    label_width: usize,
) {
    if area.width < 3 {
        return;
    }
    let row_style = if active {
        style::overlay_selection()
    } else {
        Style::default()
    };
    let inner = Rect::new(area.x + 1, area.y, area.width - 1, area.height);
    match item {
        CommandItem::Inline { label, preview } => {
            let padding = label_width.saturating_sub(label.width());
            let label = format!("{label}{}", " ".repeat(padding));
            let line = Line::from(vec![
                Span::raw(" "),
                Span::styled(label, style::text()),
                Span::raw(" ".repeat(COLUMN_GAP)),
                Span::styled(*preview, style::muted()),
            ]);
            Paragraph::new(line)
                .style(row_style)
                .render(Rect { height: 1, ..inner }, buf);
        }
        CommandItem::Stacked { label, detail } => {
            let mut lines = vec![Line::from(vec![
                Span::raw(" "),
                Span::styled(*label, style::text()),
            ])];
            if !detail.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(*detail, style::muted()),
                ]));
            }
            Paragraph::new(lines).style(row_style).render(inner, buf);
        }
    }
}

/// 内容区矩形：含边框面板向内收缩一圈。
fn inner_rect(rect: Rect) -> Rect {
    Rect::new(
        rect.x.saturating_add(1),
        rect.y.saturating_add(1),
        rect.width.saturating_sub(2),
        rect.height.saturating_sub(2),
    )
}

#[cfg(test)]
mod tests;

//! 浮层命令面板：条目列表、高亮、窗口滚动与边框绘制；无业务语义、不读状态。

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::ui::style;
use crate::ui::text;
use crate::ui::widgets::clear;
use crate::ui::widgets::scrollbar::{self, ScrollbarLayout};

/// 面板最小可用尺寸（含边框）。
const MIN_WIDTH: u16 = 3;
const MIN_HEIGHT: u16 = 3;

/// 名称列与预览列之间的间距（列）。
const COLUMN_GAP: usize = 1;

/// 名称/预览列与右侧来源标签之间的最小间距（列）。
const TAG_GAP: usize = 2;

/// 面板条目：单行（名称 + 预览 + 右对齐来源标签）或双行（图标 + 名称在上、明细在下）。
///
/// 单行条目的 `tag` 为右对齐来源标签（如 `[User]`），空串表示无标签；
/// 超宽时优先保留名称与标签，预览先截断、再整体省略。
/// 双行条目的 `icon_style` / `label_style` 由调用方按语义传入（如目录与文件分色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandItem<'a> {
    Inline {
        label: &'a str,
        preview: &'a str,
        tag: &'a str,
    },
    Stacked {
        icon: Option<&'a str>,
        icon_style: Style,
        label: &'a str,
        label_style: Style,
        detail: &'a str,
    },
}

/// 面板相对锚点行的放置偏好。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelPlacement {
    /// 按锚点行上下的可用空间择优。
    #[default]
    Auto,
    /// 始终绘制在锚点行上方（空间不足时缩高，绝不覆盖锚点行）。
    Above,
}

/// 面板视图：条目、高亮索引、窗口锚点/显式视口、锚点行与最大高度。
///
/// `anchor_row` 为内容区内的视觉行；面板绘制在其上方或下方，不覆盖该行。
/// `window_anchor` 为窗口底部锚定的条目索引：窗口以它为底向前回退填满预算；
/// `None` 表示锚定高亮。鼠标悬停只改 `active`、不改锚点，窗口因此不滚动。
/// `window_start` 为滚轮滚动后的显式视口起点，优先于 `window_anchor`：窗口从该
/// 条目起向前填充，高亮允许不在窗口内。
///
/// `current` 是"当前已生效项"（`/model` 正在用的模型、`/resume` 已绑定的会话）：
/// 该行行首显示 `❯` 且与高亮同底色，与 `active`（下一步要采纳的项）互相独立——
/// 两者可以是同一行，也可以是不同行，叠加时底色只画一份、互不覆盖。
///
/// `title` / `right_title` 绘制在顶边左右两端，`footer` 绘制在底边左端；
/// 三者均参与面板宽度计算，可用宽度不足时由 Block 自然截断。
///
/// `width` 强制面板宽度（含边框），用于需要与锚定控件等宽的面板；None 时按内容收缩。
/// `placement` 控制相对锚点行的放置；`Above` 用于必须紧贴锚点行上方的面板。
#[derive(Default)]
pub struct CommandPanelView<'a> {
    pub items: &'a [CommandItem<'a>],
    pub active: usize,
    /// 当前已生效项索引；`None` 表示本面板没有"当前项"概念（不占标记列）。
    pub current: Option<usize>,
    pub window_anchor: Option<usize>,
    pub window_start: Option<usize>,
    pub anchor_row: u16,
    /// 面板最大高度（含边框）；None 表示仅受可用空间限制。
    pub max_height: Option<u16>,
    /// 顶边左侧标题。
    pub title: Option<&'a str>,
    /// 顶边右侧状态文案（如当前目录名）。
    pub right_title: Option<&'a str>,
    /// 底边左侧快捷键提示。
    pub footer: Option<&'a str>,
    /// 强制面板宽度（含边框）；None 表示按内容收缩。
    pub width: Option<u16>,
    /// 相对锚点行的放置偏好。
    pub placement: PanelPlacement,
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
    let place_below = match view.placement {
        PanelPlacement::Auto => below >= needed || below >= above,
        // 强制上方：绝不覆盖锚点行；空间不足时缩高（可能小于正式高度而放弃）。
        PanelPlacement::Above => false,
    };
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
    let mut start = match view.window_start {
        // 显式视口：滚轮滚动后的窗口起点，高亮允许不在窗口内。
        Some(start) => start.min(max_window_start(view.items, budget)),
        None => {
            let anchor = view
                .window_anchor
                .map_or(active, |anchor| anchor.min(view.items.len() - 1));
            window_start(view.items, anchor, budget)
        }
    };
    let mut visible = visible_count(view.items, start, budget);
    // 跟随高亮时锚点窗口不含高亮（如列表变化）时回退到以高亮为底，保证高亮可见；
    // 显式视口不回退，否则滚轮滚动会被高亮拽回。
    if view.window_start.is_none() && (active < start || active >= start.saturating_add(visible)) {
        start = window_start(view.items, active, budget);
        visible = visible_count(view.items, start, budget);
    }
    if visible == 0 {
        return None;
    }
    let overflow = view.items.len() > visible;
    let width = match view.width {
        Some(width) => width.min(area.width).max(MIN_WIDTH),
        None => panel_width(area.width, view)
            .saturating_add(u16::from(overflow))
            .min(area.width),
    };
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
    clear::render_buffer(buf, layout.rect);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::muted())
        .style(style::overlay_panel());
    if let Some(title) = view.title {
        block = block.title_top(Line::from(Span::styled(
            format!(" {title} "),
            style::muted(),
        )));
    }
    if let Some(right_title) = view.right_title {
        block = block.title_top(
            Line::from(Span::styled(format!(" {right_title} "), style::muted()))
                .alignment(Alignment::Right),
        );
    }
    if let Some(footer) = view.footer {
        block = block.title_bottom(Line::from(Span::styled(
            format!(" {footer} "),
            style::muted(),
        )));
    }
    block.render(layout.rect, buf);
    render_items(buf, &layout, view);
    if let Some(scrollbar) = layout.scrollbar.as_ref() {
        // 命令面板是浮层：滚动条沿用面板底色，不透明。
        scrollbar::render_buffer_with_base(buf, scrollbar, style::overlay_panel());
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

/// 面板宽度：单行按最长标签 + 间距 + 最长预览，双行按标签与明细的较宽者，
/// 与顶边左右标题、底边提示的完整宽度取大，加两边框与左侧内边距。
fn panel_width(available: u16, view: &CommandPanelView<'_>) -> u16 {
    let items = view.items;
    let label = items
        .iter()
        .map(|item| item_label_width(item))
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
    let tag = items
        .iter()
        .filter_map(|item| match item {
            CommandItem::Inline { tag, .. } => Some(tag.width()),
            CommandItem::Stacked { .. } => None,
        })
        .max()
        .unwrap_or(0);
    let has_inline = items
        .iter()
        .any(|item| matches!(item, CommandItem::Inline { .. }));
    let has_stacked = items
        .iter()
        .any(|item| matches!(item, CommandItem::Stacked { .. }));
    let tag = if tag == 0 { 0 } else { TAG_GAP + tag + 2 };
    let marker = marker_width(view);
    let content = match (has_inline, has_stacked) {
        (true, false) => marker + label + COLUMN_GAP + preview + tag,
        (false, true) => marker + label.max(detail) + tag,
        _ => (marker + label + COLUMN_GAP + preview + tag).max(marker + label.max(detail) + tag),
    };
    // 标题与提示自带的包裹空格随文案宽度计入；最终宽度还需容纳两侧边框。
    let header = view.title.map_or(0, |title| title.width() + 2)
        + view.right_title.map_or(0, |title| title.width() + 2);
    let footer = view.footer.map_or(0, |footer| footer.width() + 2);
    let content = content.max(header).max(footer).min(usize::from(available));
    ((content + 4) as u16).min(available).max(MIN_WIDTH)
}

/// 显式视口起点的最大值：再靠后窗口就填不满预算（底部留白），钳到贴底窗口。
fn max_window_start(items: &[CommandItem<'_>], budget: usize) -> usize {
    window_start(items, items.len().saturating_sub(1), budget)
}

/// 条目标签列宽：双行条目的图标与其后一个空格计入名称列。
fn item_label_width(item: &CommandItem<'_>) -> usize {
    match item {
        CommandItem::Inline { label, .. } => label.width(),
        CommandItem::Stacked { icon, label, .. } => {
            icon.map_or(0, |icon| icon.width() + 1) + label.width()
        }
    }
}

/// "当前项"标记列宽（`❯ `）；面板没有当前项概念时为 0。
///
/// 全体条目共用同一列，标记项与未标记项因此左对齐。
fn marker_width(view: &CommandPanelView<'_>) -> usize {
    if view.current.is_some() {
        text::CURRENT_ITEM_MARKER.width() + 1
    } else {
        0
    }
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
        .map(|item| item_label_width(item))
        .max()
        .unwrap_or(0);
    let marker = marker_width(view);
    let mut y = layout.content.y;
    for (offset, item) in view
        .items
        .iter()
        .skip(layout.start)
        .take(layout.visible)
        .enumerate()
    {
        let index = layout.start + offset;
        let height = item_height(item);
        let row = Rect::new(layout.content.x, y, layout.content.width, height);
        render_item(
            buf,
            row,
            item,
            index == view.active,
            view.current == Some(index),
            marker,
            label_width,
        );
        y = y.saturating_add(height);
    }
}

/// 绘制单条目；高亮项与当前项整行铺底色，当前项另加行首 `❯` 标记。
fn render_item(
    buf: &mut Buffer,
    area: Rect,
    item: &CommandItem<'_>,
    active: bool,
    current: bool,
    marker: usize,
    label_width: usize,
) {
    if area.width < 3 {
        return;
    }
    // 当前项与高亮项共用同一底色；两者同时命中时只画一份（同一 Style，重复应用无差别）。
    let row_style = if active || current {
        style::overlay_selection()
    } else {
        Style::default()
    };
    let inner = Rect::new(
        area.x + 1,
        area.y,
        area.width.saturating_sub(1),
        area.height,
    );
    let marker_span = if current {
        format!("{} ", text::CURRENT_ITEM_MARKER)
    } else {
        // 未标记行补满同一列宽（标记 + 其后空格），名字左缘因此对齐。
        " ".repeat(marker)
    };
    match item {
        CommandItem::Inline {
            label,
            preview,
            tag,
        } => {
            let padding = label_width.saturating_sub(label.width());
            let label = format!("{label}{}", " ".repeat(padding));
            let available = usize::from(inner.width);
            let tag_label = (!tag.is_empty()).then(|| format!("[{tag}]"));
            // 名称与标签优先；预览按剩余宽度截断，缺空间时整体省略。
            // 行首占 1 列空格 + 标记列 + 名称 + 间距 + 标签。
            let reserved = 1
                + marker
                + label.width()
                + COLUMN_GAP
                + tag_label.as_ref().map_or(0, |tag| COLUMN_GAP + tag.width());
            let preview_width = available.saturating_sub(reserved);
            let preview = if preview_width == 0 {
                String::new()
            } else {
                crate::ui::text::truncate_to_width(preview, preview_width)
            };
            let mut spans = vec![Span::raw(" ")];
            if marker > 0 {
                spans.push(Span::styled(marker_span, style::accent()));
            }
            spans.push(Span::styled(label, style::text()));
            if !preview.is_empty() {
                spans.push(Span::raw(" ".repeat(COLUMN_GAP)));
                spans.push(Span::styled(preview, style::muted()));
            }
            if let Some(tag) = tag_label {
                let used: usize = spans.iter().map(|span| span.content.width()).sum();
                let pad = available.saturating_sub(used).saturating_sub(tag.width());
                if pad > 0 {
                    spans.push(Span::raw(" ".repeat(pad)));
                    spans.push(Span::styled(tag, style::muted()));
                }
            }
            let line = Line::from(spans);
            Paragraph::new(line)
                .style(row_style)
                .render(Rect { height: 1, ..inner }, buf);
        }
        CommandItem::Stacked {
            icon,
            icon_style,
            label,
            label_style,
            detail,
        } => {
            let mut spans = vec![Span::raw(" ")];
            if marker > 0 {
                spans.push(Span::styled(marker_span, style::accent()));
            }
            if let Some(icon) = icon {
                spans.push(Span::styled(*icon, *icon_style));
                spans.push(Span::raw(" "));
            }
            spans.push(Span::styled(*label, *label_style));
            let mut lines = vec![Line::from(spans)];
            if !detail.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(1 + marker)),
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

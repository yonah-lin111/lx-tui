//! 浮层命令面板：条目列表、高亮、窗口滚动与边框绘制；无业务语义、不读状态。

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Widget};
use unicode_width::UnicodeWidthStr;

use crate::ui::style;
use crate::ui::text;
use crate::ui::widgets::clear;
use crate::ui::widgets::scrollbar::{self, ScrollbarLayout};

/// 面板最小可用宽度（含边框）。
const MIN_WIDTH: u16 = 3;
/// 无边框面板顶部块的行数：1 行上方空行间隔 + 1 行顶部分割线。
const TOP_BLOCK_ROWS: u16 = 2;
/// 无边框面板最小高度（顶部 2 行 + 1 行内容）。
const MIN_BORDERLESS_HEIGHT: u16 = TOP_BLOCK_ROWS + 1;
/// 无边框面板底部提示块的行数：1 行空行间隔 + 1 行快捷键提示。
const FOOTER_BLOCK_ROWS: u16 = 2;

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

/// 顶边边框 Tab 标签页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelTab<'a> {
    pub label: &'a str,
    pub active: bool,
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
    /// 是否固定使用 `max_height` 作为面板高度（不足时内部下方留白，不随候选数自适应变化；
    /// 仅在可用空间小于 `max_height` 时按可用空间夹取）。
    pub fixed_height: bool,
    /// 顶边左侧标题。
    pub title: Option<&'a str>,
    /// 顶边 Tab 标签页组；若提供，优先于 title 绘制。
    pub tabs: Option<&'a [PanelTab<'a>]>,
    /// 顶边右侧状态文案（如当前目录名）。
    pub right_title: Option<&'a str>,
    /// 底边左侧快捷键提示。
    pub footer: Option<&'a str>,
    /// 强制面板宽度（含边框）；None 表示按内容收缩。
    pub width: Option<u16>,
    /// 相对锚点行的放置偏好。
    pub placement: PanelPlacement,
    /// 无边框透明样式（lx 输入框命令面板使用）：移除背景色与四周边框，
    /// 顶部仅绘制一条蓝色横线，条目左侧常驻 `❯` 选择列。
    pub borderless: bool,
    /// 是否隐藏正在选择项（`active`）的 `❯` 标记（用于空态/加载占位行）。
    pub hide_active_marker: bool,
    /// 禁用项掩码（下标与 `items` 对齐）：`true` 时该行用 `style::muted()` 置灰且不显示选择高亮与 `❯`。
    pub disabled: Option<&'a [bool]>,
}

/// 面板布局：面板矩形、条目内容区、滚动条、首个可见条目与可见条目数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelLayout {
    pub rect: Rect,
    pub inner: Rect,
    /// 条目绘制与命中区；滚动条出现时不含其占用列。
    pub content: Rect,
    pub scrollbar: Option<ScrollbarLayout>,
    pub start: usize,
    pub visible: usize,
    pub tabs: Vec<Rect>,
}

/// 计算面板布局；空间不足或没有可见条目时返回 None。
pub fn layout(area: Rect, view: &CommandPanelView<'_>) -> Option<PanelLayout> {
    let has_header = view.title.is_some()
        || view.right_title.is_some()
        || view.tabs.is_some_and(|tabs| !tabs.is_empty());
    let has_footer = view.footer.is_some();
    let available_rows = match view.placement {
        PanelPlacement::Auto => area
            .height
            .saturating_sub(view.anchor_row.saturating_add(1))
            .max(view.anchor_row),
        PanelPlacement::Above => view.anchor_row.min(area.height),
    };
    // 无边框面板：顶部「1 行空行间隔 + 1 行分割线」+ 底部「1 行空行间隔 + 1 行快捷键提示」；
    // 有边框面板（对齐 herdr）：空间充裕（≥ 7 行）时，有标题占顶部 3 行（`┌──┐` + 粗体标题行 + 1 行呼吸空白），
    // 有底栏占底部 3 行（1 行呼吸空白 + `│ footer │` + `└──┘`）；极窄矮空间（< 7 行）时退化为仅保留上下边框以优先展示候选。
    let roomy = available_rows >= 7;
    let top_rows: u16 = if view.borderless {
        TOP_BLOCK_ROWS
    } else if has_header && roomy {
        3
    } else {
        1
    };
    let bottom_rows: u16 = if view.borderless {
        if has_footer { FOOTER_BLOCK_ROWS } else { 0 }
    } else if has_footer && roomy {
        3
    } else {
        1
    };
    let frame_rows: u16 = top_rows + bottom_rows;
    let min_height = if view.borderless {
        MIN_BORDERLESS_HEIGHT + bottom_rows
    } else {
        frame_rows.saturating_add(1)
    };
    if view.items.is_empty() || area.width < MIN_WIDTH || area.height < min_height {
        return None;
    }
    let content_rows: u16 = view.items.iter().map(item_height).sum();
    let needed = content_rows.saturating_add(frame_rows);
    let needed = match view.max_height {
        Some(max_height) if view.fixed_height => max_height,
        Some(max_height) => needed.min(max_height),
        None => needed,
    };
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
    if height < min_height {
        return None;
    }
    let y = if place_below {
        area.y + view.anchor_row + 1
    } else {
        area.y + view.anchor_row - height
    };
    let budget = usize::from(height.saturating_sub(frame_rows));
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
    let inner = if view.borderless {
        borderless_inner_rect(rect, bottom_rows)
    } else {
        inner_rect(rect, top_rows, bottom_rows)
    };
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
    let tab_y = rect.y.saturating_add(1);
    let show_header = view.borderless || top_rows > 1;
    let tabs = match view.tabs {
        Some(tabs) if !tabs.is_empty() && show_header => {
            let mut out = Vec::with_capacity(tabs.len());
            let mut x = if view.borderless {
                if let Some(title) = view.title {
                    let text_w = u16::try_from(UnicodeWidthStr::width(title)).unwrap_or(0);
                    rect.x
                        .saturating_add(1)
                        .saturating_add(text_w)
                        .saturating_add(1)
                } else {
                    rect.x
                }
            } else if let Some(title) = view.title {
                let text_w = u16::try_from(UnicodeWidthStr::width(title)).unwrap_or(0);
                rect.x
                    .saturating_add(2)
                    .saturating_add(text_w)
                    .saturating_add(1)
            } else {
                rect.x.saturating_add(2)
            };
            let right_limit = rect.right().saturating_sub(1);
            for tab in tabs {
                let width = u16::try_from(UnicodeWidthStr::width(tab.label))
                    .unwrap_or(u16::MAX)
                    .saturating_add(2);
                if x.saturating_add(width) > right_limit {
                    break;
                }
                out.push(Rect::new(x, tab_y, width, 1));
                x = x.saturating_add(width).saturating_add(1);
            }
            out
        }
        _ => Vec::new(),
    };
    Some(PanelLayout {
        rect,
        inner,
        content,
        scrollbar,
        start,
        visible,
        tabs,
    })
}

/// 在内容区内绘制命令面板；返回实际绘制矩形，空间不足时返回 None。
pub fn render(area: Rect, buf: &mut Buffer, view: &CommandPanelView<'_>) -> Option<Rect> {
    let layout = layout(area, view)?;
    clear::render_buffer(buf, layout.rect);
    let has_tabs = view.tabs.is_some_and(|tabs| !tabs.is_empty());
    let has_header = has_tabs || view.title.is_some() || view.right_title.is_some();
    if view.borderless {
        render_top_rule(buf, &layout, view, has_tabs);
    } else {
        let block = Block::bordered().border_style(style::accent());
        block.render(layout.rect, buf);
        if has_header && layout.inner.y > layout.rect.y.saturating_add(1) {
            render_bordered_header(buf, &layout, view, has_tabs);
        }
        if let Some(footer) = view.footer
            && layout.inner.bottom() < layout.rect.bottom().saturating_sub(1)
        {
            render_bordered_footer(buf, layout.rect, footer);
        }
    }
    if let Some(tabs) = view.tabs {
        for (i, tab_rect) in layout.tabs.iter().enumerate() {
            if let Some(tab) = tabs.get(i) {
                let tab_style = if tab.active {
                    style::right_tab_active()
                } else {
                    style::muted()
                };
                let text = format!(" {} ", tab.label);
                buf.set_style(*tab_rect, Style::reset());
                buf.set_stringn(
                    tab_rect.x,
                    tab_rect.y,
                    &text,
                    usize::from(tab_rect.width),
                    tab_style,
                );
            }
        }
    }
    render_items(buf, &layout, view);
    if view.borderless
        && let Some(footer) = view.footer
    {
        render_borderless_footer(buf, layout.rect, footer);
    }
    if let Some(scrollbar) = layout.scrollbar.as_ref() {
        scrollbar::render_buffer(buf, scrollbar);
    }
    Some(layout.rect)
}

/// 绘制有边框命令面板的顶栏（`│ Title / Tabs ─── right_title │`）。
fn render_bordered_header(
    buf: &mut Buffer,
    layout: &PanelLayout,
    view: &CommandPanelView<'_>,
    has_tabs: bool,
) {
    let rect = layout.rect;
    let header_y = rect.y.saturating_add(1);
    let mut left_end = rect.x.saturating_add(1);
    if let Some(title) = view.title {
        let x = rect.x.saturating_add(2);
        let max_w = usize::from(rect.right().saturating_sub(1).saturating_sub(x));
        if max_w > 0 {
            let draw_w = u16::try_from(title.width().min(max_w)).unwrap_or(u16::MAX);
            buf.set_stringn(x, header_y, title, max_w, style::strong());
            left_end = x.saturating_add(draw_w);
        }
    }
    if has_tabs {
        for (i, tab_rect) in layout.tabs.iter().enumerate() {
            if i + 1 < layout.tabs.len() {
                let sep_x = tab_rect.right();
                if sep_x < rect.right().saturating_sub(1)
                    && let Some(cell) = buf.cell_mut((sep_x, header_y))
                {
                    cell.set_char('·');
                    cell.set_style(style::muted());
                }
            }
        }
        if let Some(last_tab) = layout.tabs.last() {
            left_end = last_tab.right();
        }
    }
    if let Some(right_title) = view.right_title {
        let right_edge = rect.right().saturating_sub(2);
        let min_x = left_end.saturating_add(1);
        let avail = usize::from(right_edge.saturating_sub(min_x));
        if avail >= 1 {
            let clipped = text::truncate_to_width(right_title, avail);
            let text_w = u16::try_from(clipped.width()).unwrap_or(u16::MAX);
            if text_w <= right_edge.saturating_sub(min_x) {
                let x = right_edge.saturating_sub(text_w);
                buf.set_stringn(x, header_y, &clipped, usize::from(text_w), style::muted());
            }
        }
    }
}

/// 绘制有边框命令面板的底栏（无 `├───┤` 分割线，上方保留 1 行呼吸空白）。
fn render_bordered_footer(buf: &mut Buffer, rect: Rect, footer: &str) {
    let y = rect.bottom().saturating_sub(2);
    let max_w = usize::from(rect.width.saturating_sub(4));
    if max_w > 0 {
        let clipped = text::truncate_to_width(footer, max_w);
        buf.set_stringn(rect.x.saturating_add(2), y, &clipped, max_w, style::muted());
    }
}

/// 绘制无边框命令面板底部的快捷键提示行（上方留 1 行空行间隔，无边框、无分割线，muted 单行，超宽截断）。
fn render_borderless_footer(buf: &mut Buffer, rect: Rect, footer: &str) {
    let y = rect.bottom().saturating_sub(1);
    let text = text::truncate_to_width(footer, usize::from(rect.width));
    buf.set_stringn(rect.x, y, &text, usize::from(rect.width), style::muted());
}

/// 绘制无边框命令面板的顶部蓝色横线及下方干净标题栏（无横线穿透文字）。
fn render_top_rule(
    buf: &mut Buffer,
    layout: &PanelLayout,
    view: &CommandPanelView<'_>,
    has_tabs: bool,
) {
    let rect = layout.rect;
    let rule_y = rect.y;
    for x in rect.x..rect.right() {
        if let Some(cell) = buf.cell_mut((x, rule_y)) {
            cell.set_char('─');
            cell.set_style(style::accent());
        }
    }
    let header_y = rect.y.saturating_add(1);
    let mut left_end = rect.x;
    if let Some(title) = view.title {
        let x = rect.x.saturating_add(1);
        let max_w = usize::from(rect.right().saturating_sub(1).saturating_sub(x));
        if max_w > 0 {
            let draw_w = u16::try_from(title.width().min(max_w)).unwrap_or(u16::MAX);
            buf.set_stringn(x, header_y, title, max_w, style::strong());
            left_end = x.saturating_add(draw_w);
        }
    }
    if has_tabs {
        for (i, tab_rect) in layout.tabs.iter().enumerate() {
            if i + 1 < layout.tabs.len() {
                let sep_x = tab_rect.right();
                if sep_x < rect.right().saturating_sub(1)
                    && let Some(cell) = buf.cell_mut((sep_x, header_y))
                {
                    cell.set_char('·');
                    cell.set_style(style::muted());
                }
            }
        }
        if let Some(last_tab) = layout.tabs.last() {
            left_end = last_tab.right();
        }
    }
    if let Some(right_title) = view.right_title {
        let right_edge = rect.right().saturating_sub(1);
        let min_x = left_end.saturating_add(1);
        let avail = usize::from(right_edge.saturating_sub(min_x));
        if avail >= 1 {
            let clipped = text::truncate_to_width(right_title, avail);
            let text_w = u16::try_from(clipped.width()).unwrap_or(u16::MAX);
            if text_w <= right_edge.saturating_sub(min_x) {
                let x = right_edge.saturating_sub(text_w);
                buf.set_stringn(x, header_y, &clipped, usize::from(text_w), style::muted());
            }
        }
    }
}

/// 命中面板顶边框 Tab：返回 Tab 索引；未命中返回 None。
pub fn tab_at(layout: &PanelLayout, column: u16, row: u16) -> Option<usize> {
    layout
        .tabs
        .iter()
        .position(|rect| rect.contains(Position::new(column, row)))
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
    let tabs_header = match view.tabs {
        Some(tabs) if !tabs.is_empty() => {
            let title_w = view.title.map_or(0, |title| title.width() + 1);
            let total_tabs: usize = tabs.iter().map(|t| t.label.width() + 2).sum();
            let separators = tabs.len().saturating_sub(1);
            title_w
                + total_tabs
                + separators
                + view.right_title.map_or(0, |title| title.width() + 2)
        }
        _ => {
            view.title.map_or(0, |title| title.width() + 2)
                + view.right_title.map_or(0, |title| title.width() + 2)
        }
    };
    // 底边提示：有边框面板带两侧包裹空格；无边框面板直接从面板左缘绘制单行提示。
    let footer = view.footer.map_or(0, |footer| {
        footer.width() + if view.borderless { 0 } else { 2 }
    });
    let frame_pad = if view.borderless { 0 } else { 4 };
    let content = content
        .max(tabs_header)
        .max(footer)
        .min(usize::from(available));
    ((content + frame_pad) as u16).min(available).max(MIN_WIDTH)
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

/// "当前项"或无边框面板的选择标记列宽（`❯ `）。
///
/// 全体条目共用同一列，标记项与未标记项因此左对齐。
fn marker_width(view: &CommandPanelView<'_>) -> usize {
    if view.borderless || view.current.is_some() {
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
    if view.hide_active_marker
        && let [
            CommandItem::Inline {
                label,
                preview: "",
                tag: "",
            },
        ] = view.items
    {
        let available = usize::from(layout.content.width);
        let clipped = crate::ui::text::truncate_to_width(label, available);
        let text_w = u16::try_from(clipped.width()).unwrap_or(0);
        let x = layout
            .content
            .x
            .saturating_add(layout.content.width.saturating_sub(text_w) / 2);
        let y = layout
            .content
            .y
            .saturating_add(layout.content.height.saturating_sub(1) / 2);
        buf.set_stringn(x, y, &clipped, available, style::muted());
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
        let is_disabled = view
            .disabled
            .and_then(|mask| mask.get(index))
            .copied()
            .unwrap_or(false);
        let show_active = !view.hide_active_marker
            && (!is_disabled || view.window_anchor.is_some())
            && index == view.active;
        render_item(
            buf,
            row,
            item,
            show_active,
            !is_disabled && view.current == Some(index),
            view.hide_active_marker || is_disabled,
            marker,
            label_width,
            view.borderless,
        );
        y = y.saturating_add(height);
    }
}

/// 绘制单条目。
///
/// - 带边框模式（Prompt 面板）：高亮项与当前项整行铺底色，当前项另加行首蓝色 `❯` 标记。
/// - 无边框模式（lx 命令面板）：不铺任何行背景色；正在选择的项（`active`）左侧显示默认颜色 `❯`，
///   已生效项（`current`）主名称标为蓝色（`style::accent()`），且未处于 `active` 时左侧显示蓝色 `❯`。
#[allow(clippy::too_many_arguments)]
fn render_item(
    buf: &mut Buffer,
    area: Rect,
    item: &CommandItem<'_>,
    active: bool,
    current: bool,
    placeholder: bool,
    marker: usize,
    label_width: usize,
    borderless: bool,
) {
    if area.width < 3 {
        return;
    }
    let row_style = if !borderless && active {
        style::overlay_selection()
    } else {
        Style::default()
    };
    let inner = if borderless {
        area
    } else {
        Rect::new(
            area.x + 1,
            area.y,
            area.width.saturating_sub(1),
            area.height,
        )
    };
    let lead = usize::from(!borderless);
    let (marker_span, marker_style) = if borderless {
        if active {
            let s = if placeholder {
                style::muted()
            } else {
                style::text()
            };
            (format!("{} ", text::CURRENT_ITEM_MARKER), s)
        } else if current {
            (format!("{} ", text::CURRENT_ITEM_MARKER), style::accent())
        } else {
            (" ".repeat(marker), style::text())
        }
    } else if current {
        (format!("{} ", text::CURRENT_ITEM_MARKER), style::accent())
    } else {
        // 未标记行补满同一列宽（标记 + 其后空格），名字左缘因此对齐。
        (" ".repeat(marker), style::accent())
    };
    match item {
        CommandItem::Inline {
            label,
            preview,
            tag,
        } => {
            let available = usize::from(inner.width);
            let tag_label = (!tag.is_empty()).then(|| format!("[{tag}]"));
            let tag_width = tag_label.as_ref().map_or(0, |t| t.width());
            let max_label_width = if tag_width > 0 {
                available.saturating_sub(lead + marker + tag_width + 1)
            } else {
                available.saturating_sub(lead + marker)
            };
            let effective_label_width = label_width.min(max_label_width);
            let truncated_label = crate::ui::text::truncate_to_width(label, effective_label_width);
            let padding = effective_label_width.saturating_sub(truncated_label.width());
            let label = format!("{truncated_label}{}", " ".repeat(padding));
            // 名称与标签优先；预览按剩余宽度截断，缺空间时整体省略。
            // 行首占前导空格 + 标记列 + 名称 + 间距 + 标签。
            let reserved = lead
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
            let mut spans = Vec::new();
            if lead > 0 {
                spans.push(Span::raw(" ".repeat(lead)));
            }
            if marker > 0 {
                spans.push(Span::styled(marker_span, marker_style));
            }
            let item_label_style = if current {
                style::accent()
            } else if placeholder {
                style::muted()
            } else {
                style::text()
            };
            spans.push(Span::styled(label, item_label_style));
            if !preview.is_empty() {
                spans.push(Span::raw(" ".repeat(COLUMN_GAP)));
                spans.push(Span::styled(preview, style::muted()));
            }
            if let Some(tag) = tag_label {
                let used: usize = spans.iter().map(|span| span.content.width()).sum();
                if used + tag.width() <= available {
                    let pad = available - used - tag.width();
                    if pad > 0 {
                        spans.push(Span::raw(" ".repeat(pad)));
                    }
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
            let mut spans = Vec::new();
            if lead > 0 {
                spans.push(Span::raw(" ".repeat(lead)));
            }
            if marker > 0 {
                spans.push(Span::styled(marker_span, marker_style));
            }
            if let Some(icon) = icon {
                spans.push(Span::styled(*icon, *icon_style));
                spans.push(Span::raw(" "));
            }
            let item_label_style = if borderless && current {
                style::accent()
            } else {
                *label_style
            };
            spans.push(Span::styled(*label, item_label_style));
            let mut lines = vec![Line::from(spans)];
            if !detail.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(lead + marker)),
                    Span::styled(*detail, style::muted()),
                ]));
            }
            Paragraph::new(lines).style(row_style).render(inner, buf);
        }
    }
}

/// 内容区矩形：含边框面板扣除顶部框与底部框行数，左右各扣除 1 列边框。
fn inner_rect(rect: Rect, top_rows: u16, bottom_rows: u16) -> Rect {
    Rect::new(
        rect.x.saturating_add(1),
        rect.y.saturating_add(top_rows),
        rect.width.saturating_sub(2),
        rect.height
            .saturating_sub(top_rows)
            .saturating_sub(bottom_rows),
    )
}

/// 无边框面板内容区矩形：扣除顶部两行（1 行空行 + 1 行分割线）与底部可选快捷键提示行，左右不留边框。
fn borderless_inner_rect(rect: Rect, footer_rows: u16) -> Rect {
    Rect::new(
        rect.x,
        rect.y.saturating_add(TOP_BLOCK_ROWS),
        rect.width,
        rect.height
            .saturating_sub(TOP_BLOCK_ROWS)
            .saturating_sub(footer_rows),
    )
}

#[cfg(test)]
mod tests;

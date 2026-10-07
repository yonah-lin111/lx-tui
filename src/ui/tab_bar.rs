//! 标签栏：几何、渲染与命中；标签矩形与 `[+]/[<]/[>]` 按钮由同一布局函数产出。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::state::{AppState, Workspace, tab_label};
use crate::ui::layout::ViewLayout;
use crate::ui::{exit_button, style, text};

/// 标签文本左右各留一列空格。
const TAB_PADDING: usize = 2;
/// 标签之间的竖直分隔线宽度。
const TAB_SEPARATOR: usize = 1;

/// 标签栏布局：可见标签（索引 + 矩形）、按钮矩形与滚动范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabBarLayout {
    pub tabs: Vec<(usize, Rect)>,
    /// 每个标签的完整宽度，与标签索引一一对应。
    pub tab_widths: Vec<u16>,
    pub add: Option<Rect>,
    pub scroll_left: Option<Rect>,
    pub scroll_right: Option<Rect>,
    /// 实际生效的滚动偏移（已按上限钳制）。
    pub scroll: usize,
    pub max_scroll: usize,
    pub overflow: bool,
}

/// 单个标签的完整宽度（含内边距与分隔线）。
fn tab_width(tab_name: Option<&str>, index: usize) -> u16 {
    let label_width = tab_label(index, tab_name).chars().count();
    u16::try_from(label_width + TAB_PADDING + TAB_SEPARATOR).unwrap_or(u16::MAX)
}

/// 按当前几何计算标签栏布局；不溢出时不出现滚动按钮，`[+]` 紧跟最后一个标签。
pub fn layout(view: &ViewLayout, workspace: &Workspace, scroll: usize) -> TabBarLayout {
    let area = view.tab_bar;
    if area.width == 0 || area.height == 0 {
        return TabBarLayout {
            tabs: Vec::new(),
            tab_widths: Vec::new(),
            add: None,
            scroll_left: None,
            scroll_right: None,
            scroll: 0,
            max_scroll: 0,
            overflow: false,
        };
    }
    let right = exit_button(view)
        .map(|exit| exit.x)
        .unwrap_or_else(|| area.right());
    let available = right.saturating_sub(area.x);
    let widths: Vec<u16> = workspace
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| tab_width(tab.name.as_deref(), index))
        .collect();
    let total: u32 = widths.iter().map(|width| u32::from(*width)).sum();
    let add_width = text::ADD_TAB_LABEL.chars().count() as u16;
    let overflow = total.saturating_add(u32::from(add_width)) > u32::from(available);
    if !overflow {
        let mut x = area.x;
        let mut tabs = Vec::with_capacity(widths.len());
        for (index, width) in widths.iter().enumerate() {
            tabs.push((index, Rect::new(x, area.y, *width, 1)));
            x = x.saturating_add(*width);
        }
        let add = Rect::new(x, area.y, add_width.min(right.saturating_sub(x)), 1);
        let add = (add.width > 0).then_some(add);
        return TabBarLayout {
            tabs,
            tab_widths: widths,
            add,
            scroll_left: None,
            scroll_right: None,
            scroll: 0,
            max_scroll: 0,
            overflow: false,
        };
    }

    // 溢出：`[<]` 贴标签区左缘，`[+] [>]` 依次贴 `[exit]` 左侧，标签在中间视口内滚动。
    let button_width = text::TAB_SCROLL_LEFT_LABEL.chars().count() as u16;
    let scroll_right = Rect::new(
        right.saturating_sub(button_width),
        area.y,
        button_width.min(available),
        1,
    );
    let add = Rect::new(
        scroll_right.x.saturating_sub(add_width),
        area.y,
        add_width.min(scroll_right.x.saturating_sub(area.x)),
        1,
    );
    let viewport_x = area.x.saturating_add(button_width).min(add.x);
    let viewport = Rect::new(viewport_x, area.y, add.x.saturating_sub(viewport_x), 1);
    let max_scroll = max_scroll(&widths, viewport.width);
    let scroll = scroll.min(max_scroll);
    let mut x = viewport.x;
    let mut tabs = Vec::new();
    for (index, width) in widths.iter().enumerate().skip(scroll) {
        let visible = (*width).min(viewport.right().saturating_sub(x));
        if visible == 0 {
            break;
        }
        tabs.push((index, Rect::new(x, area.y, visible, 1)));
        x = x.saturating_add(*width);
    }
    TabBarLayout {
        tabs,
        tab_widths: widths,
        add: (add.width > 0).then_some(add),
        scroll_left: Some(Rect::new(area.x, area.y, button_width.min(available), 1)),
        scroll_right: (scroll_right.width > 0).then_some(scroll_right),
        scroll,
        max_scroll,
        overflow: true,
    }
}

/// 最大滚动偏移：能让最后一个标签完整可见的最小偏移；标签比视口还宽时退化为最右可见。
fn max_scroll(widths: &[u16], viewport_width: u16) -> usize {
    let Some(last) = widths.len().checked_sub(1) else {
        return 0;
    };
    let viewport = u32::from(viewport_width);
    let last_width = u32::from(widths[last]);
    let before = |scroll: usize| -> u32 {
        widths[scroll..last]
            .iter()
            .map(|width| u32::from(*width))
            .sum()
    };
    (0..widths.len())
        .find(|&scroll| before(scroll).saturating_add(last_width) <= viewport)
        .or_else(|| (0..widths.len()).find(|&scroll| before(scroll) < viewport))
        .unwrap_or(0)
}

/// 命中标签：返回可见标签索引；分隔线属于其左侧标签。
pub fn tab_at(layout: &TabBarLayout, column: u16, row: u16) -> Option<usize> {
    layout
        .tabs
        .iter()
        .find_map(|(index, rect)| rect.contains((column, row).into()).then_some(*index))
}

/// 保证激活标签完整可见的目标滚动偏移。
pub fn reveal_scroll(bar: &TabBarLayout, workspace: &Workspace) -> usize {
    if !bar.overflow {
        return 0;
    }
    let active = workspace.active_tab;
    let full = bar.tab_widths.get(active).copied().unwrap_or(0);
    let visible = bar
        .tabs
        .iter()
        .find(|(index, _)| *index == active)
        .is_some_and(|(_, rect)| rect.width == full);
    if visible {
        bar.scroll
    } else {
        active.min(bar.max_scroll)
    }
}

/// 渲染标签栏：激活标签青底黑字、悬停标签终端反显（只铺标签文字区，不含尾部 `|` 分隔符）、
/// `[+]` 强调色、滚动按钮可滚强调色/不可滚 muted。
pub fn render(frame: &mut Frame<'_>, view: &ViewLayout, state: &AppState) {
    let workspace = state.active_workspace();
    let bar = layout(view, workspace, state.tab_scroll);
    for (index, rect) in &bar.tabs {
        let Some(tab) = workspace.tabs.get(*index) else {
            continue;
        };
        let label = tab_label(*index, tab.name.as_deref());
        let label_style = if *index == workspace.active_tab {
            style::selected_item()
        } else if state.tab_hover == Some(*index) {
            style::selection()
        } else {
            style::muted()
        };
        let label_width = usize::from(rect.width).saturating_sub(TAB_PADDING + TAB_SEPARATOR);
        let label_text = text::ellipsize(&label, label_width);
        let text = format!(" {:<width$} ", label_text, width = label_width);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(text, label_style),
                Span::styled(text::TAB_SEPARATOR, style::muted()),
            ])),
            *rect,
        );
    }
    if let Some(area) = bar.add {
        render_label(frame, area, text::ADD_TAB_LABEL, style::accent());
    }
    if let Some(area) = bar.scroll_left {
        let can_scroll = bar.scroll > 0;
        let button_style = if can_scroll {
            style::text()
        } else {
            style::muted()
        };
        render_label(frame, area, text::TAB_SCROLL_LEFT_LABEL, button_style);
    }
    if let Some(area) = bar.scroll_right {
        let can_scroll = bar.scroll < bar.max_scroll;
        let button_style = if can_scroll {
            style::text()
        } else {
            style::muted()
        };
        render_label(frame, area, text::TAB_SCROLL_RIGHT_LABEL, button_style);
    }
}

/// 在矩形内绘制定宽标签。
fn render_label(
    frame: &mut Frame<'_>,
    area: Rect,
    label: &str,
    label_style: ratatui::style::Style,
) {
    frame.render_widget(Paragraph::new(Span::styled(label, label_style)), area);
}

#[cfg(test)]
mod tests;

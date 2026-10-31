//! 菜单组件：锚点定位、项渲染与命中；不依赖应用领域数据。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// 菜单最小宽度（列，含边框）。
const MIN_WIDTH: u16 = 14;
/// 菜单左右内边距合计（含边框）。
const HORIZONTAL_PADDING: usize = 4;

/// 菜单几何：整体区域与每项矩形。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuLayout {
    pub area: Rect,
    pub item_rects: Vec<Rect>,
}

/// 按锚点、标题与标签计算菜单几何；锚点与尺寸均夹取到屏幕内。
///
/// 有标题时顶部占 3 行（`┌──┐` + `│ Title ─── │` + `├──┤`），无标题占 1 行顶边。
pub fn layout(screen: Rect, anchor: (u16, u16), title: &str, labels: &[&str]) -> MenuLayout {
    let max_label = labels
        .iter()
        .map(|label| label.width())
        .max()
        .unwrap_or_default();
    let max_title = title.width().saturating_add(4);
    let width = u16::try_from(max_label.saturating_add(HORIZONTAL_PADDING).max(max_title))
        .unwrap_or(u16::MAX)
        .max(MIN_WIDTH)
        .min(screen.width);
    let header_rows: usize = if title.is_empty() { 1 } else { 3 };
    let height = u16::try_from(labels.len().saturating_add(header_rows + 1))
        .unwrap_or(u16::MAX)
        .min(screen.height);
    let x = anchor
        .0
        .clamp(screen.x, screen.x + screen.width.saturating_sub(width));
    let y = anchor
        .1
        .clamp(screen.y, screen.y + screen.height.saturating_sub(height));
    let area = Rect::new(x, y, width, height);
    let item_top = area.y.saturating_add(header_rows as u16);
    let item_rects = (0..labels.len() as u16)
        .map(|index| {
            Rect::new(
                area.x + 1,
                item_top + index,
                area.width.saturating_sub(2),
                1,
            )
        })
        .collect();
    MenuLayout { area, item_rects }
}

/// 命中菜单项索引；未命中返回 None。
pub fn item_at(layout: &MenuLayout, column: u16, row: u16) -> Option<usize> {
    layout
        .item_rects
        .iter()
        .position(|rect| rect.contains((column, row).into()))
}

/// 渲染菜单：清底、强调色边框与顶栏标题框、逐项文本；选中项使用悬停高亮，待确认项使用危险红底高亮。
pub fn render(
    frame: &mut Frame<'_>,
    layout: &MenuLayout,
    title: &str,
    labels: &[&str],
    selected: Option<usize>,
) {
    render_with_confirm(frame, layout, title, labels, selected, None);
}

/// 渲染菜单（支持指定某一项处于行内二次确认态）。
pub fn render_with_confirm(
    frame: &mut Frame<'_>,
    layout: &MenuLayout,
    title: &str,
    labels: &[&str],
    selected: Option<usize>,
    confirming: Option<usize>,
) {
    clear::render(frame, layout.area);
    let block = Block::bordered().border_style(style::accent());
    frame.render_widget(block, layout.area);
    if !title.is_empty() && layout.area.height >= 4 && layout.area.width >= 4 {
        let max_title = usize::from(layout.area.width.saturating_sub(4));
        let fitted = text::ellipsize(title, max_title);
        let title_rect = Rect::new(
            layout.area.x + 2,
            layout.area.y + 1,
            layout.area.width.saturating_sub(4),
            1,
        );
        frame.render_widget(
            Paragraph::new(Span::styled(fitted, style::border_title())),
            title_rect,
        );
        crate::ui::widgets::modal::paint_divider(
            frame.buffer_mut(),
            layout.area,
            layout.area.y + 2,
            style::accent(),
        );
    }
    let width = usize::from(layout.area.width.saturating_sub(2));
    for (index, label) in labels.iter().enumerate() {
        let Some(rect) = layout.item_rects.get(index) else {
            continue;
        };
        if rect.y >= layout.area.bottom().saturating_sub(1) {
            continue;
        }
        let item_style = if confirming == Some(index) {
            style::menu_confirm()
        } else if selected == Some(index) {
            style::overlay_selection()
        } else {
            style::text()
        };
        let rendered = if label.width() <= width {
            (*label).to_string()
        } else {
            text::ellipsize(label, width)
        };
        let line = Line::from(Span::styled(rendered, item_style));
        frame.render_widget(Paragraph::new(line).style(item_style), *rect);
    }
}

#[cfg(test)]
mod tests;

//! 菜单组件：锚点定位、项渲染与命中；不依赖应用领域数据。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

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
pub fn layout(screen: Rect, anchor: (u16, u16), title: &str, labels: &[&str]) -> MenuLayout {
    let max_label = labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or_default();
    // 边框标题以 ` title ` 占位，加减两角。
    let max_title = title.chars().count().saturating_add(4);
    let width = u16::try_from(max_label.saturating_add(HORIZONTAL_PADDING).max(max_title))
        .unwrap_or(u16::MAX)
        .max(MIN_WIDTH)
        .min(screen.width);
    let height = u16::try_from(labels.len().saturating_add(2))
        .unwrap_or(u16::MAX)
        .min(screen.height);
    let x = anchor
        .0
        .clamp(screen.x, screen.x + screen.width.saturating_sub(width));
    let y = anchor
        .1
        .clamp(screen.y, screen.y + screen.height.saturating_sub(height));
    let area = Rect::new(x, y, width, height);
    let item_rects = (0..labels.len() as u16)
        .map(|index| {
            Rect::new(
                area.x + 1,
                area.y + 1 + index,
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

/// 渲染菜单：清底、强调色边框与标题、逐项文本；选中项反显。
pub fn render(
    frame: &mut Frame<'_>,
    layout: &MenuLayout,
    title: &str,
    labels: &[&str],
    selected: Option<usize>,
) {
    frame.render_widget(Clear, layout.area);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::accent());
    if !title.is_empty() {
        let max_title = usize::from(layout.area.width.saturating_sub(2));
        block = block.title(Span::styled(
            format!(" {} ", text::ellipsize(title, max_title)),
            style::muted(),
        ));
    }
    frame.render_widget(block, layout.area);
    let width = usize::from(layout.area.width.saturating_sub(2));
    for (index, label) in labels.iter().enumerate() {
        let Some(rect) = layout.item_rects.get(index) else {
            continue;
        };
        let item_style = if selected == Some(index) {
            style::selection()
        } else {
            style::text()
        };
        let line = Line::from(Span::styled(text::ellipsize(label, width), item_style));
        // 行级样式让选中项整行反显，而不是只反显文字。
        frame.render_widget(Paragraph::new(line).style(item_style), *rect);
    }
}

#[cfg(test)]
mod tests;

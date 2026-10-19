//! 滚动条组件：几何、命中与渲染；不依赖应用领域数据。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::ui::style;

/// thumb 符号。
///
/// 轨道不再绘制任何符号（只保留占位列与命中几何），右侧只留这一条滑动的实心条：
/// 轨道与 thumb 叠在一起的两道竖线在浮层深底上观感很脏，去轨道后与主流终端 UI 一致。
const THUMB_SYMBOL: &str = "▐";

/// 滚动条几何：轨道、thumb 与视口信息；不需要滚动时为 None。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollbarLayout {
    pub track: Rect,
    pub thumb: Rect,
    pub total: usize,
    pub visible: usize,
}

/// 计算滚动条几何：`area` 为列表内容区，轨道贴其右缘一列；`total <= visible` 时返回 None。
pub fn layout(area: Rect, total: usize, visible: usize, offset: usize) -> Option<ScrollbarLayout> {
    if visible == 0 || total <= visible || area.width == 0 || area.height == 0 {
        return None;
    }
    let track = Rect::new(area.right() - 1, area.y, 1, area.height);
    let track_height = usize::from(track.height);
    let thumb_len = (visible.saturating_mul(track_height) / total)
        .max(1)
        .min(track_height);
    let max_offset = total - visible;
    let max_thumb_top = track_height - thumb_len;
    let offset = offset.min(max_offset);
    let thumb_top = offset
        .saturating_mul(max_thumb_top)
        .checked_div(max_offset)
        .unwrap_or(0);
    let thumb = Rect::new(
        track.x,
        track.y + u16::try_from(thumb_top).unwrap_or(0),
        1,
        u16::try_from(thumb_len).unwrap_or(1),
    );
    Some(ScrollbarLayout {
        track,
        thumb,
        total,
        visible,
    })
}

/// 命中 thumb：返回抓取偏移（点击行相对 thumb 顶部）。
pub fn thumb_grab_offset(layout: &ScrollbarLayout, row: u16) -> Option<u16> {
    if row >= layout.thumb.y && row < layout.thumb.bottom() {
        Some(row - layout.thumb.y)
    } else {
        None
    }
}

/// 点击轨道：以 thumb 中心对齐点击行，返回目标偏移。
pub fn offset_from_track_row(layout: &ScrollbarLayout, row: u16) -> usize {
    let row_offset =
        row.clamp(layout.track.y, layout.track.bottom().saturating_sub(1)) - layout.track.y;
    let thumb_len = usize::from(layout.thumb.height);
    offset_for_thumb_top(
        layout,
        usize::from(row_offset).saturating_sub(thumb_len / 2),
    )
}

/// 拖拽 thumb 到行：返回目标偏移；`grab` 为按下时的抓取偏移。
pub fn offset_from_drag_row(layout: &ScrollbarLayout, row: u16, grab: u16) -> usize {
    let row_offset =
        row.clamp(layout.track.y, layout.track.bottom().saturating_sub(1)) - layout.track.y;
    offset_for_thumb_top(
        layout,
        usize::from(row_offset).saturating_sub(usize::from(grab)),
    )
}

/// 由 thumb 顶部行换算滚动偏移。
fn offset_for_thumb_top(layout: &ScrollbarLayout, thumb_top: usize) -> usize {
    let max_offset = layout.total.saturating_sub(layout.visible);
    let max_thumb_top =
        usize::from(layout.track.height).saturating_sub(usize::from(layout.thumb.height));
    if max_thumb_top == 0 {
        return 0;
    }
    thumb_top.min(max_thumb_top).saturating_mul(max_offset) / max_thumb_top
}

/// 渲染滚动条：轨道留空（不画符号），thumb 用终端原生前景色。
///
/// 轨道列仍占位并参与命中（轨道点击跳转、拖拽 thumb 都依赖 [`ScrollbarLayout::track`]），
/// 只是不再绘制 `▕` 竖线。非浮层区域保持透明底；浮层（弹窗、命令面板等）用
/// [`render_with_base`] 传入面板底色。
pub fn render(frame: &mut Frame<'_>, layout: &ScrollbarLayout) {
    render_with_base(frame, layout, Style::default());
}

/// 在缓冲区内渲染滚动条（透明底）。
pub fn render_buffer(buf: &mut Buffer, layout: &ScrollbarLayout) {
    render_buffer_with_base(buf, layout, Style::default());
}

/// 渲染滚动条并叠加面板底色：thumb 保留自身前景与修饰符，背景沿用 `base`。
pub fn render_with_base(frame: &mut Frame<'_>, layout: &ScrollbarLayout, base: Style) {
    render_buffer_with_base(frame.buffer_mut(), layout, base);
}

/// 在缓冲区内渲染滚动条并叠加面板底色；`base` 通常为 `style::overlay_panel()`。
///
/// 轨道不绘制符号，但会按 `base` 重置为面板底色，保证浮层底下的旧内容不透过滚动条列。
pub fn render_buffer_with_base(buf: &mut Buffer, layout: &ScrollbarLayout, base: Style) {
    for y in layout.track.y..layout.track.bottom() {
        if let Some(cell) = buf.cell_mut((layout.track.x, y)) {
            cell.reset();
            cell.set_symbol(" ");
            cell.set_style(base);
        }
    }
    for y in layout.thumb.y..layout.thumb.bottom() {
        if let Some(cell) = buf.cell_mut((layout.thumb.x, y)) {
            cell.reset();
            cell.set_symbol(THUMB_SYMBOL);
            cell.set_style(base.patch(style::text()));
        }
    }
}

#[cfg(test)]
mod tests;

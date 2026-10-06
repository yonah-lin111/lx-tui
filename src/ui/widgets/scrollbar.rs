//! 滚动条组件：几何、命中与渲染；不依赖应用领域数据。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::ui::style;

/// 轨道符号。
const TRACK_SYMBOL: &str = "▕";
/// thumb 符号。
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

/// 渲染滚动条：轨道 muted、thumb accent。
pub fn render(frame: &mut Frame<'_>, layout: &ScrollbarLayout) {
    let buf = frame.buffer_mut();
    for y in layout.track.y..layout.track.bottom() {
        if let Some(cell) = buf.cell_mut((layout.track.x, y)) {
            cell.reset();
            cell.set_symbol(TRACK_SYMBOL);
            cell.set_style(style::muted());
        }
    }
    for y in layout.thumb.y..layout.thumb.bottom() {
        if let Some(cell) = buf.cell_mut((layout.thumb.x, y)) {
            cell.reset();
            cell.set_symbol(THUMB_SYMBOL);
            cell.set_style(style::accent());
        }
    }
}

#[cfg(test)]
mod tests;

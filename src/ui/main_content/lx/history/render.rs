//! 历史绘制：按内容行偏移把块写进 lx 页网格。
//!
//! 从 `history.rs` 拆出（单文件行数上限）。行数与绘制数据同源：
//! [`super::snapshot`] 里每块的 `rows` 就是 `paint.rows()`，这里只按视口挑块与行。
//!
//! 三层裁剪：整条时间线条目在视口外 → 整块在视口外 → 块内按行窗口裁剪。

use ratatui::layout::Rect;

use crate::app::state::Lx;

use super::cache::{BlockKey, PaneCache};
use super::content_rect;

/// 渲染历史内容；`scroll` 为顶部行偏移（0 表示从最早开始）。`rect` 为内容区
/// （body，不含置顶行；含右侧滚动条列）。
///
/// 返回值仍是本次推进的内容行数，与 [`super::layout`] 的 `total_rows` **必须相等**。
pub(crate) fn render(
    grid: &mut super::Grid,
    rect: Rect,
    lx: &Lx,
    scroll: usize,
    cache: &mut PaneCache,
) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    let content = content_rect(rect);
    let snapshot = cache.snapshot(content.width, lx);
    let viewport_end = scroll.saturating_add(usize::from(rect.height));
    let mut used: Vec<BlockKey> = Vec::new();
    for entry in &snapshot.entries {
        // 整条在视口外：不排版、不绘制。
        if entry.end <= scroll || entry.start >= viewport_end {
            continue;
        }
        for block in &entry.blocks {
            let end = block.start.saturating_add(block.rows);
            if end <= scroll || block.start >= viewport_end {
                continue;
            }
            used.push(block.key);
            block
                .paint
                .draw(grid, content, scroll, block.start, lx, block.source);
        }
    }
    // 视口内真正画过的块刷新 LRU；快照重建本身不算使用。
    for key in used {
        cache.mark_used(key);
    }
    snapshot.total_rows
}

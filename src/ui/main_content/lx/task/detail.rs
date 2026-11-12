//! BTW / Agents 详情区共用绘制：同一份 `HistorySource`、量行与阅读锚点。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::ui::main_content::lx::history::{self, HistorySource, StepKind};
use crate::ui::style;

use super::super::Grid;

/// 详情区的滚动状态。
#[derive(Debug, Clone, Copy)]
pub(crate) struct DetailView<'a> {
    pub source: HistorySource<'a>,
    pub scroll: usize,
    pub follow: bool,
}

/// 没有历史可画时的占位。
#[derive(Debug, Clone, Copy)]
pub(crate) enum EmptyState<'a> {
    Placeholder(&'a str),
    Loading { label: &'a str, frame: usize },
}

/// 绘制详情区并返回总行数。
pub(crate) fn draw(
    grid: &mut Grid,
    rect: Rect,
    view: DetailView<'_>,
    empty: EmptyState<'_>,
    cache: &mut crate::ui::PaneCache,
) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    if !view.source.has_history() {
        draw_empty(grid, rect, empty);
        return 1;
    }
    let rendered = history::view_source_plain(rect, view.source, cache);
    let visible = usize::from(rendered.body.height);
    let max = rendered.total_rows.saturating_sub(visible);
    let top = if view.follow {
        max
    } else {
        view.scroll.min(max)
    };
    let total = history::render_source(grid, rendered.body, view.source, top, cache);
    if let Some(bar) = history::scrollbar(rendered.body, rendered.total_rows, visible, top) {
        for y in bar.thumb.y..bar.thumb.bottom() {
            grid.set(bar.thumb.x, y, '▐', style::text());
        }
    }
    total
}

/// 量详情区总行数；返回 `(total_rows, fresh)`。
pub(crate) fn measure(
    rect: Rect,
    view: DetailView<'_>,
    cache: &mut crate::ui::PaneCache,
) -> (usize, bool) {
    let content_width = history::content_rect(rect).width;
    if content_width == 0 {
        return (usize::from(rect.height), true);
    }
    if !view.source.has_history() {
        return (1, true);
    }
    cache.set_viewport_rows(usize::from(rect.height));
    history::measure_cached(content_width, view.source, cache)
}

/// 变化前的阅读锚点（只读上一帧同宽度快照）。
pub(crate) fn anchor_before(
    rect: Rect,
    view: DetailView<'_>,
    scroll: usize,
    cache: &mut crate::ui::PaneCache,
) -> Option<history::HistoryAnchor> {
    let width = history::content_rect(rect).width;
    if width == 0 {
        return None;
    }
    let snapshot = cache.snapshot_cached(width, view.source)?;
    history::anchor_at(&snapshot, scroll, width)
}

/// 变化后的锚点行（快照刚由 [`measure`] 重建）。
pub(crate) fn anchor_row(
    rect: Rect,
    view: DetailView<'_>,
    anchor: &history::HistoryAnchor,
    cache: &mut crate::ui::PaneCache,
) -> Option<usize> {
    let width = history::content_rect(rect).width;
    if width == 0 {
        return None;
    }
    let snapshot = cache.snapshot_ready_source(width, view.source);
    history::anchor_row(&snapshot, view.source, anchor, width)
}

/// 空内容：占位文本或与主历史一致的呼吸 loading。
pub(crate) fn draw_empty(grid: &mut Grid, rect: Rect, empty: EmptyState<'_>) {
    match empty {
        EmptyState::Placeholder(label) => super::draw_placeholder(grid, rect, label),
        EmptyState::Loading { label, frame } => {
            let mark = history::StepMark::Running;
            let mark_text = format!("{} ", mark.glyph(StepKind::Body, frame, true));
            let total_w = u16::try_from(mark_text.width() + label.width()).unwrap_or(u16::MAX);
            let x = rect.x + rect.width.saturating_sub(total_w) / 2;
            grid.put_clipped(
                x,
                rect.y,
                &mark_text,
                style::chat_step_mark_running_body(),
                rect.right().saturating_sub(x),
            );
            let text_x = x.saturating_add(2);
            grid.put_clipped(
                text_x,
                rect.y,
                label,
                style::muted(),
                rect.right().saturating_sub(text_x),
            );
        }
    }
}

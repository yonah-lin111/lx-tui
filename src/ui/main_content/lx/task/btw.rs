//! `BTW` 页签：上方对话（复用历史渲染）+ 下方 QA 列表与独立输入框。
//!
//! 只读状态、只写 Buffer；尺寸/可见行由事件循环在绘制前写入状态。

use ratatui::layout::Rect;

use crate::app::state::{Lx, TaskPage};
use crate::ui::main_content::lx::history::{self, HistorySource, HistoryViewport};
use crate::ui::{style, text};

use super::super::Grid;
use super::list_rect;

/// 底边框 + 顶部 tab 行已由 `draw_list` 绘制；这里只排 QA 列表与输入框。
pub(crate) fn draw_bottom(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
    page: &TaskPage,
) -> Option<(u16, u16)> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let thread = lx.btw.active_thread();
    let qa_total = lx.btw.qa.len();
    let show_qa = qa_total > 1 && rect.height >= 5;
    let composer_h = composer_height(rect.height, show_qa);
    let composer = Rect::new(
        rect.x,
        rect.bottom().saturating_sub(composer_h),
        rect.width,
        composer_h,
    );
    let qa_rect = Rect::new(
        rect.x,
        rect.y,
        rect.width,
        composer.y.saturating_sub(rect.y),
    );
    if show_qa && qa_rect.height > 0 {
        draw_qa_list(grid, qa_rect, lx, page);
    }
    let Some(thread) = thread else {
        super::draw_placeholder(grid, composer, text::LX_BTW_EMPTY_THREAD);
        return None;
    };
    let editor = thread.composer.editor();
    let visible = usize::from(composer_h.saturating_sub(2)).max(1);
    let total = editor.visual_rows().len().max(1);
    let max_scroll = total.saturating_sub(visible);
    let scroll = editor.scroll().min(max_scroll);
    super::super::input::draw(grid, composer, editor, true, scroll, visible)
}

/// 输入框占用的行数：至少 3 行（上下边框 + 一行正文），最多 4 行。
fn composer_height(inner_h: u16, show_qa: bool) -> u16 {
    if show_qa {
        inner_h.saturating_sub(2).max(3)
    } else {
        inner_h.clamp(3, 4)
    }
}

/// 输入框矩形（`task_list` 内部、QA 列表之下）；供 `input_rect` 与命中复用。
pub(crate) fn composer_rect(rect: Rect) -> Rect {
    let inner = list_rect(rect);
    let show_qa = inner.height >= 5;
    let height = composer_height(inner.height, show_qa);
    Rect::new(
        inner.x,
        inner.bottom().saturating_sub(height),
        inner.width,
        height,
    )
}

/// QA 列表可视区起始索引。
pub(super) fn window_start(page: &TaskPage, visible: usize) -> usize {
    if visible == 0 || page.btw_len <= visible {
        return 0;
    }
    let max = page.btw_len.saturating_sub(visible);
    page.btw_window
        .viewport
        .map(|start| start.min(max))
        .unwrap_or_else(|| {
            page.btw_window
                .active
                .min(page.btw_len.saturating_sub(1))
                .saturating_sub(visible.saturating_sub(1))
                .min(max)
        })
}

fn draw_qa_list(grid: &mut Grid, rect: Rect, lx: &Lx, page: &TaskPage) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    let visible = usize::from(rect.height);
    let start = window_start(page, visible);
    for (offset, summary) in lx.btw.qa.iter().skip(start).take(visible).enumerate() {
        let index = start + offset;
        let y = rect.y.saturating_add(u16::try_from(offset).unwrap_or(0));
        let active = index == page.btw_window.active;
        let marker = if active { "❯ " } else { "  " };
        let status = text::btw_status_label(&summary.status);
        let line = format!(
            "{marker}Q{}  {}  {status}",
            index.saturating_add(1),
            summary.title
        );
        grid.put_clipped(
            rect.x,
            y,
            &line,
            if active {
                style::selected_item()
            } else {
                style::muted()
            },
            rect.width,
        );
    }
}

/// 对话区行数：与绘制同源；消息为空时至少 1 行占位。
pub(super) fn detail_rows(
    rect: Rect,
    thread: &crate::app::state::lx::BtwThreadState,
    cache: &mut crate::ui::PaneCache,
) -> (usize, bool) {
    let content_width = history::content_rect(rect).width;
    if content_width == 0 {
        return (usize::from(rect.height), true);
    }
    if thread.chat.messages.is_empty() {
        return (1, true);
    }
    let source =
        HistorySource::from_chat(&thread.chat, &thread.compact).with_viewport(HistoryViewport {
            scroll: thread.scroll,
            follow_bottom: thread.follow,
        });
    cache.set_viewport_rows(usize::from(rect.height));
    history::measure_cached(content_width, source, cache)
}

/// 上方对话区（复用 Agents 详情同一套历史渲染）。
pub(crate) fn draw_conversation(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
    cache: &mut crate::ui::PaneCache,
) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    let Some(thread) = lx.btw.active_thread() else {
        super::draw_placeholder(grid, rect, text::LX_BTW_EMPTY_THREAD);
        return 1;
    };
    if thread.chat.messages.is_empty() {
        if thread.loaded {
            super::draw_placeholder(grid, rect, text::LX_BTW_EMPTY_THREAD);
        } else {
            super::draw_placeholder(grid, rect, text::LX_BTW_THREAD_LOADING);
        }
        return 1;
    }
    let source =
        HistorySource::from_chat(&thread.chat, &thread.compact).with_viewport(HistoryViewport {
            scroll: thread.scroll,
            follow_bottom: thread.follow,
        });
    let view = history::view_source_plain(rect, source, cache);
    let visible = usize::from(view.body.height);
    let max = view.total_rows.saturating_sub(visible);
    let top = if thread.follow {
        max
    } else {
        thread.scroll.min(max)
    };
    let total = history::render_source(grid, view.body, source, top, cache);
    if let Some(bar) = history::scrollbar(view.body, view.total_rows, visible, top) {
        for y in bar.thumb.y..bar.thumb.bottom() {
            grid.set(bar.thumb.x, y, '▐', style::text());
        }
    }
    total
}

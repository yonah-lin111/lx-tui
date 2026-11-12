//! `BTW` 页签：上方对话（复用历史渲染）+ 下方一行归属提示与独立输入框。
//!
//! 只读状态、只写 Buffer；尺寸/可见行由事件循环在绘制前写入状态。
//!
//! 底部固定两段：
//! - 一行 `QA n · <所属主问题>` + 右侧 `To: <回答者>`，说明这条侧线程挂在哪个主问答上；
//! - 输入框默认一行（上下边框 + 一行正文），随软换行增长到 3 行，和主输入框同一观感。

use ratatui::layout::Rect;

use crate::app::state::{Lx, TaskPage};
use crate::ui::main_content::lx::history::{self, HistorySource, HistoryViewport};
use crate::ui::{style, text};

use super::super::Grid;
use super::list_rect;

/// 输入框最多展示的正文行数（超出后在框内滚动）。
pub(super) const COMPOSER_MAX_ROWS: u16 = 3;
/// 归属提示行高度。
const HEADER_ROWS: u16 = 1;

/// 底边框 + 顶部 tab 行已由 `draw_list` 绘制；这里只排归属提示与输入框。
pub(crate) fn draw_bottom(
    grid: &mut Grid,
    rect: Rect,
    lx: &Lx,
    _page: &TaskPage,
) -> Option<(u16, u16)> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    // 调用方传进来的已经是 `list_rect`（顶边框 + tab 行之下）；这里直接使用，
    // 再减一次会把内容区压到 2 行、composer 永远放不下。
    let inner = rect;
    let composer = composer_rect_in(inner, lx);
    // 归属行贴在输入框上方，中间不留空行（底部只有「归属 + 输入」两段）。
    let header_y = composer.map_or(inner.y, |composer| composer.y.saturating_sub(HEADER_ROWS));
    let header = Rect::new(
        inner.x,
        header_y.max(inner.y),
        inner.width,
        HEADER_ROWS.min(inner.height),
    );
    if header.height > 0 {
        draw_header(grid, header, lx);
    }
    let Some(composer) = composer else {
        if header.height == 0 {
            super::draw_placeholder(grid, inner, text::LX_BTW_QA_EMPTY);
        }
        return None;
    };
    let thread = lx.btw.active_thread()?;
    let editor = thread.composer.editor();
    let visible = usize::from(composer.height.saturating_sub(2)).max(1);
    let total = editor.visual_rows().len().max(1);
    let max_scroll = total.saturating_sub(visible);
    let scroll = editor.scroll().min(max_scroll);
    super::super::input::draw(grid, composer, editor, true, scroll, visible)
}

/// 输入框矩形（`task_list` 内部、归属提示之下）；未打开线程时 None。
pub(crate) fn composer_rect(rect: Rect, lx: &Lx) -> Option<Rect> {
    composer_rect_in(list_rect(rect), lx)
}

fn composer_rect_in(inner: Rect, lx: &Lx) -> Option<Rect> {
    let thread = lx.btw.active_thread()?;
    let avail = inner.height.saturating_sub(HEADER_ROWS);
    if avail < 3 {
        return None;
    }
    let rows = usize::from(COMPOSER_MAX_ROWS)
        .min(thread.composer.editor().visual_rows().len().max(1))
        .max(1);
    let height = u16::try_from(rows)
        .unwrap_or(COMPOSER_MAX_ROWS)
        .saturating_add(2);
    let height = height.min(avail);
    Some(Rect::new(
        inner.x,
        inner.bottom().saturating_sub(height),
        inner.width,
        height,
    ))
}

/// 归属提示：`QA 2 · 你有什么功能`，右侧当前回答者集合。
fn draw_header(grid: &mut Grid, rect: Rect, lx: &Lx) {
    let Some(thread) = lx.btw.active_thread() else {
        grid.put_clipped(
            rect.x,
            rect.y,
            text::LX_BTW_THREAD_LOADING,
            style::muted(),
            rect.width,
        );
        return;
    };
    let index = lx
        .btw
        .qa
        .iter()
        .position(|summary| summary.qa_id == thread.qa_id)
        .map(|index| index.saturating_add(1));
    let title = lx
        .btw
        .qa
        .iter()
        .find(|summary| summary.qa_id == thread.qa_id)
        .map(|summary| summary.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(text::LX_BTW_QA_UNTITLED);
    let label = match index {
        Some(index) => format!("QA {index} · {title}"),
        None => format!("QA · {title}"),
    };
    let targets = thread.target_label();
    // 右侧目标优先保证完整；左侧归属按剩余宽度截断。
    let targets_w =
        u16::try_from(unicode_width::UnicodeWidthStr::width(targets.as_str())).unwrap_or(u16::MAX);
    let gap = u16::from(rect.width > targets_w.saturating_add(4));
    let left_w = rect.width.saturating_sub(targets_w.saturating_add(gap));
    grid.put_clipped(rect.x, rect.y, &label, style::muted(), left_w);
    if gap > 0 {
        let x = rect.x.saturating_add(left_w);
        grid.put_clipped(x, rect.y, &targets, style::muted(), targets_w);
    }
}

/// 对话区行数：与绘制同源；消息为空时至少 1 行占位。
pub(crate) fn detail_rows(
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

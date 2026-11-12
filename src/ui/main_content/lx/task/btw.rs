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
/// 归属胶囊最大宽度（含左右空格与右侧留白）；超出按标题截断。
const QA_BADGE_MAX_COLS: u16 = 48;

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
    let Some(composer) = composer_rect_in(inner, lx) else {
        super::draw_placeholder(grid, inner, text::LX_BTW_QA_EMPTY);
        return None;
    };
    let thread = lx.btw.active_thread()?;
    let editor = thread.composer.editor();
    let visible = usize::from(composer.height.saturating_sub(2)).max(1);
    let total = editor.visual_rows().len().max(1);
    let max_scroll = total.saturating_sub(visible);
    let scroll = editor.scroll().min(max_scroll);
    let dims = super::super::input::draw(grid, composer, editor, true, scroll, visible);
    // 主问题归属放到输入框顶边框右上角（与主会话标题胶囊同一位置）。
    draw_qa_badge(grid, composer, lx);
    dims
}

/// 输入框矩形（`task_list` 内部、归属提示之下）；未打开线程时 None。
pub(crate) fn composer_rect(rect: Rect, lx: &Lx) -> Option<Rect> {
    composer_rect_in(list_rect(rect), lx)
}

fn composer_rect_in(inner: Rect, lx: &Lx) -> Option<Rect> {
    let thread = lx.btw.active_thread()?;
    let avail = inner.height;
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

/// 主问题归属胶囊：` QA 2 · 你有什么功能 · To: lx `，画在输入框顶边框右侧。
///
/// 与主会话标题胶囊同一位置；背景/前景用固定的暗底亮字（不随壁纸或会话配色变化），
/// 保证在透明终端背景上也可读。
fn draw_qa_badge(grid: &mut Grid, rect: Rect, lx: &Lx) {
    let Some(thread) = lx.btw.active_thread() else {
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
    let prefix = match index {
        Some(index) => format!("QA {index} · "),
        None => "QA · ".to_string(),
    };
    let targets = format!(" · {}", thread.target_label());
    let room = rect
        .width
        .saturating_sub(6)
        .min(QA_BADGE_MAX_COLS)
        .saturating_sub(
            u16::try_from(unicode_width::UnicodeWidthStr::width(prefix.as_str())).unwrap_or(0),
        )
        .saturating_sub(
            u16::try_from(unicode_width::UnicodeWidthStr::width(targets.as_str())).unwrap_or(0),
        );
    if room < 4 {
        return;
    }
    let title = text::truncate_to_width(title, usize::from(room));
    if title.is_empty() {
        return;
    }
    let badge = format!(" {prefix}{title}{targets} ");
    let badge_w =
        u16::try_from(unicode_width::UnicodeWidthStr::width(badge.as_str())).unwrap_or(u16::MAX);
    if rect.width <= badge_w.saturating_add(2) {
        return;
    }
    let x = rect.right().saturating_sub(badge_w).saturating_sub(1);
    grid.put(x, rect.y, &badge, style::btw_qa_badge());
}

/// `/btwList`：主 QA 选择器（标题行 + 列表行）。
///
/// 列表项右侧 `● side thread` 表示该主问题已经有旁路对话；当前高亮行用选中样式，
/// 仅影响文字背景，不改变列表整体宽度。
fn draw_picker(grid: &mut Grid, rect: Rect, lx: &Lx) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    grid.put_clipped(
        rect.x,
        rect.y,
        text::LX_BTW_LIST_TITLE,
        style::muted(),
        rect.width,
    );
    let list_y = rect.y.saturating_add(1);
    let visible = usize::from(rect.height.saturating_sub(1));
    if visible == 0 {
        return 1;
    }
    if lx.btw.qa.is_empty() {
        super::draw_placeholder(
            grid,
            Rect::new(rect.x, list_y, rect.width, rect.height.saturating_sub(1)),
            text::LX_BTW_LIST_EMPTY,
        );
        return 1;
    }
    let selected = lx.btw.picking.unwrap_or(0);
    let start = lx.btw.picker_window(visible);
    let mark_w = u16::try_from(unicode_width::UnicodeWidthStr::width(
        text::LX_BTW_LIST_THREAD_MARK,
    ))
    .unwrap_or(14);
    for (offset, summary) in lx.btw.qa.iter().skip(start).take(visible).enumerate() {
        let index = start + offset;
        let y = list_y.saturating_add(u16::try_from(offset).unwrap_or(0));
        let is_selected = index == selected;
        let row_style = if is_selected {
            style::selected_item()
        } else {
            style::muted()
        };
        // 未选中行不画选择标记，但保持同宽占位。
        let prefix = if is_selected {
            format!("{}Q{} ", text::AGENT_SELECTED_MARKER, index + 1)
        } else {
            format!("  Q{} ", index + 1)
        };
        let prefix_w = u16::try_from(unicode_width::UnicodeWidthStr::width(prefix.as_str()))
            .unwrap_or(u16::MAX);
        let has_thread = lx.btw.has_side_thread(summary);
        let reserve = if has_thread {
            mark_w.saturating_add(2)
        } else {
            0
        };
        if prefix_w.saturating_add(reserve) >= rect.width {
            continue;
        }
        let title_room = usize::from(rect.width.saturating_sub(prefix_w).saturating_sub(reserve));
        grid.put_clipped(rect.x, y, &prefix, row_style, prefix_w);
        let title = if summary.title.trim().is_empty() {
            text::LX_BTW_QA_UNTITLED
        } else {
            summary.title.as_str()
        };
        let title = text::truncate_to_width(title, title_room);
        grid.put_clipped(
            rect.x.saturating_add(prefix_w),
            y,
            &title,
            row_style,
            u16::try_from(title_room).unwrap_or(0),
        );
        if has_thread {
            let x = rect.right().saturating_sub(mark_w);
            grid.put_clipped(x, y, text::LX_BTW_LIST_THREAD_MARK, style::accent(), mark_w);
        }
    }
    visible.min(lx.btw.qa.len())
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
    if lx.btw.picking.is_some() {
        return draw_picker(grid, rect, lx);
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

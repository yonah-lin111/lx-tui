//! `BTW` 页签：上方对话（与 Agents 详情同源）+ 下方 QA 列表 / 独立输入框。
//!
//! 列表态固定 5 行正文（搜索行由 `task.rs` 绘制）；`Shift+Enter` 进入编辑态后
//! 只保留 composer，退出时退回列表并保留草稿。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::Lx;
use crate::ui::main_content::lx::history::{self, HistorySource, HistoryViewport};
use crate::ui::{style, text};

use super::super::Grid;
use super::detail::{self, DetailView, EmptyState};

/// 输入框最多展示的正文行数（超出后在框内滚动）。
pub(super) const COMPOSER_MAX_ROWS: u16 = 3;
/// 归属胶囊最大宽度（含左右空格与右侧留白）；超出按标题截断。
const QA_BADGE_MAX_COLS: u16 = 48;
/// 列表行首标记占位。
const MARKER_COLS: u16 = 2;
/// 标题与右侧状态之间的最小空档。
const ROW_GAP: u16 = 2;

/// 列表态：绘制过滤后的 QA 行。搜索行由调用方绘制。
pub(crate) fn draw_qa_list(grid: &mut Grid, rect: Rect, lx: &Lx) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    if lx.btw.qa.is_empty() {
        let label = if lx.btw.qa_loaded {
            text::LX_BTW_LIST_EMPTY
        } else {
            text::LX_BTW_LIST_LOADING
        };
        super::draw_placeholder(grid, rect, label);
        return;
    }
    let filtered = lx.btw.filtered_qa_indices();
    if filtered.is_empty() {
        super::draw_placeholder(grid, rect, text::LX_BTW_SEARCH_EMPTY);
        return;
    }
    let visible = usize::from(rect.height);
    let start = super::window_start_at(&lx.btw.qa_window, filtered.len(), visible);
    for (offset, position) in (start..filtered.len()).take(visible).enumerate() {
        let Some(index) = filtered.get(position).copied() else {
            continue;
        };
        let Some(summary) = lx.btw.qa.get(index) else {
            continue;
        };
        let y = rect.y.saturating_add(u16::try_from(offset).unwrap_or(0));
        let selected = lx.btw.selected_index() == Some(position);
        if selected {
            put_grid(
                grid,
                rect.x,
                y,
                text::AGENT_SELECTED_MARKER,
                style::accent(),
                rect.right(),
            );
        }
        let row_x = rect.x.saturating_add(MARKER_COLS);
        let row_w = rect.width.saturating_sub(MARKER_COLS);
        if row_w == 0 {
            continue;
        }
        let title = if summary.title.trim().is_empty() {
            text::LX_BTW_QA_UNTITLED.to_string()
        } else {
            text::chat::single_line_label(&summary.title).into_owned()
        };
        let label = format!("QA {} · {title}", index + 1);
        let (mut right, right_style) = row_status(lx, summary);
        let right_w = u16::try_from(right.width()).unwrap_or(u16::MAX);
        if right_w.saturating_add(ROW_GAP) >= row_w {
            right = "";
        }
        let right_w = u16::try_from(right.width()).unwrap_or(0);
        let gap = if right.is_empty() { 0 } else { ROW_GAP };
        let title_w = row_w.saturating_sub(right_w.saturating_add(gap));
        let label = text::truncate_to_width(&label, usize::from(title_w));
        let style = if selected {
            style::text()
        } else {
            style::muted()
        };
        put_grid(grid, row_x, y, &label, style, row_x.saturating_add(title_w));
        if !right.is_empty() {
            put_grid(
                grid,
                rect.right().saturating_sub(right_w),
                y,
                right,
                right_style,
                rect.right(),
            );
        }
    }
}

/// 行右侧状态：活动/失败状态优先，其次是“已有旁路线程”。
fn row_status(
    lx: &Lx,
    summary: &crate::app::state::lx::btw::BtwQaSummary,
) -> (&'static str, ratatui::style::Style) {
    let status = summary.status.as_str();
    if matches!(status, "queued" | "running") {
        return (text::btw::btw_status_label(status), style::accent());
    }
    if status == "failed" {
        return (text::btw::btw_status_label(status), style::agent_fail());
    }
    if status == "interrupted" {
        return (text::btw::btw_status_label(status), style::agent_warn());
    }
    if lx.btw.has_side_thread(summary) {
        return (text::LX_BTW_LIST_THREAD_MARK, style::muted());
    }
    ("", style::muted())
}

/// 编辑态绘制：composer 占满传入矩形（高度由 `task_panel_height` 决定）。
pub(crate) fn draw_edit(grid: &mut Grid, rect: Rect, lx: &Lx, focused: bool) -> Option<(u16, u16)> {
    if rect.width == 0 || rect.height < 3 {
        return None;
    }
    let thread = lx.btw.active_thread()?;
    let editor = thread.composer.editor();
    let visible = usize::from(rect.height.saturating_sub(2)).max(1);
    let total = editor.visual_rows().len().max(1);
    let max_scroll = total.saturating_sub(visible);
    let scroll = editor.scroll().min(max_scroll);
    let dims = super::super::input::draw(grid, rect, editor, focused, scroll, visible);
    grid.put(
        rect.x.saturating_add(1),
        rect.y,
        &format!(" {} ", text::LX_BTW_INPUT_MARK),
        style::accent(),
    );
    draw_qa_badge(grid, rect, lx);
    dims
}

/// composer 矩形：仅编辑态、且当前 QA 已有线程时返回。
pub(crate) fn composer_rect(rect: Rect, lx: &Lx) -> Option<Rect> {
    if !lx.btw.editing || rect.height < 3 || lx.btw.active_thread().is_none() {
        return None;
    }
    Some(rect)
}

/// 主问题归属胶囊：` QA 2 · 标题 · To: lx `，画在 composer 顶边框右侧。
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
    let width = |value: &str| {
        u16::try_from(unicode_width::UnicodeWidthStr::width(value)).unwrap_or(u16::MAX)
    };
    let max = rect.width.saturating_sub(12).min(QA_BADGE_MAX_COLS);
    let prefix_w = width(prefix.as_str());
    let targets = format!(" · {}", thread.target_label());
    let (room, targets) = if max
        .saturating_sub(prefix_w)
        .saturating_sub(width(targets.as_str()))
        >= 4
    {
        (
            max.saturating_sub(prefix_w)
                .saturating_sub(width(targets.as_str())),
            targets.as_str(),
        )
    } else {
        (max.saturating_sub(prefix_w), "")
    };
    if room < 4 {
        return;
    }
    let title = text::truncate_to_width(title, usize::from(room));
    if title.is_empty() {
        return;
    }
    let badge = format!(" {prefix}{title}{targets} ");
    let badge_w = width(badge.as_str());
    if rect.width <= badge_w.saturating_add(8) {
        return;
    }
    let x = rect.right().saturating_sub(badge_w).saturating_sub(1);
    grid.put(x, rect.y, &badge, style::btw_qa_badge());
}

/// 对话区行数：与绘制同源；没有历史时按占位/loading 计 1 行。
pub(crate) fn detail_rows(
    rect: Rect,
    thread: &crate::app::state::lx::BtwThreadState,
    cache: &mut crate::ui::PaneCache,
) -> (usize, bool) {
    let view = detail_view(thread);
    detail::measure(rect, view, cache)
}

/// 上方对话区：与 Agents 详情共用同一套历史渲染、空态与滚动条。
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
    let empty = if thread.loaded {
        EmptyState::Placeholder(text::LX_BTW_EMPTY_THREAD)
    } else {
        EmptyState::Loading {
            label: text::LX_BTW_THREAD_LOADING,
            frame: thread.chat.loading_frame,
        }
    };
    detail::draw(grid, rect, detail_view(thread), empty, cache)
}

fn detail_view(thread: &crate::app::state::lx::BtwThreadState) -> DetailView<'_> {
    DetailView {
        source: HistorySource::from_chat(&thread.chat, &thread.compact).with_viewport(
            HistoryViewport {
                scroll: thread.scroll,
                follow_bottom: thread.follow,
            },
        ),
        scroll: thread.scroll,
        follow: thread.follow,
    }
}

/// 变化前的阅读锚点。
pub(crate) fn anchor_before(
    area: Rect,
    lx: &Lx,
    cache: &mut crate::ui::PaneCache,
    scroll: usize,
) -> Option<history::HistoryAnchor> {
    let rect = super::super::layout(area, lx).task_output?;
    let thread = lx.btw.active_thread()?;
    detail::anchor_before(rect, detail_view(thread), scroll, cache)
}

/// 变化后的锚点行。
pub(crate) fn anchor_row(
    area: Rect,
    lx: &Lx,
    cache: &mut crate::ui::PaneCache,
    anchor: &history::HistoryAnchor,
) -> Option<usize> {
    let rect = super::super::layout(area, lx).task_output?;
    let thread = lx.btw.active_thread()?;
    detail::anchor_row(rect, detail_view(thread), anchor, cache)
}

/// 逐字符写入网格并裁剪到 `limit` 列；返回写入结束后的下一列。
fn put_grid(
    grid: &mut Grid,
    x: u16,
    y: u16,
    content: &str,
    style: ratatui::style::Style,
    limit: u16,
) -> u16 {
    let width = limit.saturating_sub(x);
    grid.put_clipped(x, y, content, style, width);
    x.saturating_add(u16::try_from(content.width()).unwrap_or(0))
        .min(limit)
}

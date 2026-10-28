//! lx 页主内容区（主消息列表与后台管理主内容区）的选区几何、反显绘制与跨屏取词。
//!
//! 选区以**内容行坐标**（`0..total_rows`）与**正文列坐标**（`0..rect.width`，已扣除右侧
//! 1 列滚动条槽）存储：
//! - 滚轮滚动视口时，锚点钉在原内容行上；拖拽中终点随滚轮延伸，松开后高亮随文字一起上下滚动；
//! - 复制时按内容行区间分段排版取词，滚出当前屏幕的内容行同样完整提取，且绝不复制滚动条符号。

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::app::selection::Selection;
use crate::app::state::{Lx, TaskTab};

use super::grid::{CONTINUATION, Grid};
use super::history::{self, PaneCache};
use super::task;

/// 分段取词的单批最大行数：控制跨屏长选区时的临时网格内存占用。
const COPY_CHUNK_ROWS: usize = 256;

/// lx 主内容区（聊天历史或后台任务主内容区）的选区几何：已扣除置顶行与右侧滚动条槽。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ContentGeom {
    /// 正文可视矩形（绝对屏幕坐标，不含置顶行与最右 1 列滚动条槽）。
    pub(crate) rect: Rect,
    /// 当前视口顶部对应的内容行索引（0 基）。
    pub(crate) scroll: usize,
    /// 内容总行数。
    pub(crate) total_rows: usize,
}

impl ContentGeom {
    /// 屏幕坐标是否落在该主内容区的正文范围内（不含滚动条列）。
    pub(crate) fn contains(&self, column: u16, row: u16) -> bool {
        self.rect.width > 0 && self.rect.height > 0 && self.rect.contains((column, row).into())
    }

    /// 屏幕坐标 →（内容行索引，正文列偏移）；越界时钳到当前可视正文区边界。
    pub(crate) fn cell(&self, column: u16, row: u16) -> (i32, u16) {
        let bottom = self.rect.bottom().saturating_sub(1);
        let local_row = row.clamp(self.rect.y, bottom).saturating_sub(self.rect.y);
        let content_row =
            i32::try_from(self.scroll.saturating_add(usize::from(local_row))).unwrap_or(i32::MAX);
        let right = self.rect.right().saturating_sub(1);
        let local_col = column.clamp(self.rect.x, right).saturating_sub(self.rect.x);
        (content_row, local_col)
    }
}

/// 计算当前活动主内容区（主消息列表或后台管理主内容区）的选区几何；欢迎页返回 None。
pub(crate) fn content_geom(area: Rect, lx: &Lx, cache: &mut PaneCache) -> Option<ContentGeom> {
    if let Some(page) = lx.task.as_ref() {
        let output_rect = super::layout(area, lx).task_output?;
        if output_rect.width == 0 || output_rect.height == 0 {
            return None;
        }
        if page.tab == TaskTab::Agents
            && let Some(run) = lx.agents.runs.get(page.agents_window.active)
        {
            if run.chat.messages.is_empty() {
                let rect = history::content_rect(output_rect);
                return (rect.width > 0 && rect.height > 0).then_some(ContentGeom {
                    rect,
                    scroll: 0,
                    total_rows: 1,
                });
            }
            let source = history::HistorySource::from_chat(&run.chat, &run.compact);
            let view = history::view_source_plain(output_rect, source, cache);
            let rect = history::content_rect(view.body);
            if rect.width == 0 || rect.height == 0 || view.total_rows == 0 {
                return None;
            }
            let visible = usize::from(view.body.height);
            let max = view.total_rows.saturating_sub(visible);
            let scroll = if run.follow { max } else { run.scroll.min(max) };
            return Some(ContentGeom {
                rect,
                scroll,
                total_rows: view.total_rows,
            });
        }
        let width = task::output_content_width(output_rect);
        let rect = Rect::new(output_rect.x, output_rect.y, width, output_rect.height);
        if rect.width == 0 || rect.height == 0 {
            return None;
        }
        let total_rows = page.output.len().max(1);
        let scroll = if page.output.is_empty() {
            0
        } else {
            page.output_scroll.min(page.output.len())
        };
        return Some(ContentGeom {
            rect,
            scroll,
            total_rows,
        });
    }
    let view = super::history_view(area, lx, cache)?;
    let rect = history::content_rect(view.body);
    if rect.width == 0 || rect.height == 0 || view.total_rows == 0 {
        return None;
    }
    Some(ContentGeom {
        rect,
        scroll: view.scroll,
        total_rows: view.total_rows,
    })
}

/// 在已绘制的缓冲区上按内容行坐标反显选区（仅作用于正文列，绝不反显右侧滚动条列）。
pub(super) fn paint_selection(buf: &mut Buffer, geom: ContentGeom, selection: &Selection) {
    for dy in 0..geom.rect.height {
        let Ok(content_row) = i32::try_from(geom.scroll.saturating_add(usize::from(dy))) else {
            break;
        };
        let y = geom.rect.y.saturating_add(dy);
        for dx in 0..geom.rect.width {
            if !selection.contains(content_row, dx) {
                continue;
            }
            let x = geom.rect.x.saturating_add(dx);
            if let Some(cell) = buf.cell_mut((x, y))
                && cell.diff_option != CellDiffOption::Skip
            {
                let reversed = cell.style().add_modifier(Modifier::REVERSED);
                cell.set_style(reversed);
            }
        }
    }
}

/// 提取主内容区选区文本（内容行坐标、正文列坐标）；空选区返回空串。
///
/// 按内容行区间分段渲染到仅含正文列的网格（不含滚动条槽），因此：
/// - 滚轮滚出当前视口的选中行仍能完整复制；
/// - 右侧滚动条字符 `▐` 永远不会混入复制文本。
pub fn copy_text(
    area: Rect,
    lx: &Lx,
    range: ((i32, u16), (i32, u16)),
    cache: &mut PaneCache,
) -> String {
    let (start, end) = if range.0 <= range.1 {
        range
    } else {
        (range.1, range.0)
    };
    if start == end {
        return String::new();
    }
    let Some(geom) = content_geom(area, lx, cache) else {
        return String::new();
    };
    if geom.total_rows == 0 || geom.rect.width == 0 {
        return String::new();
    }
    let total_i32 = i32::try_from(geom.total_rows).unwrap_or(i32::MAX);
    if end.0 < 0 || start.0 >= total_i32 {
        return String::new();
    }
    let first_row = usize::try_from(start.0.max(0))
        .unwrap_or(0)
        .min(geom.total_rows - 1);
    let last_row = usize::try_from(end.0.max(0))
        .unwrap_or(0)
        .min(geom.total_rows - 1);
    let mut lines: Vec<String> = Vec::new();
    let mut chunk_start = first_row;
    while chunk_start <= last_row {
        let chunk_end = chunk_start
            .saturating_add(COPY_CHUNK_ROWS - 1)
            .min(last_row);
        let chunk_rows = u16::try_from(chunk_end - chunk_start + 1).unwrap_or(u16::MAX);
        let chunk_rect = Rect::new(0, 0, geom.rect.width, chunk_rows);
        let mut grid = Grid::new(chunk_rect);
        render_chunk(&mut grid, chunk_rect, lx, chunk_start, cache);
        for row in chunk_start..=chunk_end {
            let Some((col0, col1)) = row_col_bounds(row, start, end, geom.rect.width) else {
                continue;
            };
            let y = u16::try_from(row - chunk_start).unwrap_or(0);
            let mut line = String::new();
            for col in col0..=col1 {
                let ch = grid.cell(col, y).ch;
                if ch != CONTINUATION {
                    line.push(ch);
                }
            }
            lines.push(line.trim_end().to_string());
        }
        chunk_start = chunk_end.saturating_add(1);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// 将指定内容行区间 `[chunk_start, chunk_start + chunk_rect.height)` 渲染到无滚动条的网格。
fn render_chunk(
    grid: &mut Grid,
    chunk_rect: Rect,
    lx: &Lx,
    chunk_start: usize,
    cache: &mut PaneCache,
) {
    if let Some(page) = lx.task.as_ref() {
        if page.tab == TaskTab::Agents
            && let Some(run) = lx.agents.runs.get(page.agents_window.active)
        {
            if run.chat.messages.is_empty() {
                if chunk_start == 0 {
                    task::agents::draw_empty_detail(grid, chunk_rect, run, lx.chat.loading_frame);
                }
            } else {
                let source = history::HistorySource::from_chat(&run.chat, &run.compact);
                let body_rect = Rect::new(
                    0,
                    0,
                    chunk_rect.width.saturating_add(history::SCROLLBAR_COLUMNS),
                    chunk_rect.height,
                );
                history::render_source(grid, body_rect, source, chunk_start, cache);
            }
        } else {
            task::draw_output_rows(grid, chunk_rect, page, chunk_start);
        }
        return;
    }
    let source = history::HistorySource::from(lx);
    let body_rect = Rect::new(
        0,
        0,
        chunk_rect.width.saturating_add(history::SCROLLBAR_COLUMNS),
        chunk_rect.height,
    );
    history::render_source(grid, body_rect, source, chunk_start, cache);
}

/// 单行在选区 `(start, end)` 内的包含列范围 `(col0, col1)`（钳到 `0..width`）。
fn row_col_bounds(
    row: usize,
    start: (i32, u16),
    end: (i32, u16),
    width: u16,
) -> Option<(u16, u16)> {
    let row_i32 = i32::try_from(row).ok()?;
    if row_i32 < start.0 || row_i32 > end.0 || width == 0 {
        return None;
    }
    let max_col = width - 1;
    let col0 = if row_i32 == start.0 { start.1 } else { 0 };
    let col1 = if row_i32 == end.0 {
        end.1.min(max_col)
    } else {
        max_col
    };
    (col0 <= col1 && col0 <= max_col).then_some((col0, col1))
}

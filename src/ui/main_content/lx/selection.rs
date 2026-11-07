//! lx 页主内容区（主消息列表与后台管理主内容区）的选区几何、反显绘制与跨屏取词。
//!
//! 选区以**内容行坐标**（`0..total_rows`）与**正文列坐标**（`0..rect.width`，已扣除右侧
//! 1 列滚动条槽）存储：
//! - 滚轮滚动视口时，锚点钉在原内容行上；拖拽中终点随滚轮延伸，松开后高亮随文字一起上下滚动；
//! - 复制时按内容行区间分段排版取词，滚出当前屏幕的内容行同样完整提取，且绝不复制滚动条符号。

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::app::selection::{Selection, SelectionUnit, char_class};
use crate::app::state::{Lx, TaskTab};

use super::grid::{CONTINUATION, Grid};
use super::history::{self, PaneCache};
use super::task;

mod table;

#[cfg(test)]
mod tests;

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
        if page.tab == TaskTab::Agents {
            let run = page
                .selected_agent_index(&lx.agents)
                .and_then(|index| lx.agents.runs.get(index))?;
            if run.chat.messages.is_empty() {
                let rect = history::content_rect(output_rect);
                return (rect.width > 0 && rect.height > 0).then_some(ContentGeom {
                    rect,
                    scroll: 0,
                    total_rows: 1,
                });
            }
            // 选区几何优先走 `resolve_task_agents_visible` 每帧维护的 run 字段：
            // 与绘制同一口径，且不在滚轮/拖拽回调里重新量行。
            // 尚未量过（冷启动/测试直接构造）时回退到完整视图，保证几何正确。
            if !run.rows_dirty && run.total_rows > 0 {
                let rect = history::content_rect(output_rect);
                if rect.width == 0 || rect.height == 0 {
                    return None;
                }
                let total = run.total_rows;
                let visible = usize::from(rect.height);
                let max = total.saturating_sub(visible);
                let scroll = if run.follow { max } else { run.scroll.min(max) };
                return Some(ContentGeom {
                    rect,
                    scroll,
                    total_rows: total,
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
    // 选区几何在滚轮/拖拽事件里每帧都要算：只读已就绪快照，不在此同步重排。
    let view = super::history_hit::history_view_ready(area, lx, cache)?;
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
///
/// 对 Markdown 表格行自动按三级单元格选区（`SingleCell` / `ColumnLocked` / `Grid`）
/// 仅反显命中的单元格真实文本区间，跳过所有表格框线与补齐留白。
pub(super) fn paint_selection(
    buf: &mut Buffer,
    grid: &Grid,
    geom: ContentGeom,
    selection: &Selection,
) {
    if selection.range().is_none() {
        return;
    }
    let anchor = selection.anchor();
    let cursor = selection.cursor();
    for dy in 0..geom.rect.height {
        let Ok(content_row) = i32::try_from(geom.scroll.saturating_add(usize::from(dy))) else {
            break;
        };
        let y = geom.rect.y.saturating_add(dy);
        let grid_dy = y.saturating_sub(grid.area().y);
        if let Some(table_row) = grid.table_row_at(grid_dy) {
            let x_offset = geom.rect.x.saturating_sub(grid.area().x);
            let local_table_row = super::grid::GridTableRow {
                base_col: table_row.base_col.saturating_sub(x_offset),
                line_ref: table_row.line_ref.clone(),
            };
            let Some(plan) =
                table::resolve_table_selection(&local_table_row, content_row, anchor, cursor)
            else {
                continue;
            };
            let line_offset = table_row.line_ref.line_offset;
            for dx in 0..geom.rect.width {
                if !plan.contains_col(line_offset, dx) {
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
            continue;
        }
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

/// 按选区粒度（单字 / 词 / 行）在主内容区计算 `cell` 所在词或行的列闭区间。
pub(crate) fn unit_cell_range(
    area: Rect,
    lx: &Lx,
    cell: (i32, u16),
    unit: SelectionUnit,
    cache: &mut PaneCache,
) -> ((i32, u16), (i32, u16)) {
    if unit == SelectionUnit::Character {
        return (cell, cell);
    }
    let Some(geom) = content_geom(area, lx, cache) else {
        return (cell, cell);
    };
    if geom.rect.width == 0 || geom.total_rows == 0 {
        return (cell, cell);
    }
    let max_col = geom.rect.width - 1;
    let row = usize::try_from(cell.0.max(0))
        .unwrap_or(0)
        .min(geom.total_rows - 1);
    let chunk_rect = Rect::new(0, 0, geom.rect.width, 1);
    let mut grid = Grid::new(chunk_rect);
    render_chunk(&mut grid, chunk_rect, lx, row, cache);
    if let Some(table_row) = grid.table_row_at(0)
        && let Some(range) = table::table_unit_cell_range(&grid, table_row, cell, unit)
    {
        return range;
    }
    match unit {
        SelectionUnit::Character => (cell, cell),
        SelectionUnit::Line => ((cell.0, 0), (cell.0, max_col)),
        SelectionUnit::Word => {
            let mut tokens: Vec<(u16, u16, crate::app::selection::CharClass)> =
                Vec::with_capacity(usize::from(geom.rect.width));
            for x in 0..geom.rect.width {
                let ch = grid.cell(x, 0).ch;
                if ch == CONTINUATION {
                    if let Some(last) = tokens.last_mut() {
                        last.1 = x;
                    }
                } else {
                    tokens.push((x, x, char_class(ch)));
                }
            }
            if tokens.is_empty() {
                return (cell, cell);
            }
            let target_col = cell.1.min(max_col);
            let idx = tokens
                .iter()
                .position(|(start, end, _)| *start <= target_col && target_col <= *end)
                .unwrap_or(tokens.len() - 1);
            let target_class = tokens[idx].2;
            let mut left = idx;
            while left > 0 && tokens[left - 1].2 == target_class {
                left -= 1;
            }
            let mut right = idx;
            while right + 1 < tokens.len() && tokens[right + 1].2 == target_class {
                right += 1;
            }
            ((cell.0, tokens[left].0), (cell.0, tokens[right].1))
        }
    }
}

/// 提取主内容区选区文本（内容行坐标、正文列坐标）；空选区返回空串。
///
/// 按内容行区间分段渲染到仅含正文列的网格（不含滚动条槽），因此：
/// - 滚轮滚出当前视口的选中行仍能完整复制；
/// - 右侧滚动条字符 `▐` 永远不会混入复制文本；
/// - Markdown 表格行自动按三级单元格选区去边框提取（软折行无缝拼接、`<br>` 保换行、同行多列 `\t`）。
pub fn copy_text(
    area: Rect,
    lx: &Lx,
    range: ((i32, u16), (i32, u16)),
    cache: &mut PaneCache,
) -> String {
    let (anchor, cursor) = range;
    let (start, end) = if anchor <= cursor {
        (anchor, cursor)
    } else {
        (cursor, anchor)
    };
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
        let mut row = chunk_start;
        while row <= chunk_end {
            let y = u16::try_from(row - chunk_start).unwrap_or(0);
            if let Some(table_row) = grid.table_row_at(y).cloned() {
                let row_i32 = i32::try_from(row).unwrap_or(i32::MAX);
                if let Some(plan) =
                    table::resolve_table_selection(&table_row, row_i32, anchor, cursor)
                {
                    let t_first = usize::try_from(plan.table_start_row.max(0))
                        .unwrap_or(0)
                        .min(geom.total_rows - 1);
                    let t_last = usize::try_from(plan.table_end_row.max(0))
                        .unwrap_or(0)
                        .min(geom.total_rows - 1)
                        .max(t_first);
                    let t_rows = u16::try_from(t_last - t_first + 1).unwrap_or(u16::MAX);
                    let t_rect = Rect::new(0, 0, geom.rect.width, t_rows);
                    let mut t_grid = Grid::new(t_rect);
                    render_chunk(&mut t_grid, t_rect, lx, t_first, cache);

                    let mut present_offsets = Vec::with_capacity(usize::from(t_rows));
                    let mut last_actual_row = row;
                    for ty in 0..t_rows {
                        let abs_row = t_first.saturating_add(usize::from(ty));
                        let abs_row_i32 = i32::try_from(abs_row).unwrap_or(i32::MAX);
                        if let Some(tr) = t_grid.table_row_at(ty) {
                            let off_i32 = i32::try_from(tr.line_ref.line_offset).unwrap_or(0);
                            if abs_row_i32.saturating_sub(off_i32) == plan.table_start_row
                                && tr.base_col == plan.base_col
                                && tr.line_ref.table == plan.table
                            {
                                present_offsets.push(tr.line_ref.line_offset);
                                last_actual_row = abs_row;
                            }
                        }
                    }
                    lines.extend(table::extract_table_lines(
                        &t_grid,
                        t_first,
                        &plan,
                        &present_offsets,
                    ));
                    let next_row = last_actual_row.saturating_add(1);
                    if next_row > chunk_end {
                        chunk_start = next_row;
                        break;
                    }
                    row = next_row;
                    continue;
                }
            }
            if let Some((col0, col1)) = row_col_bounds(row, start, end, geom.rect.width) {
                let mut line = String::new();
                for col in col0..=col1 {
                    let ch = grid.cell(col, y).ch;
                    if ch != CONTINUATION {
                        line.push(ch);
                    }
                }
                lines.push(line.trim_end().to_string());
            }
            row = row.saturating_add(1);
        }
        if row > chunk_end {
            chunk_start = chunk_end.saturating_add(1);
        }
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
        if page.tab == TaskTab::Agents {
            let Some(run) = page
                .selected_agent_index(&lx.agents)
                .and_then(|index| lx.agents.runs.get(index))
            else {
                return;
            };
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

//! `lx` 消息列表 Markdown 表格的单元格感知选区解析、边框防手抖吸附与去边框 TSV 复制。
//!
//! 对齐 `opencode-dev`（`@opentui/core` `TextTableRenderable`）的三级选区锁定模型：
//! - [`TableSelectionMode::SingleCell`]：起点与终点位于同一单元格内（含软折行或 `<br>` 多行），
//!   反显高亮与复制严格限制在该单元格内部，绝不跨入相邻列或 `│` 边框；
//! - [`TableSelectionMode::ColumnLocked`]：起点与终点位于同一列的不同逻辑行，仅锁定选中该列；
//! - [`TableSelectionMode::Grid`]：跨列或跨表格外部拖选，选中范围内的多列单元格并以 `\t` / `\n` 拼接。

use std::sync::Arc;

use crate::app::selection::{CharClass, SelectionUnit, char_class};
use crate::ui::markdown_render::{TableLayoutMeta, TableLineKind};

use super::super::grid::{CONTINUATION, Grid, GridTableRow};

/// 表格内选区的三级锁定模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TableSelectionMode {
    /// 起点与终点位于同一单元格内（含单元格内多行软折行或 `<br>`）。
    SingleCell { row_idx: usize, col_idx: usize },
    /// 起点与终点位于同一列的不同逻辑行。
    ColumnLocked {
        col_idx: usize,
        first_row_idx: usize,
        last_row_idx: usize,
    },
    /// 跨列或从表格外部跨入表格的网格选区。
    Grid {
        first_row_idx: usize,
        last_row_idx: usize,
        first_col_idx: usize,
        last_col_idx: usize,
    },
}

/// 针对单张表格解析出的选区计划。
#[derive(Debug, Clone)]
pub(super) struct TableSelectionPlan {
    pub(super) table: Arc<TableLayoutMeta>,
    pub(super) table_start_row: i32,
    pub(super) table_end_row: i32,
    pub(super) base_col: u16,
    pub(super) mode: TableSelectionMode,
    /// `SingleCell` 模式下规范化后的单元格内部起止 `(line_offset, content_col)`。
    pub(super) linear_bounds: Option<((usize, u16), (usize, u16))>,
}

impl TableSelectionPlan {
    /// 判定表格内 `line_offset` 行上某个单元格 `col_idx` 是否处于当前选区覆盖的行/列范围内（不含空白填充行）。
    pub(super) fn is_cell_line_selected(&self, line_offset: usize, col_idx: usize) -> bool {
        let Some(TableLineKind::Cells { row_idx, cells, .. }) = self.table.lines.get(line_offset)
        else {
            return false;
        };
        let Some(slice) = cells.get(col_idx) else {
            return false;
        };
        if slice.is_filler {
            return false;
        }
        match self.mode {
            TableSelectionMode::SingleCell {
                row_idx: target_row,
                col_idx: target_col,
            } => {
                if *row_idx != target_row || col_idx != target_col {
                    return false;
                }
                let Some(((start_line, _), (end_line, _))) = self.linear_bounds else {
                    return false;
                };
                line_offset >= start_line && line_offset <= end_line
            }
            TableSelectionMode::ColumnLocked {
                col_idx: target_col,
                first_row_idx,
                last_row_idx,
            } => col_idx == target_col && *row_idx >= first_row_idx && *row_idx <= last_row_idx,
            TableSelectionMode::Grid {
                first_row_idx,
                last_row_idx,
                first_col_idx,
                last_col_idx,
            } => {
                *row_idx >= first_row_idx
                    && *row_idx <= last_row_idx
                    && col_idx >= first_col_idx
                    && col_idx <= last_col_idx
            }
        }
    }

    /// 判定表格内 `line_offset` 行上某个单元格 `col_idx` 被选中的绝对内容列闭区间 `Some((col0, col1))`；
    /// 未选中、水平边框行、空白填充行或空文本行返回 `None`。
    pub(super) fn selected_cell_cols(
        &self,
        line_offset: usize,
        col_idx: usize,
    ) -> Option<(u16, u16)> {
        if !self.is_cell_line_selected(line_offset, col_idx) {
            return None;
        }
        let TableLineKind::Cells { cells, .. } = self.table.lines.get(line_offset)? else {
            return None;
        };
        let col_span = self.table.columns.get(col_idx)?;
        let slice = cells.get(col_idx)?;
        if slice.text_start >= slice.text_end {
            return None;
        }
        let text_col0 = self.base_col.saturating_add(slice.text_start);
        let text_col1 = self
            .base_col
            .saturating_add(slice.text_end)
            .saturating_sub(1);
        let content_col0 = self.base_col.saturating_add(col_span.content_start);
        let content_col1 = self
            .base_col
            .saturating_add(col_span.content_end)
            .saturating_sub(1);

        let (raw_col0, raw_col1) = match self.mode {
            TableSelectionMode::SingleCell { .. } => {
                let ((start_line, start_col), (end_line, end_col)) = self.linear_bounds?;
                let c0 = if line_offset == start_line {
                    start_col
                } else {
                    content_col0
                };
                let c1 = if line_offset == end_line {
                    end_col
                } else {
                    content_col1
                };
                (c0, c1)
            }
            TableSelectionMode::ColumnLocked { .. } | TableSelectionMode::Grid { .. } => {
                (text_col0, text_col1)
            }
        };

        let col0 = raw_col0.max(text_col0);
        let col1 = raw_col1.min(text_col1);
        (col0 <= col1).then_some((col0, col1))
    }

    /// 返回表格物理行 `line_offset` 上所有被选中的单元格列闭区间列表（供渲染反显一次性遍历）。
    pub(super) fn selected_col_ranges(&self, line_offset: usize) -> Vec<(u16, u16)> {
        let (first_col, last_col) = match self.mode {
            TableSelectionMode::SingleCell { col_idx, .. }
            | TableSelectionMode::ColumnLocked { col_idx, .. } => (col_idx, col_idx),
            TableSelectionMode::Grid {
                first_col_idx,
                last_col_idx,
                ..
            } => (first_col_idx, last_col_idx),
        };
        let mut ranges = Vec::with_capacity(last_col.saturating_sub(first_col) + 1);
        for col_idx in first_col..=last_col {
            if let Some(range) = self.selected_cell_cols(line_offset, col_idx) {
                ranges.push(range);
            }
        }
        ranges
    }

    /// 返回当前选区计划实际涉及的表格物理行偏移闭区间 `(first_line_offset, last_line_offset)`。
    pub(super) fn selected_line_offset_range(&self) -> (usize, usize) {
        let (first_row_idx, last_row_idx) = match self.mode {
            TableSelectionMode::SingleCell { row_idx, .. } => (row_idx, row_idx),
            TableSelectionMode::ColumnLocked {
                first_row_idx,
                last_row_idx,
                ..
            }
            | TableSelectionMode::Grid {
                first_row_idx,
                last_row_idx,
                ..
            } => (first_row_idx, last_row_idx),
        };
        let first_line = self
            .table
            .logical_rows
            .get(first_row_idx)
            .map_or(0, |r| r.first_line);
        let last_line = self
            .table
            .logical_rows
            .get(last_row_idx)
            .map_or_else(|| self.table.lines.len().saturating_sub(1), |r| r.last_line);
        (first_line, last_line.max(first_line))
    }
}

/// 判定两份表格布局元数据是否对应同一张表格：优先 `Arc::ptr_eq` O(1) 短路，未缓存路径回退结构比较。
pub(super) fn same_table_meta(a: &Arc<TableLayoutMeta>, b: &Arc<TableLayoutMeta>) -> bool {
    Arc::ptr_eq(a, b) || a == b
}

/// 将行内列闭区间 `[col0, col1]` 对齐到 `grid` 第 `y` 行的完整双宽字符（CJK 等）边界：
/// - 若 `col0` 落在双宽字符右半格（`CONTINUATION`），左扩至主格 `col0 - 1`；
/// - 若 `col1` 落在双宽字符主格且 `col1 + 1` 为 `CONTINUATION`，右扩至 `col1 + 1`。
pub(super) fn align_wide_char_cols(grid: &Grid, y: u16, col0: u16, col1: u16) -> (u16, u16) {
    let width = grid.area().width;
    if width == 0 || col0 > col1 {
        return (col0, col1);
    }
    let mut c0 = col0.min(width - 1);
    let mut c1 = col1.min(width - 1);
    if c0 > 0 && grid.cell(c0, y).ch == CONTINUATION {
        c0 -= 1;
    }
    if c1 + 1 < width
        && grid.cell(c1, y).ch != CONTINUATION
        && grid.cell(c1 + 1, y).ch == CONTINUATION
    {
        c1 += 1;
    }
    (c0, c1)
}

/// 根据当前已渲染的 `grid` 扫描与 `table_row`（位于 `content_row`）同属一个连续可见表格片段的真实内容行上下界。
///
/// 当表格因汇报折叠（`REPORT_PREVIEW_ROWS`）或工具命令预览（`Preview::head_tail`）被截断时，
/// 视口内紧邻的非表格行会将有效行区间严格截断，防止在表格外部拖选时误反显表格行。
pub(super) fn rendered_table_row_bounds(
    grid: &Grid,
    grid_first_row: usize,
    grid_y_offset: u16,
    grid_rows: u16,
    x_offset: u16,
    table_row: &GridTableRow,
    content_row: i32,
) -> (i32, i32) {
    let line_offset_i32 = i32::try_from(table_row.line_ref.line_offset).unwrap_or(0);
    let theo_start = content_row.saturating_sub(line_offset_i32);
    let total_lines_i32 = i32::try_from(table_row.line_ref.table.lines.len()).unwrap_or(0);
    let theo_end = theo_start.saturating_add(total_lines_i32.saturating_sub(1));

    let first_i32 = i32::try_from(grid_first_row).unwrap_or(0);
    let cur_dy = u16::try_from(content_row.saturating_sub(first_i32).max(0)).unwrap_or(0);
    let row_matches = |dy: u16, abs_row: i32| -> bool {
        let Some(tr) = grid.table_row_at(grid_y_offset.saturating_add(dy)) else {
            return false;
        };
        let tr_base = tr.base_col.saturating_sub(x_offset);
        let off_i32 = i32::try_from(tr.line_ref.line_offset).unwrap_or(0);
        abs_row.saturating_sub(off_i32) == theo_start
            && tr_base == table_row.base_col
            && same_table_meta(&tr.line_ref.table, &table_row.line_ref.table)
    };

    let mut actual_start = theo_start;
    for d in (0..cur_dy).rev() {
        let r = first_i32.saturating_add(i32::from(d));
        if r < theo_start {
            break;
        }
        if !row_matches(d, r) {
            actual_start = r.saturating_add(1);
            break;
        }
    }

    let mut actual_end = theo_end;
    for d in (cur_dy.saturating_add(1))..grid_rows {
        let r = first_i32.saturating_add(i32::from(d));
        if r > theo_end {
            break;
        }
        if !row_matches(d, r) {
            actual_end = r.saturating_sub(1);
            break;
        }
    }

    (actual_start, actual_end.max(actual_start))
}

/// 根据当前表格行元数据、实际渲染行上下界 `actual_bounds` 与选区起止点解析表格选区计划。
pub(super) fn resolve_table_selection(
    table_row: &GridTableRow,
    content_row: i32,
    actual_bounds: (i32, i32),
    anchor: (i32, u16),
    cursor: (i32, u16),
) -> Option<TableSelectionPlan> {
    let table = &table_row.line_ref.table;
    let base_col = table_row.base_col;
    if table.columns.is_empty() || table.logical_rows.is_empty() || table.lines.is_empty() {
        return None;
    }
    let line_offset_i32 = i32::try_from(table_row.line_ref.line_offset).unwrap_or(0);
    let table_start_row = content_row.saturating_sub(line_offset_i32);
    let total_lines_i32 = i32::try_from(table.lines.len()).unwrap_or(0);
    let table_end_row = table_start_row.saturating_add(total_lines_i32.saturating_sub(1));

    let actual_start_row = actual_bounds.0.max(table_start_row);
    let actual_end_row = actual_bounds.1.min(table_end_row).max(actual_start_row);

    let (min_pt, max_pt) = if anchor <= cursor {
        (anchor, cursor)
    } else {
        (cursor, anchor)
    };
    if max_pt.0 < actual_start_row || min_pt.0 > actual_end_row {
        return None;
    }

    let table_right = base_col.saturating_add(table.total_width);
    let anchor_in_table = anchor.0 >= actual_start_row && anchor.0 <= actual_end_row;
    let cursor_in_table = cursor.0 >= actual_start_row && cursor.0 <= actual_end_row;

    if anchor_in_table && cursor_in_table {
        // 两端行号均在表内，但横坐标同时在表格左边界外侧或同时在右边界外侧：不触发内部单元格吸附。
        if (anchor.1 < base_col && cursor.1 < base_col)
            || (anchor.1 >= table_right && cursor.1 >= table_right)
        {
            return None;
        }

        let max_line = table.lines.len().saturating_sub(1);
        let anchor_line = usize::try_from(anchor.0.saturating_sub(table_start_row))
            .unwrap_or(0)
            .min(max_line);
        let cursor_line = usize::try_from(cursor.0.saturating_sub(table_start_row))
            .unwrap_or(0)
            .min(max_line);

        let anchor_row_idx = resolve_line_logical_row(table, anchor_line);
        let anchor_col_idx = resolve_anchor_col(table, base_col, anchor.1);

        // 单点零宽选区（如双击落在水平边框行、空白填充行或空单元格返回的退化点）：
        // 若当前物理行在该列并无有效文本，直接返回 None，禁止钳入邻近行误选单字。
        if anchor == cursor {
            let valid_single_point = match table.lines.get(anchor_line) {
                Some(TableLineKind::Cells { cells, .. }) => cells
                    .get(anchor_col_idx)
                    .is_some_and(|slice| !slice.is_filler && slice.text_start < slice.text_end),
                _ => false,
            };
            if !valid_single_point {
                return None;
            }
        }

        let focus_row_idx = resolve_focus_row(table, anchor_row_idx, cursor_line);
        let focus_col_idx = resolve_focus_col(table, base_col, anchor_col_idx, cursor.1);

        if anchor_row_idx == focus_row_idx && anchor_col_idx == focus_col_idx {
            let p_anchor = clamp_point_to_cell(
                table,
                base_col,
                anchor_row_idx,
                anchor_col_idx,
                anchor_line,
                anchor.1,
            );
            let p_focus = clamp_point_to_cell(
                table,
                base_col,
                anchor_row_idx,
                anchor_col_idx,
                cursor_line,
                cursor.1,
            );
            let bounds = if p_anchor <= p_focus {
                (p_anchor, p_focus)
            } else {
                (p_focus, p_anchor)
            };
            return Some(TableSelectionPlan {
                table: Arc::clone(table),
                table_start_row,
                table_end_row: actual_end_row,
                base_col,
                mode: TableSelectionMode::SingleCell {
                    row_idx: anchor_row_idx,
                    col_idx: anchor_col_idx,
                },
                linear_bounds: Some(bounds),
            });
        }

        let first_row_idx = anchor_row_idx.min(focus_row_idx);
        let last_row_idx = anchor_row_idx.max(focus_row_idx);
        if anchor_col_idx == focus_col_idx {
            return Some(TableSelectionPlan {
                table: Arc::clone(table),
                table_start_row,
                table_end_row: actual_end_row,
                base_col,
                mode: TableSelectionMode::ColumnLocked {
                    col_idx: anchor_col_idx,
                    first_row_idx,
                    last_row_idx,
                },
                linear_bounds: None,
            });
        }

        let first_col_idx = anchor_col_idx.min(focus_col_idx);
        let last_col_idx = anchor_col_idx.max(focus_col_idx);
        return Some(TableSelectionPlan {
            table: Arc::clone(table),
            table_start_row,
            table_end_row: actual_end_row,
            base_col,
            mode: TableSelectionMode::Grid {
                first_row_idx,
                last_row_idx,
                first_col_idx,
                last_col_idx,
            },
            linear_bounds: None,
        });
    }

    // 至少一端在表格外部（跨段落与表格联合拖选）：进入表格的部分按 Grid 选区处理。
    let max_line = table.lines.len().saturating_sub(1);
    let last_col = table.columns.len().saturating_sub(1);

    let (first_row_idx, first_col_idx) = if min_pt.0 < actual_start_row {
        let start_line_off = usize::try_from(actual_start_row.saturating_sub(table_start_row))
            .unwrap_or(0)
            .min(max_line);
        (resolve_line_logical_row(table, start_line_off), 0)
    } else {
        let min_line = usize::try_from(min_pt.0.saturating_sub(table_start_row))
            .unwrap_or(0)
            .min(max_line);
        (
            resolve_line_logical_row(table, min_line),
            resolve_anchor_col(table, base_col, min_pt.1),
        )
    };

    let (last_row_idx, last_col_idx) = if max_pt.0 > actual_end_row {
        let end_line_off = usize::try_from(actual_end_row.saturating_sub(table_start_row))
            .unwrap_or(0)
            .min(max_line);
        (resolve_line_logical_row(table, end_line_off), last_col)
    } else {
        let max_line_off = usize::try_from(max_pt.0.saturating_sub(table_start_row))
            .unwrap_or(0)
            .min(max_line);
        (
            resolve_line_logical_row(table, max_line_off),
            resolve_anchor_col(table, base_col, max_pt.1),
        )
    };

    Some(TableSelectionPlan {
        table: Arc::clone(table),
        table_start_row,
        table_end_row: actual_end_row,
        base_col,
        mode: TableSelectionMode::Grid {
            first_row_idx: first_row_idx.min(last_row_idx),
            last_row_idx: first_row_idx.max(last_row_idx),
            first_col_idx: first_col_idx.min(last_col_idx),
            last_col_idx: first_col_idx.max(last_col_idx),
        },
        linear_bounds: None,
    })
}

/// 表格内双击选词 / 三击选整格的区间计算。
pub(super) fn table_unit_cell_range(
    grid: &Grid,
    table_row: &GridTableRow,
    cell: (i32, u16),
    unit: SelectionUnit,
) -> Option<((i32, u16), (i32, u16))> {
    let table = &table_row.line_ref.table;
    let base_col = table_row.base_col;
    if table.columns.is_empty() || table.logical_rows.is_empty() || table.lines.is_empty() {
        return None;
    }
    let table_right = base_col.saturating_add(table.total_width);
    if cell.1 < base_col || cell.1 >= table_right {
        return None;
    }

    let line_offset = table_row.line_ref.line_offset;
    let row_idx = resolve_line_logical_row(table, line_offset);
    let col_idx = resolve_anchor_col(table, base_col, cell.1);
    let col_span = table.columns.get(col_idx)?;
    let content_col0 = base_col.saturating_add(col_span.content_start);
    let content_col1 = base_col
        .saturating_add(col_span.content_end)
        .saturating_sub(1)
        .max(content_col0);

    match unit {
        SelectionUnit::Character => Some((cell, cell)),
        SelectionUnit::Line => {
            // 三击：选中当前单元格的全部多行内容（首条物理行起点 → 最后一条非空白填充行终点）。
            let logical = table.logical_rows.get(row_idx)?;
            let last_non_filler = last_non_filler_line_of_cell(table, row_idx, col_idx);
            let line_offset_i32 = i32::try_from(line_offset).unwrap_or(0);
            let table_start_row = cell.0.saturating_sub(line_offset_i32);
            let start_row =
                table_start_row.saturating_add(i32::try_from(logical.first_line).unwrap_or(0));
            let end_row =
                table_start_row.saturating_add(i32::try_from(last_non_filler).unwrap_or(0));
            Some(((start_row, content_col0), (end_row, content_col1)))
        }
        SelectionUnit::Word => {
            // 双击：严格在当前单元格当行的真实文本区间 `[text_col0, text_col1]` 内分词。
            let TableLineKind::Cells { cells, .. } = table.lines.get(line_offset)? else {
                let c = cell.1.clamp(content_col0, content_col1);
                return Some(((cell.0, c), (cell.0, c)));
            };
            let slice = cells.get(col_idx)?;
            if slice.is_filler || slice.text_start >= slice.text_end {
                let c = cell.1.clamp(content_col0, content_col1);
                return Some(((cell.0, c), (cell.0, c)));
            }
            let text_col0 = base_col.saturating_add(slice.text_start);
            let text_col1 = base_col
                .saturating_add(slice.text_end)
                .saturating_sub(1)
                .max(text_col0);

            let mut tokens: Vec<(u16, u16, CharClass)> =
                Vec::with_capacity(usize::from(text_col1.saturating_sub(text_col0) + 1));
            for x in text_col0..=text_col1 {
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
                let c = cell.1.clamp(text_col0, text_col1);
                return Some(((cell.0, c), (cell.0, c)));
            }
            let target_col = cell.1.clamp(text_col0, text_col1);
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
            Some(((cell.0, tokens[left].0), (cell.0, tokens[right].1)))
        }
    }
}

/// 从已渲染的表格网格行中提取选中文本：
/// - 单元格内软折行无缝拼接（不插入多余 `\n`，且不含 `pad_line` 补齐空格）；
/// - 单元格内显式 `<br>`（含连续 `<br><br>`）硬换行保留 `\n`；
/// - 同一逻辑行多列以 `\t` 连接（`ColumnLocked` / `Grid` 保留空单元格占位），跨逻辑行以 `\n` 连接，剔除全部表格边框字符。
pub(super) fn extract_table_lines(
    grid: &Grid,
    grid_start_row: usize,
    plan: &TableSelectionPlan,
    present_line_offsets: &[usize],
) -> Vec<String> {
    let (first_row_idx, last_row_idx, first_col_idx, last_col_idx) = match plan.mode {
        TableSelectionMode::SingleCell { row_idx, col_idx } => (row_idx, row_idx, col_idx, col_idx),
        TableSelectionMode::ColumnLocked {
            col_idx,
            first_row_idx,
            last_row_idx,
        } => (first_row_idx, last_row_idx, col_idx, col_idx),
        TableSelectionMode::Grid {
            first_row_idx,
            last_row_idx,
            first_col_idx,
            last_col_idx,
        } => (first_row_idx, last_row_idx, first_col_idx, last_col_idx),
    };
    let is_single_cell = matches!(plan.mode, TableSelectionMode::SingleCell { .. });

    let mut out_rows: Vec<String> = Vec::new();
    for row_idx in first_row_idx..=last_row_idx {
        let Some(logical) = plan.table.logical_rows.get(row_idx) else {
            continue;
        };
        let row_has_rendered_line = (logical.first_line..=logical.last_line)
            .any(|line_offset| present_line_offsets.binary_search(&line_offset).is_ok());
        if !row_has_rendered_line {
            continue;
        }

        let mut row_cells: Vec<String> = Vec::with_capacity(last_col_idx - first_col_idx + 1);
        for col_idx in first_col_idx..=last_col_idx {
            let mut cell_text = String::new();
            let mut prev_hard_break = false;
            let mut has_prev_line = false;

            for line_offset in logical.first_line..=logical.last_line {
                if present_line_offsets.binary_search(&line_offset).is_err() {
                    continue;
                }
                if !plan.is_cell_line_selected(line_offset, col_idx) {
                    continue;
                }
                let Some(TableLineKind::Cells { cells, .. }) = plan.table.lines.get(line_offset)
                else {
                    continue;
                };
                let Some(slice) = cells.get(col_idx) else {
                    continue;
                };

                if has_prev_line && prev_hard_break {
                    let trimmed_len = cell_text.trim_end_matches(' ').len();
                    cell_text.truncate(trimmed_len);
                    cell_text.push('\n');
                }

                if let Some((raw_col0, raw_col1)) = plan.selected_cell_cols(line_offset, col_idx) {
                    let offset_i32 = i32::try_from(line_offset).unwrap_or(0);
                    let content_row_i32 = plan.table_start_row.saturating_add(offset_i32);
                    if let Ok(content_row) = usize::try_from(content_row_i32)
                        && let Some(rel_y) = content_row
                            .checked_sub(grid_start_row)
                            .and_then(|dy| u16::try_from(dy).ok())
                    {
                        let (col0, col1) = align_wide_char_cols(grid, rel_y, raw_col0, raw_col1);
                        for col in col0..=col1 {
                            let ch = grid.cell(col, rel_y).ch;
                            if ch != CONTINUATION {
                                cell_text.push(ch);
                            }
                        }
                    }
                }

                has_prev_line = true;
                prev_hard_break = slice.hard_break;
            }

            let trimmed = cell_text.trim_end_matches(' ');
            if is_single_cell {
                if !trimmed.is_empty() {
                    row_cells.push(trimmed.to_string());
                }
            } else {
                row_cells.push(trimmed.to_string());
            }
        }
        if !row_cells.is_empty() {
            out_rows.push(row_cells.join("\t"));
        }
    }
    out_rows
}

/// 将表格内物理行偏移 `line_offset` 映射为所属（或最近）的逻辑行下标 `row_idx`。
fn resolve_line_logical_row(table: &TableLayoutMeta, line_offset: usize) -> usize {
    let last_row = table.logical_rows.len().saturating_sub(1);
    match table.lines.get(line_offset) {
        Some(TableLineKind::Cells { row_idx, .. }) => (*row_idx).min(last_row),
        Some(TableLineKind::Border { nearest_row }) => (*nearest_row).min(last_row),
        None => last_row,
    }
}

/// 将按下起点列坐标映射为列下标（单元格内 1 格留白与紧邻右竖线均归属该列）。
fn resolve_anchor_col(table: &TableLayoutMeta, base_col: u16, col: u16) -> usize {
    if col <= base_col {
        return 0;
    }
    let rel_col = col.saturating_sub(base_col);
    for (idx, span) in table.columns.iter().enumerate() {
        if rel_col >= span.box_start && rel_col <= span.box_end {
            return idx;
        }
    }
    table.columns.len().saturating_sub(1)
}

/// 将拖动焦点列坐标映射为列下标（具备针对 `anchor_col_idx` 自身左右竖线及表格外侧的防手抖吸附）。
fn resolve_focus_col(
    table: &TableLayoutMeta,
    base_col: u16,
    anchor_col_idx: usize,
    cursor_col: u16,
) -> usize {
    let last_col = table.columns.len().saturating_sub(1);
    let anchor_col_idx = anchor_col_idx.min(last_col);
    let Some(anchor_span) = table.columns.get(anchor_col_idx) else {
        return 0;
    };

    // 起点列的防手抖吸附区：包含该列左右 1 格留白以及紧邻的左右 `│` 竖线；
    // 若起点在最左列则左扩到 0，若起点在最右列则右扩到 u16::MAX。
    let snap_left = if anchor_col_idx == 0 {
        0
    } else {
        base_col.saturating_add(anchor_span.box_start.saturating_sub(1))
    };
    let snap_right = if anchor_col_idx == last_col {
        u16::MAX
    } else {
        base_col.saturating_add(anchor_span.box_end)
    };

    if cursor_col >= snap_left && cursor_col <= snap_right {
        return anchor_col_idx;
    }

    if cursor_col < snap_left {
        let rel_col = cursor_col.saturating_sub(base_col);
        for (idx, span) in table.columns.iter().enumerate().take(anchor_col_idx) {
            if rel_col >= span.box_start.saturating_sub(1) && rel_col < span.box_end {
                return idx;
            }
        }
        return 0;
    }

    let rel_col = cursor_col.saturating_sub(base_col);
    for (idx, span) in table.columns.iter().enumerate().skip(anchor_col_idx + 1) {
        if rel_col >= span.box_start && rel_col <= span.box_end {
            return idx;
        }
    }
    last_col
}

/// 将拖动焦点物理行映射为逻辑行下标（对 `anchor_row_idx` 紧邻的上下水平分割线执行防手抖吸附）。
fn resolve_focus_row(
    table: &TableLayoutMeta,
    anchor_row_idx: usize,
    cursor_line_offset: usize,
) -> usize {
    let last_row = table.logical_rows.len().saturating_sub(1);
    let anchor_row_idx = anchor_row_idx.min(last_row);
    let Some(anchor_logical) = table.logical_rows.get(anchor_row_idx) else {
        return 0;
    };

    if cursor_line_offset >= anchor_logical.first_line
        && cursor_line_offset <= anchor_logical.last_line
    {
        return anchor_row_idx;
    }
    // 紧贴起点逻辑行上方或下方的水平边框线：吸附回起点逻辑行。
    if (cursor_line_offset.saturating_add(1) == anchor_logical.first_line
        || cursor_line_offset == anchor_logical.last_line.saturating_add(1))
        && matches!(
            table.lines.get(cursor_line_offset),
            Some(TableLineKind::Border { .. })
        )
    {
        return anchor_row_idx;
    }

    match table.lines.get(cursor_line_offset) {
        Some(TableLineKind::Cells { row_idx, .. }) => (*row_idx).min(last_row),
        Some(TableLineKind::Border { nearest_row }) => {
            if cursor_line_offset > anchor_logical.last_line {
                // 向下拖动停在某条水平分割线上：归属该分割线上方已滑过的逻辑行。
                cursor_line_offset
                    .checked_sub(1)
                    .and_then(|prev| match table.lines.get(prev) {
                        Some(TableLineKind::Cells { row_idx, .. }) => {
                            Some((*row_idx).min(last_row))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| (*nearest_row).min(last_row))
            } else {
                // 向上拖动停在某条水平分割线上：归属该分割线下方已滑过的逻辑行。
                table
                    .lines
                    .get(cursor_line_offset.saturating_add(1))
                    .and_then(|next| match next {
                        TableLineKind::Cells { row_idx, .. } => Some((*row_idx).min(last_row)),
                        _ => None,
                    })
                    .unwrap_or_else(|| (*nearest_row).min(last_row))
            }
        }
        None => last_row,
    }
}

/// 查找某单元格 `(row_idx, col_idx)` 的最后一条非空白填充物理行偏移。
fn last_non_filler_line_of_cell(table: &TableLayoutMeta, row_idx: usize, col_idx: usize) -> usize {
    let Some(logical) = table.logical_rows.get(row_idx) else {
        return 0;
    };
    for line_offset in (logical.first_line..=logical.last_line).rev() {
        if let Some(TableLineKind::Cells { cells, .. }) = table.lines.get(line_offset)
            && let Some(slice) = cells.get(col_idx)
            && !slice.is_filler
        {
            return line_offset;
        }
    }
    logical.first_line
}

/// 将任意表内坐标 `(raw_line_offset, raw_col)` 钳入指定单元格 `(row_idx, col_idx)` 的有效内容区。
fn clamp_point_to_cell(
    table: &TableLayoutMeta,
    base_col: u16,
    row_idx: usize,
    col_idx: usize,
    raw_line_offset: usize,
    raw_col: u16,
) -> (usize, u16) {
    let Some(logical) = table.logical_rows.get(row_idx) else {
        return (raw_line_offset, raw_col);
    };
    let Some(col_span) = table.columns.get(col_idx) else {
        return (raw_line_offset, raw_col);
    };
    let content_col0 = base_col.saturating_add(col_span.content_start);
    let content_col1 = base_col
        .saturating_add(col_span.content_end)
        .saturating_sub(1)
        .max(content_col0);
    let last_non_filler = last_non_filler_line_of_cell(table, row_idx, col_idx);

    if raw_line_offset < logical.first_line {
        (logical.first_line, content_col0)
    } else if raw_line_offset > last_non_filler {
        (last_non_filler, content_col1)
    } else {
        (raw_line_offset, raw_col.clamp(content_col0, content_col1))
    }
}

#[cfg(test)]
mod tests;

//! prompt 视口几何与坐标映射：软换行、滚动、选区与单元格/字节换算。

use crate::app::paste::{clamp_cursor, mask_pasted_text, snap_pasted_range};

use super::super::markdown;
use super::{Prompt, VisualRow, char_width, display_width};

/// 多击选词/选行返回的（单元格闭区间，字节半开区间）。
pub type UnitCellSpan = (((i32, u16), (i32, u16)), (usize, usize));

impl Prompt {
    /// 按内容宽度软换行得到的可见视觉行；块行扣除左侧色条槽、展开芯片行扣除 2 列缩进后再换行。
    pub fn visual_rows(&self) -> Vec<VisualRow> {
        let width = usize::from(self.width.max(1));
        let margins = usize::from(crate::layout::PROMPT_BLOCK_GUTTER_LEFT);
        let masked = mask_pasted_text(&self.text);
        let scan = markdown::block_scan(&masked);
        let proj = self.projection();
        let proj_text = proj.text();
        let lines: Vec<&str> = proj_text.split('\n').collect();
        let mut rows = Vec::new();
        let mut line_start = 0;
        let mut prev_raw_line = None;
        for segment in &lines {
            let line_end = line_start + segment.len();
            let line = proj.raw_line_of_display(line_start);
            let info = scan.infos.get(line).copied().flatten();
            if prev_raw_line != Some(line)
                && info.is_some_and(|info| info.role == markdown::BlockRole::Start)
            {
                rows.push(VisualRow {
                    line,
                    start: line_start,
                    end: line_start,
                    top_border: true,
                });
            }
            prev_raw_line = Some(line);
            let indent = usize::from(proj.row_indent(line_start, line_end));
            let row_width = if info.is_some() {
                width.saturating_sub(margins + indent).max(1)
            } else {
                width.saturating_sub(indent).max(1)
            };
            let mut row_start = line_start;
            let mut col = 0;
            for (offset, ch) in segment.char_indices() {
                let absolute = line_start + offset;
                let cell = char_width(ch);
                if col > 0 && col + cell > row_width {
                    rows.push(VisualRow {
                        line,
                        start: row_start,
                        end: absolute,
                        top_border: false,
                    });
                    row_start = absolute;
                    col = 0;
                }
                col += cell;
            }
            rows.push(VisualRow {
                line,
                start: row_start,
                end: line_end,
                top_border: false,
            });
            line_start = line_end + 1;
        }
        rows
    }

    /// 光标在视口内的坐标（行、列，列含模板块左边框槽与展开预览缩进）；滚出视口时为 None。
    pub fn cursor_cell(&self) -> Option<(u16, u16)> {
        if self.height == 0 || self.width == 0 {
            return None;
        }
        let rows = self.visual_rows();
        if rows.is_empty() {
            return None;
        }
        let proj = self.projection();
        let (row, col) = self.cursor_visual_in(&rows);
        let visible = row.checked_sub(self.scroll)?;
        if visible >= usize::from(self.height) {
            return None;
        }
        let gutter = usize::from(
            self.line_gutter(rows[row].line) + proj.row_indent(rows[row].start, rows[row].end),
        );
        let col = (col + gutter).min(usize::from(self.width) - 1);
        Some((visible as u16, col as u16))
    }

    /// 按视觉行滚动视口；光标不动，超界时钳到可滚动范围；返回是否实际移动。
    pub fn scroll_by(&mut self, lines: isize) -> bool {
        let max = self.max_scroll();
        let target = (self.scroll.min(max) as isize).saturating_add(lines);
        let next = target.clamp(0, max as isize) as usize;
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    /// 直接设置视口滚动偏移（滚动条点击/拖拽）；越界钳到可滚动范围。
    pub fn scroll_to(&mut self, offset: usize) {
        self.scroll = offset.min(self.max_scroll());
    }

    /// 底部虚拟空占位行数：当前编辑视口高度的四分之一（向下取整）。
    ///
    /// 只计入最大滚动值与滚动条几何，不写入正文、不参与光标/点击/选区映射。
    /// 高度不足 4 行时取整为 0（矮视口没有额外留白）。
    pub fn bottom_padding(&self) -> usize {
        usize::from(self.height) / 4
    }

    /// 视口内的行是否落在正文之后的底部留白（含空文档时的占位区）。
    pub fn row_in_padding(&self, viewport_row: u16) -> bool {
        self.scroll.saturating_add(usize::from(viewport_row)) >= self.visual_rows().len()
    }

    /// 真实文本的最大滚动偏移（视觉行数减视口高度，不含底部留白）。
    pub fn text_max_scroll(&self) -> usize {
        self.visual_rows()
            .len()
            .saturating_sub(usize::from(self.height.max(1)))
    }

    /// 最大可滚动偏移：真实文本上限加底部虚拟空占位。
    pub(super) fn max_scroll(&self) -> usize {
        (self.visual_rows().len() + self.bottom_padding())
            .saturating_sub(usize::from(self.height.max(1)))
    }

    /// 将视口单元格坐标映射为光标位置；超出文本时钳到最近行行尾。
    pub fn set_cursor_from_cell(&mut self, row: u16, col: u16) {
        self.break_group();
        self.cursor = self.viewport_offset((row, col), false);
        self.settle();
    }

    /// 直接将光标设置到指定字节偏移（多击选词/选行时同步光标）。
    pub fn set_cursor_offset(&mut self, offset: usize) {
        self.break_group();
        let clamped = self.text.floor_char_boundary(offset.min(self.text.len()));
        self.cursor = clamp_cursor(&self.text, clamped);
        self.settle();
    }

    /// 按选区粒度（单字 / 词 / 逻辑行）计算内容行坐标下的单元格闭区间与字节范围。
    pub fn unit_cell_range(
        &self,
        content_row: usize,
        col: u16,
        unit: crate::app::selection::SelectionUnit,
    ) -> UnitCellSpan {
        use crate::app::selection::SelectionUnit;
        let rows = self.visual_rows();
        let row_i32 = i32::try_from(content_row).unwrap_or(0);
        if rows.is_empty() || unit == SelectionUnit::Character {
            let pos = self.content_offset_at(content_row, col, false);
            return (((row_i32, col), (row_i32, col)), (pos, pos));
        }
        let proj = self.projection();
        let index = content_row.min(rows.len() - 1);
        match unit {
            SelectionUnit::Character => unreachable!(),
            SelectionUnit::Line => {
                let target_line = rows[index].line;
                let first_row = rows
                    .iter()
                    .position(|r| r.line == target_line)
                    .unwrap_or(index);
                let last_row = rows
                    .iter()
                    .rposition(|r| r.line == target_line)
                    .unwrap_or(index);
                let start_byte = proj.display_to_raw_directed(rows[first_row].start, false);
                let end_byte = proj.display_to_raw_directed(rows[last_row].end, true);
                let snapped = snap_pasted_range(&self.text, start_byte..end_byte);
                let max_col = self.width.saturating_sub(1);
                (
                    (
                        (i32::try_from(first_row).unwrap_or(0), 0),
                        (i32::try_from(last_row).unwrap_or(0), max_col),
                    ),
                    (snapped.start, snapped.end),
                )
            }
            SelectionUnit::Word => {
                let pos = self.content_offset_at(index, col, false);
                let (start_byte, end_byte) = unit.byte_range(&self.text, pos);
                let snapped = snap_pasted_range(&self.text, start_byte..end_byte);
                if snapped.start >= snapped.end {
                    return (
                        ((row_i32, col), (row_i32, col)),
                        (snapped.start, snapped.end),
                    );
                }
                let Some(cells) = self.cell_range_for_byte_span(snapped.start, snapped.end) else {
                    return (
                        ((row_i32, col), (row_i32, col)),
                        (snapped.start, snapped.end),
                    );
                };
                (cells, (snapped.start, snapped.end))
            }
        }
    }

    /// 原始字节区间 → 内容行单元格闭区间（起止字符所在视觉行）；跳过顶部虚拟边框行。
    pub fn cell_range_for_byte_span(
        &self,
        start_byte: usize,
        end_byte: usize,
    ) -> Option<((i32, u16), (i32, u16))> {
        let start_byte = self
            .text
            .floor_char_boundary(start_byte.min(self.text.len()));
        let end_byte = self.text.floor_char_boundary(end_byte.min(self.text.len()));
        if start_byte >= end_byte {
            return None;
        }
        let proj = self.projection();
        let disp_start = proj.raw_to_display(start_byte);
        let disp_end = proj.raw_to_display(end_byte);
        self.cell_range_for_display_span(disp_start, disp_end)
    }

    /// 投影显示文本字节区间 → 内容行单元格闭区间。
    pub fn cell_range_for_display_span(
        &self,
        start_byte: usize,
        end_byte: usize,
    ) -> Option<((i32, u16), (i32, u16))> {
        let proj = self.projection();
        let disp_text = proj.text();
        let start_byte = disp_text.floor_char_boundary(start_byte.min(disp_text.len()));
        let end_byte = disp_text.floor_char_boundary(end_byte.min(disp_text.len()));
        if start_byte >= end_byte {
            return None;
        }
        let rows = self.visual_rows();
        if rows.is_empty() {
            return None;
        }
        let row_at = |byte: usize| {
            rows.iter().enumerate().find_map(|(index, row)| {
                (!row.top_border
                    && row.start <= byte
                    && (byte < row.end || (byte == row.end && row.start == row.end)))
                    .then_some(index)
            })
        };
        let start_idx = row_at(start_byte)?;
        let start_gutter = self.line_gutter(rows[start_idx].line)
            + proj.row_indent(rows[start_idx].start, rows[start_idx].end);
        let start_col = start_gutter
            .saturating_add(display_width(&disp_text[rows[start_idx].start..start_byte]) as u16);
        let slice = &disp_text[start_byte..end_byte];
        let (last_rel, last_ch) = slice
            .char_indices()
            .rfind(|(_, ch)| char_width(*ch) > 0)
            .or_else(|| slice.char_indices().next_back())?;
        let last_char_byte = start_byte + last_rel;
        let end_idx = row_at(last_char_byte).unwrap_or(start_idx);
        let end_gutter = self.line_gutter(rows[end_idx].line)
            + proj.row_indent(rows[end_idx].start, rows[end_idx].end);
        let end_col = end_gutter
            .saturating_add(display_width(&disp_text[rows[end_idx].start..last_char_byte]) as u16)
            .saturating_add(char_width(last_ch).saturating_sub(1) as u16);
        Some((
            (i32::try_from(start_idx).unwrap_or(0), start_col),
            (i32::try_from(end_idx).unwrap_or(0), end_col),
        ))
    }

    /// 选区（内容行坐标，端点包含）对应的字节范围；空选区或选中内容为空时返回 None。
    pub fn selection_bounds(&self, start: (u16, u16), end: (u16, u16)) -> Option<(usize, usize)> {
        self.selection_bounds_with(start, end, false)
    }

    /// 选区对应的原始字节范围；`allow_single_cell` 为真时允许双击选中的单列词；覆盖折叠芯片时自动吸附整块。
    pub fn selection_bounds_with(
        &self,
        start: (u16, u16),
        end: (u16, u16),
        allow_single_cell: bool,
    ) -> Option<(usize, usize)> {
        if !allow_single_cell && start == end {
            return None;
        }
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let from = self.content_offset(first, false);
        let to = self.content_offset(last, true);
        let snapped = snap_pasted_range(&self.text, from..to);
        (snapped.start < snapped.end).then_some((snapped.start, snapped.end))
    }

    /// 取选区文本（内容行坐标，端点包含）；折叠芯片复制完整 token，展开芯片复制所见原文。
    pub fn selection_text(&self, start: (u16, u16), end: (u16, u16)) -> Option<String> {
        self.selection_text_with(start, end, false)
    }

    /// 取选区文本；`allow_single_cell` 为真时允许双击选中的单列词。
    pub fn selection_text_with(
        &self,
        start: (u16, u16),
        end: (u16, u16),
        allow_single_cell: bool,
    ) -> Option<String> {
        if !allow_single_cell && start == end {
            return None;
        }
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let rows = self.visual_rows();
        if rows.is_empty() {
            return None;
        }
        let proj = self.projection();
        let proj_text = proj.text();
        let first_row = usize::from(first.0).min(rows.len() - 1);
        let last_row = usize::from(last.0).min(rows.len() - 1);
        let mut lines: Vec<String> = Vec::new();
        let mut prev_end: Option<usize> = None;
        for (offset, row) in rows[first_row..=last_row].iter().enumerate() {
            if row.top_border {
                if prev_end.is_none_or(|pe| row.start > pe) {
                    lines.push(String::new());
                    prev_end = Some(row.start);
                }
                continue;
            }
            let index = first_row + offset;
            let gutter = self.line_gutter(row.line) + proj.row_indent(row.start, row.end);
            let slice = &proj_text[row.start..row.end];
            let from = if index == first_row {
                let col_offset = column_offset(slice, first.1.saturating_sub(gutter));
                if !allow_single_cell
                    && col_offset < slice.len()
                    && slice[..col_offset].bytes().all(|byte| byte == b' ')
                {
                    row.start
                } else {
                    row.start + col_offset
                }
            } else {
                row.start
            };
            let to = if index == last_row {
                row.start + column_offset_inclusive(slice, last.1.saturating_sub(gutter))
            } else {
                row.end
            };
            if prev_end.is_none_or(|pe| row.start > pe) {
                lines.push(String::new());
            }
            prev_end = Some(row.end);
            if from < to
                && let Some(current) = lines.last_mut()
            {
                current.push_str(&proj.copy_display_range(from..to));
            }
        }
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// 光标在视觉行中的索引与显示列（跨折行边界时归入下一视觉行）。
    pub(super) fn cursor_visual_in(&self, rows: &[VisualRow]) -> (usize, usize) {
        if rows.is_empty() {
            return (0, 0);
        }
        let proj = self.projection();
        let disp_cursor = proj.raw_to_display(self.cursor);
        let disp_text = proj.text();
        for (index, row) in rows.iter().enumerate() {
            if row.top_border {
                continue;
            }
            if disp_cursor < row.start {
                return (index, 0);
            }
            if disp_cursor < row.end {
                return (index, display_width(&disp_text[row.start..disp_cursor]));
            }
            if disp_cursor == row.end {
                let continues = rows
                    .get(index + 1)
                    .is_some_and(|next| !next.top_border && next.start == row.end);
                if !continues {
                    return (index, display_width(&disp_text[row.start..row.end]));
                }
            }
        }
        let index = rows.len().saturating_sub(1);
        let row = rows[index];
        (index, display_width(&disp_text[row.start..row.end]))
    }

    /// 视口坐标映射到原始文本字节偏移；`after` 为真时取该单元格字符之后。
    fn viewport_offset(&self, (row, col): (u16, u16), after: bool) -> usize {
        self.content_offset_at(self.scroll.saturating_add(usize::from(row)), col, after)
    }

    /// 内容行坐标映射到原始文本字节偏移；`after` 为真时取该单元格字符之后。
    fn content_offset(&self, (row, col): (u16, u16), after: bool) -> usize {
        self.content_offset_at(usize::from(row), col, after)
    }

    /// 视觉行索引（内容坐标）映射到原始文本字节偏移。
    pub(super) fn content_offset_at(&self, index: usize, col: u16, after: bool) -> usize {
        let disp = self.display_content_offset_at(index, col, after);
        self.projection().display_to_raw_directed(disp, after)
    }

    /// 视觉行索引（内容坐标）映射到投影显示文本字节偏移。
    pub(super) fn display_content_offset_at(&self, index: usize, col: u16, after: bool) -> usize {
        let rows = self.visual_rows();
        if rows.is_empty() {
            return 0;
        }
        let proj = self.projection();
        let index = index.min(rows.len() - 1);
        let visual = rows[index];
        let gutter = self.line_gutter(visual.line) + proj.row_indent(visual.start, visual.end);
        let col = col.saturating_sub(gutter);
        let slice = &proj.text()[visual.start..visual.end];
        let mut cell = 0;
        for (offset, ch) in slice.char_indices() {
            let width = char_width(ch);
            if usize::from(col) < cell + width {
                return visual.start
                    + if after {
                        offset + ch.len_utf8()
                    } else {
                        offset
                    };
            }
            cell += width;
        }
        visual.end
    }

    /// 块行左侧色条槽宽度；非块行为 0（渲染、光标与鼠标坐标映射共用）。
    pub(super) fn line_gutter(&self, line: usize) -> u16 {
        let masked = mask_pasted_text(&self.text);
        if markdown::block_span_at_line(&masked, line).is_some() {
            crate::layout::PROMPT_BLOCK_GUTTER_LEFT
        } else {
            0
        }
    }

    /// 调整滚动量，保证光标所在视觉行可见；已在底部留白内滚动时不回拉。
    pub(super) fn scroll_cursor_into_view(&mut self) {
        let rows = self.visual_rows();
        if rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let height = usize::from(self.height.max(1));
        let (row, _) = self.cursor_visual_in(&rows);
        if row < self.scroll {
            self.scroll = if height > 1 && row > 0 && rows[row - 1].top_border {
                row - 1
            } else {
                row
            };
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// 让逻辑行区间 `[start_line, end_line]` 尽量完整可见；块高于视口时保证起始行可见。
    pub(super) fn scroll_lines_into_view(&mut self, start_line: usize, end_line: usize) {
        let rows = self.visual_rows();
        if rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let Some(first) = rows.iter().position(|row| row.line >= start_line) else {
            return;
        };
        let last = rows
            .iter()
            .rposition(|row| row.line <= end_line)
            .unwrap_or(first);
        let height = usize::from(self.height.max(1));
        let span = last.saturating_sub(first) + 1;
        if span >= height || first < self.scroll {
            self.scroll = first;
        } else if last >= self.scroll + height {
            self.scroll = last + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }
}

/// 显示列 → 行内字节偏移（宽字符取整到字符边界）。
fn column_offset(slice: &str, column: u16) -> usize {
    let mut cell = 0;
    for (offset, ch) in slice.char_indices() {
        let width = char_width(ch);
        if usize::from(column) < cell + width {
            return offset;
        }
        cell += width;
    }
    slice.len()
}

/// 显示列 → 该列字符之后的字节偏移（选区终点含端点）。
fn column_offset_inclusive(slice: &str, column: u16) -> usize {
    let mut cell = 0;
    for (offset, ch) in slice.char_indices() {
        let width = char_width(ch);
        if usize::from(column) < cell + width {
            return offset + ch.len_utf8();
        }
        cell += width;
    }
    slice.len()
}

//! prompt 视口几何与坐标映射：软换行、折叠隐藏、滚动、选区与单元格/字节换算。
//!
//! 折叠的补充块只保留起始行；隐藏行不进入视觉行序，因此光标、点击、选区与滚动
//! 共用同一份可见行口径。

use super::super::markdown;
use super::fold;
use super::{Prompt, VisualRow, char_width, display_width};

/// 多击选词/选行返回的（单元格闭区间，字节半开区间）。
pub type UnitCellSpan = (((i32, u16), (i32, u16)), (usize, usize));

impl Prompt {
    /// 按内容宽度软换行得到的可见视觉行；块行扣除左右边框槽后再换行，
    /// 折叠补充块的隐藏行整体跳过。
    pub fn visual_rows(&self) -> Vec<VisualRow> {
        let width = usize::from(self.width.max(1));
        let margins = usize::from(
            crate::layout::PROMPT_TEMPLATE_GUTTER_LEFT
                + crate::layout::PROMPT_TEMPLATE_GUTTER_RIGHT,
        );
        let scan = markdown::block_scan(&self.text);
        let lines: Vec<&str> = self.text.split('\n').collect();
        // 块聚焦时只保留该块的逻辑行区间（含起止行）；范围失效时退回普通视图。
        let focus = self.focused_block_range(&scan);
        let mut hidden = vec![false; lines.len()];
        for block in fold::folded_blocks_from(&scan, &self.text, &self.expanded_supplements) {
            for line in block.hidden_from_line..=block.end_line {
                if let Some(slot) = hidden.get_mut(line) {
                    *slot = true;
                }
            }
        }
        let mut rows = Vec::new();
        let mut line_start = 0;
        for (line, segment) in lines.iter().enumerate() {
            let line_end = line_start + segment.len();
            // 聚焦块外的逻辑行整体跳过（软换行判定在块内照旧）。
            let outside_focus = focus.is_some_and(|(start, end)| line < start || line > end);
            if hidden.get(line).copied().unwrap_or(false) || outside_focus {
                line_start = line_end + 1;
                continue;
            }
            let info = scan.infos.get(line).copied().flatten();
            let width = if info.is_some() {
                width.saturating_sub(margins).max(1)
            } else {
                width
            };
            let mut row_start = line_start;
            let mut col = 0;
            for (offset, ch) in segment.char_indices() {
                let absolute = line_start + offset;
                let cell = char_width(ch);
                if col > 0 && col + cell > width {
                    rows.push(VisualRow {
                        line,
                        start: row_start,
                        end: absolute,
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
            });
            line_start = line_end + 1;
        }
        rows
    }
    /// 光标在视口内的坐标（行、列，列含模板块左边框槽）；滚出视口时为 None。
    pub fn cursor_cell(&self) -> Option<(u16, u16)> {
        if self.height == 0 || self.width == 0 {
            return None;
        }
        let rows = self.visual_rows();
        if rows.is_empty() {
            return None;
        }
        let (row, col) = self.cursor_visual_in(&rows);
        let visible = row.checked_sub(self.scroll)?;
        if visible >= usize::from(self.height) {
            return None;
        }
        let gutter = usize::from(self.line_gutter(rows[row].line));
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
    ///
    /// 与滚动条几何共用同一口径——滚动条 `total = 视觉行 + 留白`、可拖距离
    /// `total - 视口高`，这里直接按同一公式计算，因此「滚动条不可见 ⇔ 上限为 0 ⇔
    /// 滚轮/拖拽/轨道点击一律无效」是恒等式，不需要第二套溢出判据。
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
        self.cursor = self.text.floor_char_boundary(offset.min(self.text.len()));
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
                let start_byte = rows[first_row].start;
                let end_byte = rows[last_row].end;
                let max_col = self.width.saturating_sub(1);
                (
                    (
                        (i32::try_from(first_row).unwrap_or(0), 0),
                        (i32::try_from(last_row).unwrap_or(0), max_col),
                    ),
                    (start_byte, end_byte),
                )
            }
            SelectionUnit::Word => {
                let pos = self.content_offset_at(index, col, false);
                let (start_byte, end_byte) = unit.byte_range(&self.text, pos);
                if start_byte >= end_byte {
                    return (((row_i32, col), (row_i32, col)), (start_byte, end_byte));
                }
                let start_idx = rows
                    .iter()
                    .position(|r| r.start <= start_byte && start_byte < r.end)
                    .unwrap_or(index);
                let start_gutter = self.line_gutter(rows[start_idx].line);
                let start_col = start_gutter.saturating_add(display_width(
                    &self.text[rows[start_idx].start..start_byte],
                ) as u16);
                let last_char_byte =
                    super::text::prev_boundary(&self.text, end_byte).max(start_byte);
                let end_idx = rows
                    .iter()
                    .position(|r| r.start <= last_char_byte && last_char_byte < r.end)
                    .unwrap_or(start_idx);
                let end_gutter = self.line_gutter(rows[end_idx].line);
                let ch_width = self.text[last_char_byte..]
                    .chars()
                    .next()
                    .map_or(1, char_width)
                    .max(1) as u16;
                let end_col = end_gutter
                    .saturating_add(
                        display_width(&self.text[rows[end_idx].start..last_char_byte]) as u16,
                    )
                    .saturating_add(ch_width - 1);
                (
                    (
                        (i32::try_from(start_idx).unwrap_or(0), start_col),
                        (i32::try_from(end_idx).unwrap_or(0), end_col),
                    ),
                    (start_byte, end_byte),
                )
            }
        }
    }
    /// 选区（内容行坐标，端点包含）对应的字节范围；空选区或选中内容为空时返回 None。
    pub fn selection_bounds(&self, start: (u16, u16), end: (u16, u16)) -> Option<(usize, usize)> {
        self.selection_bounds_with(start, end, false)
    }
    /// 选区对应的字节范围；`allow_single_cell` 为真时允许双击选中的单列词。
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
        (from < to).then_some((from, to))
    }
    /// 取选区文本（内容行坐标，端点包含）；只拼接可见行，折叠区不产出文字。
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
        let first_row = usize::from(first.0).min(rows.len() - 1);
        let last_row = usize::from(last.0).min(rows.len() - 1);
        let mut lines: Vec<String> = Vec::new();
        let mut previous_line = None;
        for (offset, row) in rows[first_row..=last_row].iter().enumerate() {
            let index = first_row + offset;
            let slice = &self.text[row.start..row.end];
            let from = if index == first_row {
                row.start + column_offset(slice, first.1)
            } else {
                row.start
            };
            let to = if index == last_row {
                row.start + column_offset_inclusive(slice, last.1)
            } else {
                row.end
            };
            if previous_line != Some(row.line) {
                lines.push(String::new());
                previous_line = Some(row.line);
            }
            if from < to
                && let Some(current) = lines.last_mut()
            {
                current.push_str(&self.text[from..to]);
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
        for (index, row) in rows.iter().enumerate() {
            // 光标落在折叠区留下的空隙：归到其后首个可见行行首。
            if self.cursor < row.start {
                return (index, 0);
            }
            if self.cursor < row.end {
                return (index, display_width(&self.text[row.start..self.cursor]));
            }
            if self.cursor == row.end {
                let continues = rows
                    .get(index + 1)
                    .is_some_and(|next| next.start == row.end && next.line == row.line);
                if !continues {
                    return (index, display_width(&self.text[row.start..row.end]));
                }
            }
        }
        let index = rows.len().saturating_sub(1);
        let row = rows[index];
        (index, display_width(&self.text[row.start..row.end]))
    }
    /// 视口坐标映射到文本字节偏移；`after` 为真时取该单元格字符之后。
    fn viewport_offset(&self, (row, col): (u16, u16), after: bool) -> usize {
        self.content_offset_at(self.scroll.saturating_add(usize::from(row)), col, after)
    }
    /// 内容行坐标映射到文本字节偏移；`after` 为真时取该单元格字符之后。
    fn content_offset(&self, (row, col): (u16, u16), after: bool) -> usize {
        self.content_offset_at(usize::from(row), col, after)
    }
    /// 视觉行索引（内容坐标）映射到文本字节偏移。
    fn content_offset_at(&self, index: usize, col: u16, after: bool) -> usize {
        let rows = self.visual_rows();
        if rows.is_empty() {
            return 0;
        }
        let index = index.min(rows.len() - 1);
        let visual = rows[index];
        let col = col.saturating_sub(self.line_gutter(visual.line));
        let slice = &self.text[visual.start..visual.end];
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
    /// 块行左边框槽宽度；非块行为 0（渲染、光标与鼠标坐标映射共用）。
    fn line_gutter(&self, line: usize) -> u16 {
        if markdown::block_span_at_line(&self.text, line).is_some() {
            crate::layout::PROMPT_TEMPLATE_GUTTER_LEFT
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
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
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

//! prompt 视口几何与坐标映射：显示投影、软换行、滚动、选区范围与单元格/字节换算。
//!
//! 集中视图（`Compact`）下安全补充子块不逐行显示，父任务末尾插入一个合成附件入口；
//! 原文视图（`Source`）完整显示。所有行高、光标、命中与滚动共用同一份显示投影。

use super::super::markdown;
use super::presentation::{
    self, PromptDisplayRow, SupplementIndex, project_compact, project_source, range_hits_hidden,
};
use super::{Prompt, PromptFocus, PromptPresentation, VisualRow, char_width, display_width};

impl Prompt {
    /// 显示投影行：集中视图含合成附件入口，原文视图全部为真实文本行。
    pub fn display_rows(&self) -> Vec<PromptDisplayRow> {
        let width = usize::from(self.width.max(1));
        match self.presentation {
            PromptPresentation::Compact => project_compact(&self.text, width).rows,
            PromptPresentation::Source => project_source(&self.text, width).rows,
        }
    }

    /// 当前显示的附件索引；原文视图下原文本完整可见，不作为隐藏区。
    pub fn supplement_index(&self) -> SupplementIndex {
        match self.presentation {
            PromptPresentation::Compact => presentation::supplement_index(&self.text),
            PromptPresentation::Source => SupplementIndex::default(),
        }
    }

    /// 按内容宽度软换行得到的可见文本行；隐藏附件行不在此列。
    pub fn visual_rows(&self) -> Vec<VisualRow> {
        self.display_rows()
            .into_iter()
            .filter_map(|row| match row {
                PromptDisplayRow::Text { line, start, end } => Some(VisualRow { line, start, end }),
                PromptDisplayRow::Supplements { .. } => None,
            })
            .collect()
    }

    /// 光标在视口内的坐标（行、列，列含模板块左边框槽）；
    /// 滚出视口或焦点在合成附件入口时为 None（入口焦点用标记表达，不显示硬件光标）。
    pub fn cursor_cell(&self) -> Option<(u16, u16)> {
        if self.height == 0 || self.width == 0 {
            return None;
        }
        let display = self.display_rows();
        let (index, col) = self.display_cursor_in(&display);
        let visible = index.checked_sub(self.scroll)?;
        if visible >= usize::from(self.height) {
            return None;
        }
        let PromptDisplayRow::Text { line, .. } = display[index] else {
            return None;
        };
        let gutter = usize::from(self.line_gutter(line));
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
        self.scroll.saturating_add(usize::from(viewport_row)) >= self.display_rows().len()
    }

    /// 真实文本的最大滚动偏移（显示行数减视口高度，不含底部留白）。
    pub fn text_max_scroll(&self) -> usize {
        self.display_rows()
            .len()
            .saturating_sub(usize::from(self.height.max(1)))
    }

    /// 最大可滚动偏移：真实文本上限加底部虚拟空占位。
    ///
    /// 与滚动条几何共用同一口径——滚动条 `total = 显示行 + 留白`、可拖距离
    /// `total - 视口高`，这里直接按同一公式计算，因此「滚动条不可见 ⇔ 上限为 0 ⇔
    /// 滚轮/拖拽/轨道点击一律无效」是恒等式，不需要第二套溢出判据。
    fn max_scroll(&self) -> usize {
        (self.display_rows().len() + self.bottom_padding())
            .saturating_sub(usize::from(self.height.max(1)))
    }

    /// 将视口单元格坐标映射为光标位置；超出文本时钳到最近显示行。
    ///
    /// 命中合成附件入口时把文本光标停在该入口对应的父任务末尾，
    /// 入口焦点由鼠标层按命中矩形另行设置。
    pub fn set_cursor_from_cell(&mut self, row: u16, col: u16) {
        self.break_group();
        self.focus = PromptFocus::Text;
        self.cursor =
            self.content_offset_at(self.scroll.saturating_add(usize::from(row)), col, false);
        self.settle();
    }

    /// 选区（内容行坐标，端点包含）对应的原文字节范围；空选区或选中内容为空时返回 None。
    ///
    /// 该范围只表达「连续原文区间」，写入前仍需用 [`Self::edit_hits_hidden`] 判定跨隐藏区。
    pub fn selection_bounds(&self, start: (u16, u16), end: (u16, u16)) -> Option<(usize, usize)> {
        if start == end {
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

    /// 取选区文本；集中视图只提取可见原文，排除隐藏附件与合成入口。
    pub fn selection_text(&self, start: (u16, u16), end: (u16, u16)) -> Option<String> {
        if start == end {
            return None;
        }
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.visible_selection_text(first, last)
    }

    /// 可见选区的确定提取：`start`/`end` 为显示行坐标（含端点）。
    ///
    /// 软换行之间直接连接；跨原始逻辑行保留一个换行；合成入口不产出文字。
    fn visible_selection_text(&self, start: (u16, u16), end: (u16, u16)) -> Option<String> {
        let rows = self.display_rows();
        let first = usize::from(start.0);
        let last = usize::from(end.0).min(rows.len().saturating_sub(1));
        let slice = rows.get(first..=last)?;
        let mut out = String::new();
        let mut previous_line: Option<usize> = None;
        let mut copied = false;
        for (index, row) in slice.iter().enumerate() {
            let absolute = first + index;
            let PromptDisplayRow::Text {
                line,
                start: from,
                end: to,
            } = *row
            else {
                continue;
            };
            let from_offset = if absolute == first {
                (from + self.column_offset(&self.text[from..to], start.1)).min(to)
            } else {
                from
            };
            let to_offset = if absolute == last {
                (from + self.column_offset_inclusive(&self.text[from..to], end.1)).min(to)
            } else {
                to
            };
            if from_offset >= to_offset {
                continue;
            }
            if previous_line.is_some_and(|previous| previous != line) {
                out.push('\n');
            }
            out.push_str(&self.text[from_offset..to_offset]);
            previous_line = Some(line);
            copied = true;
        }
        (copied && !out.is_empty()).then_some(out)
    }

    /// 显示列 → 行内字节偏移（宽字符取整到字符边界）。
    fn column_offset(&self, slice: &str, column: u16) -> usize {
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
    fn column_offset_inclusive(&self, slice: &str, column: u16) -> usize {
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

    /// 光标在显示行中的索引与显示列（跨折行边界时归入下一显示行）。
    pub(super) fn display_cursor_in(&self, rows: &[PromptDisplayRow]) -> (usize, usize) {
        let mut text_index = 0;
        for (index, row) in rows.iter().enumerate() {
            let PromptDisplayRow::Text { start, end, .. } = *row else {
                continue;
            };
            text_index += 1;
            // 光标落在被隐藏区间留下的空隙里：钳到该行行首，避免反向切片。
            if self.cursor < start {
                return (index, 0);
            }
            if self.cursor < end {
                return (index, display_width(&self.text[start..self.cursor]));
            }
            if self.cursor == end {
                let continues = rows
                    .get(index + 1)
                    .is_some_and(|next| matches!(next, PromptDisplayRow::Text { start: next_start, .. } if *next_start == end));
                if !continues {
                    return (index, display_width(&self.text[start..end]));
                }
            }
        }
        let _ = text_index;
        let fallback = rows.len().saturating_sub(1);
        match rows.get(fallback).copied() {
            Some(PromptDisplayRow::Text { start, end, .. }) => {
                (fallback, display_width(&self.text[start..end]))
            }
            _ => (fallback, 0),
        }
    }

    /// 内容行坐标映射到文本字节偏移；`after` 为真时取该单元格字符之后。
    fn content_offset(&self, (row, col): (u16, u16), after: bool) -> usize {
        self.content_offset_at(usize::from(row), col, after)
    }

    /// 显示行索引映射到文本字节偏移；合成入口映射到父任务结束行起点。
    fn content_offset_at(&self, index: usize, col: u16, after: bool) -> usize {
        let rows = self.display_rows();
        let index = index.min(rows.len().saturating_sub(1));
        match rows.get(index).copied() {
            Some(PromptDisplayRow::Text { line, start, end }) => {
                let col = col.saturating_sub(self.line_gutter(line));
                let slice = &self.text[start..end];
                let offset = self.column_offset(slice, col);
                if after && offset < slice.len() {
                    let next = self.text[start + offset..end]
                        .chars()
                        .next()
                        .map_or(0, char::len_utf8);
                    start + offset + next
                } else {
                    start + offset
                }
            }
            Some(PromptDisplayRow::Supplements { parent_start, .. }) => self
                .line_bounds(parent_start)
                .map(|(from, _)| from)
                .unwrap_or(0)
                .min(self.text.len()),
            None => self.text.len(),
        }
    }

    /// 显示行索引对应的附件入口（父任务起始行、数量）。
    pub fn supplement_entry_at_row(&self, index: usize) -> Option<(usize, usize)> {
        match self.display_rows().get(index).copied() {
            Some(PromptDisplayRow::Supplements {
                parent_start,
                count,
            }) => Some((parent_start, count)),
            _ => None,
        }
    }

    /// 块行左边框槽宽度；非块行为 0（渲染、光标与鼠标坐标映射共用）。
    fn line_gutter(&self, line: usize) -> u16 {
        if markdown::block_span_at_line(&self.text, line).is_some() {
            crate::layout::PROMPT_TEMPLATE_GUTTER_LEFT
        } else {
            0
        }
    }

    /// 调整滚动量，保证光标（或附件入口焦点）所在显示行可见；已在底部留白内滚动时不回拉。
    pub(super) fn scroll_cursor_into_view(&mut self) {
        let rows = self.display_rows();
        let height = usize::from(self.height.max(1));
        let row = match self.focus {
            PromptFocus::Supplements { parent_start } => rows
                .iter()
                .position(|row| {
                    matches!(row, PromptDisplayRow::Supplements { parent_start: parent, .. } if *parent == parent_start)
                })
                .unwrap_or(0),
            PromptFocus::Text => self.display_cursor_in(&rows).0,
        };
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// 编辑区间是否与集中视图的隐藏附件相交（写入拒绝判定）。
    pub fn edit_hits_hidden(&self, from: usize, to: usize) -> bool {
        range_hits_hidden(&self.supplement_index(), from, to)
    }

    /// 光标是否落在隐藏附件内部。
    fn cursor_in_hidden(&self) -> Option<(usize, usize)> {
        if self.focus.is_supplements() {
            return None;
        }
        self.supplement_index()
            .entries
            .iter()
            .find(|entry| self.cursor > entry.range.start && self.cursor < entry.range.end)
            .map(|entry| (entry.parent_start, entry.range.start))
    }

    /// 焦点归一：光标落入隐藏附件时转到该父任务的附件入口。
    pub(super) fn normalize_hidden_focus(&mut self) {
        if let Some((parent_start, boundary)) = self.cursor_in_hidden() {
            self.cursor = boundary;
            self.focus = PromptFocus::Supplements { parent_start };
        }
    }
}

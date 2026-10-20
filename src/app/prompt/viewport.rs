//! prompt 视口几何与坐标映射：软换行、滚动、选区范围与单元格/字节换算。

use super::super::markdown;
use super::{Prompt, VisualRow, char_width, display_width};

impl Prompt {
    /// 按内容宽度软换行得到的视觉行；块行扣除左右边框槽后再换行。
    pub fn visual_rows(&self) -> Vec<VisualRow> {
        let width = usize::from(self.width.max(1));
        let margins = usize::from(
            crate::layout::PROMPT_TEMPLATE_GUTTER_LEFT
                + crate::layout::PROMPT_TEMPLATE_GUTTER_RIGHT,
        );
        let infos = markdown::block_line_infos(&self.text);
        let mut rows = Vec::new();
        let mut line_start = 0;
        for (line, segment) in self.text.split('\n').enumerate() {
            let info = infos.get(line).copied().flatten();
            let line_end = line_start + segment.len();
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
    fn max_scroll(&self) -> usize {
        (self.visual_rows().len() + self.bottom_padding())
            .saturating_sub(usize::from(self.height.max(1)))
    }
    /// 将视口单元格坐标映射为光标位置；超出文本时钳到最近行行尾。
    pub fn set_cursor_from_cell(&mut self, row: u16, col: u16) {
        self.break_group();
        self.cursor = self.viewport_offset((row, col), false);
        self.settle();
    }
    /// 选区（内容行坐标，端点包含）对应的字节范围；空选区或选中内容为空时返回 None。
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
    /// 取选区文本（内容行坐标，端点包含）；空选区或选中内容为空时返回 None。
    pub fn selection_text(&self, start: (u16, u16), end: (u16, u16)) -> Option<String> {
        let (from, to) = self.selection_bounds(start, end)?;
        Some(self.text[from..to].to_string())
    }
    /// 光标在视觉行中的索引与显示列（跨折行边界时归入下一视觉行）。
    pub(super) fn cursor_visual_in(&self, rows: &[VisualRow]) -> (usize, usize) {
        for (index, row) in rows.iter().enumerate() {
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

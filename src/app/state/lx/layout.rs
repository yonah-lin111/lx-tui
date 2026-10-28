//! lx 输入框软换行、视口滚动与单元格映射。

use crate::app::prompt::text;

use super::LxRow;

impl super::Lx {
    /// 同步输入框内容宽度（列）与可视高度（内容行）；用于软换行、视觉行移动与滚动。
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width.max(1);
        self.height = usize::from(height.max(1));
        self.scroll_cursor_into_view();
    }

    /// 输入框内容宽度（列）；ask 面板同步手写编辑缓冲的软换行宽度用。
    pub fn input_width(&self) -> u16 {
        self.width
    }

    /// 输入框视口首视觉行索引。
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 滚轮滚动输入框视口（正数向下）；返回视口是否移动。
    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let max = self.max_scroll();
        let next = (self.scroll.min(max) as isize)
            .saturating_add(delta)
            .clamp(0, max as isize) as usize;
        let moved = next != self.scroll;
        self.scroll = next;
        moved
    }

    /// 真实内容的最大滚动偏移。
    fn max_scroll(&self) -> usize {
        self.visual_rows().len().saturating_sub(self.height.max(1))
    }

    /// 调整滚动量，保证光标所在视觉行可见。
    pub(super) fn scroll_cursor_into_view(&mut self) {
        let (row, _) = self.cursor_visual();
        let height = self.height.max(1);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll.saturating_add(height) {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }

    /// 直接设置视口滚动偏移（滚动条点击/拖拽）；越界钳到可滚动范围。
    pub fn scroll_to(&mut self, offset: usize) {
        self.scroll = offset.min(self.max_scroll());
    }

    /// 视口单元格（视觉行、内容列）→ 输入缓冲字节偏移；越界钳到最近位置。
    pub fn offset_at_cell(&self, row: usize, col: u16) -> usize {
        let rows = self.visual_rows();
        let index = row.min(rows.len().saturating_sub(1));
        let visual = rows[index];
        let mut cell = 0;
        for (offset, ch) in self.input[visual.start..visual.end].char_indices() {
            let width = text::char_width(ch);
            if usize::from(col) < cell + width {
                return visual.start + offset;
            }
            cell += width;
        }
        visual.end
    }

    /// 按内容宽度软换行得到的视觉行。
    pub fn visual_rows(&self) -> Vec<LxRow> {
        let width = usize::from(self.width.max(1));
        let mut rows = Vec::new();
        let mut line_start = 0;
        for segment in self.input.split('\n') {
            let line_end = line_start + segment.len();
            let mut row_start = line_start;
            let mut col = 0;
            for (offset, ch) in segment.char_indices() {
                let absolute = line_start + offset;
                let cell = text::char_width(ch);
                if col > 0 && col + cell > width {
                    rows.push(LxRow {
                        start: row_start,
                        end: absolute,
                    });
                    row_start = absolute;
                    col = 0;
                }
                col += cell;
            }
            rows.push(LxRow {
                start: row_start,
                end: line_end,
            });
            line_start = line_end + 1;
        }
        rows
    }

    /// 光标在视觉行中的索引与显示列（跨折行边界归入下一视觉行）。
    pub fn cursor_visual(&self) -> (usize, usize) {
        let rows = self.visual_rows();
        self.cursor_visual_in(&rows)
    }

    pub(super) fn cursor_visual_in(&self, rows: &[LxRow]) -> (usize, usize) {
        for (index, row) in rows.iter().enumerate() {
            if self.cursor < row.end {
                return (
                    index,
                    text::display_width(&self.input[row.start..self.cursor]),
                );
            }
            if self.cursor == row.end {
                let continues = rows
                    .get(index + 1)
                    .is_some_and(|next| next.start == row.end);
                if !continues {
                    return (index, text::display_width(&self.input[row.start..row.end]));
                }
            }
        }
        let index = rows.len().saturating_sub(1);
        let row = rows[index];
        (index, text::display_width(&self.input[row.start..row.end]))
    }
}

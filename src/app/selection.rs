//! 选区模型：prompt 等编辑器的文本选择（内容行、0 基列）。
//!
//! 与渲染和取词一致用内容行坐标：视口滚动不影响选区（锚点天然钉在文本上）。
//! 终端窗格的选区由 `terminal::Terminal` 内部的仿真器持有，本类型只服务编辑器类窗格。

use crate::layout::PaneId;

/// 一次文本选择；`anchor` 为按下点，`cursor` 为当前拖动点，均为内容行坐标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pane: PaneId,
    anchor: (i32, u16),
    cursor: (i32, u16),
    dragging: bool,
}

impl Selection {
    /// 以按下点开始一次选择。
    pub fn begin(pane: PaneId, row: i32, col: u16) -> Self {
        Self {
            pane,
            anchor: (row, col),
            cursor: (row, col),
            dragging: true,
        }
    }

    /// 构造覆盖指定面板全文的选区（起始 (0, 0)，结束 (max_row, max_col)）。
    pub fn full(pane: PaneId, max_row: i32, max_col: u16) -> Self {
        Self {
            pane,
            anchor: (0, 0),
            cursor: (max_row, max_col),
            dragging: false,
        }
    }

    /// 所属窗格。
    pub fn pane(&self) -> PaneId {
        self.pane
    }

    /// 扩展到新的拖动点。
    pub fn drag(&mut self, row: i32, col: u16) {
        self.cursor = (row, col);
    }

    /// 鼠标松开：结束拖动，选区保留。
    pub fn finish(&mut self) {
        self.dragging = false;
    }

    /// 是否仍处于按住拖动状态。
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// 规范化后的包含范围（左上 -> 右下）；未发生拖动时为 None。
    pub fn range(&self) -> Option<((i32, u16), (i32, u16))> {
        if self.anchor == self.cursor {
            return None;
        }
        Some(if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        })
    }

    /// 单元格是否落在选区内（端点包含）。
    pub fn contains(&self, row: i32, col: u16) -> bool {
        let Some(((start_row, start_col), (end_row, end_col))) = self.range() else {
            return false;
        };
        if row < start_row || row > end_row {
            return false;
        }
        if start_row == end_row {
            return col >= start_col && col <= end_col;
        }
        if row == start_row {
            return col >= start_col;
        }
        if row == end_row {
            return col <= end_col;
        }
        true
    }
}

#[cfg(test)]
mod tests;

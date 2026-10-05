//! 选区模型：本标签内单次文本选择，坐标为窗格内容区（0 基行列）。

use crate::layout::PaneId;

/// 一次文本选择；`anchor` 为按下点，`cursor` 为当前拖动点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pane: PaneId,
    anchor: (u16, u16),
    cursor: (u16, u16),
    dragging: bool,
}

impl Selection {
    /// 以按下点开始一次选择。
    pub fn begin(pane: PaneId, row: u16, col: u16) -> Self {
        Self {
            pane,
            anchor: (row, col),
            cursor: (row, col),
            dragging: true,
        }
    }

    /// 所属窗格。
    pub fn pane(&self) -> PaneId {
        self.pane
    }

    /// 扩展到新的拖动点。
    pub fn drag(&mut self, row: u16, col: u16) {
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
    pub fn range(&self) -> Option<((u16, u16), (u16, u16))> {
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
    pub fn contains(&self, row: u16, col: u16) -> bool {
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

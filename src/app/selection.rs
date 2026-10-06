//! 选区模型：prompt 等编辑器的视口文本选择（0 基行列）。
//!
//! 终端窗格的选区由 `terminal::Terminal` 内部的仿真器持有（内容坐标），
//! 本类型只服务编辑器类窗格。

use crate::layout::PaneId;

/// 一次文本选择；`anchor` 为按下点，`cursor` 为当前拖动点。
///
/// 行坐标有符号：视口滚动后锚点按滚动量平移可暂时越过视口边界（负值在视口上方）。
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

    /// 所属窗格。
    pub fn pane(&self) -> PaneId {
        self.pane
    }

    /// 扩展到新的拖动点。
    pub fn drag(&mut self, row: i32, col: u16) {
        self.cursor = (row, col);
    }

    /// 视口滚动后平移两个端点，使选区继续钉在原文本上。
    pub fn shift_rows(&mut self, delta: i32) {
        self.anchor.0 += delta;
        self.cursor.0 += delta;
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

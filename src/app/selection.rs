//! 选区模型：本标签内单次文本选择，坐标为窗格内容区（0 基行列）。

use crate::layout::PaneId;

/// 一次文本选择；`anchor` 为按下点，`cursor` 为当前拖动点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pane: PaneId,
    anchor: (u16, u16),
    cursor: (u16, u16),
}

impl Selection {
    /// 以按下点开始一次选择。
    pub fn begin(pane: PaneId, row: u16, col: u16) -> Self {
        Self {
            pane,
            anchor: (row, col),
            cursor: (row, col),
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
mod tests {
    use super::*;

    fn selection() -> Selection {
        let mut selection = Selection::begin(PaneId::from_raw_for_test(1), 1, 2);
        selection.drag(3, 4);
        selection
    }

    #[test]
    fn range_normalizes_drag_direction() {
        let forward = selection();
        assert_eq!(forward.range(), Some(((1, 2), (3, 4))));
        let mut backward = Selection::begin(PaneId::from_raw_for_test(1), 3, 4);
        backward.drag(1, 2);
        assert_eq!(backward.range(), Some(((1, 2), (3, 4))));
    }

    #[test]
    fn click_without_drag_has_no_range() {
        let selection = Selection::begin(PaneId::from_raw_for_test(1), 1, 2);
        assert_eq!(selection.range(), None);
        assert!(!selection.contains(1, 2));
    }

    #[test]
    fn contains_marks_reading_order_cells() {
        let selection = selection();
        assert!(selection.contains(1, 2));
        assert!(selection.contains(1, 9));
        assert!(selection.contains(2, 0));
        assert!(selection.contains(3, 4));
        assert!(selection.contains(3, 0));
        assert!(!selection.contains(1, 1));
        assert!(!selection.contains(3, 5));
        assert!(!selection.contains(0, 2));
        assert!(!selection.contains(4, 2));
    }

    #[test]
    fn single_line_range_limits_columns() {
        let mut selection = Selection::begin(PaneId::from_raw_for_test(1), 2, 1);
        selection.drag(2, 5);
        assert!(selection.contains(2, 1));
        assert!(selection.contains(2, 5));
        assert!(!selection.contains(2, 0));
        assert!(!selection.contains(2, 6));
        assert!(!selection.contains(3, 2));
    }
}

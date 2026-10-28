//! 选区模型：prompt 等编辑器的文本选择（内容行、0 基列）。
//!
//! 与渲染和取词一致用内容行坐标：视口滚动不影响选区（锚点天然钉在文本上）。
//! 终端窗格的选区由 `terminal::Terminal` 内部的仿真器持有，本类型只服务编辑器类窗格。

use std::time::Instant;

use crate::layout::PaneId;

/// 选区来源；复制反馈锚点与剪切资格按它分派。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionSource {
    Screen,
    Prompt,
    Terminal(PaneId),
    Lx(PaneId),
    LxInput(PaneId),
    Ask(PaneId),
    Todo,
    OverlayText,
    PageRename,
}

/// 最终画面坐标（0 基，绝对行列）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScreenPoint {
    pub row: u16,
    pub col: u16,
}

impl ScreenPoint {
    pub fn new(row: u16, col: u16) -> Self {
        Self { row, col }
    }
}

/// 选择单位：单击按字符、双击按 Unicode 词、三击按屏幕视觉行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionUnit {
    Character,
    Word,
    Line,
}

/// 冻结的一格文字；宽字符 `width` 为 2，续格不被记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedCell {
    pub symbol: String,
    pub width: u8,
}

impl SelectedCell {
    /// 该格是否是一个宽字符的首格。
    pub fn is_wide_lead(&self) -> bool {
        self.width > 1
    }
}

/// 冻结的一行：完整视觉行文本（用于滚动/缩放后按内容重定位）与选中片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrozenRow {
    /// 该行完整可见文本（宽字符只保留首格）。
    pub full: String,
    /// 选中起点列（相对整行）。
    pub start_col: u16,
    /// 选中终点列（含端点，相对整行）。
    pub end_col: u16,
    /// 选中片段，按屏幕列顺序；宽字符只保留首格。
    pub cells: Vec<SelectedCell>,
}

/// 选择时冻结的画面片段；复制只读它，不受后续动画/toast 改写。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SelectedCells {
    pub rows: Vec<FrozenRow>,
    pub start: Option<ScreenPoint>,
    pub end: Option<ScreenPoint>,
}

impl SelectedCells {
    /// 按屏幕视觉行拼接纯文本：行间插入 `\n`，保留选中范围内的边框、空格与软折行。
    pub fn text(&self) -> String {
        let mut lines = Vec::with_capacity(self.rows.len());
        for row in &self.rows {
            let mut line = String::new();
            for cell in &row.cells {
                if cell.width > 0 {
                    line.push_str(&cell.symbol);
                }
            }
            lines.push(line);
        }
        lines.join("\n")
    }

    /// 是否为空选区。
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// 普通屏幕选择：端点使用绝对屏幕坐标；拖拽单位在开始/扩展时已展开。
#[derive(Debug, Clone)]
pub struct ScreenSelection {
    /// 本次拖动的原始按下点（用于按词/行扩展，不随扩展改变）。
    base: ScreenPoint,
    anchor: ScreenPoint,
    cursor: ScreenPoint,
    unit: SelectionUnit,
    dragging: bool,
    /// 当前画面是否还能找到冻结内容；找不到时不画高亮，但保留选区。
    visible: bool,
    frozen: SelectedCells,
}

impl ScreenSelection {
    pub fn new(anchor: ScreenPoint, unit: SelectionUnit, frozen: SelectedCells) -> Self {
        Self {
            base: anchor,
            anchor,
            cursor: anchor,
            unit,
            dragging: true,
            visible: true,
            frozen,
        }
    }

    pub fn base(&self) -> ScreenPoint {
        self.base
    }

    pub fn anchor(&self) -> ScreenPoint {
        self.anchor
    }

    pub fn cursor(&self) -> ScreenPoint {
        self.cursor
    }

    pub fn unit(&self) -> SelectionUnit {
        self.unit
    }

    pub fn set_unit(&mut self, unit: SelectionUnit) {
        self.unit = unit;
    }

    pub fn set_range(&mut self, anchor: ScreenPoint, cursor: ScreenPoint) {
        self.anchor = anchor;
        self.cursor = cursor;
        self.dragging = true;
    }

    /// 按新画面重新定位（滚动/缩放后）；保留冻结 payload。
    pub fn remap(&mut self, anchor: ScreenPoint, cursor: ScreenPoint) {
        self.base = anchor;
        self.anchor = anchor;
        self.cursor = cursor;
        self.dragging = false;
        self.visible = true;
    }

    pub fn frozen(&self) -> &SelectedCells {
        &self.frozen
    }

    pub fn set_frozen(&mut self, frozen: SelectedCells) {
        self.frozen = frozen;
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn finish(&mut self) {
        self.dragging = false;
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// 规范化后的屏幕范围 `(start, end)`；无范围返回 None。
    ///
    /// Word/Line 单位可能只覆盖一个非空白格，此时 anchor 与 cursor 相同，
    /// 但冻结内容非空，仍应保留为有效选区；Character 单击的冻结内容为空，
    /// 仍按无选区处理。
    pub fn range(&self) -> Option<(ScreenPoint, ScreenPoint)> {
        if self.anchor == self.cursor && self.frozen.is_empty() {
            return None;
        }
        Some(if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        })
    }

    /// 屏幕格是否落在选区内（端点包含，按阅读顺序）。
    pub fn contains(&self, point: ScreenPoint) -> bool {
        let Some((start, end)) = self.range() else {
            return false;
        };
        if point < start || point > end {
            return false;
        }
        true
    }
}

/// 同格多击判定窗口；折叠卡片延迟也复用它，保证双击/三击先于折叠生效。
pub(crate) const MULTI_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(250);

/// 点击计数现场：同一格、250 ms 内 1→2→3→1。
#[derive(Debug, Clone, Copy)]
pub struct ScreenClick {
    pub point: ScreenPoint,
    pub at: Instant,
    pub count: u8,
}

impl ScreenClick {
    /// 记录一次点击并返回本次选择单位。
    pub fn next(&mut self, point: ScreenPoint, now: Instant) -> SelectionUnit {
        if self.point == point && now.saturating_duration_since(self.at) <= MULTI_CLICK_WINDOW {
            self.count = if self.count >= 3 { 1 } else { self.count + 1 };
        } else {
            self.count = 1;
        }
        self.point = point;
        self.at = now;
        match self.count {
            2 => SelectionUnit::Word,
            3 => SelectionUnit::Line,
            _ => SelectionUnit::Character,
        }
    }
}

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

    /// 以固定端点构造一次非拖拽选区（键盘选择）。
    pub fn text_range(pane: PaneId, anchor: (i32, u16), cursor: (i32, u16)) -> Self {
        Self {
            pane,
            anchor,
            cursor,
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

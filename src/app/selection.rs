//! 选区模型：prompt 等编辑器的文本选择（内容行、0 基列）与多击选词/选行粒度。
//!
//! 与渲染和取词一致用内容行坐标：视口滚动不影响选区（锚点天然钉在文本上）。
//! 终端窗格的选区由 `terminal::Terminal` 内部的仿真器持有，本类型只服务编辑器类窗格。

use std::time::{Duration, Instant};

use crate::layout::PaneId;

/// 文本选区多击判定时间窗口（400ms，对齐 codex）。
pub const SELECTION_CLICK_WINDOW: Duration = Duration::from_millis(400);

/// 多击选区粒度：单击逐字、双击按词、三击按行（对齐 codex `SelectionUnit`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionUnit {
    #[default]
    Character,
    Word,
    Line,
}

/// 选词字符分类：连续同类字符构成双击选词边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Word,
    Cjk,
    Whitespace,
    Punct,
}

/// 判定单个字符的选词分类。
pub fn char_class(ch: char) -> CharClass {
    if ch.is_whitespace() {
        CharClass::Whitespace
    } else if is_cjk(ch) {
        CharClass::Cjk
    } else if ch.is_alphanumeric() || ch == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
            | 0x20000..=0x2FA1F
    )
}

impl SelectionUnit {
    /// 按连续点击次数（1/2/3）映射选区粒度。
    pub fn from_clicks(clicks: u8) -> Self {
        match clicks {
            2 => Self::Word,
            3 => Self::Line,
            _ => Self::Character,
        }
    }

    /// 在字符串 `text` 中围绕字节偏移 `offset` 计算当前粒度对应的字节范围 `(start, end)`。
    pub fn byte_range(self, text: &str, offset: usize) -> (usize, usize) {
        let offset = text.floor_char_boundary(offset.min(text.len()));
        match self {
            Self::Character => (offset, offset),
            Self::Line => {
                let start = text[..offset].rfind('\n').map_or(0, |idx| idx + 1);
                let end = text[offset..]
                    .find('\n')
                    .map_or(text.len(), |idx| offset + idx);
                (start, end)
            }
            Self::Word => {
                let line_start = text[..offset].rfind('\n').map_or(0, |idx| idx + 1);
                let line_end = text[offset..]
                    .find('\n')
                    .map_or(text.len(), |idx| offset + idx);
                let line = &text[line_start..line_end];
                if line.is_empty() {
                    return (line_start, line_start);
                }
                let mut rel = offset.saturating_sub(line_start).min(line.len());
                if rel == line.len()
                    && let Some((prev_idx, _)) = line.char_indices().next_back()
                {
                    rel = prev_idx;
                }
                let Some(ch) = line[rel..].chars().next() else {
                    return (offset, offset);
                };
                let class = char_class(ch);
                let mut start = rel;
                for (idx, prev) in line[..rel].char_indices().rev() {
                    if char_class(prev) != class {
                        break;
                    }
                    start = idx;
                }
                let mut end = rel + ch.len_utf8();
                for (idx, next) in line[end..].char_indices() {
                    if char_class(next) != class {
                        break;
                    }
                    end = rel + ch.len_utf8() + idx + next.len_utf8();
                }
                (line_start + start, line_start + end)
            }
        }
    }
}

/// 计算同一屏幕坐标连续左键按下的点击次数（1 → 2 → 3 → 1 循环，对齐 codex `click_count`）。
pub fn next_click_count(
    last_click: &mut Option<(Instant, u16, u16, u8)>,
    column: u16,
    row: u16,
    now: Instant,
) -> u8 {
    let clicks = last_click
        .filter(|(at, x, y, _)| {
            now.saturating_duration_since(*at) < SELECTION_CLICK_WINDOW && *x == column && *y == row
        })
        .map_or(1, |(_, _, _, clicks)| clicks % 3 + 1);
    *last_click = Some((now, column, row, clicks));
    clicks
}

/// 一次文本选择；`anchor` 为按下点，`cursor` 为当前拖动点，均为内容行坐标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pane: PaneId,
    origin: ((i32, u16), (i32, u16)),
    anchor: (i32, u16),
    cursor: (i32, u16),
    unit: SelectionUnit,
    dragging: bool,
}

impl Selection {
    /// 以按下点开始一次选择。
    pub fn begin(pane: PaneId, row: i32, col: u16) -> Self {
        Self {
            pane,
            origin: ((row, col), (row, col)),
            anchor: (row, col),
            cursor: (row, col),
            unit: SelectionUnit::Character,
            dragging: true,
        }
    }

    /// 以初始区间（双击选词或三击选行）开始一次选择；后续拖拽以该区间为原点按粒度扩展。
    pub fn begin_range(
        pane: PaneId,
        start: (i32, u16),
        end: (i32, u16),
        unit: SelectionUnit,
    ) -> Self {
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        Self {
            pane,
            origin: (first, last),
            anchor: first,
            cursor: last,
            unit,
            dragging: true,
        }
    }

    /// 构造覆盖指定面板全文的选区（起始 (0, 0)，结束 (max_row, max_col)）。
    pub fn full(pane: PaneId, max_row: i32, max_col: u16) -> Self {
        Self {
            pane,
            origin: ((0, 0), (max_row, max_col)),
            anchor: (0, 0),
            cursor: (max_row, max_col),
            unit: SelectionUnit::Character,
            dragging: false,
        }
    }

    /// 以固定端点构造一次非拖拽选区（键盘选择）。
    pub fn text_range(pane: PaneId, anchor: (i32, u16), cursor: (i32, u16)) -> Self {
        Self {
            pane,
            origin: (anchor, cursor),
            anchor,
            cursor,
            unit: SelectionUnit::Character,
            dragging: false,
        }
    }

    /// 所属窗格。
    pub fn pane(&self) -> PaneId {
        self.pane
    }

    /// 选区粒度（单字 / 词 / 行）。
    pub fn unit(&self) -> SelectionUnit {
        self.unit
    }

    /// 初始选中区间（双击/三击按下时的锚点词或行）。
    pub fn origin(&self) -> ((i32, u16), (i32, u16)) {
        self.origin
    }

    /// 扩展到新的拖动点。
    pub fn drag(&mut self, row: i32, col: u16) {
        if self.unit == SelectionUnit::Character {
            self.cursor = (row, col);
        } else {
            self.drag_range((row, col), (row, col));
        }
    }

    /// 按粒度区间扩展选区：双向保留初始 `origin` 词/行并延伸到目标词/行。
    pub fn drag_range(&mut self, start: (i32, u16), end: (i32, u16)) {
        if self.unit == SelectionUnit::Character {
            self.cursor = end;
            return;
        }
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        self.anchor = self.origin.0.min(first);
        self.cursor = self.origin.1.max(last);
    }

    /// 鼠标松开：结束拖动，选区保留。
    pub fn finish(&mut self) {
        self.dragging = false;
    }

    /// 是否仍处于按住拖动状态。
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// 规范化后的包含范围（左上 -> 右下）；单击未发生拖动时为 None。
    pub fn range(&self) -> Option<((i32, u16), (i32, u16))> {
        if self.unit == SelectionUnit::Character && self.anchor == self.cursor {
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

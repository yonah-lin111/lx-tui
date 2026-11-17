//! 紧凑切换器的状态与导航：行快照、高亮、视口与悬停。

use crate::app::state::CompactRow;

/// 紧凑切换器状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactSwitcher {
    pub rows: Vec<CompactRow>,
    /// 当前高亮行下标；总是可选行（分区标题不进入）。
    pub selected: usize,
    /// 视口顶部行号。
    pub scroll: usize,
    /// 鼠标悬停行；只铺底色，不移动高亮。
    pub hover: Option<usize>,
    /// 可视行数；终端 resize 时由几何同步。
    pub viewport_rows: usize,
}

impl CompactSwitcher {
    /// 用行快照构造；高亮落到第一个可选行。
    pub fn new(rows: Vec<CompactRow>, viewport_rows: usize) -> Self {
        let viewport_rows = viewport_rows.max(1);
        let selected = rows
            .iter()
            .position(CompactRow::is_selectable)
            .unwrap_or_default();
        let mut switcher = Self {
            rows,
            selected,
            scroll: 0,
            hover: None,
            viewport_rows,
        };
        switcher.ensure_visible();
        switcher
    }

    /// 同步可视行数并夹取滚动位置。
    pub fn set_viewport(&mut self, viewport_rows: usize) {
        self.viewport_rows = viewport_rows.max(1);
        self.scroll = self.scroll.min(self.max_scroll());
        self.ensure_visible();
    }

    /// 视口最大顶部行号。
    pub fn max_scroll(&self) -> usize {
        self.rows.len().saturating_sub(self.viewport_rows)
    }

    /// 当前高亮行；越界或非可选时返回 None。
    pub fn selected_row(&self) -> Option<&CompactRow> {
        self.rows
            .get(self.selected)
            .filter(|row| row.is_selectable())
    }

    /// 按可选行移动高亮；`delta` 为跨过的可选行数。到头即停，不循环。
    pub fn move_selection(&mut self, delta: isize) {
        if delta == 0 || self.rows.is_empty() {
            return;
        }
        let step: isize = if delta > 0 { 1 } else { -1 };
        let mut remaining = delta.unsigned_abs();
        let mut index = self.selected;
        while remaining > 0 {
            let mut next = index;
            loop {
                let candidate = next.checked_add_signed(step);
                let Some(candidate) = candidate else {
                    break;
                };
                if candidate >= self.rows.len() {
                    break;
                }
                next = candidate;
                if self.rows[next].is_selectable() {
                    break;
                }
            }
            if next == index {
                break;
            }
            index = next;
            remaining -= 1;
        }
        self.selected = index;
        self.ensure_visible();
    }

    /// 翻页：按可视行数移动。
    pub fn page(&mut self, direction: isize) {
        let step = self.viewport_rows as isize;
        self.move_selection(direction.saturating_mul(step));
    }

    /// 选中指定行；分区标题忽略。
    pub fn select(&mut self, index: usize) {
        if self.rows.get(index).is_some_and(CompactRow::is_selectable) {
            self.selected = index;
            self.ensure_visible();
        }
    }

    /// 把高亮行拉回视口内。
    pub fn ensure_visible(&mut self) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.viewport_rows > 0
            && self.selected >= self.scroll.saturating_add(self.viewport_rows)
        {
            self.scroll = self
                .selected
                .saturating_add(1)
                .saturating_sub(self.viewport_rows);
        }
        self.scroll = self.scroll.min(self.max_scroll());
    }
}

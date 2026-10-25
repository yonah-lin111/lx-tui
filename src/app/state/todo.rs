//! 全局 todo 面板状态：列表、选中、视口、编辑现场与删除撤销槽。
//!
//! 纯数据、无 IO；列表恒按 `position` 升序且 `position == 下标`，
//! 因此「行下标」可以直接当「position」用，渲染与命中无需换算。

use crate::app::overlay::TextInput;
use crate::db::todo::{TodoRecord, TodoStatus};

/// 编辑目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoEditTarget {
    /// 末尾追加。
    New,
    /// 当前项之后插入。
    After(usize),
    /// 编辑当前项本身。
    Current(usize),
}

/// 编辑现场：`target` 为 None 表示新建（提交时才分配 id 与 position）。
#[derive(Debug)]
pub struct TodoEdit {
    /// 编辑的既有条目 id；None 表示新建。
    pub target: Option<String>,
    pub input: TextInput,
    /// 新建时的插入位置（提交时用）。
    pub at: usize,
}

impl TodoEdit {
    /// 新建一条待编辑记录。
    pub fn new(target: Option<String>, text: impl Into<String>, at: usize) -> Self {
        Self {
            target,
            input: TextInput::new(text),
            at,
        }
    }
}

/// todo 面板状态。
#[derive(Debug, Default)]
pub struct TodoPanel {
    /// 全量列表，始终按 `position` 升序且 `position == 下标`（不变式）。
    items: Vec<TodoRecord>,
    /// 选中行下标；列表为空时为 None。
    selected: Option<usize>,
    /// 视口首行下标；0 表示贴顶。
    scroll: usize,
    /// 编辑现场。
    editing: Option<TodoEdit>,
    /// 删除撤销槽：一次删除（单条或一批）的记录快照。
    undo: Option<Vec<TodoRecord>>,
    /// 等待第二个 `g` 的标记（`gg` 跳首）。
    pending_g: bool,
    /// 最近一次渲染的可见行数；0 表示尚未渲染（此时不钳制视口）。
    ///
    /// 几何只有一个来源（`ui::right_panel::todo::visible_rows`），由事件层每帧
    /// 通过 [`TodoPanel::set_viewport_rows`] 回写；`app` 不自己算布局。
    rows: usize,
}

impl TodoPanel {
    /// 空列表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 全部条目（按显示顺序）。
    pub fn items(&self) -> &[TodoRecord] {
        &self.items
    }

    /// 条目数量。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 选中行下标。
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// 视口首行下标。
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 最近一帧的可见行数（0 表示尚未渲染）。
    pub fn viewport_rows(&self) -> usize {
        self.rows
    }

    /// 回写最近一帧的可见行数；顺带钳制视口。返回是否发生变化。
    pub fn set_viewport_rows(&mut self, rows: usize) -> bool {
        if self.rows == rows {
            return false;
        }
        self.rows = rows;
        let max = self.items.len().saturating_sub(rows);
        if self.scroll > max {
            self.scroll = max;
        }
        true
    }

    /// 编辑现场。
    pub fn editing(&self) -> Option<&TodoEdit> {
        self.editing.as_ref()
    }

    /// 是否正在编辑。
    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// 撤销槽是否有内容。
    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }

    /// 完成项数量（工具栏计数用）。
    pub fn done_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == TodoStatus::Done)
            .count()
    }

    /// 非完成项数量。
    pub fn pending_count(&self) -> usize {
        self.items.len().saturating_sub(self.done_count())
    }

    /// 选中条目。
    pub fn selected_item(&self) -> Option<&TodoRecord> {
        self.selected.and_then(|index| self.items.get(index))
    }

    /// 整体替换列表（启动加载）：重置选中、视口、编辑与撤销槽。
    pub fn replace(&mut self, items: Vec<TodoRecord>) {
        self.items = items;
        self.normalize_positions();
        self.selected = (!self.items.is_empty()).then_some(0);
        self.scroll = 0;
        self.editing = None;
        self.undo = None;
        self.pending_g = false;
    }

    /// 内存侧位置归一：`position == 下标`。
    pub fn normalize_positions(&mut self) {
        for (index, item) in self.items.iter_mut().enumerate() {
            item.position = index as i64;
        }
    }

    /// 钳制选中到合法范围并让选中可见；列表为空时清空选中。
    pub fn clamp_selection(&mut self) {
        if self.items.is_empty() {
            self.selected = None;
            self.scroll = 0;
        } else if let Some(index) = self.selected {
            self.selected = Some(index.min(self.items.len() - 1));
        } else {
            self.selected = Some(0);
        }
        self.scroll_into_view(self.rows);
    }

    /// 移动选中；`delta` 可传 `isize::MIN/MAX` 做跳首尾（内部饱和）。
    pub fn move_selection(&mut self, delta: isize) -> bool {
        let Some(current) = self.selected else {
            return false;
        };
        let len = self.items.len();
        let next = if delta == isize::MIN {
            0
        } else if delta == isize::MAX {
            len - 1
        } else {
            current.saturating_add_signed(delta).min(len - 1)
        };
        if next == current {
            return false;
        }
        self.selected = Some(next);
        self.scroll_into_view(self.rows);
        true
    }

    /// 选中指定行。
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.items.len() || self.selected == Some(index) {
            return false;
        }
        self.selected = Some(index);
        self.scroll_into_view(self.rows);
        true
    }

    /// 视口滚到目标偏移（钳制到 `0..=max`）。
    pub fn set_scroll(&mut self, offset: usize) -> bool {
        let max = self.items.len().saturating_sub(self.rows);
        let next = offset.min(max);
        if next == self.scroll {
            return false;
        }
        self.scroll = next;
        true
    }

    /// 按滚轮步数滚视口；不移动选中。可见行数取最近一帧的回写值。
    pub fn scroll_by(&mut self, direction: isize) -> bool {
        let max = self.items.len().saturating_sub(self.rows);
        let next = if direction < 0 {
            self.scroll.saturating_sub(direction.unsigned_abs())
        } else {
            self.scroll
                .saturating_add(direction.unsigned_abs())
                .min(max)
        };
        if next == self.scroll {
            return false;
        }
        self.scroll = next;
        true
    }

    /// 保证选中行在视口内（键盘移动后调用）。
    pub fn scroll_into_view(&mut self, rows: usize) -> bool {
        let Some(selected) = self.selected else {
            return false;
        };
        if rows == 0 {
            return false;
        }
        let max = self.items.len().saturating_sub(rows);
        let next = if selected < self.scroll {
            selected
        } else if selected >= self.scroll.saturating_add(rows) {
            selected.saturating_add(1).saturating_sub(rows)
        } else {
            self.scroll
        };
        let next = next.min(max);
        if next == self.scroll {
            return false;
        }
        self.scroll = next;
        true
    }

    /// 进入编辑现场。
    pub fn begin_edit(&mut self, edit: TodoEdit) {
        self.editing = Some(edit);
        self.pending_g = false;
    }

    /// 结束编辑并返回现场。
    pub fn take_edit(&mut self) -> Option<TodoEdit> {
        self.editing.take()
    }

    /// 当前编辑输入（若有）。
    pub fn edit_input_mut(&mut self) -> Option<&mut TextInput> {
        self.editing.as_mut().map(|edit| &mut edit.input)
    }

    /// 在 `at` 处插入一条记录并归一位置；返回新下标。
    pub fn insert(&mut self, at: usize, mut record: TodoRecord) -> usize {
        let at = at.min(self.items.len());
        record.position = at as i64;
        self.items.insert(at, record);
        self.normalize_positions();
        at
    }

    /// 移除一条并归一位置；返回被移除的记录。
    pub fn remove(&mut self, index: usize) -> Option<TodoRecord> {
        if index >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(index);
        self.normalize_positions();
        Some(removed)
    }

    /// 就地修改一条的文本与状态。
    pub fn update(&mut self, index: usize, text: String, status: TodoStatus) -> Option<TodoRecord> {
        let item = self.items.get_mut(index)?;
        item.text = text;
        item.status = status;
        Some(item.clone())
    }

    /// 就地改状态；返回改动后的记录（无变化为 None）。
    pub fn cycle_status(&mut self, index: usize) -> Option<TodoRecord> {
        let item = self.items.get_mut(index)?;
        item.status = item.status.cycle();
        Some(item.clone())
    }

    /// 取走全部指定状态的条目（`[clear done]`）；返回被删记录。
    pub fn take_with_status(&mut self, status: TodoStatus) -> Vec<TodoRecord> {
        let (removed, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|item| item.status == status);
        self.items = kept;
        self.normalize_positions();
        removed
    }

    /// 按原位置把一批记录插回（撤销）。
    ///
    /// 已在列表里的 id 直接跳过：撤销槽是一次性的，但重复恢复（同一批被调用两次）
    /// 不该在列表里造出重复条目——列表不变式比「照单全收」重要。
    pub fn restore(&mut self, records: Vec<TodoRecord>) {
        let mut records = records;
        records.sort_by_key(|record| record.position);
        for record in records {
            if self.items.iter().any(|item| item.id == record.id) {
                continue;
            }
            let at = usize::try_from(record.position.max(0))
                .unwrap_or(usize::MAX)
                .min(self.items.len());
            self.items.insert(at, record);
        }
        self.normalize_positions();
    }

    /// 写入撤销槽（覆盖上一条）。
    pub fn set_undo(&mut self, records: Vec<TodoRecord>) {
        self.undo = (!records.is_empty()).then_some(records);
    }

    /// 取走撤销槽内容（撤销一次后即失效）。
    pub fn take_undo(&mut self) -> Option<Vec<TodoRecord>> {
        self.undo.take()
    }

    /// `gg` 的挂起标记。
    pub fn pending_g(&self) -> bool {
        self.pending_g
    }

    /// 设置 `gg` 的挂起标记；返回是否发生变化。
    pub fn set_pending_g(&mut self, pending: bool) -> bool {
        if self.pending_g == pending {
            return false;
        }
        self.pending_g = pending;
        true
    }
}

#[cfg(test)]
mod tests;

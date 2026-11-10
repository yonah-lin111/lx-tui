//! 全局 todo 日程面板状态：按日切片列表、选中、视口、编辑现场、聚合组、跨日顺延、多图表与删除撤销槽。
//!
//! 纯数据、无 IO；当前选中日期的列表恒按 `position` 升序且 `position == 下标`，
//! 因此「行下标」可以直接当「position」用，渲染与命中无需换算。

pub mod group;
pub mod schedule;

use std::collections::BTreeMap;

pub use group::TodoGroupRole;
pub use schedule::{TodoChartKind, TodoStats, WeekDayView};

use crate::app::overlay::TextInput;
use crate::db::todo::{TodoRecord, TodoStatus, today_date};

use group::normalize_slice_groups_and_positions;

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
    /// 多行编辑视口的首行偏移（由 `resolve_todo_viewport` 随光标滑动更新）。
    pub scroll_row: usize,
    /// 新建幽灵行归属的聚合组 ID（`o` / `Shift+Enter` 在组内触发时继承该组 ID；`a` / `[+]` 为 None）。
    pub group_id: Option<String>,
    /// 新建幽灵行的初始状态（组内新建时继承该组当前状态；普通新建为 `TodoStatus::Todo`）。
    pub initial_status: TodoStatus,
}

impl TodoEdit {
    /// 新建一条待编辑记录。
    pub fn new(target: Option<String>, text: impl Into<String>, at: usize) -> Self {
        Self {
            target,
            input: TextInput::new(text),
            at,
            scroll_row: 0,
            group_id: None,
            initial_status: TodoStatus::Todo,
        }
    }

    /// 设定新建幽灵行的聚合组归属与初始状态。
    pub fn with_group(mut self, group_id: Option<String>, initial_status: TodoStatus) -> Self {
        self.group_id = group_id;
        self.initial_status = initial_status;
        self
    }
}

/// todo 面板状态。
#[derive(Debug)]
pub struct TodoPanel {
    /// 当前选中查看的日期（`YYYY-MM-DD`）。
    selected_date: String,
    /// 今日日期基准（`YYYY-MM-DD`）。
    today_date: String,
    /// 当前选中日期的条目列表，始终按 `position` 升序且 `position == 下标`（不变式）。
    items: Vec<TodoRecord>,
    /// 其它日期的条目归档（按日期键索引，各日内部保持 `position == 下标`）。
    other_dates: BTreeMap<String, Vec<TodoRecord>>,
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
    rows: usize,
    /// 最近一帧同步的各显示条目视觉行高（`1..=3`）；空切片时退回按单行估算。
    item_rows: Vec<u16>,
    /// 最近一帧同步的编辑框首行可用列宽（软折行导航与渲染同源）。
    edit_text_width: usize,
    /// 最近一帧同步的编辑框续行可用列宽（`P(n)` 不缩进，比首行宽 3 列）。
    edit_cont_width: usize,
    /// 是否自动将选中/编辑项滚入视口（键盘移动/选中/编辑时置 true，手动滚轮/滚动条置 false）。
    follow_selected: bool,
    /// 当前激活的统计图表类型。
    active_chart: TodoChartKind,
    /// 是否已在当前会话中忽略历史未完成顺延提醒。
    rollover_dismissed: bool,
}

impl Default for TodoPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl TodoPanel {
    /// 空列表，默认定位到今天。
    pub fn new() -> Self {
        let today = today_date();
        Self {
            selected_date: today.clone(),
            today_date: today,
            items: Vec::new(),
            other_dates: BTreeMap::new(),
            selected: None,
            scroll: 0,
            editing: None,
            undo: None,
            pending_g: false,
            rows: 0,
            item_rows: Vec::new(),
            edit_text_width: 0,
            edit_cont_width: 0,
            follow_selected: true,
            active_chart: TodoChartKind::StatusAndRate,
            rollover_dismissed: false,
        }
    }

    /// 当前选中日期的全部条目（按显示顺序）。
    pub fn items(&self) -> &[TodoRecord] {
        &self.items
    }

    /// 当前选中日期的条目数量。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 当前显示序列总行数（含新建幽灵槽位）。
    pub fn display_len(&self) -> usize {
        let ghost = self
            .editing
            .as_ref()
            .map_or(0, |edit| usize::from(edit.target.is_none()));
        self.items.len() + ghost
    }

    /// 当前选中日期是否为空。
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
        let max = self.max_scroll();
        if self.scroll > max {
            self.scroll = max;
        }
        true
    }

    /// 同步当前视口的物理行数、各显示条目的视觉行高（`1..=3`）、多行编辑视口起点
    /// 与编辑框可用列宽，并按需钳制 `scroll` 或跟随焦点项。
    pub fn sync_viewport(
        &mut self,
        rows: usize,
        item_rows: Vec<u16>,
        edit_scroll_row: Option<usize>,
        edit_text_width: usize,
    ) -> bool {
        self.sync_viewport_indented(
            rows,
            item_rows,
            edit_scroll_row,
            edit_text_width,
            edit_text_width,
        )
    }

    /// 同步当前视口的物理行数、各显示条目的视觉行高（`1..=3`）、多行编辑视口起点、
    /// 编辑框首行与续行可用列宽，并按需钳制 `scroll` 或跟随焦点项。
    pub fn sync_viewport_indented(
        &mut self,
        rows: usize,
        item_rows: Vec<u16>,
        edit_scroll_row: Option<usize>,
        edit_text_width: usize,
        edit_cont_width: usize,
    ) -> bool {
        let mut changed = self.rows != rows
            || self.item_rows != item_rows
            || self.edit_text_width != edit_text_width
            || self.edit_cont_width != edit_cont_width;
        self.rows = rows;
        self.item_rows = item_rows;
        self.edit_text_width = edit_text_width;
        self.edit_cont_width = edit_cont_width;
        if let (Some(edit), Some(scroll_row)) = (self.editing.as_mut(), edit_scroll_row)
            && edit.scroll_row != scroll_row
        {
            edit.scroll_row = scroll_row;
            changed = true;
        }
        let max = self.max_scroll();
        if self.scroll > max {
            self.scroll = max;
            changed = true;
        }
        if self.follow_selected {
            if self.is_editing() {
                changed |= self.scroll_to_edit();
            } else {
                changed |= self.scroll_into_view(self.rows);
            }
        }
        changed
    }

    /// 单个显示槽位的视觉行高（未同步时默认 1 行）。
    fn item_height(&self, display: usize) -> usize {
        if self.item_rows.len() == self.display_len() {
            return usize::from(self.item_rows[display]).max(1);
        }
        if let Some(edit) = self.editing.as_ref().filter(|e| e.target.is_none())
            && self.item_rows.len() + 1 == self.display_len()
        {
            if display == edit.at {
                return 1;
            }
            let idx = if display < edit.at {
                display
            } else {
                display - 1
            };
            return usize::from(self.item_rows[idx]).max(1);
        }
        self.item_rows
            .get(display)
            .copied()
            .map_or(1, |h| usize::from(h).max(1))
    }

    /// 当前视口下能完整显示末尾条目的最大起始条目下标。
    pub fn max_scroll(&self) -> usize {
        self.max_scroll_for_rows(self.rows)
    }

    fn max_scroll_for_rows(&self, rows: usize) -> usize {
        let total = self.display_len();
        if total == 0 {
            return 0;
        }
        if rows == 0 {
            return total.saturating_sub(1);
        }
        let mut used = 0usize;
        for index in (0..total).rev() {
            let height = self.item_height(index);
            if used + height > rows {
                return if index == total - 1 {
                    total - 1
                } else {
                    index + 1
                };
            }
            used += height;
        }
        0
    }

    /// 编辑现场。
    pub fn editing(&self) -> Option<&TodoEdit> {
        self.editing.as_ref()
    }

    /// 最近一帧同步的编辑框首行可用列宽（软折行导航用）。
    pub fn edit_text_width(&self) -> usize {
        self.edit_text_width
    }

    /// 最近一帧同步的编辑框续行可用列宽（软折行导航用）。
    pub fn edit_cont_width(&self) -> usize {
        self.edit_cont_width.max(self.edit_text_width)
    }

    /// 是否正在编辑。
    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    /// 撤销槽是否有内容。
    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }

    /// 选中条目。
    pub fn selected_item(&self) -> Option<&TodoRecord> {
        self.selected.and_then(|index| self.items.get(index))
    }

    /// 整体替换列表（启动加载或测试注入）：按 `entry_date` 分发到当前选中日期与归档日期。
    pub fn replace(&mut self, items: Vec<TodoRecord>) {
        self.items.clear();
        self.other_dates.clear();
        for mut item in items {
            if item.entry_date.is_empty() || item.entry_date == self.selected_date {
                item.entry_date = self.selected_date.clone();
                self.items.push(item);
            } else {
                self.other_dates
                    .entry(item.entry_date.clone())
                    .or_default()
                    .push(item);
            }
        }
        self.items.sort_by_key(|item| item.position);
        self.normalize_positions();
        let today = self.today_date.clone();
        for (date, list) in &mut self.other_dates {
            list.sort_by_key(|item| item.position);
            normalize_slice_groups_and_positions(list, date, &today);
        }
        self.selected = (!self.items.is_empty()).then_some(0);
        self.scroll = 0;
        self.editing = None;
        self.undo = None;
        self.pending_g = false;
        self.item_rows.clear();
        self.follow_selected = true;
        self.rollover_dismissed = false;
    }

    /// 内存侧位置与聚合组不变量归一。
    pub fn normalize_positions(&mut self) {
        normalize_slice_groups_and_positions(
            &mut self.items,
            &self.selected_date,
            &self.today_date,
        );
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
        self.follow_selected = true;
        self.scroll_into_view(self.rows);
    }

    /// 移动选中；`delta` 可传 `isize::MIN/MAX` 做跳首尾（内部饱和）。
    pub fn move_selection(&mut self, delta: isize) -> bool {
        if self.items.is_empty() {
            return false;
        }
        let last = self.items.len() - 1;
        let current = self.selected.unwrap_or(0).min(last);
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(last)
        };
        if self.selected == Some(next) {
            return false;
        }
        self.selected = Some(next);
        self.follow_selected = true;
        self.scroll_into_view(self.rows);
        true
    }

    /// 选中指定行。
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.items.len() || self.selected == Some(index) {
            return false;
        }
        self.selected = Some(index);
        self.follow_selected = true;
        self.scroll_into_view(self.rows);
        true
    }

    /// 视口滚到目标偏移（钳制到 `0..=max`，含新建幽灵行槽位）。
    pub fn set_scroll(&mut self, offset: usize) -> bool {
        self.follow_selected = false;
        let max = self.max_scroll();
        let next = offset.min(max);
        if next == self.scroll {
            return false;
        }
        self.scroll = next;
        true
    }

    /// 按行数增量滚动视口（滚轮）。
    pub fn scroll_by(&mut self, delta: isize) -> bool {
        let next = if delta < 0 {
            self.scroll.saturating_sub(delta.unsigned_abs())
        } else {
            self.scroll.saturating_add(delta as usize)
        };
        self.set_scroll(next)
    }

    /// 把编辑行滚入视口（新建/插入/编辑开始时调用）。
    pub fn scroll_to_edit(&mut self) -> bool {
        let Some(edit) = self.editing.as_ref() else {
            return false;
        };
        let target = match &edit.target {
            None => edit.at,
            Some(id) => self
                .items
                .iter()
                .position(|item| item.id == *id)
                .unwrap_or(edit.at),
        };
        self.follow_selected = true;
        self.scroll_target_into_view(target, self.rows)
    }

    /// 保证选中行在视口内（键盘移动后调用）。
    pub fn scroll_into_view(&mut self, rows: usize) -> bool {
        let Some(selected) = self.selected else {
            return false;
        };
        let shift = self
            .editing
            .as_ref()
            .is_some_and(|edit| edit.target.is_none() && edit.at <= selected);
        let target = selected + usize::from(shift);
        self.follow_selected = true;
        self.scroll_target_into_view(target, rows)
    }

    fn scroll_target_into_view(&mut self, target: usize, rows: usize) -> bool {
        let total = self.display_len();
        if rows == 0 || total == 0 {
            return false;
        }
        let target = target.min(total - 1);
        let next = if target < self.scroll {
            target
        } else {
            let mut used = 0usize;
            let mut min_start = target;
            for index in (self.scroll..=target).rev() {
                let height = self.item_height(index);
                if used + height > rows {
                    min_start = if index == target { target } else { index + 1 };
                    break;
                }
                used += height;
                min_start = index;
            }
            min_start
        };
        let max = self.max_scroll_for_rows(rows);
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
        self.follow_selected = true;
    }

    /// 结束编辑并返回现场。
    pub fn take_edit(&mut self) -> Option<TodoEdit> {
        self.editing.take()
    }

    /// 当前编辑输入（只读）。
    pub fn edit_input(&self) -> Option<&TextInput> {
        self.editing.as_ref().map(|edit| &edit.input)
    }

    /// 当前编辑输入（若有）。
    pub fn edit_input_mut(&mut self) -> Option<&mut TextInput> {
        if self.editing.is_some() {
            self.follow_selected = true;
        }
        self.editing.as_mut().map(|edit| &mut edit.input)
    }

    /// 清除当前编辑输入框选区（不触发视口跟随重置）。
    pub fn clear_selection(&mut self) {
        if let Some(edit) = self.editing.as_mut() {
            edit.input.clear_selection();
        }
    }

    /// 在 `at` 处插入一条记录并归一位置；返回插入条目在归一化后的下标。
    pub fn insert(&mut self, at: usize, mut record: TodoRecord) -> usize {
        let at = at.min(self.items.len());
        let id = record.id.clone();
        record.entry_date = self.selected_date.clone();
        record.position = at as i64;
        self.items.insert(at, record);
        self.normalize_positions();
        self.item_rows.clear();
        self.items
            .iter()
            .position(|item| item.id == id)
            .unwrap_or(at)
    }

    /// 移除一条并归一位置；返回被移除的记录。
    pub fn remove(&mut self, index: usize) -> Option<TodoRecord> {
        if index >= self.items.len() {
            return None;
        }
        let removed = self.items.remove(index);
        self.normalize_positions();
        if self.item_rows.len() == self.items.len() + 1 && index < self.item_rows.len() {
            self.item_rows.remove(index);
        } else {
            self.item_rows.clear();
        }
        Some(removed)
    }

    /// 就地修改一条的文本与状态。
    pub fn update(&mut self, index: usize, text: String, status: TodoStatus) -> Option<TodoRecord> {
        let today = self.today_date.clone();
        let item = self.items.get_mut(index)?;
        item.text = text;
        item.status = status;
        item.completed_date = (status == TodoStatus::Done).then_some(today);
        Some(item.clone())
    }

    /// 就地循环切换优先级（`P0 → P1 → P2 → P3 → P0`）；返回改动后的记录。
    pub fn cycle_priority(&mut self, index: usize) -> Option<TodoRecord> {
        let item = self.items.get_mut(index)?;
        item.priority = item.priority.cycle();
        Some(item.clone())
    }

    /// 按原位置把一批记录插回（撤销）。
    pub fn restore(&mut self, records: Vec<TodoRecord>) {
        let mut records = records;
        records.sort_by_key(|record| record.position);
        let today = self.today_date.clone();
        for mut record in records {
            if record.entry_date.is_empty() || record.entry_date == self.selected_date {
                if self.items.iter().any(|item| item.id == record.id) {
                    continue;
                }
                record.entry_date = self.selected_date.clone();
                let at = usize::try_from(record.position.max(0))
                    .unwrap_or(usize::MAX)
                    .min(self.items.len());
                self.items.insert(at, record);
            } else {
                let date = record.entry_date.clone();
                let list = self.other_dates.entry(date.clone()).or_default();
                if list.iter().any(|item| item.id == record.id) {
                    continue;
                }
                let at = usize::try_from(record.position.max(0))
                    .unwrap_or(usize::MAX)
                    .min(list.len());
                list.insert(at, record);
                normalize_slice_groups_and_positions(list, &date, &today);
            }
        }
        self.normalize_positions();
        self.item_rows.clear();
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

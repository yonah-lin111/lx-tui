//! 全局 todo 日程面板状态：按日切片列表、选中、视口、编辑现场、跨日顺延、多图表与删除撤销槽。
//!
//! 纯数据、无 IO；当前选中日期的列表恒按 `position` 升序且 `position == 下标`，
//! 因此「行下标」可以直接当「position」用，渲染与命中无需换算。

use std::collections::BTreeMap;

use crate::app::overlay::TextInput;
use crate::db::todo::{
    TodoDayStats, TodoPriority, TodoRecord, TodoStatus, day_of_month, recent_dates, shift_date,
    today_date, week_dates, weekday_initial, weekday_short,
};

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

/// 统计图表种类（一次仅展示一个，支持主动切换）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TodoChartKind {
    /// 三态分布与完成率（1/3）。
    #[default]
    StatusAndRate,
    /// 优先级 P0..P3 分布（2/3）。
    Priority,
    /// 近 7 日计划与完成双柱趋势（3/3）。
    Trend7Day,
}

impl TodoChartKind {
    /// 顺时针切换到下一张图表。
    pub fn next(self) -> Self {
        match self {
            Self::StatusAndRate => Self::Priority,
            Self::Priority => Self::Trend7Day,
            Self::Trend7Day => Self::StatusAndRate,
        }
    }

    /// 逆时针切换到上一张图表。
    pub fn prev(self) -> Self {
        match self {
            Self::StatusAndRate => Self::Trend7Day,
            Self::Priority => Self::StatusAndRate,
            Self::Trend7Day => Self::Priority,
        }
    }

    /// 1 基序号（1..=3）。
    pub fn number(self) -> usize {
        match self {
            Self::StatusAndRate => 1,
            Self::Priority => 2,
            Self::Trend7Day => 3,
        }
    }
}

/// 周感知带中单日的只读投影。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekDayView {
    pub date: String,
    pub day_of_month: u32,
    pub weekday_short: &'static str,
    pub weekday_initial: &'static str,
    pub is_today: bool,
    pub is_selected: bool,
    pub planned_count: usize,
    pub completed_count: usize,
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
            active_chart: TodoChartKind::StatusAndRate,
            rollover_dismissed: false,
        }
    }

    /// 当前选中查看的日期（`YYYY-MM-DD`）。
    pub fn selected_date(&self) -> &str {
        &self.selected_date
    }

    /// 今日日期基准（`YYYY-MM-DD`）。
    pub fn today_date(&self) -> &str {
        &self.today_date
    }

    /// 是否正在查看今天。
    pub fn is_viewing_today(&self) -> bool {
        self.selected_date == self.today_date
    }

    /// 设定今日日期基准（测试或跨日刷新用）；若此前停留在旧今日则同步跟随。
    pub fn set_today_date(&mut self, today: impl Into<String>) {
        let today = today.into();
        let was_viewing_today = self.selected_date == self.today_date;
        self.today_date = today.clone();
        if was_viewing_today && self.selected_date != today {
            let _ = self.select_date(today);
        }
    }

    /// 切换到指定日期（`YYYY-MM-DD`）；返回是否发生了日期变化。
    pub fn select_date(&mut self, date: impl Into<String>) -> bool {
        let date = date.into();
        if date.is_empty() || date == self.selected_date {
            return false;
        }
        let old_date = std::mem::replace(&mut self.selected_date, date.clone());
        let mut old_items = std::mem::take(&mut self.items);
        if !old_items.is_empty() {
            normalize_slice_positions(&mut old_items, &old_date);
            self.other_dates.insert(old_date, old_items);
        } else {
            self.other_dates.remove(&old_date);
        }
        self.items = self.other_dates.remove(&date).unwrap_or_default();
        self.normalize_positions();
        self.selected = (!self.items.is_empty()).then_some(0);
        self.scroll = 0;
        self.editing = None;
        self.undo = None;
        self.pending_g = false;
        true
    }

    /// 按周偏移当前选中日期（`delta_weeks` 为正向前、为负向后）。
    pub fn shift_week(&mut self, delta_weeks: i32) -> bool {
        if delta_weeks == 0 {
            return false;
        }
        let next = shift_date(&self.selected_date, delta_weeks.saturating_mul(7));
        self.select_date(next)
    }

    /// 当前选中日期的全部条目（按显示顺序）。
    pub fn items(&self) -> &[TodoRecord] {
        &self.items
    }

    /// 当前选中日期的条目数量。
    pub fn len(&self) -> usize {
        self.items.len()
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

    /// 当前激活的统计图表类型。
    pub fn active_chart(&self) -> TodoChartKind {
        self.active_chart
    }

    /// 切换到下一张图表。
    pub fn next_chart(&mut self) -> bool {
        self.active_chart = self.active_chart.next();
        true
    }

    /// 切换到上一张图表。
    pub fn prev_chart(&mut self) -> bool {
        self.active_chart = self.active_chart.prev();
        true
    }

    /// 直接设定图表类型。
    pub fn set_chart(&mut self, chart: TodoChartKind) -> bool {
        if self.active_chart == chart {
            return false;
        }
        self.active_chart = chart;
        true
    }

    /// 历史日期（`< today_date`）中未完成条目的总数。
    pub fn past_incomplete_count(&self) -> usize {
        let mut count = 0;
        for (date, list) in &self.other_dates {
            if date.as_str() < self.today_date.as_str() {
                count += list
                    .iter()
                    .filter(|item| item.status != TodoStatus::Done)
                    .count();
            }
        }
        if self.selected_date.as_str() < self.today_date.as_str() {
            count += self
                .items
                .iter()
                .filter(|item| item.status != TodoStatus::Done)
                .count();
        }
        count
    }

    /// 是否应当显示「将历史未完成顺延到今天」横幅。
    ///
    /// 仅当正在查看今日、未被手动关闭、且存在历史未完成项时显示（对齐 lx-agent）。
    pub fn has_rollover_banner(&self) -> bool {
        self.is_viewing_today() && !self.rollover_dismissed && self.past_incomplete_count() > 0
    }

    /// 手动忽略顺延横幅（当前会话内不再提示）。
    pub fn dismiss_rollover(&mut self) -> bool {
        if !self.has_rollover_banner() {
            return false;
        }
        self.rollover_dismissed = true;
        true
    }

    /// 将所有历史日期（`< today_date`）的未完成条目顺延移动到今天末尾，返回被移动的条目 id 列表。
    pub fn rollover_past_incomplete_to_today(&mut self) -> Vec<String> {
        let today = self.today_date.clone();
        if self.selected_date != today {
            let _ = self.select_date(today.clone());
        }
        let past_dates: Vec<String> = self
            .other_dates
            .keys()
            .filter(|date| date.as_str() < today.as_str())
            .cloned()
            .collect();
        let mut moved_ids = Vec::new();
        let mut moved_records = Vec::new();
        for date in past_dates {
            let Some(records) = self.other_dates.remove(&date) else {
                continue;
            };
            let mut kept = Vec::new();
            for mut record in records {
                if record.status == TodoStatus::Done {
                    kept.push(record);
                } else {
                    record.entry_date = today.clone();
                    moved_ids.push(record.id.clone());
                    moved_records.push(record);
                }
            }
            if !kept.is_empty() {
                normalize_slice_positions(&mut kept, &date);
                self.other_dates.insert(date, kept);
            }
        }
        if !moved_records.is_empty() {
            self.items.extend(moved_records);
            self.normalize_positions();
            if self.selected.is_none() && !self.items.is_empty() {
                self.selected = Some(0);
            }
        }
        self.rollover_dismissed = true;
        moved_ids
    }

    /// 查询指定日期的 `(计划总数, 已完成数)`。
    pub fn day_counts(&self, date: &str) -> (usize, usize) {
        if date == self.selected_date {
            return (self.items.len(), self.done_count());
        }
        let Some(list) = self.other_dates.get(date) else {
            return (0, 0);
        };
        let done = list
            .iter()
            .filter(|item| item.status == TodoStatus::Done)
            .count();
        (list.len(), done)
    }

    /// 生成当前选中日期所在周（周一至周日）的 7 日感知带视图数据。
    pub fn week_days(&self) -> [WeekDayView; 7] {
        let dates = week_dates(&self.selected_date);
        std::array::from_fn(|index| {
            let date = dates[index].clone();
            let (planned_count, completed_count) = self.day_counts(&date);
            let is_today = date == self.today_date;
            let is_selected = date == self.selected_date;
            WeekDayView {
                day_of_month: day_of_month(&date),
                weekday_short: weekday_short(&date),
                weekday_initial: weekday_initial(&date),
                is_today,
                is_selected,
                planned_count,
                completed_count,
                date,
            }
        })
    }

    /// 生成以当前选中日期为终点、最近 7 天的每日统计序列（用于 7 日趋势图表）。
    pub fn trend_7_days(&self) -> Vec<TodoDayStats> {
        recent_dates(&self.selected_date, 7)
            .into_iter()
            .map(|date| {
                let (planned_count, completed_count) = self.day_counts(&date);
                TodoDayStats {
                    date,
                    planned_count,
                    completed_count,
                }
            })
            .collect()
    }

    /// 回写最近一帧的可见行数；顺带钳制视口。返回是否发生变化。
    pub fn set_viewport_rows(&mut self, rows: usize) -> bool {
        if self.rows == rows {
            return false;
        }
        self.rows = rows;
        let ghost = self
            .editing
            .as_ref()
            .map_or(0, |edit| usize::from(edit.target.is_none()));
        let max = (self.items.len() + ghost).saturating_sub(rows);
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

    /// 完成项数量（工具栏与统计用）。
    pub fn done_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == TodoStatus::Done)
            .count()
    }

    /// 未开始数量（底部统计柱图用）。
    pub fn open_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == TodoStatus::Todo)
            .count()
    }

    /// 进行中数量（底部统计用）。
    pub fn in_progress_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == TodoStatus::InProgress)
            .count()
    }

    /// 指定优先级的条目数量（优先级分布图表用）。
    pub fn priority_count(&self, priority: TodoPriority) -> usize {
        self.items
            .iter()
            .filter(|item| item.priority == priority)
            .count()
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
        self.normalize_positions();
        for (date, list) in &mut self.other_dates {
            list.sort_by_key(|item| item.position);
            normalize_slice_positions(list, date);
        }
        self.selected = (!self.items.is_empty()).then_some(0);
        self.scroll = 0;
        self.editing = None;
        self.undo = None;
        self.pending_g = false;
        self.rollover_dismissed = false;
    }

    /// 内存侧位置归一：`position == 下标`。
    pub fn normalize_positions(&mut self) {
        normalize_slice_positions(&mut self.items, &self.selected_date);
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
        let max = if self.rows == 0 {
            self.items.len().saturating_sub(1)
        } else {
            self.items.len().saturating_sub(self.rows)
        };
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
        let ghost = usize::from(edit.target.is_none());
        let target = edit.at.min(self.items.len() + ghost);
        if self.rows == 0 {
            return false;
        }
        let total = self.items.len() + ghost;
        let max = total.saturating_sub(self.rows);
        let next = if target < self.scroll {
            target
        } else if target >= self.scroll.saturating_add(self.rows) {
            target.saturating_add(1).saturating_sub(self.rows)
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
        record.entry_date = self.selected_date.clone();
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
        let today = self.today_date.clone();
        let item = self.items.get_mut(index)?;
        item.text = text;
        item.status = status;
        item.completed_date = (status == TodoStatus::Done).then_some(today);
        Some(item.clone())
    }

    /// 就地改状态；返回改动后的记录（无变化为 None）。
    pub fn cycle_status(&mut self, index: usize) -> Option<TodoRecord> {
        let today = self.today_date.clone();
        let item = self.items.get_mut(index)?;
        item.status = item.status.cycle();
        item.completed_date = (item.status == TodoStatus::Done).then_some(today);
        Some(item.clone())
    }

    /// 就地循环切换优先级（`P0 → P1 → P2 → P3 → P0`）；返回改动后的记录。
    pub fn cycle_priority(&mut self, index: usize) -> Option<TodoRecord> {
        let item = self.items.get_mut(index)?;
        item.priority = item.priority.cycle();
        Some(item.clone())
    }

    /// 按 lx-agent 规则对当前日期条目排序：未完成优先 → P0..P3 → 原 position → id。
    ///
    /// 返回排序后的 ID 序列；若列表不足 2 项或顺序未发生改变则返回 None。
    pub fn sort_by_priority(&mut self) -> Option<Vec<String>> {
        if self.items.len() < 2 {
            return None;
        }
        let selected_id = self.selected_item().map(|item| item.id.clone());
        let before_ids: Vec<String> = self.items.iter().map(|item| item.id.clone()).collect();
        self.items.sort_by(|left, right| {
            let left_done = left.status == TodoStatus::Done;
            let right_done = right.status == TodoStatus::Done;
            if left_done != right_done {
                return left_done.cmp(&right_done);
            }
            if left.priority != right.priority {
                return left.priority.cmp(&right.priority);
            }
            if left.position != right.position {
                return left.position.cmp(&right.position);
            }
            left.id.cmp(&right.id)
        });
        self.normalize_positions();
        if let Some(id) = selected_id
            && let Some(new_index) = self.items.iter().position(|item| item.id == id)
        {
            self.selected = Some(new_index);
            self.scroll_into_view(self.rows);
        }
        let after_ids: Vec<String> = self.items.iter().map(|item| item.id.clone()).collect();
        (before_ids != after_ids).then_some(after_ids)
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
    pub fn restore(&mut self, records: Vec<TodoRecord>) {
        let mut records = records;
        records.sort_by_key(|record| record.position);
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
                normalize_slice_positions(list, &date);
            }
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

fn normalize_slice_positions(items: &mut [TodoRecord], date: &str) {
    for (index, item) in items.iter_mut().enumerate() {
        item.position = index as i64;
        if item.entry_date.is_empty() {
            item.entry_date = date.to_string();
        }
    }
}

#[cfg(test)]
mod tests;

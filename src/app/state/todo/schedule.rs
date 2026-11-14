//! Todo 日程切换、历史未完成顺延与 7 日周感知带投影。

use crate::db::todo::{
    TodoStatus, day_of_month, shift_date, week_dates, weekday_initial, weekday_short,
};

use super::TodoPanel;
use super::group::normalize_slice_groups_and_positions;

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

impl TodoPanel {
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
        if self.today_date == today {
            return;
        }
        let was_viewing_today = self.selected_date == self.today_date;
        self.today_date = today.clone();
        self.rollover_dismissed = false;
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
            normalize_slice_groups_and_positions(&mut old_items, &old_date, &self.today_date);
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
        self.item_rows.clear();
        self.follow_selected = true;
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
                normalize_slice_groups_and_positions(&mut kept, &date, &today);
                self.other_dates.insert(date, kept);
            }
        }
        if !moved_records.is_empty() {
            self.items.extend(moved_records);
            self.normalize_positions();
            self.item_rows.clear();
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

    /// 完成项数量（周感知带按日统计用）。
    pub fn done_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == TodoStatus::Done)
            .count()
    }
}

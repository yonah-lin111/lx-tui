//! Todo 聚合组（Group）角色投影、不变量归一化、合并/拆分/解散、整组状态循环与保组优先级排序。

use std::collections::BTreeMap;
use std::ops::Range;

use crate::db::todo::{TodoRecord, TodoStatus, new_todo_group_id};

use super::TodoPanel;

/// 条目在列表中的聚合角色（由 [`TodoPanel::group_role`] 纯函数派生，不冗余存储）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TodoGroupRole {
    /// 独立普通待办（`group_id == None`）。
    Single,
    /// 聚合组内成员（`count >= 2`）。
    Member {
        group_id: String,
        /// 在组内的 0 基下标（`0..count`）。
        offset: usize,
        /// 组内总成员数。
        count: usize,
    },
}

impl TodoGroupRole {
    /// 是否处于聚合组内。
    pub fn is_grouped(&self) -> bool {
        matches!(self, Self::Member { .. })
    }

    /// 是否承载整组唯一的状态框 `[ ]`（单条恒为真；聚合组固定在 `(count - 1) / 2`）。
    pub fn is_status_anchor(&self) -> bool {
        match self {
            Self::Single => true,
            Self::Member { offset, count, .. } => *offset == (*count - 1) / 2,
        }
    }
}

impl TodoPanel {
    /// 查询下标 `index` 处条目在聚合组中的角色。
    pub fn group_role(&self, index: usize) -> TodoGroupRole {
        let Some(item) = self.items.get(index) else {
            return TodoGroupRole::Single;
        };
        let Some(ref gid) = item.group_id else {
            return TodoGroupRole::Single;
        };
        let range = self.group_range(index);
        let count = range.len();
        if count < 2 {
            TodoGroupRole::Single
        } else {
            TodoGroupRole::Member {
                group_id: gid.clone(),
                offset: index.saturating_sub(range.start),
                count,
            }
        }
    }

    /// 下标 `index` 处的条目是否属于某个聚合组（`count >= 2`）。
    pub fn is_grouped(&self, index: usize) -> bool {
        self.group_role(index).is_grouped()
    }

    /// 返回 `index` 所在连续块（单条为 `index..index+1`，聚合组为 `start..end`）的下标区间。
    pub fn group_range(&self, index: usize) -> Range<usize> {
        if index >= self.items.len() {
            let clamped = self.items.len();
            return clamped..clamped;
        }
        let Some(gid) = self.items[index].group_id.as_deref() else {
            return index..(index + 1);
        };
        let mut start = index;
        while start > 0 && self.items[start - 1].group_id.as_deref() == Some(gid) {
            start -= 1;
        }
        let mut end = index + 1;
        while end < self.items.len() && self.items[end].group_id.as_deref() == Some(gid) {
            end += 1;
        }
        start..end
    }

    /// `index` 所在条目（或聚合组）下方是否还有其他条目可供向下聚合。
    pub fn has_next_to_group(&self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        self.group_range(index).end < self.items.len()
    }

    /// 将 `index` 所在条目（或所在聚合组）与紧邻的下一项（或下一组）聚合成同一组，
    /// 被并入项的状态与完成日期统一对齐到 `self.items[index]`。
    pub fn merge_with_next(&mut self, index: usize) -> bool {
        if index >= self.items.len() {
            return false;
        }
        let range = self.group_range(index);
        let next_start = range.end;
        if next_start >= self.items.len() {
            return false;
        }
        let next_range = self.group_range(next_start);
        let target_gid = self.items[index]
            .group_id
            .clone()
            .unwrap_or_else(new_todo_group_id);
        let base_status = self.items[index].status;
        let base_completed = (base_status == TodoStatus::Done).then(|| {
            self.items[index]
                .completed_date
                .clone()
                .unwrap_or_else(|| self.today_date.clone())
        });
        for item in &mut self.items[range.start..next_range.end] {
            item.group_id = Some(target_gid.clone());
            item.status = base_status;
            item.completed_date = base_completed.clone();
        }
        self.normalize_positions();
        self.item_rows.clear();
        true
    }

    /// 将 `from_index` 处的单条待办移入 `target_index` 所在条目（或聚合组）末尾，
    /// 状态与完成日期对齐到 `target_index`，并将选中焦点跟随至移动后的条目。
    pub fn merge_item_into(&mut self, from_index: usize, target_index: usize) -> bool {
        if from_index >= self.items.len()
            || target_index >= self.items.len()
            || from_index == target_index
        {
            return false;
        }
        if self.items[from_index].group_id.is_some()
            && self.items[from_index].group_id == self.items[target_index].group_id
        {
            return false;
        }
        let target_status = self.items[target_index].status;
        let target_completed = (target_status == TodoStatus::Done).then(|| {
            self.items[target_index]
                .completed_date
                .clone()
                .unwrap_or_else(|| self.today_date.clone())
        });
        let target_gid = self.items[target_index]
            .group_id
            .clone()
            .unwrap_or_else(new_todo_group_id);
        let target_range = self.group_range(target_index);
        for item in &mut self.items[target_range] {
            item.group_id = Some(target_gid.clone());
        }

        let mut moved = self.items.remove(from_index);
        let moved_id = moved.id.clone();
        moved.group_id = Some(target_gid.clone());
        moved.status = target_status;
        moved.completed_date = target_completed;

        let insert_at = self
            .items
            .iter()
            .rposition(|item| item.group_id.as_deref() == Some(target_gid.as_str()))
            .map_or(self.items.len(), |last| last + 1);
        self.items.insert(insert_at, moved);
        self.normalize_positions();
        self.item_rows.clear();
        if let Some(new_idx) = self.items.iter().position(|item| item.id == moved_id) {
            self.selected = Some(new_idx);
            self.follow_selected = true;
            self.scroll_into_view(self.rows);
        }
        true
    }

    /// 将 `index` 处的组内子项拆出聚合组（挪到该组紧邻下方并清除 `group_id`）；
    /// 若原组只剩 1 条，归一化会自动将剩余条目退化为独立普通待办。
    pub fn split_from_group(&mut self, index: usize) -> bool {
        if index >= self.items.len() || self.items[index].group_id.is_none() {
            return false;
        }
        let range = self.group_range(index);
        if range.len() < 2 {
            self.items[index].group_id = None;
            return false;
        }
        let mut detached = self.items.remove(index);
        let detached_id = detached.id.clone();
        detached.group_id = None;
        let insert_at = (range.end - 1).min(self.items.len());
        self.items.insert(insert_at, detached);
        self.normalize_positions();
        self.item_rows.clear();
        if let Some(new_idx) = self.items.iter().position(|item| item.id == detached_id) {
            self.selected = Some(new_idx);
            self.follow_selected = true;
            self.scroll_into_view(self.rows);
        }
        true
    }

    /// 将 `index` 所在聚合组的全部成员一次性解除聚合，原地恢复为多条独立普通待办。
    pub fn ungroup_at(&mut self, index: usize) -> bool {
        let Some(gid) = self.items.get(index).and_then(|item| item.group_id.clone()) else {
            return false;
        };
        for item in &mut self.items {
            if item.group_id.as_deref() == Some(gid.as_str()) {
                item.group_id = None;
            }
        }
        self.normalize_positions();
        self.item_rows.clear();
        true
    }

    /// 循环切换状态（`Todo -> InProgress -> Done -> Todo`）：
    /// 若 `index` 属于聚合组，整组所有成员同步更新并返回全部变更后的记录；
    /// 若为独立单条，仅更新该条并返回单元素 `Vec`；越界返回空 `Vec`。
    pub fn cycle_status(&mut self, index: usize) -> Vec<TodoRecord> {
        if index >= self.items.len() {
            return Vec::new();
        }
        let today = self.today_date.clone();
        let next_status = self.items[index].status.cycle();
        let completed_date = (next_status == TodoStatus::Done).then_some(today);
        let range = self.group_range(index);
        let mut updated = Vec::with_capacity(range.len());
        for item in &mut self.items[range] {
            item.status = next_status;
            item.completed_date = completed_date.clone();
            updated.push(item.clone());
        }
        updated
    }

    /// 保组优先级排序：
    /// - 聚合组作为不可分割整体参与外部排序，外层按 `(is_done, 组内最高优先级, 块首原 position, 块首原 id)` 升序；
    /// - 每个聚合组内部同步按 `(priority, position, id)` 升序重排；
    /// - 返回排序后的 ID 序列；若列表不足 2 项或顺序未发生改变则返回 `None`。
    pub fn sort_by_priority(&mut self) -> Option<Vec<String>> {
        if self.items.len() < 2 {
            return None;
        }
        let selected_id = self.selected_item().map(|item| item.id.clone());
        let before_ids: Vec<String> = self.items.iter().map(|item| item.id.clone()).collect();

        let mut blocks: Vec<(i64, String, Vec<TodoRecord>)> = Vec::new();
        let mut idx = 0;
        while idx < self.items.len() {
            let range = self.group_range(idx);
            let first_position = self.items[range.start].position;
            let first_id = self.items[range.start].id.clone();
            let mut block = self.items[range.clone()].to_vec();
            if block.len() >= 2 {
                block.sort_by(|left, right| {
                    left.priority
                        .cmp(&right.priority)
                        .then_with(|| left.position.cmp(&right.position))
                        .then_with(|| left.id.cmp(&right.id))
                });
            }
            blocks.push((first_position, first_id, block));
            idx = range.end;
        }

        blocks.sort_by(|(l_pos, l_id, left), (r_pos, r_id, right)| {
            let left_done = left[0].status == TodoStatus::Done;
            let right_done = right[0].status == TodoStatus::Done;
            let left_prio = left
                .iter()
                .map(|item| item.priority)
                .min()
                .unwrap_or_default();
            let right_prio = right
                .iter()
                .map(|item| item.priority)
                .min()
                .unwrap_or_default();
            left_done
                .cmp(&right_done)
                .then_with(|| left_prio.cmp(&right_prio))
                .then_with(|| l_pos.cmp(r_pos))
                .then_with(|| l_id.cmp(r_id))
        });

        self.items = blocks.into_iter().flat_map(|(_, _, block)| block).collect();
        self.normalize_positions();
        self.item_rows.clear();
        if let Some(id) = selected_id
            && let Some(new_index) = self.items.iter().position(|item| item.id == id)
        {
            self.selected = Some(new_index);
            self.scroll_into_view(self.rows);
        }
        let after_ids: Vec<String> = self.items.iter().map(|item| item.id.clone()).collect();
        (before_ids != after_ids).then_some(after_ids)
    }
}

/// 归一化单日列表的聚合组不变量与密集位置：
/// 1. 相同 `group_id` 的成员强制归拢连续（按该组首次出现位置聚拢）；
/// 2. 成员数 `< 2` 的孤儿 `group_id` 自动清除为 `None`；
/// 3. 同一 `group_id` 的所有成员强制对齐到该组首条成员的 `status` 与 `completed_date`；
/// 4. `position == index as i64`。
pub(super) fn normalize_slice_groups_and_positions(
    items: &mut Vec<TodoRecord>,
    date: &str,
    today: &str,
) {
    for item in items.iter_mut() {
        if item.group_id.as_deref().is_some_and(str::is_empty) {
            item.group_id = None;
        }
    }

    // 1. 同组归拢连续：按首次出现顺序把同 group_id 成员聚拢在一起
    let mut compacted: Vec<TodoRecord> = Vec::with_capacity(items.len());
    let mut visited = vec![false; items.len()];
    for i in 0..items.len() {
        if visited[i] {
            continue;
        }
        visited[i] = true;
        let gid = items[i].group_id.clone();
        compacted.push(items[i].clone());
        if let Some(ref group_id) = gid {
            for j in (i + 1)..items.len() {
                if !visited[j] && items[j].group_id.as_deref() == Some(group_id.as_str()) {
                    visited[j] = true;
                    compacted.push(items[j].clone());
                }
            }
        }
    }
    *items = compacted;

    // 2. 统计各 group_id 成员数，消除 count < 2 的孤儿组，并统一同组状态
    let mut counts: BTreeMap<String, (usize, TodoStatus, Option<String>)> = BTreeMap::new();
    for item in items.iter() {
        if let Some(ref gid) = item.group_id {
            counts
                .entry(gid.clone())
                .and_modify(|(c, _, _)| *c += 1)
                .or_insert_with(|| {
                    let completed = (item.status == TodoStatus::Done).then(|| {
                        item.completed_date
                            .clone()
                            .unwrap_or_else(|| today.to_string())
                    });
                    (1, item.status, completed)
                });
        }
    }

    for (index, item) in items.iter_mut().enumerate() {
        item.position = index as i64;
        if item.entry_date.is_empty() {
            item.entry_date = date.to_string();
        }
        if let Some(ref gid) = item.group_id.clone()
            && let Some((count, group_status, group_completed)) = counts.get(gid)
        {
            if *count < 2 {
                item.group_id = None;
            } else {
                item.status = *group_status;
                item.completed_date = group_completed.clone();
            }
        }
    }
}

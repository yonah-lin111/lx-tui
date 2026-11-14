//! Todo 看板只读查询：筛选、分页、汇总、按日聚合与聚合组统计。
//!
//! 与写路径严格分离：这里只发 `SELECT`，不改位置、不清理聚合组。日期范围一律按
//! `entry_date`（未完成条目没有 `completed_date`，按完成日期筛会漏掉全部待办）；
//! `Overdue` 的“今天”由调用方显式传入，持久层不读系统时钟，便于测试。

use rusqlite::params;

use super::{TodoPriority, TodoRecord, TodoStatus, map_todo_row};
use crate::db::{Database, Result};

/// 单页返回行数上限；与服务层 `pageSize` 上限一致。
const MAX_PAGE_SIZE: usize = 200;

/// 筛选条件；字段全为 `None` 表示不设限。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TodoFilter {
    /// 起始日期（含），`YYYY-MM-DD`。
    pub start_date: Option<String>,
    /// 结束日期（含），`YYYY-MM-DD`。
    pub end_date: Option<String>,
    pub status: Option<TodoStatus>,
    pub priority: Option<TodoPriority>,
    /// 文本子串；ASCII 大小写不敏感，中文按原样子串匹配。
    pub search: Option<String>,
}

/// 明细行：库记录 + 所属聚合组标签（独立条目为 `None`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoItemRow {
    pub record: TodoRecord,
    pub group_label: Option<String>,
}

/// 一页明细与分页元信息；`page` / `page_size` 为钳制后的实际值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoPage {
    pub rows: Vec<TodoItemRow>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

/// 汇总卡片计数；`overdue` 忽略日期范围。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TodoSummary {
    pub total: usize,
    pub open: usize,
    pub in_progress: usize,
    pub done: usize,
    pub overdue: usize,
}

/// 按日聚合行；`remaining = planned - done`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoDailyRow {
    pub date: String,
    pub planned: usize,
    pub done: usize,
    pub remaining: usize,
}

/// 聚合组统计行；`label` 取组内最早条目的文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoGroupRow {
    pub group_id: String,
    pub date: String,
    pub label: String,
    pub items: usize,
    pub open: usize,
    pub in_progress: usize,
    pub done: usize,
    pub top_priority: TodoPriority,
}

/// 固定筛选片段：`?1..?5` = 起止日期、状态、优先级、搜索。
///
/// 列名不带表别名；主查询使用 `todo t` 别名时 SQLite 仍能解析未限定列名。
const FILTER_WHERE: &str = "WHERE (?1 IS NULL OR entry_date >= ?1)
       AND (?2 IS NULL OR entry_date <= ?2)
       AND (?3 IS NULL OR status = ?3)
       AND (?4 IS NULL OR priority = ?4)
       AND (?5 IS NULL OR instr(lower(text), lower(?5)) > 0)";

impl Database {
    /// 按筛选条件分页读取明细；`group_label` 取组内最早条目的文本。
    ///
    /// `page` / `page_size` 只做最小钳制（`page >= 1`、`page_size` 1..=200），
    /// 保证不构造负数 `LIMIT/OFFSET`；更严格的参数校验由服务层负责。
    pub fn query_todos_page(
        &self,
        filter: &TodoFilter,
        page: usize,
        page_size: usize,
    ) -> Result<TodoPage> {
        let page = page.max(1);
        let page_size = page_size.clamp(1, MAX_PAGE_SIZE);
        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM todo {FILTER_WHERE}"),
            params![
                filter.start_date.as_deref(),
                filter.end_date.as_deref(),
                filter.status.map(TodoStatus::as_str),
                filter.priority.map(TodoPriority::as_str),
                filter.search.as_deref(),
            ],
            |row| row.get(0),
        )?;

        let offset = page.saturating_sub(1).saturating_mul(page_size);
        let sql = format!(
            r#"SELECT t.id, t.entry_date, t.text, t.status, t.priority, t.completed_date,
                      t.position, t.group_id,
                      (SELECT t2.text FROM todo t2
                        WHERE t2.group_id = t.group_id
                        ORDER BY t2.entry_date ASC, t2.position ASC, t2.id ASC
                        LIMIT 1) AS group_label
               FROM todo t
               {FILTER_WHERE}
               ORDER BY t.entry_date DESC, t.position ASC, t.id ASC
               LIMIT ?6 OFFSET ?7"#
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                filter.start_date.as_deref(),
                filter.end_date.as_deref(),
                filter.status.map(TodoStatus::as_str),
                filter.priority.map(TodoPriority::as_str),
                filter.search.as_deref(),
                i64::try_from(page_size).unwrap_or(i64::MAX),
                i64::try_from(offset).unwrap_or(i64::MAX),
            ],
            |row| {
                Ok(TodoItemRow {
                    record: map_todo_row(row)?,
                    group_label: row.get(8)?,
                })
            },
        )?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(TodoPage {
            rows: items,
            total: count_to_usize(total),
            page,
            page_size,
        })
    }

    /// 汇总卡片计数；`overdue` 统计 `entry_date < today` 且未完成的条目，
    /// 忽略日期范围但遵守状态、优先级与搜索筛选。
    pub fn query_todo_summary(&self, filter: &TodoFilter, today: &str) -> Result<TodoSummary> {
        let counts: (i64, i64, i64, i64) = self.conn.query_row(
            &format!(
                r#"SELECT COUNT(*),
                          COALESCE(SUM(CASE WHEN status = 'todo' THEN 1 ELSE 0 END), 0),
                          COALESCE(SUM(CASE WHEN status = 'in_progress' THEN 1 ELSE 0 END), 0),
                          COALESCE(SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END), 0)
                   FROM todo
                   {FILTER_WHERE}"#
            ),
            params![
                filter.start_date.as_deref(),
                filter.end_date.as_deref(),
                filter.status.map(TodoStatus::as_str),
                filter.priority.map(TodoPriority::as_str),
                filter.search.as_deref(),
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;

        let overdue: i64 = self.conn.query_row(
            "SELECT COUNT(*)
             FROM todo
             WHERE entry_date < ?1
               AND status != 'done'
               AND (?2 IS NULL OR status = ?2)
               AND (?3 IS NULL OR priority = ?3)
               AND (?4 IS NULL OR instr(lower(text), lower(?4)) > 0)",
            params![
                today,
                filter.status.map(TodoStatus::as_str),
                filter.priority.map(TodoPriority::as_str),
                filter.search.as_deref(),
            ],
            |row| row.get(0),
        )?;

        Ok(TodoSummary {
            total: count_to_usize(counts.0),
            open: count_to_usize(counts.1),
            in_progress: count_to_usize(counts.2),
            done: count_to_usize(counts.3),
            overdue: count_to_usize(overdue),
        })
    }

    /// 按 `entry_date` 聚合每日计划数、完成数与剩余数（`planned - done`），日期倒序。
    pub fn query_todo_daily(&self, filter: &TodoFilter) -> Result<Vec<TodoDailyRow>> {
        let sql = format!(
            r#"SELECT entry_date,
                      COUNT(*),
                      COALESCE(SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END), 0)
               FROM todo
               {FILTER_WHERE}
               GROUP BY entry_date
               ORDER BY entry_date DESC"#
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                filter.start_date.as_deref(),
                filter.end_date.as_deref(),
                filter.status.map(TodoStatus::as_str),
                filter.priority.map(TodoPriority::as_str),
                filter.search.as_deref(),
            ],
            |row| {
                let planned = count_to_usize(row.get::<_, i64>(1)?);
                let done = count_to_usize(row.get::<_, i64>(2)?);
                Ok(TodoDailyRow {
                    date: row.get(0)?,
                    planned,
                    done,
                    remaining: planned.saturating_sub(done),
                })
            },
        )?;
        let mut daily = Vec::new();
        for row in rows {
            daily.push(row?);
        }
        Ok(daily)
    }

    /// 聚合组统计：只统计 `group_id` 非空的成员，标签取组内最早条目的文本。
    pub fn query_todo_groups(&self, filter: &TodoFilter) -> Result<Vec<TodoGroupRow>> {
        let sql = format!(
            r#"SELECT group_id,
                      MIN(entry_date) AS group_date,
                      COUNT(*) AS items,
                      COALESCE(SUM(CASE WHEN status = 'todo' THEN 1 ELSE 0 END), 0) AS open_count,
                      COALESCE(SUM(CASE WHEN status = 'in_progress' THEN 1 ELSE 0 END), 0) AS in_progress_count,
                      COALESCE(SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END), 0) AS done_count,
                      MIN(CASE priority
                            WHEN 'P0' THEN 0
                            WHEN 'P2' THEN 2
                            WHEN 'P3' THEN 3
                            ELSE 1
                          END) AS priority_rank,
                      MIN(position) AS first_position,
                      (SELECT t2.text FROM todo t2
                        WHERE t2.group_id = todo.group_id
                        ORDER BY t2.entry_date ASC, t2.position ASC, t2.id ASC
                        LIMIT 1) AS label
               FROM todo
               {FILTER_WHERE} AND group_id IS NOT NULL
               GROUP BY group_id
               ORDER BY group_date DESC, first_position ASC"#
        );
        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(
            params![
                filter.start_date.as_deref(),
                filter.end_date.as_deref(),
                filter.status.map(TodoStatus::as_str),
                filter.priority.map(TodoPriority::as_str),
                filter.search.as_deref(),
            ],
            |row| {
                Ok(TodoGroupRow {
                    group_id: row.get(0)?,
                    date: row.get(1)?,
                    label: row.get(8)?,
                    items: count_to_usize(row.get(2)?),
                    open: count_to_usize(row.get(3)?),
                    in_progress: count_to_usize(row.get(4)?),
                    done: count_to_usize(row.get(5)?),
                    top_priority: priority_from_rank(row.get(6)?),
                })
            },
        )?;
        let mut groups = Vec::new();
        for row in rows {
            groups.push(row?);
        }
        Ok(groups)
    }
}

/// SQLite 计数转 `usize`；异常负值钳到 0。
fn count_to_usize(value: i64) -> usize {
    usize::try_from(value.max(0)).unwrap_or(0)
}

/// 优先级排序序号（0..=3）映射回枚举；越界回落到默认档 `P1`。
fn priority_from_rank(rank: i64) -> TodoPriority {
    usize::try_from(rank.max(0))
        .ok()
        .and_then(|index| TodoPriority::ALL.get(index).copied())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;

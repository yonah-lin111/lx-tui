//! 全局 todo 日程列表的持久化：按日读取、UPSERT 写入、删除、重排、跨日顺延与撤销恢复。
//!
//! 表结构见 `migration::SCHEMA_V15_TODO` 与 `SCHEMA_V21_TODO_INDEXES`。列表是**全局**的
//! （跨工作区、跨会话共享），按本地日期 `entry_date`（`YYYY-MM-DD`）分区组织。
//!
//! `position` 是各日期内的密集递增排序位（0 基、无空洞），由写操作在单事务内维护；
//! `id` 是稳定业务键，UPSERT 与删除都以它为准。

pub mod date;

use std::collections::BTreeSet;

pub use date::*;
use rusqlite::{OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, now_ms};

/// 单条 todo 文本的字符上限；与 lx-agent 的 `content: 1..500` 一致。
///
/// 按**字符**而非字节计数：500 个汉字是 1500 字节，按字节判定会把中文拦在 167 字。
pub const MAX_TODO_TEXT_CHARS: usize = 500;

/// 重排位置时使用的临时偏移；所有行先移进该区间，再落回 `0..n`，
/// 避免中途撞 `idx_todo_date_position` 唯一索引。
const RENUMBER_OFFSET: i64 = 1_000_000;
/// 恢复撤销时插入行的临时区间（与 [`RENUMBER_OFFSET`] 不重叠）。
const RESTORE_OFFSET: i64 = 2_000_000;

/// todo 条目状态；库内以 TEXT 存储并对齐 CHECK 约束。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TodoStatus {
    Todo,
    InProgress,
    Done,
}

impl TodoStatus {
    /// 库内文本值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Done => "done",
        }
    }

    /// 由库内文本解析；未知值视为 `Todo`（一条脏数据不该让整张列表读不出来）。
    pub fn from_db(value: &str) -> Self {
        match value {
            "in_progress" => Self::InProgress,
            "done" => Self::Done,
            _ => Self::Todo,
        }
    }

    /// `Space` 循环：todo → in_progress → done → todo。
    pub fn cycle(self) -> Self {
        match self {
            Self::Todo => Self::InProgress,
            Self::InProgress => Self::Done,
            Self::Done => Self::Todo,
        }
    }
}

/// 日程优先级（四档，P0 最高），对齐 lx-agent 的 `SchedulePriority`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum TodoPriority {
    P0,
    #[default]
    P1,
    P2,
    P3,
}

impl TodoPriority {
    /// 全部四档序列（从高到低）。
    pub const ALL: [Self; 4] = [Self::P0, Self::P1, Self::P2, Self::P3];

    /// 库内与界面显示文本值。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::P0 => "P0",
            Self::P1 => "P1",
            Self::P2 => "P2",
            Self::P3 => "P3",
        }
    }

    /// 由库内文本解析；未知值回落 `P1`。
    pub fn from_db(value: &str) -> Self {
        match value {
            "P0" => Self::P0,
            "P2" => Self::P2,
            "P3" => Self::P3,
            _ => Self::P1,
        }
    }

    /// 循环切换：P0 → P1 → P2 → P3 → P0。
    pub fn cycle(self) -> Self {
        match self {
            Self::P0 => Self::P1,
            Self::P1 => Self::P2,
            Self::P2 => Self::P3,
            Self::P3 => Self::P0,
        }
    }
}

/// 单日待办统计：计划总数与已完成数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoDayStats {
    pub date: String,
    pub planned_count: usize,
    pub completed_count: usize,
}

/// 一条 todo；内存视图与库记录同构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoRecord {
    /// 稳定 id（`todo-<nanos>-<seq>`），用于 UPSERT 与删除。
    pub id: String,
    /// 归属日期（本地日期 `YYYY-MM-DD`）。
    pub entry_date: String,
    pub text: String,
    pub status: TodoStatus,
    pub priority: TodoPriority,
    /// 完成发生的本地日期；未完成时为 None。
    pub completed_date: Option<String>,
    /// 当日密集递增排序位（0 基）。
    pub position: i64,
}

impl TodoRecord {
    /// 新建一条默认归属今日、优先级 `P1` 的待办；`position` 由调用方按插入点给出。
    pub fn new(text: impl Into<String>, status: TodoStatus, position: i64) -> Self {
        let entry_date = today_date();
        Self::new_on(entry_date, text, status, TodoPriority::P1, position)
    }

    /// 在指定日期与优先级下新建一条待办。
    pub fn new_on(
        entry_date: impl Into<String>,
        text: impl Into<String>,
        status: TodoStatus,
        priority: TodoPriority,
        position: i64,
    ) -> Self {
        let entry_date = entry_date.into();
        let completed_date = (status == TodoStatus::Done).then(|| entry_date.clone());
        Self {
            id: new_external_id("todo"),
            entry_date,
            text: text.into(),
            status,
            priority,
            completed_date,
            position,
        }
    }

    /// 链式设定归属日期。
    pub fn with_date(mut self, entry_date: impl Into<String>) -> Self {
        let date = entry_date.into();
        if self.status == TodoStatus::Done && self.completed_date.is_none() {
            self.completed_date = Some(date.clone());
        }
        self.entry_date = date;
        self
    }

    /// 链式设定优先级。
    pub fn with_priority(mut self, priority: TodoPriority) -> Self {
        self.priority = priority;
        self
    }
}

fn map_todo_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TodoRecord> {
    Ok(TodoRecord {
        id: row.get(0)?,
        entry_date: row.get(1)?,
        text: row.get(2)?,
        status: TodoStatus::from_db(&row.get::<_, String>(3)?),
        priority: TodoPriority::from_db(&row.get::<_, String>(4)?),
        completed_date: row.get(5)?,
        position: row.get(6)?,
    })
}

impl Database {
    /// 读取全表，按 `(entry_date, position, id)` 升序。
    pub fn list_todos(&self) -> Result<Vec<TodoRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT id, entry_date, text, status, priority, completed_date, position
             FROM todo
             ORDER BY entry_date ASC, position ASC, id ASC",
        )?;
        let rows = statement.query_map([], map_todo_row)?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// 按日期读取该日全部条目，按 `position` 升序。
    pub fn list_todos_by_date(&self, entry_date: &str) -> Result<Vec<TodoRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT id, entry_date, text, status, priority, completed_date, position
             FROM todo
             WHERE entry_date = ?1
             ORDER BY position ASC, id ASC",
        )?;
        let rows = statement.query_map(params![entry_date], map_todo_row)?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// 查询所有历史日期（`< today`）且未完成的待办条目。
    pub fn list_past_incomplete_todos(&self, today: &str) -> Result<Vec<TodoRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT id, entry_date, text, status, priority, completed_date, position
             FROM todo
             WHERE entry_date < ?1 AND status != 'done'
             ORDER BY entry_date ASC, position ASC, id ASC",
        )?;
        let rows = statement.query_map(params![today], map_todo_row)?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// 写入一条（UPSERT 语义）。
    ///
    /// - 行已存在且归属日期未变：更新 `text`/`status`/`priority`/`completed_date`/`updated_at_ms`，**不动 `position`**；
    /// - 行已存在但归属日期改变：移入目标日期的 `record.position`，并重排原日期剩余位置；
    /// - 行不存在：在目标日期 `record.position` 处插入，并把同日 `position >= at` 的行顺移 +1。
    pub fn save_todo(&self, record: &TodoRecord) -> Result<()> {
        let text = normalize_text(&record.text)?;
        let entry_date = if record.entry_date.is_empty() {
            today_date()
        } else {
            record.entry_date.clone()
        };
        let completed_date = if record.status == TodoStatus::Done {
            Some(
                record
                    .completed_date
                    .clone()
                    .unwrap_or_else(|| entry_date.clone()),
            )
        } else {
            None
        };
        let transaction = self.conn.unchecked_transaction()?;
        let existing_date: Option<String> = transaction
            .query_row(
                "SELECT entry_date FROM todo WHERE id = ?1",
                params![record.id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        match existing_date {
            Some(old_date) if old_date == entry_date => {
                transaction.execute(
                    "UPDATE todo
                     SET text = ?1, status = ?2, priority = ?3, completed_date = ?4, updated_at_ms = ?5
                     WHERE id = ?6",
                    params![
                        text,
                        record.status.as_str(),
                        record.priority.as_str(),
                        completed_date,
                        now_ms(),
                        record.id.as_str()
                    ],
                )?;
            }
            Some(old_date) => {
                let at = record.position.max(0);
                shift_positions_from(&transaction, &entry_date, at)?;
                transaction.execute(
                    "UPDATE todo
                     SET entry_date = ?1, text = ?2, status = ?3, priority = ?4,
                         completed_date = ?5, position = ?6, updated_at_ms = ?7
                     WHERE id = ?8",
                    params![
                        entry_date.as_str(),
                        text,
                        record.status.as_str(),
                        record.priority.as_str(),
                        completed_date,
                        at,
                        now_ms(),
                        record.id.as_str()
                    ],
                )?;
                renumber_date(&transaction, &old_date)?;
                renumber_date(&transaction, &entry_date)?;
            }
            None => {
                let at = record.position.max(0);
                shift_positions_from(&transaction, &entry_date, at)?;
                transaction.execute(
                    "INSERT INTO todo
                         (id, entry_date, text, status, priority, completed_date, position, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        record.id.as_str(),
                        entry_date.as_str(),
                        text,
                        record.status.as_str(),
                        record.priority.as_str(),
                        completed_date,
                        at,
                        now_ms()
                    ],
                )?;
                renumber_date(&transaction, &entry_date)?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// 删除单条；`affected == 0` 不算错误（幂等）。删除后把所属日期剩余位置重排为 `0..n`。
    pub fn delete_todo(&self, id: &str) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        let entry_date: Option<String> = transaction
            .query_row(
                "SELECT entry_date FROM todo WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        transaction.execute("DELETE FROM todo WHERE id = ?1", params![id])?;
        if let Some(date) = entry_date {
            renumber_date(&transaction, &date)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 删除全部指定状态的条目，返回**被删记录**（供单槽撤销回填）。
    pub fn delete_todos_with_status(&self, status: TodoStatus) -> Result<Vec<TodoRecord>> {
        let transaction = self.conn.unchecked_transaction()?;
        let mut statement = transaction.prepare(
            "SELECT id, entry_date, text, status, priority, completed_date, position
             FROM todo
             WHERE status = ?1
             ORDER BY entry_date ASC, position ASC",
        )?;
        let rows = statement.query_map(params![status.as_str()], map_todo_row)?;
        let mut removed = Vec::new();
        let mut dates = BTreeSet::new();
        for row in rows {
            let item = row?;
            dates.insert(item.entry_date.clone());
            removed.push(item);
        }
        drop(statement);
        transaction.execute(
            "DELETE FROM todo WHERE status = ?1",
            params![status.as_str()],
        )?;
        for date in dates {
            renumber_date(&transaction, &date)?;
        }
        transaction.commit()?;
        Ok(removed)
    }

    /// 删除指定日期下全部指定状态的条目，返回被删记录。
    pub fn delete_todos_with_status_on(
        &self,
        entry_date: &str,
        status: TodoStatus,
    ) -> Result<Vec<TodoRecord>> {
        let transaction = self.conn.unchecked_transaction()?;
        let mut statement = transaction.prepare(
            "SELECT id, entry_date, text, status, priority, completed_date, position
             FROM todo
             WHERE entry_date = ?1 AND status = ?2
             ORDER BY position ASC",
        )?;
        let rows = statement.query_map(params![entry_date, status.as_str()], map_todo_row)?;
        let mut removed = Vec::new();
        for row in rows {
            removed.push(row?);
        }
        drop(statement);
        transaction.execute(
            "DELETE FROM todo WHERE entry_date = ?1 AND status = ?2",
            params![entry_date, status.as_str()],
        )?;
        renumber_date(&transaction, entry_date)?;
        transaction.commit()?;
        Ok(removed)
    }

    /// 批量恢复被删记录（撤销）：按各自归属日期与原 `position` 插回并重排为 `0..n`。
    pub fn restore_todos(&self, records: &[TodoRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut dates = BTreeSet::new();
        for record in records {
            dates.insert(if record.entry_date.is_empty() {
                today_date()
            } else {
                record.entry_date.clone()
            });
        }
        let transaction = self.conn.unchecked_transaction()?;
        for date in dates {
            let mut restored: Vec<&TodoRecord> = records
                .iter()
                .filter(|r| {
                    let r_date = if r.entry_date.is_empty() {
                        today_date()
                    } else {
                        r.entry_date.clone()
                    };
                    r_date == date
                })
                .collect();
            restored.sort_by_key(|record| record.position);
            transaction.execute(
                "UPDATE todo SET position = position + ?1 WHERE entry_date = ?2",
                params![RENUMBER_OFFSET, date.as_str()],
            )?;
            for (index, record) in restored.iter().enumerate() {
                let text = normalize_text(&record.text)?;
                let completed_date = if record.status == TodoStatus::Done {
                    Some(
                        record
                            .completed_date
                            .clone()
                            .unwrap_or_else(|| date.clone()),
                    )
                } else {
                    None
                };
                transaction.execute(
                    "INSERT OR REPLACE INTO todo
                         (id, entry_date, text, status, priority, completed_date, position, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    params![
                        record.id.as_str(),
                        date.as_str(),
                        text,
                        record.status.as_str(),
                        record.priority.as_str(),
                        completed_date,
                        RESTORE_OFFSET + index as i64,
                        now_ms()
                    ],
                )?;
            }
            let mut existing: Vec<(i64, String)> = Vec::new();
            let mut statement = transaction.prepare(
                "SELECT id, position FROM todo
                 WHERE entry_date = ?1 AND position >= ?2 AND position < ?3",
            )?;
            let rows = statement.query_map(
                params![date.as_str(), RENUMBER_OFFSET, RESTORE_OFFSET],
                |row| Ok((row.get::<_, i64>(1)?, row.get::<_, String>(0)?)),
            )?;
            for row in rows {
                existing.push(row?);
            }
            drop(statement);
            existing.sort_by_key(|(position, _)| *position);
            let mut ordered: Vec<String> = existing.into_iter().map(|(_, id)| id).collect();
            for record in &restored {
                let at = usize::try_from(record.position.max(0))
                    .unwrap_or(usize::MAX)
                    .min(ordered.len());
                ordered.insert(at, record.id.clone());
            }
            apply_positions(&transaction, &ordered)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 按指定 ID 顺序重排某日期下的条目位置。
    pub fn reorder_todos(&self, entry_date: &str, ordered_ids: &[String]) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "UPDATE todo SET position = position + ?1 WHERE entry_date = ?2",
            params![RENUMBER_OFFSET, entry_date],
        )?;
        let mut statement = transaction
            .prepare("SELECT id FROM todo WHERE entry_date = ?1 ORDER BY position ASC")?;
        let rows = statement.query_map(params![entry_date], |row| row.get::<_, String>(0))?;
        let existing: Vec<String> = rows.collect::<rusqlite::Result<_>>()?;
        drop(statement);
        let mut final_order: Vec<String> = Vec::with_capacity(existing.len());
        for id in ordered_ids {
            if existing.iter().any(|item| item == id) && !final_order.contains(id) {
                final_order.push(id.clone());
            }
        }
        for id in existing {
            if !final_order.contains(&id) {
                final_order.push(id);
            }
        }
        apply_positions(&transaction, &final_order)?;
        transaction.commit()?;
        Ok(())
    }

    /// 将指定的一批待办顺延移动到目标日期末尾，并重排源日期与目标日期的位置。
    pub fn rollover_todos_to_date(&self, ids: &[String], target_date: &str) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let transaction = self.conn.unchecked_transaction()?;
        let mut source_dates = BTreeSet::new();
        for (offset, id) in ids.iter().enumerate() {
            let old_date: Option<String> = transaction
                .query_row(
                    "SELECT entry_date FROM todo WHERE id = ?1",
                    params![id.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(old) = old_date
                && old != target_date
            {
                source_dates.insert(old);
                transaction.execute(
                    "UPDATE todo
                     SET entry_date = ?1, position = ?2, updated_at_ms = ?3
                     WHERE id = ?4",
                    params![
                        target_date,
                        RESTORE_OFFSET + offset as i64,
                        now_ms(),
                        id.as_str()
                    ],
                )?;
            }
        }
        for old_date in source_dates {
            renumber_date(&transaction, &old_date)?;
        }
        renumber_date(&transaction, target_date)?;
        transaction.commit()?;
        Ok(())
    }

    /// 查询闭区间 `[start_date, end_date]` 内每日的计划数与已完成数。
    pub fn query_range_stats(&self, start_date: &str, end_date: &str) -> Result<Vec<TodoDayStats>> {
        let mut statement = self.conn.prepare(
            "SELECT entry_date,
                    COUNT(*),
                    SUM(CASE WHEN status = 'done' THEN 1 ELSE 0 END)
             FROM todo
             WHERE entry_date >= ?1 AND entry_date <= ?2
             GROUP BY entry_date
             ORDER BY entry_date ASC",
        )?;
        let rows = statement.query_map(params![start_date, end_date], |row| {
            let planned: i64 = row.get(1)?;
            let completed: i64 = row.get(2)?;
            Ok(TodoDayStats {
                date: row.get(0)?,
                planned_count: usize::try_from(planned.max(0)).unwrap_or(0),
                completed_count: usize::try_from(completed.max(0)).unwrap_or(0),
            })
        })?;
        let mut stats = Vec::new();
        for row in rows {
            stats.push(row?);
        }
        Ok(stats)
    }

    /// 条目总数（测试与诊断用）。
    pub fn todo_count(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM todo", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// 读取某条 todo（测试与诊断用）。
    pub fn todo_by_id(&self, id: &str) -> Result<Option<TodoRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT id, entry_date, text, status, priority, completed_date, position
             FROM todo
             WHERE id = ?1",
        )?;
        let mut rows = statement.query_map(params![id], map_todo_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }
}

/// 文本入库前校验并规范化：trim 后为空拒绝，超过上限拒绝；返回 trim 后的文本。
fn normalize_text(text: &str) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(DbError::InvalidDraft);
    }
    if trimmed.chars().count() > MAX_TODO_TEXT_CHARS {
        return Err(DbError::MessageTooLarge);
    }
    Ok(trimmed.to_string())
}

/// 把指定日期下 `position >= at` 的行整体顺移 +1（**从大到小**逐条更新）。
fn shift_positions_from(
    transaction: &rusqlite::Transaction<'_>,
    entry_date: &str,
    at: i64,
) -> Result<()> {
    let mut statement = transaction.prepare(
        "SELECT id FROM todo WHERE entry_date = ?1 AND position >= ?2 ORDER BY position DESC",
    )?;
    let rows = statement.query_map(params![entry_date, at], |row| row.get::<_, String>(0))?;
    let ids: Vec<String> = rows.collect::<rusqlite::Result<_>>()?;
    drop(statement);
    for id in ids {
        transaction.execute(
            "UPDATE todo SET position = position + 1 WHERE id = ?1",
            params![id],
        )?;
    }
    Ok(())
}

/// 把某日期下的位置重排为 `0..n`（保持现有相对顺序）。
fn renumber_date(transaction: &rusqlite::Transaction<'_>, entry_date: &str) -> Result<()> {
    transaction.execute(
        "UPDATE todo SET position = position + ?1 WHERE entry_date = ?2 AND position < ?1",
        params![RENUMBER_OFFSET, entry_date],
    )?;
    let mut statement =
        transaction.prepare("SELECT id FROM todo WHERE entry_date = ?1 ORDER BY position ASC")?;
    let rows = statement.query_map(params![entry_date], |row| row.get::<_, String>(0))?;
    let ordered: Vec<String> = rows.collect::<rusqlite::Result<_>>()?;
    drop(statement);
    apply_positions(transaction, &ordered)
}

/// 按给定顺序写入 0 基位置。
fn apply_positions(transaction: &rusqlite::Transaction<'_>, ordered: &[String]) -> Result<()> {
    for (index, id) in ordered.iter().enumerate() {
        transaction.execute(
            "UPDATE todo SET position = ?1 WHERE id = ?2",
            params![index as i64, id.as_str()],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

//! 全局 todo 列表的持久化：读取、UPSERT 写入、删除与撤销恢复。
//!
//! 表结构见 `migration::SCHEMA_V15_TODO`。列表是**全局**的（跨工作区、跨会话共享），
//! 因此这里不接收 workspace 或 session 参数——与 `prompt_history` 同一口径。
//!
//! `position` 是密集递增排序位（0 基、无空洞），由写操作在单事务内维护；
//! `id` 是稳定业务键，UPSERT 与删除都以它为准。

use rusqlite::{OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, now_ms};

/// 单条 todo 文本的字符上限；与 lx-agent 的 `content: 1..500` 一致。
///
/// 按**字符**而非字节计数：500 个汉字是 1500 字节，按字节判定会把中文拦在 167 字。
const MAX_TODO_TEXT_CHARS: usize = 500;

/// 重排位置时使用的临时偏移；所有行先移进该区间，再落回 `0..n`，
/// 避免中途撞 `idx_todo_position` 唯一索引。
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

/// 一条 todo；内存视图与库记录同构。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoRecord {
    /// 稳定 id（`todo-<nanos>-<seq>`），用于 UPSERT 与删除。
    pub id: String,
    pub text: String,
    pub status: TodoStatus,
    /// 密集递增排序位（0 基）。
    pub position: i64,
}

impl TodoRecord {
    /// 新建一条待办；`position` 由调用方按插入点给出。
    pub fn new(text: impl Into<String>, status: TodoStatus, position: i64) -> Self {
        Self {
            id: new_external_id("todo"),
            text: text.into(),
            status,
            position,
        }
    }
}

impl Database {
    /// 读取全表，按 `position` 升序。
    pub fn list_todos(&self) -> Result<Vec<TodoRecord>> {
        let mut statement = self
            .conn
            .prepare("SELECT id, text, status, position FROM todo ORDER BY position ASC, id ASC")?;
        let rows = statement.query_map([], |row| {
            Ok(TodoRecord {
                id: row.get(0)?,
                text: row.get(1)?,
                status: TodoStatus::from_db(&row.get::<_, String>(2)?),
                position: row.get(3)?,
            })
        })?;
        let mut items = Vec::new();
        for row in rows {
            items.push(row?);
        }
        Ok(items)
    }

    /// 写入一条（UPSERT 语义）。
    ///
    /// - 行已存在：只更新 `text` / `status` / `updated_at_ms`，**不动 `position`**；
    /// - 行不存在：在 `record.position` 处插入，并把 `position >= at` 的行顺移 +1。
    ///
    /// 同一条目重复写入（改文本、切状态）幂等；「插入曾失败、之后再编辑」也会因为
    /// 走插入分支而自愈，不会留下一条库里根本不存在的幽灵行。
    pub fn save_todo(&self, record: &TodoRecord) -> Result<()> {
        let text = normalize_text(&record.text)?;
        let transaction = self.conn.unchecked_transaction()?;
        let exists = transaction
            .query_row(
                "SELECT 1 FROM todo WHERE id = ?1",
                params![record.id],
                |_| Ok(true),
            )
            .optional()?
            .is_some();
        if exists {
            transaction.execute(
                "UPDATE todo SET text = ?1, status = ?2, updated_at_ms = ?3 WHERE id = ?4",
                params![text, record.status.as_str(), now_ms(), record.id.as_str()],
            )?;
        } else {
            let at = record.position.max(0);
            shift_positions_from(&transaction, at)?;
            transaction.execute(
                "INSERT INTO todo (id, text, status, position, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![
                    record.id.as_str(),
                    text,
                    record.status.as_str(),
                    at,
                    now_ms()
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 删除单条；`affected == 0` 不算错误（幂等）。删除后把剩余位置重排为 `0..n`。
    pub fn delete_todo(&self, id: &str) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute("DELETE FROM todo WHERE id = ?1", params![id])?;
        renumber(&transaction)?;
        transaction.commit()?;
        Ok(())
    }

    /// 删除全部指定状态的条目，返回**被删记录**（供单槽撤销回填）。
    ///
    /// 删完后把剩余位置重排为 `0..n`；返回的记录保持原相对顺序。
    pub fn delete_todos_with_status(&self, status: TodoStatus) -> Result<Vec<TodoRecord>> {
        let transaction = self.conn.unchecked_transaction()?;
        let mut statement = transaction.prepare(
            "SELECT id, text, status, position FROM todo WHERE status = ?1 ORDER BY position ASC",
        )?;
        let rows = statement.query_map(params![status.as_str()], |row| {
            Ok(TodoRecord {
                id: row.get(0)?,
                text: row.get(1)?,
                status: TodoStatus::from_db(&row.get::<_, String>(2)?),
                position: row.get(3)?,
            })
        })?;
        let mut removed = Vec::new();
        for row in rows {
            removed.push(row?);
        }
        drop(statement);
        transaction.execute(
            "DELETE FROM todo WHERE status = ?1",
            params![status.as_str()],
        )?;
        renumber(&transaction)?;
        transaction.commit()?;
        Ok(removed)
    }

    /// 批量恢复被删记录（撤销）：按各自原 `position` 插回并重排为 `0..n`。
    pub fn restore_todos(&self, records: &[TodoRecord]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let mut restored: Vec<&TodoRecord> = records.iter().collect();
        restored.sort_by_key(|record| record.position);
        let transaction = self.conn.unchecked_transaction()?;
        // 现有行整体移进区间 A，恢复行先落在区间 B：两边都保持各自相对顺序，
        // 且与 `0..n` 的最终下标不重叠，重排时才不会撞唯一索引。
        transaction.execute(
            "UPDATE todo SET position = position + ?1",
            params![RENUMBER_OFFSET],
        )?;
        for (index, record) in restored.iter().enumerate() {
            let text = normalize_text(&record.text)?;
            transaction.execute(
                "INSERT OR REPLACE INTO todo
                     (id, text, status, position, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![
                    record.id.as_str(),
                    text,
                    record.status.as_str(),
                    RESTORE_OFFSET + index as i64,
                    now_ms()
                ],
            )?;
        }
        // 合并顺序：现有行按 `position` 升序，恢复行按各自原 `position` 插回。
        // 恢复行落在区间 B（`RESTORE_OFFSET` 起），现有行在区间 A：两者都 >= `RENUMBER_OFFSET`，
        // 所以这里必须用开区间 `[RENUMBER_OFFSET, RESTORE_OFFSET)` 把恢复行排除掉。
        let mut existing: Vec<(i64, String)> = Vec::new();
        let mut statement = transaction
            .prepare("SELECT id, position FROM todo WHERE position >= ?1 AND position < ?2")?;
        let rows = statement.query_map(params![RENUMBER_OFFSET, RESTORE_OFFSET], |row| {
            Ok((row.get::<_, i64>(1)?, row.get::<_, String>(0)?))
        })?;
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
        transaction.commit()?;
        Ok(())
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
        let mut statement = self
            .conn
            .prepare("SELECT id, text, status, position FROM todo WHERE id = ?1")?;
        let mut rows = statement.query_map(params![id], |row| {
            Ok(TodoRecord {
                id: row.get(0)?,
                text: row.get(1)?,
                status: TodoStatus::from_db(&row.get::<_, String>(2)?),
                position: row.get(3)?,
            })
        })?;
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

/// 把 `position >= at` 的行整体顺移 +1（**从大到小**逐条更新）。
///
/// 不能用 `UPDATE ... ORDER BY`：那需要 `SQLITE_ENABLE_UPDATE_DELETE_LIMIT` 编译选项。
/// 降序推进则保证每条都落进「刚刚腾出来」的位置，中间态永不撞 `idx_todo_position`。
fn shift_positions_from(transaction: &rusqlite::Transaction<'_>, at: i64) -> Result<()> {
    let mut statement =
        transaction.prepare("SELECT id FROM todo WHERE position >= ?1 ORDER BY position DESC")?;
    let rows = statement.query_map(params![at], |row| row.get::<_, String>(0))?;
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

/// 把全表位置重排为 `0..n`（保持现有相对顺序）。
fn renumber(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    let mut statement = transaction.prepare("SELECT id FROM todo ORDER BY position ASC")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let ordered: Vec<String> = rows.collect::<rusqlite::Result<_>>()?;
    drop(statement);
    apply_positions(transaction, &ordered)
}

/// 按给定顺序写入 0 基位置。
///
/// 调用方必须保证当前所有位置都远离 `0..n`（例如先整体加上 [`RENUMBER_OFFSET`]），
/// 否则先写的行会占住后面还没腾出来的位置，触发唯一索引冲突。
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

//! 工具调用持久化：pending 落库、running 推进、终态结算、按会话载入与异常恢复。
//!
//! 状态机 `pending -> running -> completed | failed | cancelled`，异常退出后
//! `pending/running -> interrupted`。所有写操作以 `workspace + session + message + call_id`
//! 四条件校验归属；重复 ack 幂等、冲突 ack 返回可区分错误，绝不静默忽略。
//! `begin_tool_call` 在同一事务内分配时间线位置，事务提交后才发出 ticket。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, now_ms};

/// 工具参数 JSON 单次上限（1 MiB）：防止 SSE 参数片段无限堆积撑爆内存与库。
pub const MAX_TOOL_ARGUMENT_BYTES: usize = 1024 * 1024;

/// 工具结果 JSON 单次上限（1 MiB）：结果全文只进持久层与模型上下文。
pub const MAX_TOOL_RESULT_BYTES: usize = 1024 * 1024;

/// 异常退出恢复时写入的稳定中断码（`error_text`）；不是用户可见文案。
pub const INTERRUPTED_ERROR_CODE: &str = "interrupted";

/// 工具调用记录（加载与回放用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallRecord {
    pub call_id: String,
    pub message_id: String,
    pub name: String,
    pub arguments_json: String,
    pub position: i64,
    /// pending / running / completed / failed / cancelled / interrupted
    pub status: String,
    pub result_json: Option<String>,
    pub error_text: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub duration_ms: Option<i64>,
}

/// pending 落库成功后的执行票据；只有持有 ticket 才能推进该次调用。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallTicket {
    pub session_id: String,
    pub message_id: String,
    pub call_id: String,
    pub position: i64,
    pub started_at_ms: i64,
}

/// 工具终态（`interrupted` 只能由启动恢复写入，不在调用方可选集合内）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolTerminalStatus {
    Completed,
    Failed,
    Cancelled,
}

impl ToolTerminalStatus {
    /// 存储用稳定值。
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// 一次工具执行的结构化结局；结果体为空表示无回执（如取消）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCallResult {
    pub status: ToolTerminalStatus,
    /// 规范化结果 JSON（成功/失败均可有，用于回放与 UI 摘要）；None 表示无结果体。
    pub result_json: Option<String>,
    pub error_text: Option<String>,
}

impl ToolCallResult {
    /// 便捷构造成功结果。
    pub fn completed(result_json: Option<String>) -> Self {
        Self {
            status: ToolTerminalStatus::Completed,
            result_json,
            error_text: None,
        }
    }

    /// 便捷构造失败结果；`error_text` 为给模型的精简原因。
    pub fn failed(error_text: impl Into<String>) -> Self {
        Self {
            status: ToolTerminalStatus::Failed,
            result_json: None,
            error_text: Some(error_text.into()),
        }
    }
}

impl Database {
    /// 记录一次工具调用的 pending 行并分配时间线位置；提交成功后才返回 ticket。
    ///
    /// 位置取该消息内 parts 与 tools 的 `MAX(position) + 1`（无行则为 0）；若与既有行
    /// 冲突返回 `PositionConflict`。同会话 call ID 重复返回 `ToolCallConflict`；
    /// 参数超过 1 MiB 返回 `ToolArgumentTooLarge`。会话或消息不属于该工作区时分别返回
    /// `SessionMissing` / `ToolCallMissing`。
    pub fn begin_tool_call(
        &self,
        workspace: &Path,
        session_id: &str,
        message_id: &str,
        call_id: &str,
        name: &str,
        arguments_json: &str,
    ) -> Result<ToolCallTicket> {
        if arguments_json.len() > MAX_TOOL_ARGUMENT_BYTES {
            return Err(DbError::ToolArgumentTooLarge);
        }
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let started_at_ms = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        ensure_owned_message(&transaction, &workspace_external, session_id, message_id)?;
        let duplicate: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_tool_call WHERE session_id = ?1 AND call_id = ?2",
                params![session_id, call_id],
                |row| row.get(0),
            )
            .optional()?;
        if duplicate.is_some() {
            return Err(DbError::ToolCallConflict);
        }
        let position: i64 = transaction.query_row(
            r#"SELECT COALESCE(MAX(position) + 1, 0) FROM (
                   SELECT position FROM agent_message_part WHERE message_id = ?1
                   UNION ALL
                   SELECT position FROM agent_tool_call WHERE message_id = ?1
               )"#,
            params![message_id],
            |row| row.get(0),
        )?;
        // 防御：分配值已被任一表占用时不修正位置，交由调用方中止该工具。
        let taken: Option<i64> = transaction
            .query_row(
                r#"SELECT 1 FROM agent_message_part WHERE message_id = ?1 AND position = ?2
                   UNION ALL
                   SELECT 1 FROM agent_tool_call WHERE message_id = ?1 AND position = ?2"#,
                params![message_id, position],
                |row| row.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Err(DbError::PositionConflict);
        }
        transaction
            .execute(
                r#"INSERT INTO agent_tool_call
                   (external_id, session_id, message_id, call_id, name, arguments_json,
                    position, status, started_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8)"#,
                params![
                    new_external_id("tool"),
                    session_id,
                    message_id,
                    call_id,
                    name,
                    arguments_json,
                    position,
                    started_at_ms,
                ],
            )
            .map_err(classify_insert_error)?;
        transaction.commit()?;
        Ok(ToolCallTicket {
            session_id: session_id.to_string(),
            message_id: message_id.to_string(),
            call_id: call_id.to_string(),
            position,
            started_at_ms,
        })
    }

    /// pending -> running；已在 running 时幂等返回 Ok。
    ///
    /// 行不存在返回 `ToolCallMissing`，其他状态（终态或 interrupted）返回
    /// `InvalidToolTransition`。调用方必须在本方法成功后才能执行文件操作。
    pub fn mark_tool_running(&self, workspace: &Path, ticket: &ToolCallTicket) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let updated = transaction.execute(
            r#"UPDATE agent_tool_call SET status = 'running'
               WHERE session_id = ?1 AND message_id = ?2 AND call_id = ?3 AND status = 'pending'"#,
            params![ticket.session_id, ticket.message_id, ticket.call_id],
        )?;
        if updated == 0 {
            let status = tool_call_status(&transaction, &workspace_external, ticket)?;
            // 幂等路径没有写入，直接结束事务（drop 即回滚，不留变化）。
            return match status.as_deref() {
                None => Err(DbError::ToolCallMissing),
                Some("running") => Ok(()),
                Some(_) => Err(DbError::InvalidToolTransition),
            };
        }
        transaction.commit()?;
        Ok(())
    }

    /// 结算工具调用：running -> 终态；`cancelled` 额外允许 pending -> cancelled
    /// （工具尚未开始执行即被用户取消）。
    ///
    /// 同 ticket 相同结果重复提交幂等，不同结果返回 `ToolTerminalConflict`。
    /// 结果体超过 1 MiB 返回 `ToolResultTooLarge` 且不写半行；行不在可转移状态且不是
    /// 相同终态时返回 `InvalidToolTransition` / `ToolCallMissing`。
    pub fn finish_tool_call(
        &self,
        workspace: &Path,
        ticket: &ToolCallTicket,
        result: &ToolCallResult,
    ) -> Result<()> {
        if result
            .result_json
            .as_ref()
            .is_some_and(|body| body.len() > MAX_TOOL_RESULT_BYTES)
        {
            return Err(DbError::ToolResultTooLarge);
        }
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let duration_ms = now.saturating_sub(ticket.started_at_ms).max(0);
        let mut updated = transaction.execute(
            r#"UPDATE agent_tool_call
               SET status = ?1, result_json = ?2, error_text = ?3,
                   completed_at_ms = ?4, duration_ms = ?5
               WHERE session_id = ?6 AND message_id = ?7 AND call_id = ?8 AND status = 'running'"#,
            params![
                result.status.as_str(),
                result.result_json,
                result.error_text,
                now,
                duration_ms,
                ticket.session_id,
                ticket.message_id,
                ticket.call_id,
            ],
        )?;
        if updated == 0 && result.status == ToolTerminalStatus::Cancelled {
            // 尚未开始执行的调用可直接取消（pending -> cancelled）。
            updated = transaction.execute(
                r#"UPDATE agent_tool_call
                   SET status = 'cancelled', result_json = ?1, error_text = ?2,
                       completed_at_ms = ?3, duration_ms = ?4
                   WHERE session_id = ?5 AND message_id = ?6 AND call_id = ?7
                     AND status = 'pending'"#,
                params![
                    result.result_json,
                    result.error_text,
                    now,
                    duration_ms,
                    ticket.session_id,
                    ticket.message_id,
                    ticket.call_id,
                ],
            )?;
        }
        if updated == 0 {
            return settle_without_update(&transaction, &workspace_external, ticket, result);
        }
        transaction.commit()?;
        Ok(())
    }

    /// 某会话的全部工具调用（单次查询，按消息 `sequence` 再按 `position` 有序）。
    ///
    /// 会话不属于该工作区或不存在时返回 `SessionMissing`。
    pub fn load_tool_calls_for_session(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Vec<ToolCallRecord>> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let mut statement = self.conn.prepare(
            r#"SELECT t.call_id, t.message_id, t.name, t.arguments_json, t.position, t.status,
                      t.result_json, t.error_text, t.started_at_ms, t.completed_at_ms, t.duration_ms
               FROM agent_tool_call t
               JOIN agent_message m ON m.external_id = t.message_id
               WHERE t.session_id = ?1
               ORDER BY m.sequence, t.position"#,
        )?;
        let rows = statement.query_map(params![session_id], read_tool_call_record)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 启动恢复：遗留 pending/running 全部改 interrupted（不自动重跑、不伪造结果）。
    ///
    /// 幂等：第二次调用返回 0。`completed_at_ms` 与 `duration_ms` 按当前时刻补齐。
    pub fn recover_interrupted_tool_calls(&self, sessions: &[String]) -> Result<usize> {
        let now = now_ms();
        let mut updated = 0usize;
        for session in sessions {
            updated += self.conn.execute(
                r#"UPDATE agent_tool_call
                   SET status = 'interrupted', error_text = ?1,
                       completed_at_ms = ?2,
                       duration_ms = MAX(?2 - started_at_ms, 0)
                   WHERE status IN ('pending', 'running') AND session_id = ?3"#,
                params![INTERRUPTED_ERROR_CODE, now, session],
            )?;
        }
        Ok(updated)
    }
}

/// 列名顺序固定：call_id, message_id, name, arguments_json, position, status,
/// result_json, error_text, started_at_ms, completed_at_ms, duration_ms。
pub(super) fn read_tool_call_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolCallRecord> {
    Ok(ToolCallRecord {
        call_id: row.get(0)?,
        message_id: row.get(1)?,
        name: row.get(2)?,
        arguments_json: row.get(3)?,
        position: row.get(4)?,
        status: row.get(5)?,
        result_json: row.get(6)?,
        error_text: row.get(7)?,
        started_at_ms: row.get(8)?,
        completed_at_ms: row.get(9)?,
        duration_ms: row.get(10)?,
    })
}

/// 唯一约束冲突转换为可区分错误（重复 call ID / 位置被占），其余 SQLite 错误原样上抛。
fn classify_insert_error(error: rusqlite::Error) -> DbError {
    let message = error.to_string();
    if message.contains("UNIQUE constraint failed") {
        if message.contains("position") {
            return DbError::PositionConflict;
        }
        if message.contains("call_id") {
            return DbError::ToolCallConflict;
        }
    }
    DbError::Sqlite(error)
}

/// 校验会话属于该工作区且消息属于该会话；不满足时返回可区分错误。
fn ensure_owned_message(
    transaction: &Connection,
    workspace_external: &str,
    session_id: &str,
    message_id: &str,
) -> Result<()> {
    let session: Option<i64> = transaction
        .query_row(
            "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
            params![session_id, workspace_external],
            |row| row.get(0),
        )
        .optional()?;
    if session.is_none() {
        return Err(DbError::SessionMissing);
    }
    let message: Option<i64> = transaction
        .query_row(
            "SELECT id FROM agent_message WHERE external_id = ?1 AND session_id = ?2",
            params![message_id, session_id],
            |row| row.get(0),
        )
        .optional()?;
    if message.is_none() {
        return Err(DbError::ToolCallMissing);
    }
    Ok(())
}

/// 读取工具调用当前状态（带工作区归属校验）；不存在或跨区时 None。
fn tool_call_status(
    transaction: &Connection,
    workspace_external: &str,
    ticket: &ToolCallTicket,
) -> Result<Option<String>> {
    Ok(transaction
        .query_row(
            r#"SELECT t.status FROM agent_tool_call t
               JOIN agent_session s ON s.external_id = t.session_id
               WHERE t.session_id = ?1 AND t.message_id = ?2 AND t.call_id = ?3
                 AND s.workspace_id = ?4"#,
            params![
                ticket.session_id,
                ticket.message_id,
                ticket.call_id,
                workspace_external
            ],
            |row| row.get(0),
        )
        .optional()?)
}

/// 条件更新未命中时解释原因：相同终态幂等 Ok，不同终态/非法转移返回错误。
fn settle_without_update(
    transaction: &Connection,
    workspace_external: &str,
    ticket: &ToolCallTicket,
    result: &ToolCallResult,
) -> Result<()> {
    let existing: Option<(String, Option<String>, Option<String>)> = transaction
        .query_row(
            r#"SELECT t.status, t.result_json, t.error_text FROM agent_tool_call t
               JOIN agent_session s ON s.external_id = t.session_id
               WHERE t.session_id = ?1 AND t.message_id = ?2 AND t.call_id = ?3
                 AND s.workspace_id = ?4"#,
            params![
                ticket.session_id,
                ticket.message_id,
                ticket.call_id,
                workspace_external
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((status, stored_result, stored_error)) = existing else {
        return Err(DbError::ToolCallMissing);
    };
    match status.as_str() {
        "completed" | "failed" | "cancelled" | "interrupted" => {
            let same = status == result.status.as_str()
                && stored_result == result.result_json
                && stored_error == result.error_text;
            if same {
                Ok(())
            } else {
                Err(DbError::ToolTerminalConflict)
            }
        }
        _ => Err(DbError::InvalidToolTransition),
    }
}

#[cfg(test)]
mod tests;

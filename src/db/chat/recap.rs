//! 每条 assistant 消息的回顾（recap）持久化：写入/覆盖与按会话批量加载。
//!
//! 回顾是旁路产物：不进入模型上下文、不占时间线 sequence、不递增会话 revision；
//! 每条 assistant 消息最多一条（`message_id UNIQUE`），消息删除时级联清理。

use std::path::Path;

use rusqlite::{OptionalExtension, Row, params};

use super::{Database, DbError, ModelSnapshot, Result, new_external_id, now_ms};

/// 一条已持久化的消息回顾。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecap {
    pub id: String,
    pub message_id: String,
    pub summary: String,
    pub next_action: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub variant: Option<String>,
    pub model_label: String,
    pub end_turn: i64,
    pub end_sequence: i64,
    pub created_at_ms: i64,
}

/// 一次回顾写入请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecapCommit {
    pub session_id: String,
    pub message_id: String,
    pub summary: String,
    pub next_action: String,
    pub model: ModelSnapshot,
    pub model_label: String,
    pub end_turn: i64,
    pub end_sequence: i64,
}

impl Database {
    /// 写入或覆盖一条消息回顾（同 message_id 只保留最新一份）。
    pub fn commit_recap(&self, workspace: &Path, commit: &RecapCommit) -> Result<MessageRecap> {
        if commit.summary.trim().is_empty() {
            return Err(DbError::InvalidDraft);
        }
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![commit.session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let role: Option<String> = transaction
            .query_row(
                "SELECT role FROM agent_message WHERE external_id = ?1 AND session_id = ?2",
                params![commit.message_id, commit.session_id],
                |row| row.get(0),
            )
            .optional()?;
        if role.as_deref() != Some("assistant") {
            return Err(DbError::RecapMessageMissing);
        }

        let now = now_ms();
        let external_id = new_external_id("recap");
        transaction.execute(
            r#"INSERT INTO agent_message_recap
                   (external_id, session_id, message_id, summary_text, next_action,
                    model_provider, model_id, variant, model_label, end_turn,
                    end_sequence, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
               ON CONFLICT(message_id) DO UPDATE SET
                   summary_text = excluded.summary_text,
                   next_action = excluded.next_action,
                   model_provider = excluded.model_provider,
                   model_id = excluded.model_id,
                   variant = excluded.variant,
                   model_label = excluded.model_label,
                   end_turn = excluded.end_turn,
                   end_sequence = excluded.end_sequence,
                   created_at_ms = excluded.created_at_ms"#,
            params![
                external_id,
                commit.session_id,
                commit.message_id,
                commit.summary,
                commit.next_action,
                commit.model.provider,
                commit.model.model,
                commit.model.variant,
                commit.model_label,
                commit.end_turn,
                commit.end_sequence,
                now,
            ],
        )?;
        let recap = transaction.query_row(
            r#"SELECT external_id, message_id, summary_text, next_action,
                      model_provider, model_id, variant, model_label, end_turn,
                      end_sequence, created_at_ms
               FROM agent_message_recap
               WHERE message_id = ?1"#,
            params![commit.message_id],
            read_recap,
        )?;
        transaction.commit()?;
        Ok(recap)
    }

    /// 删除一条消息回顾（只删 recap，不动消息），并推进会话 revision。
    pub fn delete_recap(
        &self,
        workspace: &Path,
        session_id: &str,
        message_id: &str,
        expected_revision: i64,
        operation_id: &str,
    ) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM agent_session WHERE external_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        if revision != expected_revision {
            return Err(DbError::RewindTargetChanged);
        }
        let deleted = transaction.execute(
            "DELETE FROM agent_message_recap WHERE session_id = ?1 AND message_id = ?2",
            params![session_id, message_id],
        )?;
        if deleted == 0 {
            return Err(DbError::RewindTargetChanged);
        }
        let now = now_ms();
        transaction.execute(
            "UPDATE agent_session SET revision = revision + 1, updated_at_ms = ?1 WHERE external_id = ?2",
            params![now, session_id],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_rewind_operation
                   (external_id, operation_id, session_id, action, target_user_message_id,
                    expected_revision, phase, user_text, created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, 'delete_recap', ?4, ?5, 'committed', '', ?6, ?6)"#,
            params![
                new_external_id("op"),
                operation_id,
                session_id,
                message_id,
                expected_revision,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 按会话批量加载全部回顾（双条件校验工作区归属）。
    pub fn load_recaps(&self, workspace: &Path, session_id: &str) -> Result<Vec<MessageRecap>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(
            r#"SELECT external_id, message_id, summary_text, next_action,
                      model_provider, model_id, variant, model_label, end_turn,
                      end_sequence, created_at_ms
               FROM agent_message_recap
               WHERE session_id = ?1
               ORDER BY created_at_ms ASC, id ASC"#,
        )?;
        let rows = statement.query_map(params![session_id], read_recap)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }
}

fn read_recap(row: &Row<'_>) -> rusqlite::Result<MessageRecap> {
    Ok(MessageRecap {
        id: row.get(0)?,
        message_id: row.get(1)?,
        summary: row.get(2)?,
        next_action: row.get(3)?,
        model_provider: row.get(4)?,
        model_id: row.get(5)?,
        variant: row.get(6)?,
        model_label: row.get(7)?,
        end_turn: row.get(8)?,
        end_sequence: row.get(9)?,
        created_at_ms: row.get(10)?,
    })
}

#[cfg(test)]
mod tests;

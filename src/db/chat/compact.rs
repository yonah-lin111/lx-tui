//! 上下文压缩（`/compact`）的持久化：检查点、生效指针与撤销。
//!
//! 分层约束（对应 design §3.1 与 §6）：
//! - 只接触 rusqlite，不发起网络请求、不读文件；
//! - 检查点与撤销共用 `agent_session.revision`：提交时比对期望 revision，变化即
//!   `RewindTargetChanged`（零写入）；
//! - 原始消息只追加、不删除：撤销只切换 `state` 与 `reverted_by_operation_id`；
//! - `sequence` 与消息、撤销提示共用编号空间（时间线合并的前提），只在同一事务内取号。
//!
//! 「active 指针」不是新表：它就是 `state='active'` 的最新检查点。装配上下文取
//! 最新 active 检查点的 `end_sequence` 作为起点，因此撤销后自然回退到上一个 active。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, now_ms};

/// 检查点状态：生效 / 已撤销。
pub const COMPACT_ACTIVE: &str = "active";
pub const COMPACT_REVERTED: &str = "reverted";

/// 一次压缩的完整记录（装配上下文、时间线与撤销都读它）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactCheckpoint {
    pub id: String,
    pub sequence: i64,
    /// 幂等键：同一次压缩的重复完成回执只写一次。
    pub operation_id: String,
    /// 增量摘要的上一个生效检查点（首个检查点为 None）。
    pub base_checkpoint_id: Option<String>,
    /// 摘要覆盖的原始消息 range（含两端）。
    pub start_sequence: i64,
    pub end_sequence: i64,
    pub summary_text: String,
    /// 实际摘要模型的显示名（必须来自真实模型快照，不能是占位）。
    pub model_label: String,
    pub state: String,
    pub created_at_ms: i64,
}

impl CompactCheckpoint {
    /// 是否仍生效（装配上下文只使用 active）。
    pub fn is_active(&self) -> bool {
        self.state == COMPACT_ACTIVE
    }
}

/// 一次成功压缩的提交参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactCommit {
    pub session_id: String,
    /// 发起压缩时冻结的会话 revision。
    pub expected_revision: i64,
    /// 增量摘要的上一个生效检查点。
    pub base_checkpoint_id: Option<String>,
    pub start_sequence: i64,
    pub end_sequence: i64,
    pub summary_text: String,
    pub model_label: String,
    /// 幂等键：同一 id 重复提交只返回既有检查点，不重复取号/写入。
    pub operation_id: String,
}

/// 提交结果（幂等重放时 `created = false`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactCommitted {
    pub checkpoint: CompactCheckpoint,
    pub created: bool,
}

/// 一次 compact 撤销：按检查点 id + 期望 revision 冻结。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactRevert {
    pub operation_id: String,
    pub session_id: String,
    pub checkpoint_id: String,
    pub expected_revision: i64,
}

impl Database {
    /// 会话内仍生效的最新检查点（装配上下文与增量范围都用它）。
    pub fn active_checkpoint(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Option<CompactCheckpoint>> {
        if self.owned_session(workspace, session_id)?.is_none() {
            return Ok(None);
        }
        self.conn
            .query_row(
                r#"SELECT external_id, sequence, operation_id, base_checkpoint_id, start_sequence,
                          end_sequence, summary_text, model_label, state, created_at_ms
                   FROM agent_compact_checkpoint
                   WHERE session_id = ?1 AND state = 'active'
                   ORDER BY sequence DESC LIMIT 1"#,
                params![session_id],
                read_checkpoint,
            )
            .optional()
            .map_err(DbError::from)
    }

    /// 会话内全部检查点（按 sequence 升序；时间线渲染要含已撤销的条目）。
    pub fn compact_checkpoints(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Vec<CompactCheckpoint>> {
        if self.owned_session(workspace, session_id)?.is_none() {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(
            r#"SELECT external_id, sequence, operation_id, base_checkpoint_id, start_sequence,
                      end_sequence, summary_text, model_label, state, created_at_ms
               FROM agent_compact_checkpoint
               WHERE session_id = ?1
               ORDER BY sequence"#,
        )?;
        let rows = statement.query_map(params![session_id], read_checkpoint)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 提交一次压缩：检查点写入 + revision 推进，同一事务完成。
    ///
    /// 幂等：同 `operation_id` 已存在时直接返回既有检查点（`created = false`）。
    /// 期望 revision 不符、目标范围非法或摘要为空时零写入。
    pub fn commit_compact(
        &self,
        workspace: &Path,
        commit: &CompactCommit,
    ) -> Result<CompactCommitted> {
        if commit.summary_text.trim().is_empty() {
            return Err(DbError::InvalidDraft);
        }
        if commit.end_sequence < commit.start_sequence {
            return Err(DbError::PositionConflict);
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
        // 幂等重放：同一 operation_id 视为同一次压缩。
        if let Some(existing) = checkpoint_by_operation(&transaction, &commit.operation_id)? {
            transaction.commit()?;
            return Ok(CompactCommitted {
                checkpoint: existing,
                created: false,
            });
        }
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM agent_session WHERE external_id = ?1",
            params![commit.session_id],
            |row| row.get(0),
        )?;
        if revision != commit.expected_revision {
            return Err(DbError::RewindTargetChanged);
        }
        // 取号：与消息、撤销提示共用同一空间。
        let sequence: i64 = transaction.query_row(
            r#"SELECT COALESCE(MAX(sequence) + 1, 0) FROM (
                   SELECT sequence FROM agent_message WHERE session_id = ?1
                   UNION ALL
                   SELECT sequence FROM agent_session_notice WHERE session_id = ?1
                   UNION ALL
                   SELECT sequence FROM agent_compact_checkpoint WHERE session_id = ?1
               )"#,
            params![commit.session_id],
            |row| row.get(0),
        )?;
        let now = now_ms();
        let id = new_external_id("compact");
        transaction.execute(
            r#"INSERT INTO agent_compact_checkpoint
                   (external_id, session_id, sequence, operation_id, base_checkpoint_id,
                    start_sequence, end_sequence, summary_text, model_label, state,
                    reverted_by_operation_id, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', NULL, ?10)"#,
            params![
                id,
                commit.session_id,
                sequence,
                commit.operation_id,
                commit.base_checkpoint_id,
                commit.start_sequence,
                commit.end_sequence,
                commit.summary_text,
                commit.model_label,
                now,
            ],
        )?;
        transaction.execute(
            "UPDATE agent_session SET revision = revision + 1, updated_at_ms = ?1 WHERE external_id = ?2",
            params![now, commit.session_id],
        )?;
        let checkpoint = CompactCheckpoint {
            id,
            sequence,
            operation_id: commit.operation_id.clone(),
            base_checkpoint_id: commit.base_checkpoint_id.clone(),
            start_sequence: commit.start_sequence,
            end_sequence: commit.end_sequence,
            summary_text: commit.summary_text.clone(),
            model_label: commit.model_label.clone(),
            state: COMPACT_ACTIVE.to_string(),
            created_at_ms: now,
        };
        transaction.commit()?;
        Ok(CompactCommitted {
            checkpoint,
            created: true,
        })
    }

    /// 撤销一次 compact：只切检查点状态与 revision，**不触碰任何消息或文件**。
    ///
    /// 撤销记录复用既有 `agent_rewind_operation`（`action = 'restore_compact'`），
    /// 不建立平行撤销系统：本函数在同一事务内写 operation 行并直接推进到 committed。
    /// 幂等：同 `operation_id` 已把目标检查点标成 reverted 时返回原结果。
    pub fn revert_compact(
        &self,
        workspace: &Path,
        revert: &CompactRevert,
    ) -> Result<CompactCheckpoint> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![revert.session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let existing: Option<(String, String, i64, Option<String>)> = transaction
            .query_row(
                r#"SELECT external_id, state, sequence, reverted_by_operation_id
                   FROM agent_compact_checkpoint
                   WHERE external_id = ?1 AND session_id = ?2"#,
                params![revert.checkpoint_id, revert.session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((id, state, _sequence, reverted_by)) = existing else {
            return Err(DbError::RewindTargetChanged);
        };
        if state == COMPACT_REVERTED {
            // 幂等重放：同一次操作已撤销，原样返回。
            let checkpoint =
                checkpoint_by_id(&transaction, &id)?.ok_or(DbError::RewindTargetChanged)?;
            transaction.commit()?;
            return Ok(checkpoint);
        }
        if reverted_by.is_some() {
            return Err(DbError::MutationConflict);
        }
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM agent_session WHERE external_id = ?1",
            params![revert.session_id],
            |row| row.get(0),
        )?;
        if revision != revert.expected_revision {
            return Err(DbError::RewindTargetChanged);
        }
        // 只能撤销当前生效的最新检查点：更晚的 active 检查点存在时目标已过期。
        let later: Option<String> = transaction
            .query_row(
                r#"SELECT external_id FROM agent_compact_checkpoint
                   WHERE session_id = ?1 AND state = 'active' AND sequence > (
                       SELECT sequence FROM agent_compact_checkpoint WHERE external_id = ?2
                   )
                   ORDER BY sequence DESC LIMIT 1"#,
                params![revert.session_id, id],
                |row| row.get(0),
            )
            .optional()?;
        if later.is_some() {
            return Err(DbError::RewindTargetChanged);
        }
        let now = now_ms();
        // 撤销审计写入既有 operation 表（与普通轮次同一套 kind/phase 约束）。
        transaction.execute(
            r#"INSERT INTO agent_rewind_operation
                   (external_id, operation_id, session_id, action, target_user_message_id,
                    expected_revision, phase, user_text, created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, 'restore_compact', ?4, ?5, 'committed', '', ?6, ?6)
               ON CONFLICT(operation_id) DO UPDATE SET
                   phase = 'committed', updated_at_ms = excluded.updated_at_ms"#,
            params![
                new_external_id("op"),
                revert.operation_id,
                revert.session_id,
                id,
                revert.expected_revision,
                now,
            ],
        )?;
        transaction.execute(
            r#"UPDATE agent_compact_checkpoint
               SET state = 'reverted', reverted_by_operation_id = ?1
               WHERE external_id = ?2"#,
            params![revert.operation_id, id],
        )?;
        transaction.execute(
            "UPDATE agent_session SET revision = revision + 1, updated_at_ms = ?1 WHERE external_id = ?2",
            params![now, revert.session_id],
        )?;
        let checkpoint =
            checkpoint_by_id(&transaction, &id)?.ok_or(DbError::RewindTargetChanged)?;
        transaction.commit()?;
        Ok(checkpoint)
    }

    /// 最新可撤销的 compact 检查点（`/rewind` 顺序判定用）。
    ///
    /// 只返回 active 的最新一条；已撤销的检查点不可再撤销（防重复）。
    pub fn latest_active_checkpoint(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Option<CompactCheckpoint>> {
        self.active_checkpoint(workspace, session_id)
    }

    /// 会话归属校验：返回 internal id 或 None。
    fn owned_session(&self, workspace: &Path, session_id: &str) -> Result<Option<i64>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        Ok(self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?)
    }
}

/// 读一行检查点。
fn read_checkpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompactCheckpoint> {
    Ok(CompactCheckpoint {
        id: row.get(0)?,
        sequence: row.get(1)?,
        operation_id: row.get(2)?,
        base_checkpoint_id: row.get(3)?,
        start_sequence: row.get(4)?,
        end_sequence: row.get(5)?,
        summary_text: row.get(6)?,
        model_label: row.get(7)?,
        state: row.get(8)?,
        created_at_ms: row.get(9)?,
    })
}

/// 按检查点 id 读取。
fn checkpoint_by_id(connection: &Connection, id: &str) -> Result<Option<CompactCheckpoint>> {
    Ok(connection
        .query_row(
            r#"SELECT external_id, sequence, operation_id, base_checkpoint_id, start_sequence,
                      end_sequence, summary_text, model_label, state, created_at_ms
               FROM agent_compact_checkpoint WHERE external_id = ?1"#,
            params![id],
            read_checkpoint,
        )
        .optional()?)
}

/// 按幂等键（`operation_id` 唯一）读取既有检查点。
fn checkpoint_by_operation(
    connection: &Connection,
    operation_id: &str,
) -> Result<Option<CompactCheckpoint>> {
    Ok(connection
        .query_row(
            r#"SELECT external_id, sequence, operation_id, base_checkpoint_id, start_sequence,
                      end_sequence, summary_text, model_label, state, created_at_ms
               FROM agent_compact_checkpoint WHERE operation_id = ?1"#,
            params![operation_id],
            read_checkpoint,
        )
        .optional()?)
}

#[cfg(test)]
mod tests;

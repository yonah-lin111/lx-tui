//! 启动恢复所需的只读扫描与结算写入。
//!
//! 这里只做 SQLite；文件比较/写回由事件层在持有 session guard 时执行。

use rusqlite::{OptionalExtension, params};

use crate::db::{Database, DbError, Result};

use super::{
    ImageRecord, ImageWrite, OP_FILES_RESTORED, OP_PREPARING, OP_RESTORING, OperationRecord,
    TurnScopeRecord, insert_image, now_ms, operation_by_id,
};

/// 一份尚未结算的 mutation（before / planned-after 镜像已读出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMutation {
    pub id: i64,
    pub target_path: String,
    pub anchor_path: Option<String>,
    pub anchor_canonical: Option<String>,
    pub before: ImageRecord,
    pub planned_after: ImageRecord,
}

impl Database {
    /// 需要启动恢复的会话：遗留流/工具/进程，以及撤销 journal 证据。
    pub fn sessions_needing_recovery(&self) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            r#"SELECT external_id FROM agent_session WHERE external_id IN (
                   SELECT session_id FROM agent_message WHERE status = 'streaming'
                   UNION SELECT session_id FROM agent_tool_call
                        WHERE status IN ('pending', 'running')
                   UNION SELECT chat_session_id FROM agent_process
                        WHERE state IN ('starting', 'running')
                   UNION SELECT session_id FROM agent_rewind_operation
                        WHERE phase IN ('preparing', 'restoring', 'files_restored')
                   UNION SELECT session_id FROM agent_file_mutation
                        WHERE state = 'prepared'
                   UNION SELECT session_id FROM agent_turn_file_scope
                        WHERE state = 'capturing'
               )
               ORDER BY external_id"#,
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 某会话所有尚未结算的 mutation。
    pub fn pending_mutations(&self, session_id: &str) -> Result<Vec<PendingMutation>> {
        let mut statement = self.conn.prepare(
            r#"SELECT m.id, m.target_path, m.anchor_path, m.anchor_canonical,
                      b.kind, b.bytes, b.mode, b.link_target, b.git_oid,
                      p.kind, p.bytes, p.mode, p.link_target, p.git_oid
               FROM agent_file_mutation m
               JOIN agent_file_image b ON b.id = m.before_image_id
               JOIN agent_file_image p ON p.id = m.planned_after_image_id
               WHERE m.session_id = ?1 AND m.state = 'prepared'
               ORDER BY m.id"#,
        )?;
        let rows = statement.query_map(params![session_id], |row| {
            Ok(PendingMutation {
                id: row.get(0)?,
                target_path: row.get(1)?,
                anchor_path: row.get(2)?,
                anchor_canonical: row.get(3)?,
                before: ImageRecord {
                    kind: row.get(4)?,
                    bytes: row.get(5)?,
                    mode: row.get(6)?,
                    link_target: row.get(7)?,
                    git_oid: row.get(8)?,
                },
                planned_after: ImageRecord {
                    kind: row.get(9)?,
                    bytes: row.get(10)?,
                    mode: row.get(11)?,
                    link_target: row.get(12)?,
                    git_oid: row.get(13)?,
                },
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 结算一条 prepared mutation：记录实际结果并计入本轮快照用量。
    pub fn settle_mutation(&self, mutation_id: i64, actual: &ImageWrite) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        let row: Option<(String, String)> = transaction
            .query_row(
                "SELECT session_id, user_message_id FROM agent_file_mutation WHERE id = ?1 AND state = 'prepared'",
                params![mutation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((session_id, user_message_id)) = row else {
            return Err(DbError::MutationConflict);
        };
        let size = actual.size();
        let image_id = insert_image(&transaction, &session_id, actual, now_ms())?;
        transaction.execute(
            r#"UPDATE agent_file_mutation
               SET actual_after_image_id = ?1, state = 'committed', success = 1, committed_at_ms = ?2
               WHERE id = ?3"#,
            params![image_id, now_ms(), mutation_id],
        )?;
        if size > 0 {
            transaction.execute(
                "UPDATE agent_turn_file_scope SET snapshot_bytes = snapshot_bytes + ?1 WHERE session_id = ?2 AND user_message_id = ?3",
                params![i64::try_from(size).unwrap_or(i64::MAX), session_id, user_message_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 把已经没有任何 prepared mutation 的 capturing scope 封口。
    pub fn seal_recovered_scopes(&self, session_id: &str) -> Result<usize> {
        Ok(self.conn.execute(
            r#"UPDATE agent_turn_file_scope
               SET state = 'complete'
               WHERE session_id = ?1 AND state = 'capturing'
                 AND NOT EXISTS (
                     SELECT 1 FROM agent_file_mutation f
                     WHERE f.session_id = ?1
                       AND f.user_message_id = agent_turn_file_scope.user_message_id
                       AND f.state = 'prepared'
                 )"#,
            params![session_id],
        )?)
    }

    /// 会话中所有未完成操作（preparing/restoring/files_restored）。
    pub fn unfinished_operations(&self, session_id: &str) -> Result<Vec<OperationRecord>> {
        let mut statement = self.conn.prepare(
            "SELECT operation_id FROM agent_rewind_operation WHERE session_id = ?1 AND phase IN (?2, ?3, ?4) ORDER BY id",
        )?;
        let rows = statement.query_map(
            params![session_id, OP_PREPARING, OP_RESTORING, OP_FILES_RESTORED],
            |row| row.get::<_, String>(0),
        )?;
        let mut operations = Vec::new();
        for operation_id in rows {
            if let Some(record) = operation_by_id(&self.conn, &operation_id?)? {
                operations.push(record);
            }
        }
        Ok(operations)
    }

    /// 按稳定 user message 读取 scope（不要求目标仍是最新轮次）。
    pub fn turn_scope_for_session(
        &self,
        session_id: &str,
        user_message_id: &str,
    ) -> Result<Option<TurnScopeRecord>> {
        Ok(self
            .conn
            .query_row(
                r#"SELECT user_message_id, state, capture_version, snapshot_bytes, files_restored
                   FROM agent_turn_file_scope
                   WHERE session_id = ?1 AND user_message_id = ?2"#,
                params![session_id, user_message_id],
                |row| {
                    Ok(TurnScopeRecord {
                        user_message_id: row.get(0)?,
                        state: row.get(1)?,
                        capture_version: row.get(2)?,
                        snapshot_bytes: row.get(3)?,
                        files_restored: row.get::<_, i64>(4)? != 0,
                    })
                },
            )
            .optional()?)
    }
}

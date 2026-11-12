//! BTW 启动恢复与认领失败收敛：按 child session guard 过滤，避免跨实例误伤。

use rusqlite::{OptionalExtension, params};

use crate::db::{Database, DbError, Result, now_ms};

impl Database {
    /// 需要启动恢复的 BTW child sessions；调用方逐个取跨进程 guard 后再恢复。
    pub fn btw_sessions_needing_recovery(&self) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            r#"SELECT DISTINCT child_session_id
               FROM agent_btw_thread t
               WHERE EXISTS (
                   SELECT 1 FROM agent_message m
                   WHERE m.session_id = t.child_session_id AND m.status = 'streaming'
               ) OR EXISTS (
                   SELECT 1 FROM agent_btw_batch b
                   JOIN agent_btw_recipient r ON r.batch_id = b.external_id
                   WHERE b.thread_id = t.external_id AND r.status IN ('queued', 'running')
               )"#,
        )?;
        statement
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 启动恢复：只处理调用方已取得 guard 的 child sessions，避免第二个实例
    /// 把另一实例仍在运行的 BTW 标记为 interrupted。
    pub fn recover_btw_startup_for(&self, sessions: &[String]) -> Result<()> {
        if sessions.is_empty() {
            return Ok(());
        }
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        for session in sessions {
            transaction.execute(
                r#"UPDATE agent_message
                   SET status = 'interrupted', completed_at_ms = ?1
                   WHERE status = 'streaming' AND session_id = ?2"#,
                params![now, session],
            )?;
            transaction.execute(
                r#"UPDATE agent_btw_recipient
                   SET status = 'interrupted', completed_at_ms = ?1
                   WHERE status IN ('queued', 'running') AND batch_id IN (
                       SELECT b.external_id FROM agent_btw_batch b
                       JOIN agent_btw_thread t ON t.external_id = b.thread_id
                       WHERE t.child_session_id = ?2
                   )"#,
                params![now, session],
            )?;
            transaction.execute(
                r#"UPDATE agent_btw_batch
                   SET status = 'interrupted', completed_at_ms = ?1
                   WHERE status IN ('queued', 'running') AND thread_id IN (
                       SELECT external_id FROM agent_btw_thread WHERE child_session_id = ?2
                   )"#,
                params![now, session],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 启动恢复全库版本（测试/无 guard 场景）；生产入口按 session guard 过滤。
    pub fn recover_btw_startup(&self) -> Result<()> {
        let sessions = self.btw_sessions_needing_recovery()?;
        self.recover_btw_startup_for(&sessions)
    }

    /// 认领失败时把回答者收敛到 failed，避免永久 queued/running。
    pub fn fail_btw_recipient(&self, recipient_id: &str, error_code: &str) -> Result<()> {
        let now = now_ms();
        let batch_id: Option<String> = self
            .conn
            .query_row(
                "SELECT batch_id FROM agent_btw_recipient WHERE external_id = ?1",
                params![recipient_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(batch_id) = batch_id else {
            return Ok(());
        };
        self.conn.execute(
            "UPDATE agent_btw_recipient SET status = 'failed', error_code = ?1, completed_at_ms = ?2 WHERE external_id = ?3 AND status IN ('queued', 'running')",
            params![error_code, now, recipient_id],
        )?;
        self.sync_btw_batch_status(&batch_id)
    }
}

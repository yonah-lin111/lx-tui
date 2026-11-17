//! BTW 启动恢复与认领失败收敛：按 child session guard 过滤，避免跨实例误伤。

use rusqlite::{OptionalExtension, params};

use crate::db::{Database, DbError, Result, now_ms};

/// 回答者终态 → 批次聚合状态。
pub(super) fn aggregate_batch_status(statuses: &[String]) -> &'static str {
    let count = |needle: &str| {
        statuses
            .iter()
            .filter(|status| status.as_str() == needle)
            .count()
    };
    let completed = count("completed");
    let interrupted = count("interrupted");
    let failed = count("failed");
    let cancelled = count("cancelled");
    if completed > 0 && (interrupted > 0 || failed > 0 || cancelled > 0) {
        "partial"
    } else if completed > 0 {
        "completed"
    } else if failed > 0 {
        "failed"
    } else if interrupted > 0 {
        // 用户停止 / 崩溃恢复截断：与失败区分，列表显示 interrupted 而不是红叉。
        "interrupted"
    } else if cancelled > 0 {
        "cancelled"
    } else {
        "failed"
    }
}

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
            // 从未启动的 `queued` 回答者不是“被打断”：按取消收敛。只有真正在
            // running 的轮次才标 interrupted，否则 QA 列表会出现没有执行内容的
            // 打断标记（多目标批次里第二个目标被文件锁挡掉后尤其明显）。
            transaction.execute(
                r#"UPDATE agent_btw_recipient
                   SET status = 'cancelled', completed_at_ms = ?1
                   WHERE status = 'queued' AND batch_id IN (
                       SELECT b.external_id FROM agent_btw_batch b
                       JOIN agent_btw_thread t ON t.external_id = b.thread_id
                       WHERE t.child_session_id = ?2
                   )"#,
                params![now, session],
            )?;
            transaction.execute(
                r#"UPDATE agent_btw_recipient
                   SET status = 'interrupted', completed_at_ms = ?1
                   WHERE status = 'running' AND batch_id IN (
                       SELECT b.external_id FROM agent_btw_batch b
                       JOIN agent_btw_thread t ON t.external_id = b.thread_id
                       WHERE t.child_session_id = ?2
                   )"#,
                params![now, session],
            )?;
            let batches: Vec<String> = {
                let mut statement = transaction.prepare(
                    r#"SELECT b.external_id FROM agent_btw_batch b
                       JOIN agent_btw_thread t ON t.external_id = b.thread_id
                       WHERE t.child_session_id = ?1 AND b.status IN ('queued', 'running')"#,
                )?;
                statement
                    .query_map(params![session], |row| row.get(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            };
            for batch_id in batches {
                let statuses: Vec<String> = {
                    let mut statement = transaction
                        .prepare("SELECT status FROM agent_btw_recipient WHERE batch_id = ?1")?;
                    statement
                        .query_map(params![batch_id], |row| row.get(0))?
                        .collect::<std::result::Result<Vec<_>, _>>()?
                };
                // 统一按回答者聚合：已完成 + 被截断 = partial，全部被截断才 interrupted。
                // 不能因为存在一个 interrupted 就把整批写成 interrupted，否则同一事实
                // 会因恢复路径不同而显示不同状态。
                let aggregate = aggregate_batch_status(&statuses);
                transaction.execute(
                    "UPDATE agent_btw_batch SET status = ?1, completed_at_ms = ?2 WHERE external_id = ?3",
                    params![aggregate, now, batch_id],
                )?;
            }
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
    ///
    /// 回答者与批次状态在同一事务内收敛；批次进入终态时同步刷新线程排序时间。
    pub fn fail_btw_recipient(&self, recipient_id: &str, error_code: &str) -> Result<()> {
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let batch_id: Option<String> = transaction
            .query_row(
                "SELECT batch_id FROM agent_btw_recipient WHERE external_id = ?1",
                params![recipient_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(batch_id) = batch_id else {
            return Ok(());
        };
        transaction.execute(
            "UPDATE agent_btw_recipient SET status = 'failed', error_code = ?1, completed_at_ms = ?2 WHERE external_id = ?3 AND status IN ('queued', 'running')",
            params![error_code, now, recipient_id],
        )?;
        self.sync_btw_batch_status_in_tx(&transaction, &batch_id)?;
        transaction.commit()?;
        Ok(())
    }
}

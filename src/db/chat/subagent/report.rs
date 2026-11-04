//! 子 agent 报告批量投递：父会话报告消息、逐轮报告领取与已投递标记。

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::super::{ModelSnapshot, TurnCommitted};
use super::record::{
    SubagentReportRecord, sync_agent_report_delivered, sync_legacy_reports_for_parent,
};
use crate::db::{Database, DbError, Result, new_external_id, now_ms};

impl Database {
    /// 后台报告批量：一条 user 报告消息（数组 JSON）+ 同轮 assistant，
    /// 并在同一事务内把对应 `agent_ids`（及 `agent_subagent_report`）标记为已投递。
    pub fn begin_report_batch(
        &self,
        workspace: &Path,
        session_id: &str,
        envelope: &str,
        reports_json: &str,
        agent_ids: &[String],
        model: &ModelSnapshot,
    ) -> Result<TurnCommitted> {
        self.begin_report_batch_with_ids(
            workspace,
            session_id,
            envelope,
            reports_json,
            agent_ids,
            &[],
            model,
        )
    }

    /// 按显式 `report_ids`（及所属 `agent_ids`）在同一事务内插入父会话报告消息并标记 delivered。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_report_batch_with_ids(
        &self,
        workspace: &Path,
        session_id: &str,
        envelope: &str,
        reports_json: &str,
        agent_ids: &[String],
        report_ids: &[String],
        model: &ModelSnapshot,
    ) -> Result<TurnCommitted> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        super::record::ensure_session_owned(&transaction, session_id, &workspace_external)?;
        let user_sequence = super::super::next_session_sequence(&transaction, session_id)?;
        let turn_index: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(turn_index) + 1, 0) FROM agent_message WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let user_message_id = new_external_id("msg");
        let assistant_message_id = new_external_id("msg");
        let assistant_sequence = user_sequence.saturating_add(1);
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM agent_session WHERE external_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status,
                    subagent_report_json, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'user', 'complete', ?5, ?6)"#,
            params![
                user_message_id,
                session_id,
                user_sequence,
                turn_index,
                reports_json,
                now
            ],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, created_at_ms)
               VALUES (?1, ?2, 0, 'text', ?3, ?4)"#,
            params![new_external_id("part"), user_message_id, envelope, now],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status,
                    model_provider, model_id, variant, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'assistant', 'streaming', ?5, ?6, ?7, ?8)"#,
            params![
                assistant_message_id,
                session_id,
                assistant_sequence,
                turn_index,
                model.provider,
                model.model,
                model.variant,
                now
            ],
        )?;
        for report_id in report_ids {
            transaction.execute(
                r#"UPDATE agent_subagent_report SET delivered_at_ms = ?1
                   WHERE external_id = ?2 AND delivered_at_ms IS NULL
                     AND agent_id IN (
                         SELECT external_id FROM agent_subagent WHERE parent_session_id = ?3
                     )"#,
                params![now, report_id, session_id],
            )?;
        }
        for agent_id in agent_ids {
            if report_ids.is_empty() {
                transaction.execute(
                    r#"UPDATE agent_subagent_report SET delivered_at_ms = ?1
                       WHERE agent_id = ?2 AND delivered_at_ms IS NULL
                         AND agent_id IN (
                             SELECT external_id FROM agent_subagent WHERE parent_session_id = ?3
                         )"#,
                    params![now, agent_id, session_id],
                )?;
            }
            sync_agent_report_delivered(&transaction, agent_id, session_id, now)?;
        }
        super::super::update_session_selection(
            &transaction,
            session_id,
            &workspace_external,
            model,
            now,
        )?;
        let revision = revision.saturating_add(1);
        transaction.execute(
            "UPDATE agent_session SET revision = ?1, updated_at_ms = ?2, last_message_at_ms = ?2 WHERE external_id = ?3",
            params![revision, now, session_id],
        )?;
        transaction.commit()?;
        Ok(TurnCommitted {
            session_id: session_id.to_string(),
            user_message_id,
            assistant_message_id,
            turn_index,
            user_sequence,
            assistant_sequence,
            revision,
            created_at_ms: now,
            attachments: Vec::new(),
        })
    }

    /// 批量把指定 `report_ids` 标记为已投递（`wait_agent` 按 `report_id` 领取时使用）。
    pub fn mark_reports_delivered_in_batch(
        &self,
        workspace: &Path,
        parent_session: &str,
        report_ids: &[String],
    ) -> Result<usize> {
        if report_ids.is_empty() {
            return Ok(0);
        }
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(0);
        };
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        super::record::ensure_session_owned(&transaction, parent_session, &workspace_external)?;
        let mut count = 0usize;
        let mut touched_agents = std::collections::BTreeSet::new();
        for report_id in report_ids {
            let agent_id: Option<String> = transaction
                .query_row(
                    r#"SELECT r.agent_id FROM agent_subagent_report r
                       JOIN agent_subagent a ON a.external_id = r.agent_id
                       WHERE r.external_id = ?1 AND a.parent_session_id = ?2"#,
                    params![report_id, parent_session],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(agent_id) = agent_id else {
                continue;
            };
            let updated = transaction.execute(
                r#"UPDATE agent_subagent_report SET delivered_at_ms = ?1
                   WHERE external_id = ?2 AND delivered_at_ms IS NULL"#,
                params![now, report_id],
            )?;
            if updated > 0 {
                count = count.saturating_add(updated);
                touched_agents.insert(agent_id);
            }
        }
        for agent_id in touched_agents {
            sync_agent_report_delivered(&transaction, &agent_id, parent_session, now)?;
        }
        transaction.commit()?;
        Ok(count)
    }

    /// 恰好一次：将该 agent 未投递的报告标为已投递；0 → 1 时返回 true，已全部投递返回 false（幂等）。
    pub fn mark_report_delivered(&self, workspace: &Path, agent_id: &str) -> Result<bool> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(false);
        };
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let parent_session: Option<String> = transaction
            .query_row(
                r#"SELECT a.parent_session_id FROM agent_subagent a
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   WHERE a.external_id = ?1 AND p.workspace_id = ?2"#,
                params![agent_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        let Some(parent_session) = parent_session else {
            return Ok(false);
        };
        let updated_reports = transaction.execute(
            r#"UPDATE agent_subagent_report SET delivered_at_ms = ?1
               WHERE agent_id = ?2 AND delivered_at_ms IS NULL"#,
            params![now, agent_id],
        )?;
        let updated_legacy = transaction.execute(
            r#"UPDATE agent_subagent SET report_delivered = 1, updated_at_ms = ?1
               WHERE external_id = ?2 AND parent_session_id = ?3 AND report_delivered = 0"#,
            params![now, agent_id, parent_session],
        )?;
        transaction.commit()?;
        Ok(updated_reports > 0 || updated_legacy > 0)
    }

    /// 按完成时间和稳定 ID 排序读取当前父会话下所有未投递的逐轮报告。
    pub fn pending_reports(
        &self,
        workspace: &Path,
        parent_session: &str,
    ) -> Result<Vec<SubagentReportRecord>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        sync_legacy_reports_for_parent(&self.conn, parent_session, &workspace_external)?;
        let mut statement = self.conn.prepare(
            r#"SELECT r.external_id, r.agent_id, r.child_user_message_id, r.status,
                      r.report_json, r.completed_at_ms, r.delivered_at_ms
               FROM agent_subagent_report r
               JOIN agent_subagent a ON a.external_id = r.agent_id
               JOIN agent_session p ON p.external_id = a.parent_session_id
               WHERE a.parent_session_id = ?1 AND p.workspace_id = ?2
                 AND r.delivered_at_ms IS NULL
               ORDER BY r.completed_at_ms ASC, r.external_id ASC"#,
        )?;
        let rows = statement.query_map(params![parent_session, workspace_external], |row| {
            Ok(SubagentReportRecord {
                report_id: row.get(0)?,
                agent_id: row.get(1)?,
                child_user_message_id: row.get(2)?,
                status: row.get(3)?,
                report_json: row.get(4)?,
                completed_at_ms: row.get(5)?,
                delivered_at_ms: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 未投递的报告（兼容入口）：`(agent_id, report_json)`。
    pub fn pending_subagent_reports(
        &self,
        workspace: &Path,
        parent_session: &str,
    ) -> Result<Vec<(String, String)>> {
        Ok(self
            .pending_reports(workspace, parent_session)?
            .into_iter()
            .map(|record| (record.agent_id, record.report_json))
            .collect())
    }
}

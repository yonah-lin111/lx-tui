//! 子 agent 持久化：spawn 事务、终态与报告、列表、报告行与启动恢复。
//!
//! 一个 subagent = 一个独立子会话（`source='subagent'`）挂到父会话；子会话的
//! 消息/parts/tools 与普通会话同一套表，回看走 `load_chat_session`。
//! `report_delivered` 是「报告恰好一次」的唯一判据。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{ChatPart, ModelSnapshot, TurnCommitted, TurnOutcome, Usage};
use crate::chat::agent::{AgentRole, SubagentReportView};
use crate::db::{Database, DbError, Result, new_external_id, now_ms};

/// 一个 subagent 的元数据行（列表用；不含消息流）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRecord {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 子会话 external id。
    pub child_session_id: String,
    /// 父会话 external id。
    pub parent_session_id: String,
    /// spawn 调用所在的主 agent assistant 消息 id。
    pub spawn_message_id: Option<String>,
    /// 角色。
    pub role: AgentRole,
    /// `pending` / `running` / `completed` / `failed` / `cancelled`。
    pub status: String,
    /// 面向用户的短标题；v17 之前的旧记录为空。
    pub title: String,
    /// 创建时刻（UTC 毫秒）。
    pub created_at_ms: i64,
    /// 完成时刻（未结束为 None）。
    pub completed_at_ms: Option<i64>,
    /// 报告 JSON（未生成或已关闭为 None）。
    pub report_json: Option<String>,
    /// 报告是否已被模型领取（恰好一次）。
    pub report_delivered: bool,
}

/// spawn 的提交结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentCommitted {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 子会话 external id。
    pub child_session_id: String,
    /// 子 assistant 消息 id（工具归属锚点）。
    pub child_assistant_id: String,
    /// 子 user 消息 id（文件 mutation 归属）。
    pub child_user_message_id: String,
    /// 创建时刻（UTC 毫秒）。
    pub created_at_ms: i64,
}

/// 报告正文与尾巴（DB 在此组装成 [`SubagentReportView`] 的 JSON）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPayload {
    /// 面向用户的短标题。
    pub title: String,
    /// 已按上限截断的正文。
    pub body: String,
    /// 去重后的改动文件数。
    pub files_changed: usize,
    /// 子 turn 耗时（毫秒）。
    pub duration_ms: Option<i64>,
}

impl Database {
    /// spawn：一个事务内建子会话 + user 消息 + streaming assistant + agent_subagent 行。
    pub fn begin_subagent_turn(
        &self,
        workspace: &Path,
        parent_session: &str,
        role: AgentRole,
        title: &str,
        task: &str,
        model: &ModelSnapshot,
        spawn_message_id: Option<&str>,
    ) -> Result<SubagentCommitted> {
        if task.trim().is_empty() {
            return Err(DbError::InvalidDraft);
        }
        let title = super::single_line_preview(title.trim(), 80);
        if title.is_empty() {
            return Err(DbError::InvalidDraft);
        }
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        ensure_session_owned(&transaction, parent_session, &workspace_external)?;
        let child_session_id = new_external_id("sess");
        transaction.execute(
            r#"INSERT INTO agent_session
                   (external_id, workspace_id, source, title, first_user_message,
                    model_provider, model_id, variant, status, archived, agent_role,
                    created_at_ms, updated_at_ms, last_message_at_ms)
               VALUES (?1, ?2, 'subagent', ?3, ?4, ?5, ?6, ?7, 'active', 0, ?8, ?9, ?9, ?9)"#,
            params![
                child_session_id,
                workspace_external,
                &title,
                super::single_line_preview(task, super::PREVIEW_CHARS),
                model.provider,
                model.model,
                model.variant,
                role.as_str(),
                now,
            ],
        )?;
        let user_message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
               VALUES (?1, ?2, 0, 0, 'user', 'complete', ?3)"#,
            params![user_message_id, child_session_id, now],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, created_at_ms)
               VALUES (?1, ?2, 0, 'text', ?3, ?4)"#,
            params![new_external_id("part"), user_message_id, task, now],
        )?;
        let assistant_message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status,
                    model_provider, model_id, variant, created_at_ms)
               VALUES (?1, ?2, 1, 0, 'assistant', 'streaming', ?3, ?4, ?5, ?6)"#,
            params![
                assistant_message_id,
                child_session_id,
                model.provider,
                model.model,
                model.variant,
                now,
            ],
        )?;
        // 子轮同样要建立文件 scope：executor 的写工具通过 recorder 提交 mutation，
        // 没有 scope 会整批拒绝（`rewind unavailable`），子 agent 的改动就无法落盘。
        super::rewind::begin_turn_scope(&transaction, &child_session_id, &user_message_id, now)?;
        let agent_id = new_external_id("sub");
        transaction.execute(
            r#"INSERT INTO agent_subagent
                   (external_id, parent_session_id, child_session_id, spawn_message_id,
                    status, role, model_provider, model_id, variant, title,
                    created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?8, ?9, ?10, ?10)"#,
            params![
                agent_id,
                parent_session,
                child_session_id,
                spawn_message_id,
                role.as_str(),
                model.provider,
                model.model,
                model.variant,
                title,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(SubagentCommitted {
            agent_id,
            child_session_id,
            child_assistant_id: assistant_message_id,
            child_user_message_id: user_message_id,
            created_at_ms: now,
        })
    }

    /// 子 turn 终态：子 assistant 消息 finish + `agent_subagent` 状态/报告。
    ///
    /// 两步顺序不可交换：先封口消息（复用普通 `finish` 的幂等/usage 语义），
    /// 再写 agent 行；第二步失败只会留下 `running`（启动恢复会纠正），不会留下
    /// 已结束消息却标着运行的半状态之外的东西。
    pub fn finish_subagent_turn(
        &self,
        workspace: &Path,
        agent_id: &str,
        child_assistant_id: &str,
        parts: &[ChatPart],
        outcome: &TurnOutcome,
        usage: Usage,
        report: Option<&ReportPayload>,
    ) -> Result<()> {
        self.finish(workspace, child_assistant_id, parts, outcome, usage)?;
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let status = match outcome {
            TurnOutcome::Complete { .. } => "completed",
            TurnOutcome::Interrupted => "cancelled",
            TurnOutcome::Failed { .. } => "failed",
        };
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let role: Option<String> = transaction
            .query_row(
                r#"SELECT a.role FROM agent_subagent a
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   WHERE a.external_id = ?1 AND p.workspace_id = ?2"#,
                params![agent_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        let role = role
            .as_deref()
            .and_then(AgentRole::parse)
            .ok_or(DbError::SessionMissing)?;
        let report_json = report.map(|payload| {
            SubagentReportView {
                agent_id: agent_id.to_string(),
                title: payload.title.clone(),
                role,
                status: status.to_string(),
                files_changed: payload.files_changed,
                duration_ms: payload.duration_ms,
                body: payload.body.clone(),
                delivered_at_ms: now,
            }
            .to_json()
        });
        transaction.execute(
            r#"UPDATE agent_subagent
               SET status = ?1, completed_at_ms = ?2, updated_at_ms = ?2,
                   report_json = COALESCE(?3, report_json)
               WHERE external_id = ?4"#,
            params![status, now, report_json, agent_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 列表：一条 SQL 联查元数据（不含消息流）。
    pub fn list_subagents(
        &self,
        workspace: &Path,
        parent_session: &str,
    ) -> Result<Vec<SubagentRecord>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT a.external_id, a.child_session_id, a.parent_session_id, a.spawn_message_id,
                      a.role, a.status, a.title, a.created_at_ms, a.completed_at_ms,
                      a.report_json, a.report_delivered
               FROM agent_subagent a
               JOIN agent_session s ON s.external_id = a.child_session_id
               JOIN agent_session p ON p.external_id = a.parent_session_id
               WHERE a.parent_session_id = ?1 AND p.workspace_id = ?2
               ORDER BY a.created_at_ms, a.id"#,
        )?;
        let rows = statement.query_map(params![parent_session, workspace_external], |row| {
            let role: Option<String> = row.get(4)?;
            Ok((
                SubagentRecord {
                    agent_id: row.get(0)?,
                    child_session_id: row.get(1)?,
                    parent_session_id: row.get(2)?,
                    spawn_message_id: row.get(3)?,
                    // 无效角色行由调用方过滤（杜绝伪造枚举值）。
                    role: role
                        .as_deref()
                        .and_then(AgentRole::parse)
                        .unwrap_or(AgentRole::Executor),
                    status: row.get(5)?,
                    title: row.get(6)?,
                    created_at_ms: row.get(7)?,
                    completed_at_ms: row.get(8)?,
                    report_json: row.get(9)?,
                    report_delivered: row.get::<_, i64>(10)? != 0,
                },
                role,
            ))
        })?;
        let mut records = Vec::new();
        for row in rows {
            let (record, role) = row?;
            if role.as_deref().and_then(AgentRole::parse).is_some() {
                records.push(record);
            }
        }
        Ok(records)
    }

    /// 后台报告批量：一条 user 报告消息（数组 JSON）+ 同轮 assistant。
    ///
    /// 报告在消息提交事务内标记已投递：消息本身已经是可回放的上下文，失败重试
    /// 不应再生成第二份卡片。`agent_ids` 必须与 `reports_json` 一一对应。
    pub fn begin_report_batch(
        &self,
        workspace: &Path,
        session_id: &str,
        envelope: &str,
        reports_json: &str,
        agent_ids: &[String],
        model: &ModelSnapshot,
    ) -> Result<TurnCommitted> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        ensure_session_owned(&transaction, session_id, &workspace_external)?;
        let (user_sequence, turn_index): (i64, i64) = transaction.query_row(
            r#"SELECT COALESCE(MAX(sequence) + 1, 0),
                      COALESCE(MAX(turn_index) + 1, 0)
               FROM agent_message WHERE session_id = ?1"#,
            params![session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
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
        for agent_id in agent_ids {
            transaction.execute(
                r#"UPDATE agent_subagent SET report_delivered = 1, updated_at_ms = ?1
                   WHERE external_id = ?2 AND parent_session_id = ?3
                     AND report_delivered = 0"#,
                params![now, agent_id, session_id],
            )?;
        }
        super::update_session_selection(&transaction, session_id, &workspace_external, model, now)?;
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

    /// 恰好一次：0 → 1 时返回 true；已投递返回 false（幂等）。
    pub fn mark_report_delivered(&self, workspace: &Path, agent_id: &str) -> Result<bool> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(false);
        };
        let updated = self.conn.execute(
            r#"UPDATE agent_subagent SET report_delivered = 1, updated_at_ms = ?1
               WHERE external_id = ?2 AND report_delivered = 0
                 AND parent_session_id IN (
                     SELECT external_id FROM agent_session WHERE workspace_id = ?3
                 )"#,
            params![now_ms(), agent_id, workspace_external],
        )?;
        Ok(updated > 0)
    }

    /// 未投递的报告（后台自动轮 / `wait_agent` 共用）：`(agent_id, report_json)`。
    pub fn pending_subagent_reports(
        &self,
        workspace: &Path,
        parent_session: &str,
    ) -> Result<Vec<(String, String)>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT a.external_id, a.report_json
               FROM agent_subagent a
               JOIN agent_session p ON p.external_id = a.parent_session_id
               WHERE a.parent_session_id = ?1 AND p.workspace_id = ?2
                 AND a.report_delivered = 0 AND a.report_json IS NOT NULL
               ORDER BY a.completed_at_ms, a.id"#,
        )?;
        let rows = statement.query_map(params![parent_session, workspace_external], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 子会话本轮**已提交**的改动路径数（去重；报告尾巴用）。
    pub fn count_changed_paths(
        &self,
        workspace: &Path,
        session_id: &str,
        user_message_id: &str,
    ) -> Result<usize> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(0);
        };
        let count: i64 = self.conn.query_row(
            r#"SELECT COUNT(DISTINCT target_path) FROM agent_file_mutation m
               WHERE m.session_id = ?1 AND m.user_message_id = ?2 AND m.state = 'committed'
                 AND m.session_id IN (
                     SELECT external_id FROM agent_session WHERE workspace_id = ?3
                 )"#,
            params![session_id, user_message_id, workspace_external],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or_default())
    }

    /// 启动恢复：running/pending → cancelled，子会话 streaming → interrupted。
    ///
    /// 幂等；不重跑任何子 turn（决策 2）。
    pub fn recover_running_subagents(&self) -> Result<usize> {
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let updated = transaction.execute(
            r#"UPDATE agent_subagent
               SET status = 'cancelled', completed_at_ms = ?1, updated_at_ms = ?1
               WHERE status IN ('pending', 'running')"#,
            params![now],
        )?;
        transaction.execute(
            r#"UPDATE agent_message SET status = 'interrupted', completed_at_ms = ?1
               WHERE status = 'streaming'
                 AND session_id IN (SELECT child_session_id FROM agent_subagent)"#,
            params![now],
        )?;
        transaction.commit()?;
        Ok(updated)
    }
}

/// 会话必须属于该工作区，否则按不存在处理。
fn ensure_session_owned(
    transaction: &Connection,
    session_id: &str,
    workspace_external: &str,
) -> Result<()> {
    let owned: Option<i64> = transaction
        .query_row(
            "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
            params![session_id, workspace_external],
            |row| row.get(0),
        )
        .optional()?;
    owned.map(|_| ()).ok_or(DbError::SessionMissing)
}

#[cfg(test)]
mod tests;

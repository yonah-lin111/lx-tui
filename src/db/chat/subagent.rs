//! 子 agent 持久化：spawn 事务、终态与逐轮报告、列表、报告批次与启动恢复。
//!
//! 一个 subagent = 一个独立子会话（`source='subagent'`）挂到父会话；子会话的
//! 消息/parts/tools 与普通会话同一套表，回看走 `load_chat_session`。
//! 输入队列与有序输入项位于 [`queue`] 子模块；逐轮报告存于 `agent_subagent_report`。

mod queue;
pub use queue::{
    ClaimedSubagentTurn, EnqueuedInput, PersistedInputItem, SubagentInputRecord, SubagentQueueState,
};

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{ChatPart, ModelSnapshot, TurnCommitted, TurnOutcome, Usage};
use crate::chat::agent::{AgentRole, MAX_REPORT_BYTES, SubagentReportView, truncate_report_body};
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
    /// 最近一条报告 JSON（未生成或已关闭为 None）。
    pub report_json: Option<String>,
    /// 报告是否已被模型全部领取。
    pub report_delivered: bool,
}

/// 一条逐轮报告记录（`agent_subagent_report`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentReportRecord {
    /// 稳定报告 ID（`rep-...`）。
    pub report_id: String,
    /// 所属子 agent ID。
    pub agent_id: String,
    /// 对应子轮的用户消息 ID。
    pub child_user_message_id: Option<String>,
    /// `completed` / `failed`。
    pub status: String,
    /// 序列化后的 [`SubagentReportView`] JSON。
    pub report_json: String,
    /// 完成时刻（UTC 毫秒）。
    pub completed_at_ms: i64,
    /// 投递时刻（未投递为 None）。
    pub delivered_at_ms: Option<i64>,
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
        let items = [PersistedInputItem::Text {
            text: task.to_string(),
        }];
        self.begin_subagent_turn_with_items(
            workspace,
            parent_session,
            None,
            role,
            title,
            task,
            &items,
            model,
            spawn_message_id,
            None,
            None,
        )
    }

    /// spawn（支持有序结构化 items 与预先分配的 `child_session_id`）。
    ///
    /// 初始图片在进入本事务前已归档到 `<images_root>/<child_session_id>/`，
    /// 本事务原子写入子会话、首轮 user/assistant 消息、附件、文件 scope、子 agent 身份与首条 running 输入。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_subagent_turn_with_items(
        &self,
        workspace: &Path,
        parent_session: &str,
        child_session_id: Option<&str>,
        role: AgentRole,
        title: &str,
        task: &str,
        items: &[PersistedInputItem],
        model: &ModelSnapshot,
        spawn_message_id: Option<&str>,
        dedupe_key: Option<&str>,
        parent_call_id: Option<&str>,
    ) -> Result<SubagentCommitted> {
        if task.trim().is_empty() && items.is_empty() {
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
        let child_session_id = child_session_id
            .filter(|id| !id.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| new_external_id("sess"));
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
        queue::insert_archived_image_attachments(&transaction, &user_message_id, items)?;
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
        // 子轮同样要建立文件 scope：executor 的写工具通过 recorder 提交 mutation。
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
        let input_id = new_external_id("input");
        let dedupe = dedupe_key
            .filter(|key| !key.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("spawn:{agent_id}"));
        transaction.execute(
            r#"INSERT INTO agent_subagent_input
                   (external_id, agent_id, dedupe_key, parent_call_id, parent_message_id,
                    position, priority, status, child_user_message_id,
                    created_at_ms, started_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 'running', ?6, ?7, ?7)"#,
            params![
                input_id,
                agent_id,
                dedupe,
                parent_call_id,
                spawn_message_id,
                user_message_id,
                now,
            ],
        )?;
        queue::insert_input_items(&transaction, &input_id, items)?;
        transaction.commit()?;
        Ok(SubagentCommitted {
            agent_id,
            child_session_id,
            child_assistant_id: assistant_message_id,
            child_user_message_id: user_message_id,
            created_at_ms: now,
        })
    }

    /// 子 turn 终态：子 assistant 消息 finish + 输入行结算 + 逐轮报告 + `agent_subagent` 状态。
    // 参数是同一事务的独立维度（消息、结算、账单、报告），拆结构体只会增加调用噪音。
    #[allow(clippy::too_many_arguments)]
    pub fn finish_subagent_turn(
        &self,
        workspace: &Path,
        agent_id: &str,
        child_assistant_id: &str,
        parts: &[ChatPart],
        outcome: &TurnOutcome,
        usage: Usage,
        billing: Usage,
        report: Option<&ReportPayload>,
    ) -> Result<()> {
        self.finish_with_billing(
            workspace,
            child_assistant_id,
            parts,
            outcome,
            usage,
            billing,
        )?;
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let status = match outcome {
            TurnOutcome::Complete { .. } => "completed",
            TurnOutcome::Interrupted => "cancelled",
            TurnOutcome::Failed { .. } => "failed",
        };
        let input_status = match outcome {
            TurnOutcome::Complete { .. } => "completed",
            TurnOutcome::Interrupted => "interrupted",
            TurnOutcome::Failed { .. } => "failed",
        };
        let input_error = match outcome {
            TurnOutcome::Failed { error_code } => Some(error_code.as_str()),
            _ => None,
        };
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let row: Option<(Option<String>, String)> = transaction
            .query_row(
                r#"SELECT a.role, a.child_session_id FROM agent_subagent a
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   WHERE a.external_id = ?1 AND p.workspace_id = ?2"#,
                params![agent_id, workspace_external],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((role, child_session_id)) = row else {
            return Err(DbError::SessionMissing);
        };
        let role = role
            .as_deref()
            .and_then(AgentRole::parse)
            .ok_or(DbError::SessionMissing)?;
        let child_user_message_id: Option<String> = transaction
            .query_row(
                r#"SELECT u.external_id FROM agent_message u
                   JOIN agent_message a ON a.session_id = u.session_id AND a.turn_index = u.turn_index
                   WHERE a.external_id = ?1 AND u.role = 'user'
                   ORDER BY u.sequence LIMIT 1"#,
                params![child_assistant_id],
                |row| row.get(0),
            )
            .optional()?
            .or_else(|| {
                transaction
                    .query_row(
                        r#"SELECT external_id FROM agent_message
                           WHERE session_id = ?1 AND role = 'user'
                           ORDER BY sequence DESC LIMIT 1"#,
                        params![child_session_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .ok()
                    .flatten()
            });
        transaction.execute(
            r#"UPDATE agent_subagent_input
               SET status = ?1, completed_at_ms = ?2, error_code = COALESCE(?3, error_code)
               WHERE agent_id = ?4 AND status = 'running'"#,
            params![input_status, now, input_error, agent_id],
        )?;
        let report_json = report.map(|payload| {
            let clean_body = truncate_report_body(&sanitize_report_text(&payload.body));
            let clean_title = super::single_line_preview(&payload.title, 80);
            SubagentReportView {
                agent_id: agent_id.to_string(),
                title: clean_title,
                role,
                status: status.to_string(),
                files_changed: payload.files_changed,
                duration_ms: payload.duration_ms,
                body: clean_body,
                delivered_at_ms: now,
            }
            .to_json()
        });
        if let Some(ref json_text) = report_json
            && matches!(status, "completed" | "failed")
        {
            transaction.execute(
                r#"INSERT INTO agent_subagent_report
                       (external_id, agent_id, child_user_message_id, status, report_json,
                        completed_at_ms, delivered_at_ms)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)"#,
                params![
                    new_external_id("rep"),
                    agent_id,
                    child_user_message_id,
                    status,
                    json_text,
                    now,
                ],
            )?;
            transaction.execute(
                r#"UPDATE agent_subagent
                   SET status = ?1, completed_at_ms = ?2, updated_at_ms = ?2,
                       report_json = ?3, report_delivered = 0
                   WHERE external_id = ?4"#,
                params![status, now, json_text, agent_id],
            )?;
        } else {
            transaction.execute(
                r#"UPDATE agent_subagent
                   SET status = ?1, completed_at_ms = ?2, updated_at_ms = ?2
                   WHERE external_id = ?3"#,
                params![status, now, agent_id],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// 按 `(workspace, parent_session, agent_id)` 查询归属校验后的子 agent 记录。
    pub fn owned_subagent(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
    ) -> Result<Option<SubagentRecord>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        let row = self
            .conn
            .query_row(
                r#"SELECT a.external_id, a.child_session_id, a.parent_session_id, a.spawn_message_id,
                          a.role, a.status, a.title, a.created_at_ms, a.completed_at_ms,
                          a.report_json, a.report_delivered
                   FROM agent_subagent a
                   JOIN agent_session s ON s.external_id = a.child_session_id
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   WHERE a.external_id = ?1 AND a.parent_session_id = ?2 AND p.workspace_id = ?3"#,
                params![agent_id, parent_session, workspace_external],
                read_subagent_row,
            )
            .optional()?;
        let Some((record, role)) = row else {
            return Ok(None);
        };
        if role.as_deref().and_then(AgentRole::parse).is_none() {
            return Ok(None);
        }
        Ok(Some(record))
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
        let rows = statement.query_map(
            params![parent_session, workspace_external],
            read_subagent_row,
        )?;
        let mut records = Vec::new();
        for row in rows {
            let (record, role) = row?;
            if role.as_deref().and_then(AgentRole::parse).is_some() {
                records.push(record);
            }
        }
        Ok(records)
    }

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
        ensure_session_owned(&transaction, session_id, &workspace_external)?;
        let user_sequence = super::next_session_sequence(&transaction, session_id)?;
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
        ensure_session_owned(&transaction, parent_session, &workspace_external)?;
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

    /// 启动恢复：running/pending → cancelled，子会话 streaming → interrupted，
    /// 运行中的输入 → interrupted（保留 queued 输入不丢、不自动重跑）。
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
        transaction.execute(
            r#"UPDATE agent_subagent_input
               SET status = 'interrupted', completed_at_ms = ?1
               WHERE status = 'running'"#,
            params![now],
        )?;
        transaction.commit()?;
        Ok(updated)
    }
}

/// 读取一行 `SubagentRecord` 与其原始角色字符串。
fn read_subagent_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(SubagentRecord, Option<String>)> {
    let role: Option<String> = row.get(4)?;
    Ok((
        SubagentRecord {
            agent_id: row.get(0)?,
            child_session_id: row.get(1)?,
            parent_session_id: row.get(2)?,
            spawn_message_id: row.get(3)?,
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
}

/// 同步 `agent_subagent.report_delivered`：当且仅当该 agent 已无未投递报告时置 1。
pub(super) fn sync_agent_report_delivered(
    transaction: &Connection,
    agent_id: &str,
    parent_session: &str,
    now: i64,
) -> Result<()> {
    transaction.execute(
        r#"UPDATE agent_subagent
           SET report_delivered = CASE
                   WHEN EXISTS (
                       SELECT 1 FROM agent_subagent_report r
                       WHERE r.agent_id = ?1 AND r.delivered_at_ms IS NULL
                   ) THEN 0
                   ELSE 1
               END,
               updated_at_ms = ?2
           WHERE external_id = ?1 AND parent_session_id = ?3"#,
        params![agent_id, now, parent_session],
    )?;
    Ok(())
}

/// 将直接写 `agent_subagent.report_json`（如旧测试/旧记录）的未投递报告同步到 `agent_subagent_report`。
fn sync_legacy_reports_for_parent(
    conn: &Connection,
    parent_session: &str,
    workspace_external: &str,
) -> Result<()> {
    conn.execute(
        r#"INSERT OR IGNORE INTO agent_subagent_report
               (external_id, agent_id, child_user_message_id, status, report_json,
                completed_at_ms, delivered_at_ms)
           SELECT
               'rep-legacy-' || a.external_id,
               a.external_id,
               (SELECT m.external_id FROM agent_message m
                WHERE m.session_id = a.child_session_id AND m.role = 'user'
                ORDER BY m.sequence DESC LIMIT 1),
               CASE WHEN a.status = 'failed' THEN 'failed' ELSE 'completed' END,
               a.report_json,
               COALESCE(a.completed_at_ms, a.updated_at_ms, a.created_at_ms),
               CASE WHEN a.report_delivered <> 0
                    THEN COALESCE(a.completed_at_ms, a.updated_at_ms, a.created_at_ms)
                    ELSE NULL
               END
           FROM agent_subagent a
           JOIN agent_session p ON p.external_id = a.parent_session_id
           WHERE a.parent_session_id = ?1 AND p.workspace_id = ?2
             AND a.report_json IS NOT NULL AND a.report_json <> ''
             AND NOT EXISTS (
                 SELECT 1 FROM agent_subagent_report r WHERE r.agent_id = a.external_id
             )"#,
        params![parent_session, workspace_external],
    )?;
    Ok(())
}

/// 剥离报告正文中的控制字符（保留换行和制表符）并限制长度。
fn sanitize_report_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_REPORT_BYTES));
    for ch in text.chars() {
        if ch == '\n' || ch == '\t' || !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// 会话必须属于该工作区，否则按不存在处理。
pub(super) fn ensure_session_owned(
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

//! 子 agent 持久化：spawn 事务、终态与逐轮报告、列表、报告批次与启动恢复。
//!
//! 一个 subagent = 一个独立子会话（`source='subagent'`）挂到父会话；子会话的
//! 消息/parts/tools 与普通会话同一套表，回看走 `load_chat_session`。
//! 输入队列与有序输入项位于 [`queue`] 子模块；记录读取位于 [`record`]；报告批量投递位于 [`report`]。

mod queue;
mod record;
mod report;
pub use queue::{
    ClaimedSubagentTurn, EnqueuedInput, PersistedInputItem, SubagentInputRecord, SubagentQueueState,
};
pub use record::{ReportPayload, SubagentCommitted, SubagentRecord, SubagentReportRecord};
pub(super) use record::{ensure_session_owned, read_subagent_row};

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::{ChatPart, ModelSnapshot, TurnOutcome, Usage};
use crate::chat::agent::{
    AgentRole, BaseRole, MAX_REPORT_BYTES, SubagentReportView, truncate_report_body,
};
use crate::db::{Database, DbError, Result, new_external_id, now_ms};

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
        self.begin_subagent_turn_with_options(
            workspace,
            parent_session,
            child_session_id,
            None,
            role,
            title,
            task,
            items,
            model,
            spawn_message_id,
            dedupe_key,
            parent_call_id,
            false,
            &None,
        )
    }

    /// spawn（支持 `nickname`、`fork_context` 历史克隆与图片路径重写）。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_subagent_turn_with_options(
        &self,
        workspace: &Path,
        parent_session: &str,
        child_session_id: Option<&str>,
        nickname: Option<&str>,
        role: AgentRole,
        title: &str,
        task: &str,
        items: &[PersistedInputItem],
        model: &ModelSnapshot,
        spawn_message_id: Option<&str>,
        dedupe_key: Option<&str>,
        parent_call_id: Option<&str>,
        fork_context: bool,
        image_rewrite: &Option<(String, String)>,
    ) -> Result<SubagentCommitted> {
        if task.trim().is_empty() && items.is_empty() {
            return Err(DbError::InvalidDraft);
        }
        let title = super::single_line_preview(title.trim(), 80);
        if title.is_empty() {
            return Err(DbError::InvalidDraft);
        }
        let clean_nickname = nickname
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
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
                    model_provider, model_id, variant, status, archived,
                    agent_nickname, agent_role, agent_path,
                    created_at_ms, updated_at_ms, last_message_at_ms)
               VALUES (?1, ?2, 'subagent', ?3, ?4, ?5, ?6, ?7, 'active', 0, ?8, ?9, ?10, ?11, ?11, ?11)"#,
            params![
                child_session_id,
                workspace_external,
                &title,
                super::single_line_preview(task, super::PREVIEW_CHARS),
                model.provider,
                model.model,
                model.variant,
                clean_nickname,
                role.as_str(),
                role.base().as_str(),
                now,
            ],
        )?;
        let (user_seq, turn_idx) = if fork_context {
            let boundary = super::fork::completed_boundary_sequence(&transaction, parent_session)?;
            super::fork::copy_completed_history_to_child(
                &transaction,
                parent_session,
                &child_session_id,
                boundary,
                image_rewrite,
            )?
        } else {
            (0, 0)
        };
        let assistant_seq = user_seq.saturating_add(1);
        let user_message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'user', 'complete', ?5)"#,
            params![user_message_id, child_session_id, user_seq, turn_idx, now],
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
               VALUES (?1, ?2, ?3, ?4, 'assistant', 'streaming', ?5, ?6, ?7, ?8)"#,
            params![
                assistant_message_id,
                child_session_id,
                assistant_seq,
                turn_idx,
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
                    status, nickname, role, agent_path, model_provider, model_id, variant, title,
                    created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'running', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)"#,
            params![
                agent_id,
                parent_session,
                child_session_id,
                spawn_message_id,
                clean_nickname,
                role.as_str(),
                role.base().as_str(),
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
        let row = transaction
            .query_row(
                r#"SELECT a.role, a.agent_path, a.nickname, a.child_session_id FROM agent_subagent a
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   WHERE a.external_id = ?1 AND p.workspace_id = ?2"#,
                params![agent_id, workspace_external],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((role, agent_path, nickname, child_session_id)) = row else {
            return Err(DbError::SessionMissing);
        };
        let base_role = agent_path.as_deref().and_then(BaseRole::parse);
        let role = role
            .as_deref()
            .and_then(|r| AgentRole::from_name_and_base(r, base_role))
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
                nickname: nickname
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
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
                          a.report_json, a.report_delivered, a.nickname, a.agent_path,
                          a.model_provider, a.model_id, a.variant
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

    /// 按 `agent_id` 精确匹配或按同父会话 `nickname`（ASCII 大小写不敏感）双向寻址子 agent。
    pub fn resolve_owned_subagent(
        &self,
        workspace: &Path,
        parent_session: &str,
        target_or_nickname: &str,
    ) -> Result<Option<SubagentRecord>> {
        let key = target_or_nickname.trim();
        if key.is_empty() {
            return Ok(None);
        }
        if let Some(record) = self.owned_subagent(workspace, parent_session, key)? {
            return Ok(Some(record));
        }
        let all = self.list_subagents(workspace, parent_session)?;
        Ok(all.into_iter().find(|rec| {
            rec.nickname
                .as_deref()
                .is_some_and(|nick| nick.eq_ignore_ascii_case(key))
        }))
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
                      a.report_json, a.report_delivered, a.nickname, a.agent_path,
                      a.model_provider, a.model_id, a.variant
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

    /// 关闭 idle 子 agent：`agent_subagent` 置 `cancelled`，其全部 `queued` 输入同事务置 `cancelled`。
    pub fn close_idle_subagent(
        &self,
        workspace: &Path,
        parent_session_id: &str,
        agent_id: &str,
    ) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        ensure_session_owned(&transaction, parent_session_id, &workspace_external)?;
        let updated = transaction.execute(
            r#"UPDATE agent_subagent
               SET status = 'cancelled', completed_at_ms = ?1, updated_at_ms = ?1
               WHERE external_id = ?2 AND parent_session_id = ?3"#,
            params![now, agent_id, parent_session_id],
        )?;
        if updated == 0 {
            return Err(DbError::SessionMissing);
        }
        transaction.execute(
            r#"UPDATE agent_subagent_input
               SET status = 'cancelled', completed_at_ms = ?1, error_code = NULL
               WHERE agent_id = ?2 AND status IN ('queued', 'interrupted')"#,
            params![now, agent_id],
        )?;
        transaction.commit()?;
        Ok(())
    }
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

#[cfg(test)]
mod tests;

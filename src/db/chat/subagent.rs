//! 子 agent 持久化：spawn 事务、终态与逐轮报告、列表、报告批次与启动恢复。
//!
//! 一个 subagent = 一个独立子会话（`source='subagent'`）挂到父会话；子会话的
//! 消息/parts/tools 与普通会话同一套表，回看走 `load_chat_session`。
//! 输入队列与有序输入项位于 [`queue`] 子模块；记录读取位于 [`record`]；报告批量投递位于 [`report`]。

mod queue;
mod record;
mod report;
pub use queue::{
    ClaimedSubagentTurn, EnqueuedInput, PersistedInputItem, SubagentInputRecord,
    SubagentQueueState, SubagentTurnEnvelopeMeta, parse_input_dedupe_meta,
};
pub use record::{ReportPayload, SubagentCommitted, SubagentRecord, SubagentReportRecord};
pub(super) use record::{ensure_session_owned, read_subagent_row};

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::{ChatPart, ModelSnapshot, TurnOutcome, Usage};
use crate::chat::agent::{AgentRole, MAX_REPORT_BYTES, SubagentReportView, truncate_report_body};
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
        self.begin_subagent_turn_with_full_options(
            workspace,
            parent_session,
            child_session_id,
            nickname,
            role,
            title,
            task,
            items,
            model,
            spawn_message_id,
            dedupe_key,
            parent_call_id,
            fork_context,
            None,
            None,
            image_rewrite,
        )
    }

    /// spawn（完整选项：规范 `agent_path` 与按轮次分叉 `fork_turns`）。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_subagent_turn_with_full_options(
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
        agent_path: Option<&str>,
        fork_turns: Option<u32>,
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
        let canonical_path = agent_path
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "/root/{}",
                    crate::chat::agent::slugify_segment(
                        clean_nickname
                            .as_deref()
                            .unwrap_or(child_session_id.as_str()),
                    )
                )
            });
        let stored_agent_path =
            crate::chat::agent::encode_stored_role_and_path(role.base(), &canonical_path);
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
                &stored_agent_path,
                now,
            ],
        )?;
        let (user_seq, turn_idx) = if fork_context || fork_turns.is_some_and(|turns| turns > 0) {
            let (start_seq, boundary) =
                super::fork::completed_fork_window(&transaction, parent_session, fork_turns)?;
            if start_seq == 0 {
                super::fork::copy_completed_history_to_child(
                    &transaction,
                    parent_session,
                    &child_session_id,
                    boundary,
                    image_rewrite,
                )?
            } else {
                super::fork::copy_completed_history_window_to_child(
                    &transaction,
                    parent_session,
                    &child_session_id,
                    start_seq,
                    boundary,
                    image_rewrite,
                )?
            }
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
                stored_agent_path,
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
        let (base_role, _) = crate::chat::agent::decode_stored_role_and_path(agent_path.as_deref());
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
        // 报告行锚到本轮输入自己的 user 消息，而不是轮内 steer 新开的片段。
        let report_user_message_id: Option<String> = transaction
            .query_row(
                r#"SELECT child_user_message_id FROM agent_subagent_input
                   WHERE agent_id = ?1 AND status = 'running'
                     AND child_user_message_id IS NOT NULL
                   LIMIT 1"#,
                params![agent_id],
                |row| row.get(0),
            )
            .optional()?;
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
                changed_paths: payload.changed_paths.clone(),
                duration_ms: payload.duration_ms,
                body: clean_body,
                delivered_at_ms: now,
            }
            .to_json()
        });
        // 半程汇报（interrupt / 预算熔断）也要落库，父 agent 才能在 `wait_agent`
        // 或报告批次里读到被中断前的进展；`close_agent` 走 `report = None` 不产生报告。
        //
        // 报告行 `status` 列受 CHECK 约束只允许 `completed` / `failed`：中断态
        // （`cancelled`）按 `failed` 归档，权威状态以 `report_json` 内的 `status` 为准。
        if let Some(ref json_text) = report_json
            && matches!(status, "completed" | "failed" | "cancelled")
        {
            let row_status = if status == "cancelled" {
                "failed"
            } else {
                status
            };
            transaction.execute(
                r#"INSERT INTO agent_subagent_report
                       (external_id, agent_id, child_user_message_id, status, report_json,
                        completed_at_ms, delivered_at_ms)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)"#,
                params![
                    new_external_id("rep"),
                    agent_id,
                    report_user_message_id
                        .as_deref()
                        .or(child_user_message_id.as_deref()),
                    row_status,
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

    /// 按 `agent_id` 精确匹配、昵称或规范路径（三合一）在根会话树内寻址子 agent。
    ///
    /// 直接子 agent 先用现有查询快路径解析；未命中时沿 `parent_session` 向上找到根
    /// 会话，再在整棵子 agent 树中按 `matches_agent_target` 匹配绝对或相对路径。
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
        let (root_session, _, caller_path) =
            self.resolve_session_tree_root(workspace, parent_session)?;
        let direct = self.list_subagents(workspace, parent_session)?;
        if let Some(record) = direct.into_iter().find(|rec| {
            crate::chat::agent::matches_agent_target(
                &rec.agent_id,
                rec.nickname.as_deref(),
                Some(&rec.agent_path),
                &caller_path,
                key,
            )
        }) {
            return Ok(Some(record));
        }
        let tree = self.list_subagent_tree(workspace, &root_session)?;
        Ok(tree.into_iter().find(|rec| {
            crate::chat::agent::matches_agent_target(
                &rec.agent_id,
                rec.nickname.as_deref(),
                Some(&rec.agent_path),
                &caller_path,
                key,
            )
        }))
    }

    /// 最近一次人类私聊的紧凑 transcript（user/assistant 文本，含轮内 steer）。
    ///
    /// 只有 `list_agents(include_private=true)` 会调用；默认路径不读取、不注入主 agent 上下文。
    /// 用 `agent_message.sequence` 而不是入队时间戳确定边界：优先级插队与同毫秒输入
    /// 都不会切错轮次。转录保留最近内容，超 4 KiB 时从尾部截取并回退到 UTF-8 边界。
    pub fn private_turn_tail(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
    ) -> Result<Option<String>> {
        const MAX_TAIL_BYTES: usize = 4096;
        const TRUNCATION_MARKER: &str = "… [truncated]\n";

        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        let start_sequence: Option<i64> = self
            .conn
            .query_row(
                r#"SELECT m.sequence
                   FROM agent_subagent_input i
                   JOIN agent_subagent a ON a.external_id = i.agent_id
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   JOIN agent_message m ON m.external_id = i.child_user_message_id
                   WHERE i.agent_id = ?1 AND a.parent_session_id = ?2
                     AND p.workspace_id = ?3
                     AND i.notify_parent = 0 AND i.child_user_message_id IS NOT NULL
                   ORDER BY m.sequence DESC LIMIT 1"#,
                params![agent_id, parent_session, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        let Some(start_sequence) = start_sequence else {
            return Ok(None);
        };
        // 下一条正式提交的 user 消息（含汇报轮）是私聊转录的硬边界。
        let end_sequence: Option<i64> = self
            .conn
            .query_row(
                r#"SELECT MIN(m.sequence)
                   FROM agent_subagent_input i
                   JOIN agent_message m ON m.external_id = i.child_user_message_id
                   WHERE i.agent_id = ?1 AND i.child_user_message_id IS NOT NULL
                     AND m.sequence > ?2"#,
                params![agent_id, start_sequence],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten();
        let child_session: Option<String> = self
            .conn
            .query_row(
                "SELECT child_session_id FROM agent_subagent WHERE external_id = ?1",
                params![agent_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(child_session) = child_session else {
            return Ok(None);
        };

        let mut statement = self.conn.prepare(
            r#"SELECT m.sequence, m.role, p.content
               FROM agent_message m
               JOIN agent_message_part p ON p.message_id = m.external_id AND p.kind = 'text'
               WHERE m.session_id = ?1 AND m.sequence >= ?2
                 AND (?3 IS NULL OR m.sequence < ?3)
                 AND m.role IN ('user', 'assistant')
               ORDER BY m.sequence ASC, p.position ASC"#,
        )?;
        let rows = statement.query_map(
            params![child_session, start_sequence, end_sequence],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        // 同一消息的多个 text part 按段落拼接，避免 `done.Now` 粘连。
        let mut messages: Vec<String> = Vec::new();
        let mut current: Option<(i64, String, String)> = None;
        for row in rows {
            let (sequence, role, content) = row?;
            if current.as_ref().map(|(seq, ..)| *seq) != Some(sequence) {
                if let Some((_, role, text)) = current.take() {
                    push_tail_message(&mut messages, &role, &text);
                }
                current = Some((sequence, role, String::new()));
            }
            if let Some((_, _, text)) = current.as_mut() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&content);
            }
        }
        if let Some((_, role, text)) = current {
            push_tail_message(&mut messages, &role, &text);
        }
        if messages.is_empty() {
            return Ok(None);
        }

        let budget = MAX_TAIL_BYTES.saturating_sub(TRUNCATION_MARKER.len());
        let mut selected: Vec<String> = Vec::new();
        let mut used = 0usize;
        let mut truncated = false;
        for message in messages.into_iter().rev() {
            let separator = usize::from(!selected.is_empty());
            if used + separator + message.len() <= budget {
                used += separator + message.len();
                selected.push(message);
                continue;
            }
            let available = budget.saturating_sub(used + separator);
            if available > 0 {
                selected.push(tail_by_bytes(&message, available));
            }
            truncated = true;
            break;
        }
        selected.reverse();
        let mut out = String::new();
        if truncated {
            out.push_str(TRUNCATION_MARKER);
        }
        out.push_str(&selected.join("\n"));
        let out = out.trim_end().to_string();
        Ok((!out.is_empty()).then_some(out))
    }

    /// 子会话本轮所有片段（含轮内 steer）的已提交改动路径并集。
    pub fn list_changed_paths_for_turn(
        &self,
        workspace: &Path,
        child_session: &str,
        user_message_ids: &[String],
    ) -> Result<(usize, Vec<String>)> {
        if user_message_ids.is_empty() {
            return Ok((0, Vec::new()));
        }
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok((0, Vec::new()));
        };
        let placeholders = std::iter::repeat_n("?", user_message_ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut values: Vec<rusqlite::types::Value> =
            Vec::with_capacity(user_message_ids.len() + 3);
        values.push(rusqlite::types::Value::Text(child_session.to_string()));
        for id in user_message_ids {
            values.push(id.clone().into());
        }
        values.push(workspace_external.into());
        let count_sql = format!(
            r#"SELECT COUNT(DISTINCT m.target_path) FROM agent_file_mutation m
               WHERE m.session_id = ?1 AND m.user_message_id IN ({placeholders})
                 AND m.state = 'committed' AND m.success = 1
                 AND m.session_id IN (
                     SELECT external_id FROM agent_session WHERE workspace_id = ?{}
                 )"#,
            user_message_ids.len() + 2
        );
        let total: i64 = self.conn.query_row(
            &count_sql,
            rusqlite::params_from_iter(values.iter()),
            |row| row.get(0),
        )?;
        let total = usize::try_from(total).unwrap_or_default();
        if total == 0 {
            return Ok((0, Vec::new()));
        }
        let limit_placeholder = user_message_ids.len() + 3;
        let list_sql = format!(
            r#"SELECT m.target_path FROM agent_file_mutation m
               WHERE m.session_id = ?1 AND m.user_message_id IN ({placeholders})
                 AND m.state = 'committed' AND m.success = 1
                 AND m.session_id IN (
                     SELECT external_id FROM agent_session WHERE workspace_id = ?{}
                 )
               GROUP BY m.target_path
               ORDER BY MIN(m.id) ASC
               LIMIT ?{limit_placeholder}"#,
            user_message_ids.len() + 2
        );
        values.push((crate::chat::agent::MAX_REPORT_CHANGED_PATHS as i64).into());
        let mut statement = self.conn.prepare(&list_sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), |row| {
            row.get::<_, String>(0)
        })?;
        let mut paths = Vec::new();
        for row in rows {
            let raw = row?;
            let relative = Path::new(&raw)
                .strip_prefix(workspace)
                .map(|path| {
                    if path.as_os_str().is_empty() {
                        ".".to_string()
                    } else {
                        path.display().to_string()
                    }
                })
                .unwrap_or(raw);
            paths.push(relative);
        }
        Ok((total, paths))
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

    /// 向上追溯会话所属根会话：返回 `(root_session_id, caller_depth, caller_agent_path)`。
    ///
    /// 根会话返回 `(session_id, 0, "/root")`；子会话返回其父/祖父链的根、深度与自身规范路径。
    /// 最多向上追溯 8 跳防环。
    pub fn resolve_session_tree_root(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<(String, usize, String)> {
        let root_path = crate::chat::agent::ROOT_AGENT_PATH.to_string();
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok((session_id.to_string(), 0, root_path));
        };
        let mut current = session_id.to_string();
        let mut depth = 0usize;
        let mut current_path = root_path;
        for _ in 0..8 {
            let row: Option<(String, Option<String>, Option<String>)> = self
                .conn
                .query_row(
                    r#"SELECT a.parent_session_id, a.agent_path, a.nickname
                       FROM agent_subagent a
                       JOIN agent_session p ON p.external_id = a.parent_session_id
                       WHERE a.child_session_id = ?1 AND p.workspace_id = ?2"#,
                    params![current, workspace_external],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((parent_id, stored_path, nickname)) = row else {
                break;
            };
            if depth == 0 {
                let (_, decoded_path) =
                    crate::chat::agent::decode_stored_role_and_path(stored_path.as_deref());
                current_path = decoded_path.unwrap_or_else(|| {
                    format!(
                        "{}/{}",
                        crate::chat::agent::ROOT_AGENT_PATH,
                        crate::chat::agent::slugify_segment(
                            nickname.as_deref().unwrap_or(current.as_str())
                        )
                    )
                });
            }
            current = parent_id;
            depth = depth.saturating_add(1);
        }
        Ok((current, depth, current_path))
    }

    /// 列出同一根会话树下的全部子孙 `SubagentRecord`（按创建时间升序）。
    pub fn list_subagent_tree(
        &self,
        workspace: &Path,
        root_session: &str,
    ) -> Result<Vec<SubagentRecord>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"WITH RECURSIVE tree(
                   external_id, child_session_id, parent_session_id, spawn_message_id,
                   role, status, title, created_at_ms, completed_at_ms, report_json,
                   report_delivered, nickname, agent_path, model_provider, model_id, variant,
                   depth
               ) AS (
                   SELECT a.external_id, a.child_session_id, a.parent_session_id, a.spawn_message_id,
                          a.role, a.status, a.title, a.created_at_ms, a.completed_at_ms,
                          a.report_json, a.report_delivered, a.nickname, a.agent_path,
                          a.model_provider, a.model_id, a.variant, 1
                   FROM agent_subagent a
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   WHERE a.parent_session_id = ?1 AND p.workspace_id = ?2
                   UNION ALL
                   SELECT c.external_id, c.child_session_id, c.parent_session_id, c.spawn_message_id,
                          c.role, c.status, c.title, c.created_at_ms, c.completed_at_ms,
                          c.report_json, c.report_delivered, c.nickname, c.agent_path,
                          c.model_provider, c.model_id, c.variant, t.depth + 1
                   FROM agent_subagent c
                   JOIN tree t ON c.parent_session_id = t.child_session_id
                   WHERE t.depth < 8
               )
               SELECT external_id, child_session_id, parent_session_id, spawn_message_id,
                      role, status, title, created_at_ms, completed_at_ms, report_json,
                      report_delivered, nickname, agent_path, model_provider, model_id, variant
               FROM tree
               ORDER BY created_at_ms ASC, external_id ASC"#,
        )?;
        let rows =
            statement.query_map(params![root_session, workspace_external], read_subagent_row)?;
        let mut records = Vec::new();
        for row in rows {
            let (record, role) = row?;
            if role.as_deref().and_then(AgentRole::parse).is_some() {
                records.push(record);
            }
        }
        Ok(records)
    }

    /// 子轮已提交的变更文件路径：返回 `(去重总数, 最多 32 条相对 workspace 的路径)`。
    ///
    /// 按首次触碰顺序取 `target_path`，能用 `workspace` 前缀剥离时输出相对路径。
    pub fn list_changed_paths(
        &self,
        workspace: &Path,
        child_session: &str,
        child_user_message_id: &str,
    ) -> Result<(usize, Vec<String>)> {
        self.list_changed_paths_for_turn(
            workspace,
            child_session,
            std::slice::from_ref(&child_user_message_id.to_string()),
        )
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
               WHERE m.session_id = ?1 AND m.user_message_id = ?2
                 AND m.state = 'committed' AND m.success = 1
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

/// 私聊转录的一条消息：跳过空文本，统一 `role: text` 前缀。
fn push_tail_message(messages: &mut Vec<String>, role: &str, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    messages.push(format!("{role}: {text}"));
}

/// 从字符串尾部截取至多 `max` 字节，并回退到 UTF-8 字符边界。
fn tail_by_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut start = text.len() - max;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
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

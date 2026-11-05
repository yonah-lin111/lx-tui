//! 子 agent 输入队列与有序输入项的数据库读写：入队、取队首、认领开轮、错误标记与取消。

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::ensure_session_owned;
use crate::chat::agent::MAX_TASK_CHARS;
use crate::db::chat::ModelSnapshot;
use crate::db::{Database, DbError, Result, new_external_id, now_ms};

/// 持久化子 agent 输入的单个有序条目（图片已归档并通过摘要校验）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistedInputItem {
    /// 纯文本项。
    Text { text: String },
    /// 已归档图片项（绝不保存原始 `data:` base64 正文）。
    Image {
        label: String,
        archive_path: String,
        mime: String,
        content_hash: String,
        width: u32,
        height: u32,
    },
}

impl PersistedInputItem {
    /// 提取全部文本项拼接成的消息正文；纯图片输入回退为 `"[N image(s)]"`。
    pub fn joined_text(items: &[Self]) -> String {
        let mut texts = Vec::new();
        let mut images = 0usize;
        for item in items {
            match item {
                Self::Text { text } => {
                    if !text.is_empty() {
                        texts.push(text.as_str());
                    }
                }
                Self::Image { .. } => {
                    images = images.saturating_add(1);
                }
            }
        }
        if !texts.is_empty() {
            texts.join("\n")
        } else if images > 0 {
            format!("[{images} image(s)]")
        } else {
            String::new()
        }
    }
}

/// 一条持久化的子 agent 输入记录（含其有序 items）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentInputRecord {
    pub submission_id: String,
    pub agent_id: String,
    pub dedupe_key: String,
    pub parent_call_id: Option<String>,
    pub parent_message_id: Option<String>,
    pub position: i64,
    pub priority: i64,
    pub status: String,
    pub child_user_message_id: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub error_code: Option<String>,
    pub items: Vec<PersistedInputItem>,
}

impl SubagentInputRecord {
    /// 是否为 `trigger_turn = true` 的唤醒输入（`msg:` 前缀表示 `trigger_turn = false` 仅入队）。
    pub fn trigger_turn(&self) -> bool {
        !self.dedupe_key.starts_with("msg:")
    }

    /// 从 `dedupe_key`（`send:<caller_session>:...` 或 `msg:<caller_session>:...`）提取发送方会话 ID。
    pub fn caller_session_id(&self) -> Option<&str> {
        parse_input_dedupe_meta(&self.dedupe_key).1
    }
}

/// 单轮子会话输入的信封元数据（由 `agent_subagent_input` 派生，零 schema 变更）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentTurnEnvelopeMeta {
    pub position: i64,
    pub trigger_turn: bool,
    pub caller_session_id: Option<String>,
}

/// 解析 `dedupe_key` 中的 `(trigger_turn, caller_session_id)`。
pub fn parse_input_dedupe_meta(dedupe_key: &str) -> (bool, Option<&str>) {
    let trigger_turn = !dedupe_key.starts_with("msg:");
    let caller_session_id = dedupe_key
        .strip_prefix("send:")
        .or_else(|| dedupe_key.strip_prefix("msg:"))
        .and_then(|rest| rest.split(':').next())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    (trigger_turn, caller_session_id)
}

/// `enqueue_input` 的提交回执。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueuedInput {
    pub submission_id: String,
    pub agent_id: String,
    pub status: String,
    pub queued_count: usize,
    /// `false` 表示命中幂等 `dedupe_key` 返回既有记录。
    pub created: bool,
}

/// `claim_input` 开启子会话新一轮的提交回执。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedSubagentTurn {
    pub submission_id: String,
    pub agent_id: String,
    pub child_session_id: String,
    pub child_user_message_id: String,
    pub child_assistant_id: String,
    pub turn_index: i64,
    pub user_sequence: i64,
    pub assistant_sequence: i64,
    pub started_at_ms: i64,
    pub items: Vec<PersistedInputItem>,
    pub queued_remaining: usize,
}

/// 单个子 agent 的队列投影摘要（供 `list_agents` 与 UI 面板读取）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubagentQueueState {
    pub queued_count: usize,
    /// 队首 queued 输入若存在错误码（如 `image_missing`），投影为 `blocked`。
    pub head_error_code: Option<String>,
    /// 最近一条输入的状态（用于区分崩溃恢复后的 `interrupted` 与显式关闭的 `cancelled`）。
    pub last_input_status: Option<String>,
}

impl Database {
    /// 将一条输入（含结构化 items）原子入队；同 `dedupe_key` 重复调用幂等返回原记录。
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_input(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
        dedupe_key: &str,
        parent_call_id: Option<&str>,
        parent_message_id: Option<&str>,
        items: &[PersistedInputItem],
        interrupt: bool,
    ) -> Result<EnqueuedInput> {
        let dedupe_key = dedupe_key.trim();
        if dedupe_key.is_empty() || items.is_empty() {
            return Err(DbError::InvalidDraft);
        }
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        ensure_agent_owned(&transaction, agent_id, parent_session, &workspace_external)?;

        if let Some((existing_id, existing_agent, existing_status)) = transaction
            .query_row(
                r#"SELECT external_id, agent_id, status
                   FROM agent_subagent_input
                   WHERE dedupe_key = ?1"#,
                params![dedupe_key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
        {
            if existing_agent != agent_id {
                return Err(DbError::PositionConflict);
            }
            let queued_count = count_queued_in_tx(&transaction, agent_id)?;
            transaction.commit()?;
            return Ok(EnqueuedInput {
                submission_id: existing_id,
                agent_id: existing_agent,
                status: existing_status,
                queued_count,
                created: false,
            });
        }

        let position: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM agent_subagent_input WHERE agent_id = ?1",
            params![agent_id],
            |row| row.get(0),
        )?;
        let priority: i64 = if interrupt { 1 } else { 0 };
        let submission_id = new_external_id("input");
        transaction.execute(
            r#"INSERT INTO agent_subagent_input
                   (external_id, agent_id, dedupe_key, parent_call_id, parent_message_id,
                    position, priority, status, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'queued', ?8)"#,
            params![
                submission_id,
                agent_id,
                dedupe_key,
                parent_call_id,
                parent_message_id,
                position,
                priority,
                now,
            ],
        )?;
        insert_input_items(&transaction, &submission_id, items)?;
        let queued_count = count_queued_in_tx(&transaction, agent_id)?;
        transaction.commit()?;
        Ok(EnqueuedInput {
            submission_id,
            agent_id: agent_id.to_string(),
            status: "queued".to_string(),
            queued_count,
            created: true,
        })
    }

    /// 取指定子 agent 的队首 `queued` 输入（按 `priority DESC, position ASC` 排序）。
    pub fn next_queued_input(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
    ) -> Result<Option<SubagentInputRecord>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        ensure_agent_owned(&self.conn, agent_id, parent_session, &workspace_external)?;
        let head = self
            .conn
            .query_row(
                r#"SELECT external_id, agent_id, dedupe_key, parent_call_id, parent_message_id,
                          position, priority, status, child_user_message_id,
                          created_at_ms, started_at_ms, completed_at_ms, error_code
                   FROM agent_subagent_input
                   WHERE agent_id = ?1 AND status = 'queued'
                   ORDER BY priority DESC, position ASC
                   LIMIT 1"#,
                params![agent_id],
                read_input_header,
            )
            .optional()?;
        let Some(mut record) = head else {
            return Ok(None);
        };
        record.items = load_items_for_input(&self.conn, &record.submission_id)?;
        Ok(Some(record))
    }

    /// 在队首 `queued` 输入上设置或清除错误码（如 `image_missing` 阻断队列）。
    pub fn set_input_error(
        &self,
        workspace: &Path,
        parent_session: &str,
        submission_id: &str,
        error_code: Option<&str>,
    ) -> Result<bool> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let updated = self.conn.execute(
            r#"UPDATE agent_subagent_input
               SET error_code = ?1
               WHERE external_id = ?2 AND status = 'queued'
                 AND agent_id IN (
                     SELECT a.external_id FROM agent_subagent a
                     JOIN agent_session p ON p.external_id = a.parent_session_id
                     WHERE a.parent_session_id = ?3 AND p.workspace_id = ?4
                 )"#,
            params![
                error_code,
                submission_id,
                parent_session,
                workspace_external
            ],
        )?;
        Ok(updated > 0)
    }

    /// 认领一条无错误的 `queued` 输入并在原 `child_session_id` 下开启新一轮 user/assistant 消息。
    pub fn claim_input(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
        submission_id: &str,
        model: &ModelSnapshot,
    ) -> Result<ClaimedSubagentTurn> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let child_session_id =
            ensure_agent_owned(&transaction, agent_id, parent_session, &workspace_external)?;

        // 防止并发重入：同一子会话不能已有 running 输入或 streaming assistant。
        let running_exists: Option<i64> = transaction
            .query_row(
                r#"SELECT 1 FROM agent_subagent_input
                   WHERE agent_id = ?1 AND status = 'running'
                   UNION ALL
                   SELECT 1 FROM agent_message
                   WHERE session_id = ?2 AND status = 'streaming'
                   LIMIT 1"#,
                params![agent_id, child_session_id],
                |row| row.get(0),
            )
            .optional()?;
        if running_exists.is_some() {
            return Err(DbError::MutationConflict);
        }

        let input_row: Option<(String, Option<String>)> = transaction
            .query_row(
                r#"SELECT status, error_code FROM agent_subagent_input
                   WHERE external_id = ?1 AND agent_id = ?2"#,
                params![submission_id, agent_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((status, error_code)) = input_row else {
            return Err(DbError::SessionMissing);
        };
        if status != "queued" || error_code.is_some() {
            return Err(DbError::MutationConflict);
        }
        let items = load_items_for_input(&transaction, submission_id)?;
        if items.is_empty() {
            return Err(DbError::InvalidDraft);
        }
        let user_text = PersistedInputItem::joined_text(&items);

        let user_sequence = super::super::next_session_sequence(&transaction, &child_session_id)?;
        let (turn_index, revision): (i64, i64) = transaction.query_row(
            r#"SELECT
                   COALESCE((SELECT MAX(turn_index) + 1 FROM agent_message WHERE session_id = ?1), 0),
                   COALESCE((SELECT revision FROM agent_session WHERE external_id = ?1), 0)"#,
            params![child_session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let assistant_sequence = user_sequence.saturating_add(1);
        let user_message_id = new_external_id("msg");
        let assistant_message_id = new_external_id("msg");

        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'user', 'complete', ?5)"#,
            params![
                user_message_id,
                child_session_id,
                user_sequence,
                turn_index,
                now
            ],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, created_at_ms)
               VALUES (?1, ?2, 0, 'text', ?3, ?4)"#,
            params![new_external_id("part"), user_message_id, user_text, now],
        )?;
        insert_archived_image_attachments(&transaction, &user_message_id, &items)?;
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status,
                    model_provider, model_id, variant, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'assistant', 'streaming', ?5, ?6, ?7, ?8)"#,
            params![
                assistant_message_id,
                child_session_id,
                assistant_sequence,
                turn_index,
                model.provider,
                model.model,
                model.variant,
                now,
            ],
        )?;
        super::super::rewind::begin_turn_scope(
            &transaction,
            &child_session_id,
            &user_message_id,
            now,
        )?;
        transaction.execute(
            r#"UPDATE agent_subagent_input
               SET status = 'running', child_user_message_id = ?1, started_at_ms = ?2, error_code = NULL
               WHERE external_id = ?3 AND status = 'queued'"#,
            params![user_message_id, now, submission_id],
        )?;
        transaction.execute(
            r#"UPDATE agent_subagent
               SET status = 'running', completed_at_ms = NULL, updated_at_ms = ?1
               WHERE external_id = ?2"#,
            params![now, agent_id],
        )?;
        let next_revision = revision.saturating_add(1);
        transaction.execute(
            r#"UPDATE agent_session
               SET updated_at_ms = ?1, last_message_at_ms = ?1, revision = ?2
               WHERE external_id = ?3"#,
            params![now, next_revision, child_session_id],
        )?;
        let queued_remaining = count_queued_in_tx(&transaction, agent_id)?;
        transaction.commit()?;
        Ok(ClaimedSubagentTurn {
            submission_id: submission_id.to_string(),
            agent_id: agent_id.to_string(),
            child_session_id,
            child_user_message_id: user_message_id,
            child_assistant_id: assistant_message_id,
            turn_index,
            user_sequence,
            assistant_sequence,
            started_at_ms: now,
            items,
            queued_remaining,
        })
    }

    /// 显式结算一条 running 输入的状态（`completed` / `failed` / `interrupted` / `cancelled`）。
    pub fn finish_input(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
        submission_id: Option<&str>,
        status: &str,
        error_code: Option<&str>,
    ) -> Result<bool> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        ensure_agent_owned(&transaction, agent_id, parent_session, &workspace_external)?;
        let updated = match submission_id {
            Some(id) => transaction.execute(
                r#"UPDATE agent_subagent_input
                   SET status = ?1, completed_at_ms = ?2, error_code = COALESCE(?3, error_code)
                   WHERE external_id = ?4 AND agent_id = ?5 AND status = 'running'"#,
                params![status, now, error_code, id, agent_id],
            )?,
            None => transaction.execute(
                r#"UPDATE agent_subagent_input
                   SET status = ?1, completed_at_ms = ?2, error_code = COALESCE(?3, error_code)
                   WHERE agent_id = ?4 AND status = 'running'"#,
                params![status, now, error_code, agent_id],
            )?,
        };
        transaction.commit()?;
        Ok(updated > 0)
    }

    /// 取消一条排队输入（`cancel_agent_input`）：
    /// - `queued`（含带 `error_code` 的 blocked 项）转为 `cancelled`；
    /// - 已经是 `cancelled` 幂等返回 `cancelled`；
    /// - `running` / `completed` / `failed` / `interrupted` 保持原状并返回当前状态，不回滚。
    pub fn cancel_input(
        &self,
        workspace: &Path,
        parent_session: &str,
        submission_id: &str,
    ) -> Result<Option<(String, String, usize)>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let row: Option<(String, String)> = transaction
            .query_row(
                r#"SELECT i.agent_id, i.status
                   FROM agent_subagent_input i
                   JOIN agent_subagent a ON a.external_id = i.agent_id
                   JOIN agent_session p ON p.external_id = a.parent_session_id
                   WHERE i.external_id = ?1 AND a.parent_session_id = ?2 AND p.workspace_id = ?3"#,
                params![submission_id, parent_session, workspace_external],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((agent_id, status)) = row else {
            return Ok(None);
        };
        let final_status = if status == "queued" {
            transaction.execute(
                r#"UPDATE agent_subagent_input
                   SET status = 'cancelled', completed_at_ms = ?1, error_code = NULL
                   WHERE external_id = ?2 AND status = 'queued'"#,
                params![now, submission_id],
            )?;
            "cancelled".to_string()
        } else {
            status
        };
        let queued_count = count_queued_in_tx(&transaction, &agent_id)?;
        transaction.commit()?;
        Ok(Some((agent_id, final_status, queued_count)))
    }

    /// 修复指定子 agent 上次崩溃遗留的 streaming 消息与 running 输入（`resume_agent` 前调用）。
    pub fn recover_subagent_for_resume(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
    ) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let child_session_id =
            ensure_agent_owned(&transaction, agent_id, parent_session, &workspace_external)?;
        transaction.execute(
            r#"UPDATE agent_message
               SET status = 'interrupted', completed_at_ms = ?1
               WHERE session_id = ?2 AND status = 'streaming'"#,
            params![now, child_session_id],
        )?;
        transaction.execute(
            r#"UPDATE agent_subagent_input
               SET status = 'interrupted', completed_at_ms = ?1
               WHERE agent_id = ?2 AND status = 'running'"#,
            params![now, agent_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 批量读取当前父会话下所有子 agent 的队列统计（`queued_count`、队首 `error_code`、最近输入状态）。
    pub fn subagent_queue_states(
        &self,
        workspace: &Path,
        parent_session: &str,
    ) -> Result<HashMap<String, SubagentQueueState>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(HashMap::new());
        };
        let mut stmt = self.conn.prepare(
            r#"SELECT i.agent_id, i.status, i.error_code, i.priority, i.position
               FROM agent_subagent_input i
               JOIN agent_subagent a ON a.external_id = i.agent_id
               JOIN agent_session p ON p.external_id = a.parent_session_id
               WHERE a.parent_session_id = ?1 AND p.workspace_id = ?2
               ORDER BY i.agent_id, i.position ASC"#,
        )?;
        let rows = stmt.query_map(params![parent_session, workspace_external], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        let mut map: HashMap<String, SubagentQueueState> = HashMap::new();
        let mut best_head: HashMap<String, (i64, i64, Option<String>)> = HashMap::new();
        for row in rows {
            let (agent_id, status, error_code, priority, position) = row?;
            let entry = map.entry(agent_id.clone()).or_default();
            entry.last_input_status = Some(status.clone());
            if status == "queued" {
                entry.queued_count = entry.queued_count.saturating_add(1);
                let replace = match best_head.get(&agent_id) {
                    None => true,
                    Some((best_prio, best_pos, _)) => {
                        priority > *best_prio || (priority == *best_prio && position < *best_pos)
                    }
                };
                if replace {
                    best_head.insert(agent_id, (priority, position, error_code));
                }
            }
        }
        for (agent_id, (_, _, head_err)) in best_head {
            if let Some(entry) = map.get_mut(&agent_id) {
                entry.head_error_code = head_err;
            }
        }
        Ok(map)
    }

    /// 读取某个子会话下各 `child_user_message_id` 对应的有序结构化输入项（供跨轮上下文回放）。
    pub fn load_subagent_turn_items(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
    ) -> Result<HashMap<String, Vec<PersistedInputItem>>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(HashMap::new());
        };
        ensure_agent_owned(&self.conn, agent_id, parent_session, &workspace_external)?;
        let mut stmt = self.conn.prepare(
            r#"SELECT i.child_user_message_id, it.kind, it.text_content, it.label,
                      it.archive_path, it.mime, it.content_hash, it.width, it.height
               FROM agent_subagent_input i
               JOIN agent_subagent_input_item it ON it.input_id = i.external_id
               WHERE i.agent_id = ?1 AND i.child_user_message_id IS NOT NULL
               ORDER BY i.position ASC, it.position ASC"#,
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            let msg_id: String = row.get(0)?;
            let item = read_persisted_item(row, 1)?;
            Ok((msg_id, item))
        })?;
        let mut map: HashMap<String, Vec<PersistedInputItem>> = HashMap::new();
        for row in rows {
            let (msg_id, item) = row?;
            map.entry(msg_id).or_default().push(item);
        }
        Ok(map)
    }

    /// 读取某个子会话下各 `child_user_message_id` 对应的信封元数据（`position`、`trigger_turn`、`caller_session_id`）。
    pub fn load_subagent_turn_envelopes(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
    ) -> Result<HashMap<String, SubagentTurnEnvelopeMeta>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(HashMap::new());
        };
        ensure_agent_owned(&self.conn, agent_id, parent_session, &workspace_external)?;
        let mut stmt = self.conn.prepare(
            r#"SELECT child_user_message_id, position, dedupe_key
               FROM agent_subagent_input
               WHERE agent_id = ?1 AND child_user_message_id IS NOT NULL
               ORDER BY position ASC"#,
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            let msg_id: String = row.get(0)?;
            let position: i64 = row.get(1)?;
            let dedupe_key: String = row.get(2)?;
            let (trigger_turn, caller) = parse_input_dedupe_meta(&dedupe_key);
            Ok((
                msg_id,
                SubagentTurnEnvelopeMeta {
                    position,
                    trigger_turn,
                    caller_session_id: caller.map(str::to_string),
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (msg_id, meta) = row?;
            map.insert(msg_id, meta);
        }
        Ok(map)
    }

    /// 判断指定子 agent 的 `queued` 队列中是否存在至少一条可自动触发新一轮的输入（非 `msg:` 仅入队项）。
    pub fn has_triggerable_queued_input(
        &self,
        workspace: &Path,
        parent_session: &str,
        agent_id: &str,
    ) -> Result<bool> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(false);
        };
        ensure_agent_owned(&self.conn, agent_id, parent_session, &workspace_external)?;
        let exists: Option<i64> = self
            .conn
            .query_row(
                r#"SELECT 1 FROM agent_subagent_input
                   WHERE agent_id = ?1 AND status = 'queued' AND dedupe_key NOT LIKE 'msg:%'
                   LIMIT 1"#,
                params![agent_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(exists.is_some())
    }
}

/// 校验 `agent_id` 属于 `(parent_session, workspace_external)` 并返回其 `child_session_id`。
fn ensure_agent_owned(
    conn: &Connection,
    agent_id: &str,
    parent_session: &str,
    workspace_external: &str,
) -> Result<String> {
    ensure_session_owned(conn, parent_session, workspace_external)?;
    let child: Option<String> = conn
        .query_row(
            r#"SELECT a.child_session_id
               FROM agent_subagent a
               JOIN agent_session p ON p.external_id = a.parent_session_id
               WHERE a.external_id = ?1 AND a.parent_session_id = ?2 AND p.workspace_id = ?3"#,
            params![agent_id, parent_session, workspace_external],
            |row| row.get(0),
        )
        .optional()?;
    child.ok_or(DbError::SessionMissing)
}

/// 统计指定 `agent_id` 当前 `status = 'queued'` 的输入条数。
fn count_queued_in_tx(conn: &Connection, agent_id: &str) -> Result<usize> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM agent_subagent_input WHERE agent_id = ?1 AND status = 'queued'",
        params![agent_id],
        |row| row.get(0),
    )?;
    Ok(usize::try_from(count).unwrap_or(0))
}

/// 在事务内写入一条输入的全部有序 `PersistedInputItem`。
pub(super) fn insert_input_items(
    transaction: &Connection,
    input_id: &str,
    items: &[PersistedInputItem],
) -> Result<()> {
    let mut stmt = transaction.prepare_cached(
        r#"INSERT INTO agent_subagent_input_item
               (input_id, position, kind, text_content, label, archive_path,
                mime, content_hash, width, height)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
    )?;
    for (idx, item) in items.iter().enumerate() {
        let pos = i64::try_from(idx).unwrap_or(0);
        match item {
            PersistedInputItem::Text { text } => {
                let clean: String = text
                    .chars()
                    .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
                    .take(MAX_TASK_CHARS)
                    .collect();
                stmt.execute(params![
                    input_id,
                    pos,
                    "text",
                    clean,
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<String>::None,
                    Option::<i64>::None,
                    Option::<i64>::None,
                ])?;
            }
            PersistedInputItem::Image {
                label,
                archive_path,
                mime,
                content_hash,
                width,
                height,
            } => {
                stmt.execute(params![
                    input_id,
                    pos,
                    "image",
                    Option::<String>::None,
                    label,
                    archive_path,
                    mime,
                    content_hash,
                    i64::from(*width),
                    i64::from(*height),
                ])?;
            }
        }
    }
    Ok(())
}

/// 将已归档的图片项同步写入子会话用户消息的 `agent_message_attachment`（不回退原路径）。
pub(super) fn insert_archived_image_attachments(
    transaction: &Connection,
    user_message_id: &str,
    items: &[PersistedInputItem],
) -> Result<()> {
    let mut stmt = transaction.prepare_cached(
        r#"INSERT INTO agent_message_attachment
               (external_id, message_id, position, kind, label, marker_start, marker_end,
                source_path, mime, width, height)
           VALUES (?1, ?2, ?3, 'image', ?4, NULL, NULL, ?5, ?6, ?7, ?8)"#,
    )?;
    let mut pos = 0i64;
    for item in items {
        if let PersistedInputItem::Image {
            label,
            archive_path,
            mime,
            width,
            height,
            ..
        } = item
        {
            stmt.execute(params![
                new_external_id("att"),
                user_message_id,
                pos,
                label,
                archive_path,
                mime,
                i64::from(*width),
                i64::from(*height),
            ])?;
            pos = pos.saturating_add(1);
        }
    }
    Ok(())
}

/// 读取一行 `agent_subagent_input` 头部。
fn read_input_header(row: &rusqlite::Row<'_>) -> rusqlite::Result<SubagentInputRecord> {
    Ok(SubagentInputRecord {
        submission_id: row.get(0)?,
        agent_id: row.get(1)?,
        dedupe_key: row.get(2)?,
        parent_call_id: row.get(3)?,
        parent_message_id: row.get(4)?,
        position: row.get(5)?,
        priority: row.get(6)?,
        status: row.get(7)?,
        child_user_message_id: row.get(8)?,
        created_at_ms: row.get(9)?,
        started_at_ms: row.get(10)?,
        completed_at_ms: row.get(11)?,
        error_code: row.get(12)?,
        items: Vec::new(),
    })
}

/// 读取单条输入下的全部有序 `PersistedInputItem`。
fn load_items_for_input(conn: &Connection, input_id: &str) -> Result<Vec<PersistedInputItem>> {
    let mut stmt = conn.prepare_cached(
        r#"SELECT kind, text_content, label, archive_path, mime, content_hash, width, height
           FROM agent_subagent_input_item
           WHERE input_id = ?1
           ORDER BY position ASC"#,
    )?;
    let rows = stmt.query_map(params![input_id], |row| read_persisted_item(row, 0))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(DbError::from)
}

/// 从给定列偏移读取一个 `PersistedInputItem`。
fn read_persisted_item(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<PersistedInputItem> {
    let kind: String = row.get(offset)?;
    if kind == "image" {
        let width: i64 = row.get::<_, Option<i64>>(offset + 6)?.unwrap_or(0);
        let height: i64 = row.get::<_, Option<i64>>(offset + 7)?.unwrap_or(0);
        Ok(PersistedInputItem::Image {
            label: row
                .get::<_, Option<String>>(offset + 2)?
                .unwrap_or_else(|| "image.png".into()),
            archive_path: row
                .get::<_, Option<String>>(offset + 3)?
                .unwrap_or_default(),
            mime: row
                .get::<_, Option<String>>(offset + 4)?
                .unwrap_or_else(|| "image/png".into()),
            content_hash: row
                .get::<_, Option<String>>(offset + 5)?
                .unwrap_or_default(),
            width: u32::try_from(width).unwrap_or(0),
            height: u32::try_from(height).unwrap_or(0),
        })
    } else {
        Ok(PersistedInputItem::Text {
            text: row
                .get::<_, Option<String>>(offset + 1)?
                .unwrap_or_default(),
        })
    }
}

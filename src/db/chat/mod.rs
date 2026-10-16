//! 聊天会话持久化：会话列表、轮次写入、流式 checkpoint 与终态提交。
//!
//! 所有读写以 `workspace + session` 双条件校验，跨工作区操作返回拒绝或空结果；
//! `sequence` 只在持有写事务时读取递增，配合 worker 串行化保证同 session 单调。
//! `checkpoint`/`finish` 以 `status='streaming'` 条件更新，迟到事件不能覆盖终态。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, now_ms};

/// 会话列表条目（历史浮层展示用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub external_id: String,
    pub title: String,
    /// 首条用户消息的单行截断预览。
    pub first_user_message: String,
    pub model_provider: String,
    pub model_id: String,
    pub variant: Option<String>,
    /// `active` / `closed`。
    pub status: String,
    pub updated_at_ms: i64,
    pub last_message_at_ms: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
}

/// 一条聊天消息的展示记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub external_id: String,
    pub sequence: i64,
    pub turn_index: i64,
    /// `user` / `assistant`。
    pub role: String,
    /// `complete` / `streaming` / `interrupted` / `failed`。
    pub status: String,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub variant: Option<String>,
    pub stop_reason: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub created_at_ms: i64,
    /// 有序内容块（text / reasoning 交错保持原始顺序）。
    pub parts: Vec<ChatPart>,
}

/// 一条消息内的一个内容块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatPart {
    pub position: i64,
    /// `text` / `reasoning`。
    pub kind: String,
    pub content: String,
    /// 供应商专属元数据（如 Anthropic thinking signature），原样往返。
    pub provider_metadata_json: Option<String>,
}

/// `begin_turn` 的提交结果：一轮的完整落库标识。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCommitted {
    pub session_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub turn_index: i64,
}

/// 发送一轮时的模型快照（写入 assistant message 与 session 选择三列）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSnapshot {
    pub provider: String,
    pub model: String,
    pub variant: Option<String>,
}

/// 一轮的最终结局；`finish` 据此写终态与 usage。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnOutcome {
    Complete {
        stop_reason: Option<String>,
    },
    Interrupted,
    Failed {
        /// 裁剪后的稳定错误码；不含密钥或原始响应体。
        error_code: String,
    },
}

/// 供应商返回的用量；未知字段保持 None，不写 0 冒充真实值。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
}

/// 历史列表分页大小。
pub const SESSION_PAGE_SIZE: usize = 50;

/// 单条消息正文最大字节数（防止异常响应撑爆库）。
const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;

/// 首条用户消息预览的最大字符数。
const PREVIEW_CHARS: usize = 120;

impl Database {
    /// 工作区下的会话列表（最近活动在前、同毫秒按 id 倒序）；分页读取。
    pub fn list_chat_sessions(
        &self,
        workspace: &Path,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<SessionSummary>> {
        let Some(external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let limit = limit.clamp(1, SESSION_PAGE_SIZE * 4) as i64;
        let mut statement = self.conn.prepare(
            r#"SELECT external_id, title, first_user_message, model_provider, model_id, variant,
                      status, updated_at_ms, last_message_at_ms, total_input_tokens, total_output_tokens
               FROM agent_session
               WHERE workspace_id = ?1
               ORDER BY updated_at_ms DESC, id DESC
               LIMIT ?2 OFFSET ?3"#,
        )?;
        let rows = statement.query_map(params![external, limit, offset as i64], |row| {
            Ok(SessionSummary {
                external_id: row.get(0)?,
                title: row.get(1)?,
                first_user_message: row.get(2)?,
                model_provider: row.get(3)?,
                model_id: row.get(4)?,
                variant: row.get(5)?,
                status: row.get(6)?,
                updated_at_ms: row.get(7)?,
                last_message_at_ms: row.get(8)?,
                total_input_tokens: row.get(9)?,
                total_output_tokens: row.get(10)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 加载单个会话及全部有序消息（含 parts）；跨工作区或不存在返回 None。
    pub fn load_chat_session(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Option<Vec<ChatMessage>>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Ok(None);
        }
        let mut statement = self.conn.prepare(
            r#"SELECT external_id, sequence, turn_index, role, status, model_provider, model_id,
                      variant, stop_reason, input_tokens, output_tokens, created_at_ms
               FROM agent_message WHERE session_id = ?1 ORDER BY sequence"#,
        )?;
        let mut messages: Vec<ChatMessage> = statement
            .query_map(params![session_id], |row| {
                Ok(ChatMessage {
                    external_id: row.get(0)?,
                    sequence: row.get(1)?,
                    turn_index: row.get(2)?,
                    role: row.get(3)?,
                    status: row.get(4)?,
                    model_provider: row.get(5)?,
                    model_id: row.get(6)?,
                    variant: row.get(7)?,
                    stop_reason: row.get(8)?,
                    input_tokens: row.get(9)?,
                    output_tokens: row.get(10)?,
                    created_at_ms: row.get(11)?,
                    parts: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut parts_statement = self.conn.prepare(
            r#"SELECT message_id, position, kind, content, provider_metadata_json
               FROM agent_message_part WHERE message_id IN (
                   SELECT external_id FROM agent_message WHERE session_id = ?1
               ) ORDER BY message_id, position"#,
        )?;
        let mut by_message: std::collections::HashMap<String, Vec<ChatPart>> =
            std::collections::HashMap::new();
        let rows = parts_statement.query_map(params![session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                ChatPart {
                    position: row.get(1)?,
                    kind: row.get(2)?,
                    content: row.get(3)?,
                    provider_metadata_json: row.get(4)?,
                },
            ))
        })?;
        for row in rows {
            let (message, part) = row?;
            by_message.entry(message).or_default().push(part);
        }
        for message in &mut messages {
            if let Some(parts) = by_message.remove(&message.external_id) {
                message.parts = parts;
            }
        }
        Ok(Some(messages))
    }

    /// 开启一轮：同事务写入 user message（含 text part）与 streaming assistant message；
    /// 首轮（`existing_session` 为 None）先建 session。
    ///
    /// 失败返回错误且不产生半轮（整个事务回滚）。空草稿由应用层拦截，不进库。
    pub fn begin_turn(
        &self,
        workspace: &Path,
        existing_session: Option<&str>,
        user_text: &str,
        model: &ModelSnapshot,
    ) -> Result<TurnCommitted> {
        if user_text.trim().is_empty() {
            return Err(DbError::InvalidDraft);
        }
        if user_text.len() > MAX_MESSAGE_BYTES {
            return Err(DbError::MessageTooLarge);
        }
        self.ensure_workspace(workspace)?;
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        // 校验 session 归属并读取下一个 sequence/turn。
        let (session_id, next_sequence, turn_index) = match existing_session {
            Some(session_id) => {
                let row: Option<(i64, i64)> = transaction
                    .query_row(
                        r#"SELECT COALESCE(MAX(sequence) + 1, 0),
                                  COALESCE(MAX(turn_index) + 1, 0)
                           FROM agent_message WHERE session_id = ?1"#,
                        params![session_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let Some((next_sequence, turn_index)) = row else {
                    return Err(DbError::SessionMissing);
                };
                // 双条件归属校验：session 必须属于该 workspace。
                let owned: Option<i64> = transaction
                    .query_row(
                        "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                        params![session_id, workspace_external],
                        |row| row.get(0),
                    )
                    .optional()?;
                if owned.is_none() {
                    return Err(DbError::SessionMissing);
                }
                (session_id.to_string(), next_sequence, turn_index)
            }
            None => {
                let session_id = new_external_id("sess");
                let title = single_line_preview(user_text, 60);
                transaction.execute(
                    r#"INSERT INTO agent_session
                           (external_id, workspace_id, source, title, first_user_message,
                            model_provider, model_id, variant, status, archived,
                            created_at_ms, updated_at_ms, last_message_at_ms)
                       VALUES (?1, ?2, 'native', ?3, ?4, ?5, ?6, ?7, 'active', 0,
                               ?8, ?8, ?8)"#,
                    params![
                        session_id,
                        workspace_external,
                        title,
                        single_line_preview(user_text, PREVIEW_CHARS),
                        model.provider,
                        model.model,
                        model.variant,
                        now,
                    ],
                )?;
                (session_id, 0, 0)
            }
        };
        let user_message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'user', 'complete', ?5)"#,
            params![user_message_id, session_id, next_sequence, turn_index, now],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, created_at_ms)
               VALUES (?1, ?2, 0, 'text', ?3, ?4)"#,
            params![new_external_id("part"), user_message_id, user_text, now],
        )?;
        let assistant_message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status,
                    model_provider, model_id, variant, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'assistant', 'streaming', ?5, ?6, ?7, ?8)"#,
            params![
                assistant_message_id,
                session_id,
                next_sequence + 1,
                turn_index,
                model.provider,
                model.model,
                model.variant,
                now,
            ],
        )?;
        // 首轮把 session 的当前选择同步为该轮模型（后续 /model 覆盖这三列）。
        if existing_session.is_some() {
            update_session_selection(&transaction, &session_id, &workspace_external, model, now)?;
        }
        transaction.execute(
            "UPDATE agent_session SET updated_at_ms = ?1, last_message_at_ms = ?1 WHERE external_id = ?2",
            params![now, session_id],
        )?;
        transaction.commit()?;
        Ok(TurnCommitted {
            session_id,
            user_message_id,
            assistant_message_id,
            turn_index,
        })
    }

    /// 流式 checkpoint：用完整 parts 快照幂等覆盖 streaming assistant 的内容。
    ///
    /// 只在 `status='streaming'` 时生效；迟到 checkpoint 不能覆盖终态。
    pub fn checkpoint(
        &self,
        workspace: &Path,
        assistant_message_id: &str,
        parts: &[ChatPart],
    ) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let session_id =
            ensure_streaming_message(&transaction, &workspace_external, assistant_message_id)?;
        replace_parts(&transaction, assistant_message_id, parts, now)?;
        transaction.execute(
            "UPDATE agent_session SET updated_at_ms = ?1 WHERE external_id = ?2",
            params![now, session_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 终态提交：写最后 parts、状态、usage 与 session 时间；重复 finish 幂等。
    ///
    /// usage 只累计实际返回的 provider 值；重复投递同结果不重复累计 token。
    pub fn finish(
        &self,
        workspace: &Path,
        assistant_message_id: &str,
        parts: &[ChatPart],
        outcome: &TurnOutcome,
        usage: Usage,
    ) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        // 已终态：同结果幂等返回，不重复累计 usage。
        let existing: Option<String> = transaction
            .query_row(
                r#"SELECT m.status FROM agent_message m
                   JOIN agent_session s ON s.external_id = m.session_id
                   WHERE m.external_id = ?1 AND s.workspace_id = ?2"#,
                params![assistant_message_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        match existing.as_deref() {
            None => return Err(DbError::SessionMissing),
            Some("streaming") => {}
            Some(_) => return Ok(()),
        }
        let session_id: String = transaction.query_row(
            "SELECT session_id FROM agent_message WHERE external_id = ?1",
            params![assistant_message_id],
            |row| row.get(0),
        )?;
        replace_parts(&transaction, assistant_message_id, parts, now)?;
        let (status, stop_reason, error_code) = match outcome {
            TurnOutcome::Complete { stop_reason } => ("complete", stop_reason.clone(), None),
            TurnOutcome::Interrupted => ("interrupted", None, None),
            TurnOutcome::Failed { error_code } => ("failed", None, Some(error_code.clone())),
        };
        transaction.execute(
            r#"UPDATE agent_message
               SET status = ?1, stop_reason = ?2, error_code = ?3,
                   input_tokens = ?4, output_tokens = ?5,
                   cache_read_tokens = ?6, cache_write_tokens = ?7,
                   completed_at_ms = ?8
               WHERE external_id = ?9 AND status = 'streaming'"#,
            params![
                status,
                stop_reason,
                error_code,
                usage.input,
                usage.output,
                usage.cache_read,
                usage.cache_write,
                now,
                assistant_message_id,
            ],
        )?;
        // session 聚合只计实际返回的 usage；None 不贡献假 0。
        if let (Some(input), Some(output)) = (usage.input, usage.output) {
            transaction.execute(
                r#"UPDATE agent_session
                   SET total_input_tokens = total_input_tokens + ?1,
                       total_output_tokens = total_output_tokens + ?2
                   WHERE external_id = ?3"#,
                params![input, output, session_id],
            )?;
        }
        transaction.execute(
            "UPDATE agent_session SET updated_at_ms = ?1, last_message_at_ms = ?1 WHERE external_id = ?2",
            params![now, session_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 更新已保存会话的当前模型选择（`/model` 在已有 session 上生效）。
    pub fn set_chat_session_model(
        &self,
        workspace: &Path,
        session_id: &str,
        model: &ModelSnapshot,
    ) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        update_session_selection(
            &transaction,
            session_id,
            &workspace_external,
            model,
            now_ms(),
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 重命名会话（双条件校验）。
    pub fn rename_chat_session(
        &self,
        workspace: &Path,
        session_id: &str,
        title: &str,
    ) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let title = single_line_preview(title, 200);
        let updated = self.conn.execute(
            "UPDATE agent_session SET title = ?1, updated_at_ms = ?2 WHERE external_id = ?3 AND workspace_id = ?4",
            params![title, now_ms(), session_id, workspace_external],
        )?;
        if updated == 0 {
            return Err(DbError::SessionMissing);
        }
        Ok(())
    }

    /// 删除会话：双条件校验后级联删除消息/parts/工具/子 Agent 关系。
    pub fn delete_chat_session(&self, workspace: &Path, session_id: &str) -> Result<()> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let updated = self.conn.execute(
            "DELETE FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
            params![session_id, workspace_external],
        )?;
        if updated == 0 {
            return Err(DbError::SessionMissing);
        }
        Ok(())
    }

    /// 会话的最近模型选择（恢复会话时回填窗格状态）。
    pub fn chat_session_model(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Option<ModelSnapshot>> {
        let Some(external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        let row: Option<(String, String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT model_provider, model_id, variant FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, external],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        Ok(row.map(|(provider, model, variant)| ModelSnapshot {
            provider,
            model,
            variant,
        }))
    }

    /// 异常重启恢复：把遗留 `streaming` assistant 标记为 `interrupted`。
    ///
    /// 保留最后 checkpoint 的 parts，不自动重发或生成第二条用户消息。
    pub fn recover_interrupted_streams(&self) -> Result<usize> {
        let updated = self.conn.execute(
            "UPDATE agent_message SET status = 'interrupted', completed_at_ms = ?1 WHERE status = 'streaming'",
            params![now_ms()],
        )?;
        Ok(updated)
    }

    /// 供检查读取的连接引用（仅 db 模块内部与测试使用）。
    #[cfg(test)]
    pub(crate) fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// 校验目标消息仍为 streaming 且属于该 workspace；返回 session external id。
fn ensure_streaming_message(
    transaction: &Connection,
    workspace_external: &str,
    assistant_message_id: &str,
) -> Result<String> {
    let row: Option<(String, String)> = transaction
        .query_row(
            r#"SELECT m.session_id, m.status FROM agent_message m
               JOIN agent_session s ON s.external_id = m.session_id
               WHERE m.external_id = ?1 AND s.workspace_id = ?2"#,
            params![assistant_message_id, workspace_external],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((session_id, status)) if status == "streaming" => Ok(session_id),
        Some(_) => Err(DbError::MessageNotStreaming),
        None => Err(DbError::SessionMissing),
    }
}

/// 用完整快照替换消息 parts：先删旧再按 position 插入，保持交错顺序。
fn replace_parts(
    transaction: &Connection,
    assistant_message_id: &str,
    parts: &[ChatPart],
    now: i64,
) -> Result<()> {
    transaction.execute(
        "DELETE FROM agent_message_part WHERE message_id = ?1",
        params![assistant_message_id],
    )?;
    for (position, part) in parts.iter().enumerate() {
        if part.content.len() > MAX_MESSAGE_BYTES {
            return Err(DbError::MessageTooLarge);
        }
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, provider_metadata_json, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                new_external_id("part"),
                assistant_message_id,
                position as i64,
                part.kind,
                part.content,
                part.provider_metadata_json,
                now,
            ],
        )?;
    }
    Ok(())
}

/// 更新 session 的模型选择三列（双条件校验）。
fn update_session_selection(
    transaction: &Connection,
    session_id: &str,
    workspace_external: &str,
    model: &ModelSnapshot,
    now: i64,
) -> Result<()> {
    let updated = transaction.execute(
        r#"UPDATE agent_session
           SET model_provider = ?1, model_id = ?2, variant = ?3, updated_at_ms = ?4
           WHERE external_id = ?5 AND workspace_id = ?6"#,
        params![
            model.provider,
            model.model,
            model.variant,
            now,
            session_id,
            workspace_external
        ],
    )?;
    if updated == 0 {
        return Err(DbError::SessionMissing);
    }
    Ok(())
}

/// 单行化并按字符数截断预览文本。
fn single_line_preview(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.chars().count() >= max_chars {
            break;
        }
        match ch {
            '\n' | '\r' => out.push(' '),
            ch if (ch as u32) < 0x20 || ch == '\u{7f}' => {}
            ch => out.push(ch),
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests;

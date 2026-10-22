//! 聊天会话持久化：会话列表、轮次写入、流式 checkpoint 与终态提交。
//!
//! 所有读写以 `workspace + session` 双条件校验，跨工作区操作返回拒绝或空结果；
//! `sequence` 只在持有写事务时读取递增，配合 worker 串行化保证同 session 单调。
//! `checkpoint`/`finish` 以 `status='streaming'` 条件更新，迟到事件不能覆盖终态。

mod process;
pub mod rewind;
mod tools;

pub use process::{NewProcess, OutputSnapshot, ProcessFinish, ProcessRecord};
pub use rewind::{
    FileAvailability, ImageRecord, ImageWrite, LatestTurn, NoticeRecord, ObservedWrite,
    OperationRecord, PathMutationWrite, PreparedTicket, RestorePath, RestorePlan, RewindCommit,
    RewindCommitted, TurnScopeRecord,
};
pub use tools::{
    INTERRUPTED_ERROR_CODE, MAX_TOOL_ARGUMENT_BYTES, MAX_TOOL_RESULT_BYTES, ToolCallRecord,
    ToolCallResult, ToolCallTicket, ToolTerminalStatus,
};

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
    /// 失败原因码（如 `http 503`、`timeout`）；仅失败时有值。
    pub error_code: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: Option<i64>,
    pub cache_write_tokens: Option<i64>,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    /// 有序内容块（text / reasoning 交错保持原始顺序）。
    pub parts: Vec<ChatPart>,
    /// 该消息的工具调用（按 `(message_id, position)` 有序）；用户消息恒为空。
    pub tools: Vec<ToolCallRecord>,
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
    /// 该 part 首个增量到达时刻（毫秒）；非流式写入（用户消息）为 None。
    pub started_at_ms: Option<i64>,
    /// 该 part 最后一个增量到达时刻（毫秒）；None 表示未知（老库/非流式）。
    pub ended_at_ms: Option<i64>,
    /// 该 part 显式完成时刻（毫秒）；仅 reasoning 使用，None 表示尚未完成或非思考块。
    pub completed_at_ms: Option<i64>,
}

/// `begin_turn` 的提交结果：一轮的完整落库标识。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCommitted {
    pub session_id: String,
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub turn_index: i64,
    /// user / assistant 在会话时间线上的显示序号（与提示共用编号空间）。
    pub user_sequence: i64,
    pub assistant_sequence: i64,
    /// 本轮提交后的会话 revision。
    pub revision: i64,
    /// 本轮落库时刻（两个消息与 session 共用），用于在线计算本轮耗时。
    pub created_at_ms: i64,
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

    /// 加载单个会话及全部有序消息（含 parts 与工具调用）；跨工作区或不存在返回 None。
    ///
    /// 消息、parts、tools 各一次批量查询（不逐消息 N+1）；parts 与 tools 分别按
    /// `position` 有序并关联到所属消息，调用方据此合并为单条步骤时间线。
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
                      variant, stop_reason, error_code, input_tokens, output_tokens,
                      cache_read_tokens, cache_write_tokens, created_at_ms, completed_at_ms
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
                    error_code: row.get(9)?,
                    input_tokens: row.get(10)?,
                    output_tokens: row.get(11)?,
                    cache_read_tokens: row.get(12)?,
                    cache_write_tokens: row.get(13)?,
                    created_at_ms: row.get(14)?,
                    completed_at_ms: row.get(15)?,
                    parts: Vec::new(),
                    tools: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut parts_statement = self.conn.prepare(
            r#"SELECT message_id, position, kind, content, provider_metadata_json,
                      started_at_ms, ended_at_ms, completed_at_ms
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
                    started_at_ms: row.get(5)?,
                    ended_at_ms: row.get(6)?,
                    completed_at_ms: row.get(7)?,
                },
            ))
        })?;
        for row in rows {
            let (message, part) = row?;
            by_message.entry(message).or_default().push(part);
        }
        // 工具调用同样一次批量读取，避免逐消息查询。
        let mut tools_statement = self.conn.prepare(
            r#"SELECT call_id, message_id, name, arguments_json, position, status,
                      result_json, error_text, started_at_ms, completed_at_ms, duration_ms
               FROM agent_tool_call WHERE session_id = ?1
               ORDER BY message_id, position"#,
        )?;
        let mut tools_by_message: std::collections::HashMap<String, Vec<ToolCallRecord>> =
            std::collections::HashMap::new();
        let rows = tools_statement.query_map(params![session_id], tools::read_tool_call_record)?;
        for row in rows {
            let tool = row?;
            tools_by_message
                .entry(tool.message_id.clone())
                .or_default()
                .push(tool);
        }
        for message in &mut messages {
            if let Some(parts) = by_message.remove(&message.external_id) {
                message.parts = parts;
            }
            if let Some(tools) = tools_by_message.remove(&message.external_id) {
                message.tools = tools;
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
        // 校验 session 归属并读取下一个 sequence/turn。sequence 与独立提示共用编号空间，
        // 必须联合取号；轮次身份仍以稳定 user message id 为准，不依赖可复用的 turn_index。
        let (session_id, next_sequence, turn_index, revision) = match existing_session {
            Some(session_id) => {
                let row: Option<(i64, i64, i64)> = transaction
                    .query_row(
                        r#"SELECT
                               (SELECT COALESCE(MAX(sequence) + 1, 0) FROM (
                                   SELECT sequence FROM agent_message WHERE session_id = ?1
                                   UNION ALL
                                   SELECT sequence FROM agent_session_notice WHERE session_id = ?1
                               )),
                               COALESCE(MAX(turn_index) + 1, 0),
                               COALESCE((SELECT revision FROM agent_session WHERE external_id = ?1), 0)
                           FROM agent_message WHERE session_id = ?1"#,
                        params![session_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                let Some((next_sequence, turn_index, revision)) = row else {
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
                (session_id.to_string(), next_sequence, turn_index, revision)
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
                (session_id, 0, 0, 0)
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
        // 新一轮在 BeginChatTurn 事务内建立文件 scope：只有它完整封口才允许文件恢复。
        rewind::begin_turn_scope(&transaction, &session_id, &user_message_id, now)?;
        let revision = revision.saturating_add(1);
        transaction.execute(
            "UPDATE agent_session SET updated_at_ms = ?1, last_message_at_ms = ?1, revision = ?2 WHERE external_id = ?3",
            params![now, revision, session_id],
        )?;
        transaction.commit()?;
        Ok(TurnCommitted {
            session_id,
            user_message_id,
            assistant_message_id,
            turn_index,
            user_sequence: next_sequence,
            assistant_sequence: next_sequence + 1,
            revision,
            created_at_ms: now,
        })
    }

    /// 流式 checkpoint：用完整 parts 快照幂等覆盖 streaming assistant 的内容。
    ///
    /// 只在 `status='streaming'` 时生效；迟到 checkpoint 不能覆盖终态。
    /// 快照保留 app 传入的 position 与 `completed_at_ms`，且不删除也不覆盖
    /// `agent_tool_call` 行（工具行另有自己的事务）。
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
    /// 与 checkpoint 一致：保留传入 position 与 part 完成时刻，不动已落库的工具行。
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
        // 终态即封口本轮文件 scope：全部 mutation ACK 已在 finish_tool 前结算；
        // 仍有 prepared 记录时保持 capturing，恢复时按“证据不足”拒绝文件动作。
        rewind::seal_turn_scope(&transaction, &session_id, assistant_message_id)?;
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
        // 改名只改标题，不动 `updated_at_ms`：改备注不是对话活动，不该把该会话
        // 顶到"最近活动"列表最前（opencode 同款：备注名不影响排序）。
        let updated = self.conn.execute(
            "UPDATE agent_session SET title = ?1 WHERE external_id = ?2 AND workspace_id = ?3",
            params![title, session_id, workspace_external],
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

    /// 异常重启恢复：把遗留 `streaming` assistant 标记为 `interrupted`，
    /// 同事务补齐这些消息里仍缺完成时刻的 reasoning part。
    ///
    /// 保留最后 checkpoint 的 parts，不自动重发或生成第二条用户消息。
    /// 幂等：第二次调用不再改动任何行，返回 0。
    pub fn recover_interrupted_streams(&self) -> Result<usize> {
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let updated = transaction.execute(
            "UPDATE agent_message SET status = 'interrupted', completed_at_ms = ?1 WHERE status = 'streaming'",
            params![now],
        )?;
        // 遗留 streaming 消息的思考块按 `COALESCE(ended_at_ms, now)` 封闭，
        // 与 v9 迁移对已终态消息的回填口径一致。
        transaction.execute(
            r#"UPDATE agent_message_part
               SET completed_at_ms = COALESCE(ended_at_ms, ?1)
               WHERE kind = 'reasoning' AND completed_at_ms IS NULL
                 AND message_id IN (
                     SELECT external_id FROM agent_message WHERE status = 'interrupted'
                 )"#,
            params![now],
        )?;
        transaction.commit()?;
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

/// 用完整快照替换消息 parts：先删旧再按传入 position 插入，保留时间线位置。
///
/// 位置由调用方（app 步骤时间线）分配，写入前逐条校验该消息内不重复；
/// 重复或负值返回 `PositionConflict`，事务整体回滚，不产生半行。
/// 本函数不触碰 `agent_tool_call`：工具行与 parts 各占自己的位置区间。
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
    let mut positions: std::collections::HashSet<i64> = std::collections::HashSet::new();
    for part in parts {
        if part.content.len() > MAX_MESSAGE_BYTES {
            return Err(DbError::MessageTooLarge);
        }
        if part.position < 0 || !positions.insert(part.position) {
            return Err(DbError::PositionConflict);
        }
        // 同一位置已被工具调用占用时同样拒绝：两条表共用消息内时间线编号。
        let taken: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM agent_tool_call WHERE message_id = ?1 AND position = ?2",
                params![assistant_message_id, part.position],
                |row| row.get(0),
            )
            .optional()?;
        if taken.is_some() {
            return Err(DbError::PositionConflict);
        }
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, provider_metadata_json,
                    created_at_ms, started_at_ms, ended_at_ms, completed_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                new_external_id("part"),
                assistant_message_id,
                part.position,
                part.kind,
                part.content,
                part.provider_metadata_json,
                now,
                part.started_at_ms,
                part.ended_at_ms,
                part.completed_at_ms,
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

/// app 层聊天步骤视图 → 存储记录 parts。
///
/// position 由调用方的时间线给出（不再按枚举重排），`completed_at_ms` 原样携带；
/// 重复或负值位置会在写入时被 `replace_parts` 拒绝为 `PositionConflict`。
pub fn chat_part_views<'a>(
    parts: impl Iterator<Item = (i64, &'a crate::app::state::ChatPartView)>,
) -> Vec<ChatPart> {
    parts
        .map(|(position, part)| ChatPart {
            position,
            kind: part.kind_str().to_string(),
            content: part.content.clone(),
            provider_metadata_json: part.provider_metadata_json.clone(),
            started_at_ms: part.started_at_ms,
            ended_at_ms: part.ended_at_ms,
            completed_at_ms: part.completed_at_ms,
        })
        .collect()
}

/// 消息步骤时间线 → DB parts（使用步骤自带的真实 `position`，不重新编号）。
///
/// 工具步骤不是内容块：它们由 `agent_tool_call` 表承载（见 `chat/tools.rs`），
/// 此处只取 `AssistantStepView::Part`。
pub fn chat_message_part_views(message: &crate::app::state::ChatMessageView) -> Vec<ChatPart> {
    chat_part_views(message.steps.iter().filter_map(|step| match step {
        crate::app::state::AssistantStepView::Part { position, part } => Some((*position, part)),
        crate::app::state::AssistantStepView::Tool { .. } => None,
    }))
}

/// 消息步骤时间线 → DB 工具记录（装配输入用；不写入 DB）。
///
/// `position` 取步骤自身编号（与 parts 共用同一编号空间，不重新编号）；
/// `result_json` 只在结算后存在，`error_text` 只在失败/中断时有值——都不伪造。
pub fn chat_message_tool_views(
    message: &crate::app::state::ChatMessageView,
) -> Vec<ToolCallRecord> {
    message
        .steps
        .iter()
        .filter_map(|step| match step {
            crate::app::state::AssistantStepView::Tool { position, call } => Some(ToolCallRecord {
                call_id: call.call_id.clone(),
                message_id: message.id.clone(),
                name: call.name.clone(),
                arguments_json: call.arguments_json.clone(),
                position: *position,
                status: call.status.as_str().to_string(),
                result_json: call.result_json.clone(),
                error_text: call.error.clone(),
                started_at_ms: 0,
                completed_at_ms: None,
                duration_ms: None,
            }),
            crate::app::state::AssistantStepView::Part { .. } => None,
        })
        .collect()
}

/// 窗格聊天视图 → 装配用 DB 消息记录（parts 与 tools 同源映射）。
///
/// **选择**：`/resume` 后的下一轮装配走「窗格内存视图 → 记录 → `context::assemble`」，
/// 不重新查库。理由：窗格视图已是「DB 历史 + 本轮增量」的唯一时间线
/// （`on_session_loaded` 按 position 合并 parts 与 tools，`on_stream_delta` 追加本轮步骤），
/// 再查库既拿不到本轮未完成步骤，也会与内存状态形成两份真相。
/// 记录里的 `sequence` 用视图下标（装配按消息顺序分组，不使用该字段的绝对值）。
pub fn chat_message_views_to_db(
    messages: &[crate::app::state::ChatMessageView],
) -> Vec<ChatMessage> {
    messages
        .iter()
        .enumerate()
        .map(|(sequence, message)| ChatMessage {
            external_id: message.id.clone(),
            sequence: sequence as i64,
            turn_index: message.turn_index,
            role: message.role.clone(),
            status: message.status.clone(),
            model_provider: None,
            model_id: None,
            variant: None,
            stop_reason: None,
            error_code: message.error_code.clone(),
            input_tokens: message.input_tokens,
            output_tokens: message.output_tokens,
            cache_read_tokens: message.cache_read_tokens,
            cache_write_tokens: None,
            created_at_ms: 0,
            completed_at_ms: None,
            parts: chat_message_part_views(message),
            tools: chat_message_tool_views(message),
        })
        .collect()
}

#[cfg(test)]
mod tests;

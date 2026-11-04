//! 会话 fork：从源会话的某条用户消息之前复制出一个独立新会话。
//!
//! 复制范围：消息、parts、工具调用、附件元数据、边界内仍生效的 compact 检查点、
//! 边界前的模型切换记录；图片目录一并复制并改写受管路径。
//!
//! 刻意**不复制**文件撤销状态（`agent_turn_file_scope` / `agent_file_mutation` /
//! `agent_file_image` / `agent_rewind_operation` / `agent_session_notice`）：fork 出来的
//! 旧轮次只能删对话，不能恢复文件；fork 之后的新轮次仍正常建立自己的撤销 scope。
//! 后台进程、子 agent 运行树也不复制，它们仍归源会话所有。

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, now_ms};

/// fork 结果：新会话 id 与被选中消息的原始正文（`Full session` 为 None）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkedChatSession {
    pub session_id: String,
    pub restore_text: Option<String>,
}

struct MessageRow {
    external_id: String,
    sequence: i64,
    turn_index: i64,
    role: String,
    status: String,
    model_provider: Option<String>,
    model_id: Option<String>,
    variant: Option<String>,
    stop_reason: Option<String>,
    error_code: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cache_read_tokens: Option<i64>,
    cache_write_tokens: Option<i64>,
    created_at_ms: i64,
    completed_at_ms: Option<i64>,
    kind: String,
    user_shell_json: Option<String>,
    subagent_report_json: Option<String>,
}

struct PartRow {
    message_id: String,
    position: i64,
    kind: String,
    content: String,
    provider_metadata_json: Option<String>,
    created_at_ms: i64,
    started_at_ms: Option<i64>,
    ended_at_ms: Option<i64>,
    completed_at_ms: Option<i64>,
}

struct ToolRow {
    message_id: String,
    call_id: String,
    namespace: Option<String>,
    name: String,
    arguments_json: String,
    position: i64,
    status: String,
    result_json: Option<String>,
    error_text: Option<String>,
    started_at_ms: i64,
    completed_at_ms: Option<i64>,
    duration_ms: Option<i64>,
}

struct AttachmentRow {
    message_id: String,
    position: i64,
    kind: String,
    label: String,
    marker_start: Option<i64>,
    marker_end: Option<i64>,
    source_path: Option<String>,
    mime: Option<String>,
    width: Option<i64>,
    height: Option<i64>,
}

struct CheckpointRow {
    external_id: String,
    sequence: i64,
    base_checkpoint_id: Option<String>,
    start_sequence: i64,
    end_sequence: i64,
    summary_text: String,
    model_label: String,
    created_at_ms: i64,
}

struct SwitchRow {
    sequence: i64,
    from_provider: Option<String>,
    from_model: Option<String>,
    from_variant: Option<String>,
    to_provider: String,
    to_model: String,
    to_variant: Option<String>,
    from_label: Option<String>,
    to_label: String,
    updated_sources_json: String,
    injected_prompt: String,
    created_at_ms: i64,
}

impl Database {
    /// 从 `session_id` 的 `before` 消息之前分叉新会话。
    ///
    /// `before = None` 复制全部消息；`Some` 时该消息本身不复制，其正文作为回填文本。
    /// 整个写入在单个事务内完成；图片目录在事务外复制，失败只告警、不阻塞 fork。
    pub fn fork_chat_session(
        &self,
        workspace: &Path,
        session_id: &str,
        before: Option<&str>,
    ) -> Result<ForkedChatSession> {
        self.ensure_workspace(workspace)?;
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let Some((parent_title, model_provider, model_id, variant)) = self
            .conn
            .query_row(
                r#"SELECT title, model_provider, model_id, variant
                   FROM agent_session
                   WHERE external_id = ?1 AND workspace_id = ?2 AND source = 'native'"#,
                params![session_id, workspace_external],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()?
        else {
            return Err(DbError::SessionMissing);
        };
        ensure_no_streaming(&self.conn, session_id)?;

        let (boundary, restore_text) = match before {
            Some(message_id) => {
                let Some((sequence, role, kind)) = self
                    .conn
                    .query_row(
                        "SELECT sequence, role, kind FROM agent_message \
                         WHERE session_id = ?1 AND external_id = ?2",
                        params![session_id, message_id],
                        |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                            ))
                        },
                    )
                    .optional()?
                else {
                    return Err(DbError::ForkBoundaryMissing);
                };
                if role != "user" || kind != "chat" {
                    return Err(DbError::ForkBoundaryMissing);
                }
                let text = self
                    .conn
                    .query_row(
                        "SELECT content FROM agent_message_part \
                         WHERE message_id = ?1 AND kind = 'text' \
                         ORDER BY position LIMIT 1",
                        params![message_id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
                    .unwrap_or_default();
                (sequence, Some(text))
            }
            None => (i64::MAX, None),
        };

        let new_session_id = new_external_id("sess");
        let image_rewrite = copy_images(session_id, &new_session_id);
        let result = self.copy_fork_rows(
            workspace_external,
            session_id,
            &new_session_id,
            &parent_title,
            model_provider,
            model_id,
            variant,
            boundary,
            &image_rewrite,
        );
        match result {
            Ok(()) => Ok(ForkedChatSession {
                session_id: new_session_id,
                restore_text,
            }),
            Err(error) => {
                crate::platform::remove_session_images(&new_session_id);
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn copy_fork_rows(
        &self,
        workspace_external: String,
        source_session: &str,
        new_session_id: &str,
        parent_title: &str,
        model_provider: Option<String>,
        model_id: Option<String>,
        variant: Option<String>,
        boundary: i64,
        image_rewrite: &Option<(String, String)>,
    ) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        // 事务内再检查一次：应用层忙判定与跨进程写入之间仍有窗口。
        ensure_no_streaming(&transaction, source_session)?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![source_session, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let now = now_ms();
        let title = fork_title(parent_title);
        let first_user_message: Option<String> = transaction
            .query_row(
                r#"SELECT p.content
                   FROM agent_message m
                   JOIN agent_message_part p ON p.message_id = m.external_id
                   WHERE m.session_id = ?1 AND m.sequence < ?2
                     AND m.role = 'user' AND m.kind = 'chat' AND p.kind = 'text'
                   ORDER BY m.sequence, p.position LIMIT 1"#,
                params![source_session, boundary],
                |row| row.get(0),
            )
            .optional()?;
        let first_user_message = super::single_line_preview(
            first_user_message.as_deref().unwrap_or(""),
            super::PREVIEW_CHARS,
        );
        let (total_input, total_output): (i64, i64) = transaction.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0) \
             FROM agent_message WHERE session_id = ?1 AND sequence < ?2",
            params![source_session, boundary],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let last_message_at_ms: Option<i64> = transaction.query_row(
            "SELECT MAX(created_at_ms) FROM agent_message WHERE session_id = ?1 AND sequence < ?2",
            params![source_session, boundary],
            |row| row.get(0),
        )?;
        transaction.execute(
            r#"INSERT INTO agent_session
                   (external_id, workspace_id, source, title, first_user_message,
                    model_provider, model_id, variant, status, archived,
                    created_at_ms, updated_at_ms, last_message_at_ms,
                    total_input_tokens, total_output_tokens, revision)
               VALUES (?1, ?2, 'native', ?3, ?4, ?5, ?6, ?7, 'active', 0,
                       ?8, ?8, ?9, ?10, ?11, 0)"#,
            params![
                new_session_id,
                workspace_external,
                title,
                first_user_message,
                model_provider,
                model_id,
                variant,
                now,
                last_message_at_ms.unwrap_or(now),
                total_input,
                total_output,
            ],
        )?;

        let messages = load_messages(&transaction, source_session, 0, boundary)?;
        let mut message_ids: HashMap<String, String> = HashMap::new();
        for message in &messages {
            let new_id = new_external_id("msg");
            transaction.execute(
                r#"INSERT INTO agent_message
                       (external_id, session_id, sequence, turn_index, role, status,
                        model_provider, model_id, variant, stop_reason, error_code,
                        input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                        created_at_ms, completed_at_ms, kind, user_shell_json, subagent_report_json)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                           ?16, ?17, ?18, ?19, ?20)"#,
                params![
                    new_id,
                    new_session_id,
                    message.sequence,
                    message.turn_index,
                    message.role,
                    message.status,
                    message.model_provider,
                    message.model_id,
                    message.variant,
                    message.stop_reason,
                    message.error_code,
                    message.input_tokens,
                    message.output_tokens,
                    message.cache_read_tokens,
                    message.cache_write_tokens,
                    message.created_at_ms,
                    message.completed_at_ms,
                    message.kind,
                    message.user_shell_json,
                    message.subagent_report_json,
                ],
            )?;
            message_ids.insert(message.external_id.clone(), new_id);
        }

        copy_parts(
            &transaction,
            &message_ids,
            source_session,
            0,
            boundary,
            false,
        )?;
        copy_tools(
            &transaction,
            new_session_id,
            &message_ids,
            source_session,
            boundary,
            image_rewrite,
        )?;
        copy_attachments(
            &transaction,
            &message_ids,
            source_session,
            boundary,
            image_rewrite,
        )?;
        copy_checkpoints(&transaction, new_session_id, source_session, 0, boundary)?;
        copy_switches(&transaction, new_session_id, source_session, 0, boundary)?;
        transaction.commit()?;
        Ok(())
    }
}

/// 校验源会话没有未结算消息（assistant 流式或本地命令运行中）。
fn ensure_no_streaming(conn: &Connection, session_id: &str) -> Result<()> {
    let streaming: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM agent_message WHERE session_id = ?1 AND status = 'streaming' LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    if streaming.is_some() {
        return Err(DbError::SessionBusy);
    }
    Ok(())
}

/// 复制源会话图片目录；返回旧/新受管路径前缀（未复制时为 None）。
pub fn copy_session_images(source_session: &str, new_session_id: &str) -> Option<(String, String)> {
    copy_images(source_session, new_session_id)
}

/// 计算父会话已完成历史消息的 `sequence` 上界（不含正在 `streaming` 的当前轮次）。
pub(super) fn completed_boundary_sequence(conn: &Connection, session_id: &str) -> Result<i64> {
    let streaming_turn: Option<i64> = conn
        .query_row(
            "SELECT MIN(turn_index) FROM agent_message WHERE session_id = ?1 AND status = 'streaming'",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(turn_idx) = streaming_turn {
        let turn_start_seq: Option<i64> = conn
            .query_row(
                "SELECT MIN(sequence) FROM agent_message WHERE session_id = ?1 AND turn_index = ?2",
                params![session_id, turn_idx],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if let Some(seq) = turn_start_seq {
            return Ok(seq);
        }
    }
    Ok(i64::MAX)
}

/// 计算子会话分叉要克隆的已完成历史窗口 `[start_sequence, boundary_sequence)`。
///
/// - `fork_turns = None`：全部已完成轮次；
/// - `fork_turns = Some(0)`：空窗口；
/// - `fork_turns = Some(n)`：最近 `n` 个已完成的 `turn_index`，起点为该轮最小 `sequence`。
pub(super) fn completed_fork_window(
    conn: &Connection,
    session_id: &str,
    fork_turns: Option<u32>,
) -> Result<(i64, i64)> {
    let boundary = completed_boundary_sequence(conn, session_id)?;
    let Some(turns) = fork_turns else {
        return Ok((0, boundary));
    };
    if turns == 0 {
        return Ok((boundary, boundary));
    }
    let mut statement = conn.prepare(
        r#"SELECT DISTINCT turn_index
           FROM agent_message
           WHERE session_id = ?1 AND sequence < ?2 AND status <> 'streaming'
           ORDER BY turn_index DESC
           LIMIT ?3"#,
    )?;
    let turn_indices: Vec<i64> = statement
        .query_map(params![session_id, boundary, i64::from(turns)], |row| {
            row.get(0)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let Some(&min_turn) = turn_indices.last() else {
        return Ok((boundary, boundary));
    };
    let start_seq: i64 = conn.query_row(
        r#"SELECT COALESCE(MIN(sequence), 0)
           FROM agent_message
           WHERE session_id = ?1 AND turn_index >= ?2 AND sequence < ?3"#,
        params![session_id, min_turn, boundary],
        |row| row.get(0),
    )?;
    Ok((start_seq, boundary))
}

/// 将源会话 `sequence < boundary` 的已完成历史轮次克隆到子会话，返回子会话下一轮可用的 `(next_sequence, next_turn_index)`。
pub(super) fn copy_completed_history_to_child(
    transaction: &Connection,
    source_session: &str,
    child_session_id: &str,
    boundary: i64,
    image_rewrite: &Option<(String, String)>,
) -> Result<(i64, i64)> {
    copy_completed_history_window_to_child(
        transaction,
        source_session,
        child_session_id,
        0,
        boundary,
        image_rewrite,
    )
}

/// 将源会话 `sequence ∈ [start_seq, boundary)` 的已完成历史克隆到子会话。
///
/// 子 agent 分叉的强制清洗：克隆消息的 token 计费字段全部置 NULL、不复制
/// `reasoning`/`thinking` 思考块、清除父级子 agent 汇报标记。
pub(super) fn copy_completed_history_window_to_child(
    transaction: &Connection,
    source_session: &str,
    child_session_id: &str,
    start_seq: i64,
    boundary: i64,
    image_rewrite: &Option<(String, String)>,
) -> Result<(i64, i64)> {
    let messages = load_messages(transaction, source_session, start_seq, boundary)?;
    let mut message_ids: HashMap<String, String> = HashMap::new();
    let mut max_turn: Option<i64> = None;
    for message in &messages {
        if message.status == "streaming" {
            continue;
        }
        // 父会话的子 agent 汇报卡片不属于子会话：只保留正文，清掉报告标记。
        // `agent_message.kind` 的合法值为 `chat` / `user_shell`，汇报行本身就是 `chat`。
        let is_subagent_report =
            message.kind == "subagent_report" || message.subagent_report_json.is_some();
        let kind = if message.kind == "subagent_report" {
            "chat"
        } else {
            message.kind.as_str()
        };
        let subagent_report_json = if is_subagent_report {
            None
        } else {
            message.subagent_report_json.as_deref()
        };
        let new_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status,
                    model_provider, model_id, variant, stop_reason, error_code,
                    input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                    created_at_ms, completed_at_ms, kind, user_shell_json, subagent_report_json)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                       ?16, ?17, ?18, ?19, ?20)"#,
            params![
                new_id,
                child_session_id,
                message.sequence,
                message.turn_index,
                message.role,
                message.status,
                message.model_provider,
                message.model_id,
                message.variant,
                message.stop_reason,
                message.error_code,
                None::<i64>,
                None::<i64>,
                None::<i64>,
                None::<i64>,
                message.created_at_ms,
                message.completed_at_ms,
                kind,
                message.user_shell_json,
                subagent_report_json,
            ],
        )?;
        max_turn = Some(max_turn.map_or(message.turn_index, |cur| cur.max(message.turn_index)));
        message_ids.insert(message.external_id.clone(), new_id);
    }

    if !message_ids.is_empty() {
        copy_parts(
            transaction,
            &message_ids,
            source_session,
            start_seq,
            boundary,
            true,
        )?;
        copy_tools(
            transaction,
            child_session_id,
            &message_ids,
            source_session,
            boundary,
            image_rewrite,
        )?;
        copy_attachments(
            transaction,
            &message_ids,
            source_session,
            boundary,
            image_rewrite,
        )?;
        copy_checkpoints(
            transaction,
            child_session_id,
            source_session,
            start_seq,
            boundary,
        )?;
        copy_switches(
            transaction,
            child_session_id,
            source_session,
            start_seq,
            boundary,
        )?;
    }

    let next_seq = super::next_session_sequence(transaction, child_session_id)?;
    let next_turn = max_turn.map_or(0, |t| t.saturating_add(1));
    Ok((next_seq, next_turn))
}

/// 复制源会话图片目录；返回旧/新受管路径前缀（未复制时为 None）。
fn copy_images(source_session: &str, new_session_id: &str) -> Option<(String, String)> {
    let root = crate::platform::sessions_root_dir().ok()?;
    match crate::platform::fork_session_images_in(&root, source_session, new_session_id) {
        Ok(true) => Some((
            crate::platform::session_image_dir(&root, source_session)
                .to_string_lossy()
                .into_owned(),
            crate::platform::session_image_dir(&root, new_session_id)
                .to_string_lossy()
                .into_owned(),
        )),
        Ok(false) => None,
        Err(error) => {
            tracing::warn!(%error, "fork image copy failed");
            None
        }
    }
}

fn load_messages(
    conn: &Connection,
    session_id: &str,
    start_seq: i64,
    boundary: i64,
) -> Result<Vec<MessageRow>> {
    let mut statement = conn.prepare(
        r#"SELECT external_id, sequence, turn_index, role, status, model_provider, model_id,
                  variant, stop_reason, error_code, input_tokens, output_tokens,
                  cache_read_tokens, cache_write_tokens, created_at_ms, completed_at_ms,
                  kind, user_shell_json, subagent_report_json
           FROM agent_message
           WHERE session_id = ?1 AND sequence >= ?2 AND sequence < ?3
           ORDER BY sequence"#,
    )?;
    let rows = statement.query_map(params![session_id, start_seq, boundary], |row| {
        Ok(MessageRow {
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
            kind: row.get(16)?,
            user_shell_json: row.get(17)?,
            subagent_report_json: row.get(18)?,
        })
    })?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(DbError::from)
}

fn copy_parts(
    conn: &Connection,
    message_ids: &HashMap<String, String>,
    source_session: &str,
    start_seq: i64,
    boundary: i64,
    sanitize_for_subagent: bool,
) -> Result<()> {
    let mut statement = conn.prepare(
        r#"SELECT p.message_id, p.position, p.kind, p.content,
                  p.provider_metadata_json, p.created_at_ms, p.started_at_ms,
                  p.ended_at_ms, p.completed_at_ms
           FROM agent_message_part p
           JOIN agent_message m ON m.external_id = p.message_id
           WHERE m.session_id = ?1 AND m.sequence >= ?2 AND m.sequence < ?3
           ORDER BY p.message_id, p.position"#,
    )?;
    let rows = statement
        .query_map(params![source_session, start_seq, boundary], |row| {
            Ok(PartRow {
                message_id: row.get(0)?,
                position: row.get(1)?,
                kind: row.get(2)?,
                content: row.get(3)?,
                provider_metadata_json: row.get(4)?,
                created_at_ms: row.get(5)?,
                started_at_ms: row.get(6)?,
                ended_at_ms: row.get(7)?,
                completed_at_ms: row.get(8)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for row in rows {
        if sanitize_for_subagent && matches!(row.kind.as_str(), "reasoning" | "thinking") {
            continue;
        }
        let Some(message_id) = message_ids.get(&row.message_id) else {
            continue;
        };
        conn.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, provider_metadata_json,
                    created_at_ms, started_at_ms, ended_at_ms, completed_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"#,
            params![
                new_external_id("part"),
                message_id,
                row.position,
                row.kind,
                row.content,
                row.provider_metadata_json,
                row.created_at_ms,
                row.started_at_ms,
                row.ended_at_ms,
                row.completed_at_ms,
            ],
        )?;
    }
    Ok(())
}

fn copy_tools(
    conn: &Connection,
    new_session_id: &str,
    message_ids: &HashMap<String, String>,
    source_session: &str,
    boundary: i64,
    image_rewrite: &Option<(String, String)>,
) -> Result<()> {
    let mut statement = conn.prepare(
        r#"SELECT t.message_id, t.call_id, t.namespace, t.name,
                  t.arguments_json, t.position, t.status, t.result_json, t.error_text,
                  t.started_at_ms, t.completed_at_ms, t.duration_ms
           FROM agent_tool_call t
           JOIN agent_message m ON m.external_id = t.message_id
           WHERE m.session_id = ?1 AND m.sequence < ?2
           ORDER BY t.message_id, t.position"#,
    )?;
    let rows = statement
        .query_map(params![source_session, boundary], |row| {
            Ok(ToolRow {
                message_id: row.get(0)?,
                call_id: row.get(1)?,
                namespace: row.get(2)?,
                name: row.get(3)?,
                arguments_json: row.get(4)?,
                position: row.get(5)?,
                status: row.get(6)?,
                result_json: row.get(7)?,
                error_text: row.get(8)?,
                started_at_ms: row.get(9)?,
                completed_at_ms: row.get(10)?,
                duration_ms: row.get(11)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for row in rows {
        let Some(message_id) = message_ids.get(&row.message_id) else {
            continue;
        };
        conn.execute(
            r#"INSERT INTO agent_tool_call
                   (external_id, session_id, message_id, call_id, namespace, name, arguments_json,
                    position, status, result_json, error_text, started_at_ms, completed_at_ms, duration_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"#,
            params![
                new_external_id("tool"),
                new_session_id,
                message_id,
                row.call_id,
                row.namespace,
                row.name,
                row.arguments_json,
                row.position,
                row.status,
                row.result_json.as_deref().map(|value| rewrite(value, image_rewrite)),
                row.error_text,
                row.started_at_ms,
                row.completed_at_ms,
                row.duration_ms,
            ],
        )?;
    }
    Ok(())
}

fn copy_attachments(
    conn: &Connection,
    message_ids: &HashMap<String, String>,
    source_session: &str,
    boundary: i64,
    image_rewrite: &Option<(String, String)>,
) -> Result<()> {
    let mut statement = conn.prepare(
        r#"SELECT a.message_id, a.position, a.kind, a.label,
                  a.marker_start, a.marker_end, a.source_path, a.mime, a.width, a.height
           FROM agent_message_attachment a
           JOIN agent_message m ON m.external_id = a.message_id
           WHERE m.session_id = ?1 AND m.sequence < ?2
           ORDER BY a.message_id, a.position"#,
    )?;
    let rows = statement
        .query_map(params![source_session, boundary], |row| {
            Ok(AttachmentRow {
                message_id: row.get(0)?,
                position: row.get(1)?,
                kind: row.get(2)?,
                label: row.get(3)?,
                marker_start: row.get(4)?,
                marker_end: row.get(5)?,
                source_path: row.get(6)?,
                mime: row.get(7)?,
                width: row.get(8)?,
                height: row.get(9)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for row in rows {
        let Some(message_id) = message_ids.get(&row.message_id) else {
            continue;
        };
        conn.execute(
            r#"INSERT INTO agent_message_attachment
                   (external_id, message_id, position, kind, label, marker_start, marker_end,
                    source_path, mime, width, height)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                new_external_id("att"),
                message_id,
                row.position,
                row.kind,
                row.label,
                row.marker_start,
                row.marker_end,
                row.source_path
                    .as_deref()
                    .map(|value| rewrite(value, image_rewrite)),
                row.mime,
                row.width,
                row.height,
            ],
        )?;
    }
    Ok(())
}

fn copy_checkpoints(
    conn: &Connection,
    new_session_id: &str,
    source_session: &str,
    start_seq: i64,
    boundary: i64,
) -> Result<()> {
    let mut statement = conn.prepare(
        r#"SELECT external_id, sequence, base_checkpoint_id, start_sequence, end_sequence,
                  summary_text, model_label, created_at_ms
           FROM agent_compact_checkpoint
           WHERE session_id = ?1 AND state = 'active'
             AND end_sequence >= ?2 AND end_sequence < ?3
           ORDER BY sequence"#,
    )?;
    let rows = statement
        .query_map(params![source_session, start_seq, boundary], |row| {
            Ok(CheckpointRow {
                external_id: row.get(0)?,
                sequence: row.get(1)?,
                base_checkpoint_id: row.get(2)?,
                start_sequence: row.get(3)?,
                end_sequence: row.get(4)?,
                summary_text: row.get(5)?,
                model_label: row.get(6)?,
                created_at_ms: row.get(7)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut ids: HashMap<String, String> = HashMap::new();
    for row in &rows {
        let new_id = new_external_id("compact");
        conn.execute(
            r#"INSERT INTO agent_compact_checkpoint
                   (external_id, session_id, sequence, operation_id, base_checkpoint_id,
                    start_sequence, end_sequence, summary_text, model_label, state,
                    reverted_by_operation_id, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', NULL, ?10)"#,
            params![
                new_id,
                new_session_id,
                row.sequence,
                new_external_id("cop"),
                row.base_checkpoint_id
                    .as_ref()
                    .and_then(|id| ids.get(id))
                    .cloned(),
                row.start_sequence,
                row.end_sequence,
                row.summary_text,
                row.model_label,
                row.created_at_ms,
            ],
        )?;
        ids.insert(row.external_id.clone(), new_id);
    }
    Ok(())
}

fn copy_switches(
    conn: &Connection,
    new_session_id: &str,
    source_session: &str,
    start_seq: i64,
    boundary: i64,
) -> Result<()> {
    let mut statement = conn.prepare(
        r#"SELECT sequence, from_provider, from_model, from_variant, to_provider, to_model,
                  to_variant, from_label, to_label, updated_sources_json, injected_prompt,
                  created_at_ms
           FROM agent_model_switch
           WHERE session_id = ?1 AND sequence >= ?2 AND sequence < ?3
           ORDER BY sequence"#,
    )?;
    let rows = statement
        .query_map(params![source_session, start_seq, boundary], |row| {
            Ok(SwitchRow {
                sequence: row.get(0)?,
                from_provider: row.get(1)?,
                from_model: row.get(2)?,
                from_variant: row.get(3)?,
                to_provider: row.get(4)?,
                to_model: row.get(5)?,
                to_variant: row.get(6)?,
                from_label: row.get(7)?,
                to_label: row.get(8)?,
                updated_sources_json: row.get(9)?,
                injected_prompt: row.get(10)?,
                created_at_ms: row.get(11)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for row in rows {
        conn.execute(
            r#"INSERT INTO agent_model_switch
                   (external_id, session_id, sequence, from_provider, from_model, from_variant,
                    to_provider, to_model, to_variant, from_label, to_label,
                    updated_sources_json, injected_prompt, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"#,
            params![
                new_external_id("msw"),
                new_session_id,
                row.sequence,
                row.from_provider,
                row.from_model,
                row.from_variant,
                row.to_provider,
                row.to_model,
                row.to_variant,
                row.from_label,
                row.to_label,
                row.updated_sources_json,
                row.injected_prompt,
                row.created_at_ms,
            ],
        )?;
    }
    Ok(())
}

/// 受管图片路径前缀替换；兼容裸路径与 JSON 转义形式；没有复制图片目录时原样返回。
fn rewrite(value: &str, image_rewrite: &Option<(String, String)>) -> String {
    match image_rewrite {
        Some((from, to)) => {
            let replaced = value.replace(from, to);
            let from_escaped = from.replace('\\', "\\\\");
            let to_escaped = to.replace('\\', "\\\\");
            if from_escaped != *from {
                replaced.replace(&from_escaped, &to_escaped)
            } else {
                replaced
            }
        }
        None => value.to_string(),
    }
}

/// `源标题 (fork #N)`；父标题已是该后缀时递增。
fn fork_title(parent: &str) -> String {
    const MARK: &str = " (fork #";
    let parent = if parent.trim().is_empty() {
        "Session"
    } else {
        parent
    };
    if let Some(start) = parent.rfind(MARK)
        && parent.ends_with(')')
        && let Ok(number) = parent[start + MARK.len()..parent.len() - 1].parse::<u32>()
    {
        return format!("{}{}{})", &parent[..start], MARK, number.saturating_add(1));
    }
    format!("{parent} (fork #1)")
}

#[cfg(test)]
mod tests;

//! 流中 steer 轮次单事务落库：封口被截断的旧 assistant，并插入新 user 与 streaming assistant。

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::super::{Database, DbError, Result, new_external_id, now_ms};
use super::{
    ChatPart, MAX_MESSAGE_BYTES, MessageAttachmentInput, ModelSnapshot, TurnCommitted, attachment,
    next_session_sequence, replace_parts, rewind,
};

impl Database {
    /// steer 提交：同一事务内关闭上一段 streaming assistant，并开启新的 user/assistant 轮。
    ///
    /// 旧片段没有最终 usage，不写消息级或会话级 token；`stop_reason = 'steer'` 同时标记
    /// 被 steer 截断的旧 assistant 与新插入的 steer user 消息。任一步失败整体回滚，不产生半轮。
    #[allow(clippy::too_many_arguments)]
    pub fn begin_steer_turn(
        &self,
        workspace: &Path,
        session_id: &str,
        previous_assistant_id: &str,
        previous_parts: &[ChatPart],
        user_text: &str,
        attachments: &[MessageAttachmentInput],
        model: &ModelSnapshot,
    ) -> Result<TurnCommitted> {
        if user_text.trim().is_empty() && attachments.is_empty() {
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
        let transaction = self.write_transaction()?;
        // 双条件校验 session 归属，并读取本轮提交前的 revision。
        let revision: Option<i64> = transaction
            .query_row(
                "SELECT revision FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        let Some(revision) = revision else {
            return Err(DbError::SessionMissing);
        };
        // previous assistant 必须属于该 session 且仍在 streaming；重复调用在这里失败。
        let previous_status: Option<String> = transaction
            .query_row(
                "SELECT status FROM agent_message WHERE external_id = ?1 AND session_id = ?2",
                params![previous_assistant_id, session_id],
                |row| row.get(0),
            )
            .optional()?;
        if previous_status.as_deref() != Some("streaming") {
            return Err(DbError::MessageNotStreaming);
        }
        // 先写旧片段最终 parts，再封口旧 assistant 与它的文件 scope。
        replace_parts(&transaction, previous_assistant_id, previous_parts, now)?;
        let closed = transaction.execute(
            r#"UPDATE agent_message
               SET status = 'complete', stop_reason = 'steer', completed_at_ms = ?1
               WHERE external_id = ?2 AND session_id = ?3 AND status = 'streaming'"#,
            params![now, previous_assistant_id, session_id],
        )?;
        if closed == 0 {
            return Err(DbError::MessageNotStreaming);
        }
        rewind::seal_turn_scope(&transaction, session_id, previous_assistant_id)?;
        // 新片段沿用同一会话编号空间；turn_index 与 revision 严格递增。
        let next_sequence = next_session_sequence(&transaction, session_id)?;
        let turn_index: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(turn_index) + 1, 0) FROM agent_message WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let user_message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, stop_reason, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'user', 'complete', 'steer', ?5)"#,
            params![user_message_id, session_id, next_sequence, turn_index, now],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, created_at_ms)
               VALUES (?1, ?2, 0, 'text', ?3, ?4)"#,
            params![new_external_id("part"), user_message_id, user_text, now],
        )?;
        let attachment_metas = attachment::insert_attachments(
            &transaction,
            session_id,
            &user_message_id,
            attachments,
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
        rewind::begin_turn_scope(&transaction, session_id, &user_message_id, now)?;
        let revision = revision.saturating_add(1);
        transaction.execute(
            "UPDATE agent_session SET updated_at_ms = ?1, last_message_at_ms = ?1, revision = ?2 WHERE external_id = ?3",
            params![now, revision, session_id],
        )?;
        transaction.commit()?;
        Ok(TurnCommitted {
            session_id: session_id.to_string(),
            user_message_id,
            assistant_message_id,
            turn_index,
            user_sequence: next_sequence,
            assistant_sequence: next_sequence + 1,
            revision,
            created_at_ms: now,
            attachments: attachment_metas,
        })
    }
}

//! 旁路问答（`/btw`）的持久化：主 QA 归属、批次、多目标回答者与级联清理。
//!
//! 分层约束：
//! - 只接触 rusqlite，不发起网络请求、不读文件；
//! - BTW 使用独立 `agent_session.source='btw'`，普通会话查询天然隔离；
//! - 主 QA 身份是稳定的 user 消息 ID，不使用可复用的 `turn_index`/`sequence`；
//! - 一个批次共享一条 user 消息，每个目标各自持有 assistant 消息与终态。

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::{ChatMessage, ModelSnapshot, TurnOutcome, Usage};
use crate::db::chat::next_session_sequence;
use crate::db::{Database, DbError, Result, new_external_id, now_ms};

/// 主 QA 的稳定锚点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QaAnchor {
    pub message_id: String,
    pub sequence: i64,
    pub turn_index: i64,
}

/// 一个副会话线程：所属主会话 + 主 QA 锚点 + 独立 child session。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwThreadRecord {
    pub id: String,
    pub parent_session_id: String,
    pub parent_user_message_id: String,
    pub child_session_id: String,
    pub title: String,
    /// 默认回答者集合（稳定 agent ID；空表示默认 lx）。
    pub answerers: Vec<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// 批次内单个回答者。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwRecipientRecord {
    pub id: String,
    pub batch_id: String,
    pub ordinal: i64,
    /// `default` / `subagent`。
    pub target_kind: String,
    pub target_key: String,
    pub agent_id: Option<String>,
    pub source_child_session_id: Option<String>,
    pub display_nickname: String,
    pub role_name: Option<String>,
    pub model_provider: String,
    pub model_id: String,
    pub variant: Option<String>,
    pub assistant_message_id: Option<String>,
    /// `queued` / `running` / `completed` / `failed` / `cancelled` / `interrupted`。
    pub status: String,
    pub error_code: Option<String>,
}

/// 一次发送的批次。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwBatchRecord {
    pub id: String,
    pub thread_id: String,
    pub user_message_id: String,
    pub status: String,
    pub created_at_ms: i64,
    pub recipients: Vec<BtwRecipientRecord>,
}

/// 线程预览（QA 列表用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwThreadSummary {
    pub thread: BtwThreadRecord,
    pub message_count: i64,
    pub last_status: String,
}

/// 线程完整视图（打开面板时加载）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwThreadView {
    pub thread: BtwThreadRecord,
    pub batches: Vec<BtwBatchRecord>,
}

/// 提交时的目标快照。模型在排队时一并冻结，避免主模型切换改变已提交问题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwTargetSpec {
    /// `default` / `subagent`。
    pub target_kind: String,
    pub target_key: String,
    pub agent_id: Option<String>,
    pub source_child_session_id: Option<String>,
    pub display_nickname: String,
    pub role_name: Option<String>,
    pub model: ModelSnapshot,
}

impl BtwTargetSpec {
    /// 默认旁路助手。
    pub fn default_assistant(model: ModelSnapshot) -> Self {
        Self {
            target_kind: "default".to_string(),
            target_key: "default".to_string(),
            agent_id: None,
            source_child_session_id: None,
            display_nickname: crate::chat::prompt::AGENT_NAME.to_string(),
            role_name: None,
            model,
        }
    }
}

/// 批次被接受后的稳定句柄。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwAccepted {
    pub thread: BtwThreadRecord,
    pub batch_id: String,
    pub user_message_id: String,
    pub recipients: Vec<BtwRecipientRecord>,
}

/// 回答者开始运行时的凭据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwRecipientStart {
    pub assistant_message_id: String,
    pub child_session_id: String,
    pub thread_id: String,
    pub parent_session_id: String,
    pub qa_message_id: String,
    pub user_message_id: String,
    pub question: String,
    pub display_nickname: String,
    pub target_kind: String,
    pub agent_id: Option<String>,
    pub source_child_session_id: Option<String>,
    pub model: ModelSnapshot,
}

/// 单个回答者结算结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwRecipientCompletion {
    pub thread_id: String,
    pub batch_id: String,
    /// 批次内全部回答者都已进入终态。
    pub batch_terminal: bool,
    pub recipient_status: String,
}

impl Database {
    /// 解析指定主会话中存活的正式 QA 锚点。
    pub fn resolve_qa_anchor(
        &self,
        workspace: &Path,
        session_id: &str,
        message_id: &str,
    ) -> Result<Option<QaAnchor>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        let mut statement = self.conn.prepare(
            r#"SELECT m.external_id, m.sequence, m.turn_index
               FROM agent_message m
               JOIN agent_session s ON s.external_id = m.session_id
               WHERE m.session_id = ?1 AND m.external_id = ?2
                 AND s.workspace_id = ?3 AND s.source = 'native'
                 AND m.role = 'user' AND m.kind = 'chat'
                 AND m.subagent_report_json IS NULL
                 AND COALESCE(m.stop_reason, '') <> 'steer'"#,
        )?;
        statement
            .query_row(params![session_id, message_id, workspace_external], |row| {
                Ok(QaAnchor {
                    message_id: row.get(0)?,
                    sequence: row.get(1)?,
                    turn_index: row.get(2)?,
                })
            })
            .optional()
            .map_err(DbError::from)
    }

    /// 主会话中最新一条正式 QA 锚点。
    pub fn latest_qa_anchor(&self, workspace: &Path, session_id: &str) -> Result<Option<QaAnchor>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        self.conn
            .query_row(
                r#"SELECT m.external_id, m.sequence, m.turn_index
                   FROM agent_message m
                   JOIN agent_session s ON s.external_id = m.session_id
                   WHERE m.session_id = ?1 AND s.workspace_id = ?2 AND s.source = 'native'
                     AND m.role = 'user' AND m.kind = 'chat'
                     AND m.subagent_report_json IS NULL
                     AND COALESCE(m.stop_reason, '') <> 'steer'
                   ORDER BY m.sequence DESC LIMIT 1"#,
                params![session_id, workspace_external],
                |row| {
                    Ok(QaAnchor {
                        message_id: row.get(0)?,
                        sequence: row.get(1)?,
                        turn_index: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(DbError::from)
    }

    /// 主会话中全部正式 QA 锚点（按时间正序）。
    pub fn qa_anchors(&self, workspace: &Path, session_id: &str) -> Result<Vec<QaAnchor>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT m.external_id, m.sequence, m.turn_index
               FROM agent_message m
               JOIN agent_session s ON s.external_id = m.session_id
               WHERE m.session_id = ?1 AND s.workspace_id = ?2 AND s.source = 'native'
                 AND m.role = 'user' AND m.kind = 'chat'
                 AND m.subagent_report_json IS NULL
                 AND COALESCE(m.stop_reason, '') <> 'steer'
               ORDER BY m.sequence"#,
        )?;
        let rows = statement.query_map(params![session_id, workspace_external], |row| {
            Ok(QaAnchor {
                message_id: row.get(0)?,
                sequence: row.get(1)?,
                turn_index: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 主会话从开头到指定 QA 结束（不含下一条正式 QA）的完整消息。
    ///
    /// 返回 `(anchor, messages)`；锚点不属于该会话时返回 None。
    pub fn load_qa_context(
        &self,
        workspace: &Path,
        session_id: &str,
        anchor_id: &str,
    ) -> Result<Option<(QaAnchor, Vec<ChatMessage>)>> {
        let Some(anchor) = self.resolve_qa_anchor(workspace, session_id, anchor_id)? else {
            return Ok(None);
        };
        let anchors = self.qa_anchors(workspace, session_id)?;
        let end = anchors
            .iter()
            .find(|candidate| candidate.sequence > anchor.sequence)
            .map_or(i64::MAX, |candidate| candidate.sequence);
        let mut messages = self
            .load_chat_session(workspace, session_id)?
            .unwrap_or_default();
        messages.retain(|message| message.sequence < end);
        Ok(Some((anchor, messages)))
    }

    /// 取或建某主 QA 的 BTW 线程（首次提问才真正创建 child session）。
    pub fn ensure_btw_thread(
        &self,
        workspace: &Path,
        parent_session_id: &str,
        qa_message_id: &str,
        title: &str,
    ) -> Result<BtwThreadRecord> {
        if let Some(thread) = self.btw_thread_for_qa(workspace, parent_session_id, qa_message_id)? {
            return Ok(thread);
        }
        self.ensure_workspace(workspace)?;
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        // 主会话必须存在且是普通 native 会话；QA 锚点必须属于它。
        let parent_ok: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2 AND source = 'native'",
                params![parent_session_id, workspace_external],
                |row| row.get(0),
            )
            .optional()?;
        if parent_ok.is_none() {
            return Err(DbError::SessionMissing);
        }
        let anchor = self
            .resolve_qa_anchor(workspace, parent_session_id, qa_message_id)?
            .ok_or(DbError::SessionMissing)?;

        let now = now_ms();
        let child_session_id = new_external_id("sess");
        let thread_id = new_external_id("btw");
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            r#"INSERT INTO agent_session
                   (external_id, workspace_id, source, title, first_user_message,
                    status, archived, created_at_ms, updated_at_ms, last_message_at_ms)
               VALUES (?1, ?2, 'btw', ?3, '', 'active', 0, ?4, ?4, ?4)"#,
            params![child_session_id, workspace_external, title, now],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_btw_thread
                   (external_id, parent_session_id, parent_user_message_id, child_session_id,
                    title, answerer_set_json, created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, '[]', ?6, ?6)"#,
            params![
                thread_id,
                parent_session_id,
                anchor.message_id,
                child_session_id,
                title,
                now
            ],
        )?;
        transaction.commit()?;
        Ok(BtwThreadRecord {
            id: thread_id,
            parent_session_id: parent_session_id.to_string(),
            parent_user_message_id: qa_message_id.to_string(),
            child_session_id,
            title: title.to_string(),
            answerers: Vec::new(),
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    /// 读取某主 QA 的线程（不存在返回 None）。
    pub fn btw_thread_for_qa(
        &self,
        workspace: &Path,
        parent_session_id: &str,
        qa_message_id: &str,
    ) -> Result<Option<BtwThreadRecord>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        self.conn
            .query_row(
                r#"SELECT t.external_id, t.parent_session_id, t.parent_user_message_id,
                          t.child_session_id, t.title, t.answerer_set_json,
                          t.created_at_ms, t.updated_at_ms
                   FROM agent_btw_thread t
                   JOIN agent_session s ON s.external_id = t.parent_session_id
                   WHERE t.parent_session_id = ?1 AND t.parent_user_message_id = ?2
                     AND s.workspace_id = ?3"#,
                params![parent_session_id, qa_message_id, workspace_external],
                read_thread_row,
            )
            .optional()
            .map_err(DbError::from)
    }

    /// 按 ID 读取线程。
    pub fn btw_thread(&self, workspace: &Path, thread_id: &str) -> Result<Option<BtwThreadRecord>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        self.conn
            .query_row(
                r#"SELECT t.external_id, t.parent_session_id, t.parent_user_message_id,
                          t.child_session_id, t.title, t.answerer_set_json,
                          t.created_at_ms, t.updated_at_ms
                   FROM agent_btw_thread t
                   JOIN agent_session s ON s.external_id = t.parent_session_id
                   WHERE t.external_id = ?1 AND s.workspace_id = ?2"#,
                params![thread_id, workspace_external],
                read_thread_row,
            )
            .optional()
            .map_err(DbError::from)
    }

    /// 某个主会话的全部 BTW 线程（按最近活动倒序）。
    pub fn list_btw_threads(
        &self,
        workspace: &Path,
        parent_session_id: &str,
    ) -> Result<Vec<BtwThreadSummary>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT t.external_id, t.parent_session_id, t.parent_user_message_id,
                      t.child_session_id, t.title, t.answerer_set_json,
                      t.created_at_ms, t.updated_at_ms,
                      (SELECT COUNT(*) FROM agent_message m WHERE m.session_id = t.child_session_id),
                      COALESCE((
                          SELECT b.status FROM agent_btw_batch b
                          WHERE b.thread_id = t.external_id
                          ORDER BY b.created_at_ms DESC LIMIT 1
                      ), 'idle')
               FROM agent_btw_thread t
               JOIN agent_session s ON s.external_id = t.parent_session_id
               WHERE t.parent_session_id = ?1 AND s.workspace_id = ?2
               ORDER BY t.updated_at_ms DESC, t.external_id DESC"#,
        )?;
        let rows = statement.query_map(params![parent_session_id, workspace_external], |row| {
            Ok(BtwThreadSummary {
                thread: read_thread_row(row)?,
                message_count: row.get(8)?,
                last_status: row.get(9)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 打开线程：线程、批次与回答者一次读出。
    pub fn load_btw_thread(
        &self,
        workspace: &Path,
        thread_id: &str,
    ) -> Result<Option<BtwThreadView>> {
        let Some(thread) = self.btw_thread(workspace, thread_id)? else {
            return Ok(None);
        };
        let mut batches_statement = self.conn.prepare(
            r#"SELECT external_id, thread_id, user_message_id, status, created_at_ms
               FROM agent_btw_batch WHERE thread_id = ?1 ORDER BY created_at_ms, external_id"#,
        )?;
        let mut batches: Vec<BtwBatchRecord> = batches_statement
            .query_map(params![thread_id], |row| {
                Ok(BtwBatchRecord {
                    id: row.get(0)?,
                    thread_id: row.get(1)?,
                    user_message_id: row.get(2)?,
                    status: row.get(3)?,
                    created_at_ms: row.get(4)?,
                    recipients: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut recipients_statement = self.conn.prepare(
            r#"SELECT r.external_id, r.batch_id, r.ordinal, r.target_kind, r.target_key,
                      r.agent_id, r.source_child_session_id, r.display_nickname, r.role_name,
                      r.model_provider, r.model_id, r.variant, r.assistant_message_id,
                      r.status, r.error_code
               FROM agent_btw_recipient r
               JOIN agent_btw_batch b ON b.external_id = r.batch_id
               WHERE b.thread_id = ?1 ORDER BY r.ordinal"#,
        )?;
        let recipients = recipients_statement
            .query_map(params![thread_id], read_recipient_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for recipient in recipients {
            if let Some(batch) = batches
                .iter_mut()
                .find(|batch| batch.id == recipient.batch_id)
            {
                batch.recipients.push(recipient);
            }
        }
        Ok(Some(BtwThreadView { thread, batches }))
    }

    /// 接受一次发送：线程（必要时创建）、user 消息、批次与去重后的回答者。
    ///
    /// `submit_id` 是幂等键：同一提交重复到达时返回已创建的记录，不再插入。
    #[allow(clippy::too_many_arguments)]
    pub fn accept_btw_batch(
        &self,
        workspace: &Path,
        parent_session_id: &str,
        qa_message_id: &str,
        title: &str,
        submit_id: &str,
        text: &str,
        targets: &[BtwTargetSpec],
    ) -> Result<BtwAccepted> {
        if text.trim().is_empty() || targets.is_empty() {
            return Err(DbError::InvalidDraft);
        }
        if let Some(existing) = self.btw_batch_by_submit(submit_id)? {
            let thread = self
                .btw_thread(workspace, &existing.thread_id)?
                .ok_or(DbError::SessionMissing)?;
            return Ok(BtwAccepted {
                thread,
                batch_id: existing.id,
                user_message_id: existing.user_message_id,
                recipients: existing.recipients,
            });
        }
        let thread = self.ensure_btw_thread(workspace, parent_session_id, qa_message_id, title)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let turn_index: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(turn_index) + 1, 0) FROM agent_message WHERE session_id = ?1",
            params![thread.child_session_id],
            |row| row.get(0),
        )?;
        let sequence = next_session_sequence(&transaction, &thread.child_session_id)?;
        let user_message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'user', 'complete', ?5)"#,
            params![
                user_message_id,
                thread.child_session_id,
                sequence,
                turn_index,
                now
            ],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, created_at_ms)
               VALUES (?1, ?2, 0, 'text', ?3, ?4)"#,
            params![new_external_id("part"), user_message_id, text, now],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_btw_batch
                   (external_id, thread_id, user_message_id, target_set_json, status, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'queued', ?5)"#,
            params![
                submit_id,
                thread.id,
                user_message_id,
                target_set_json(targets),
                now
            ],
        )?;
        let mut recipients = Vec::with_capacity(targets.len());
        for (ordinal, target) in targets.iter().enumerate() {
            let recipient_id = new_external_id("btwr");
            transaction.execute(
                r#"INSERT INTO agent_btw_recipient
                       (external_id, batch_id, ordinal, target_kind, target_key, agent_id,
                        source_child_session_id, display_nickname, role_name,
                        model_provider, model_id, variant, status, created_at_ms)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'queued', ?13)"#,
                params![
                    recipient_id,
                    submit_id,
                    i64::try_from(ordinal).unwrap_or(i64::MAX),
                    target.target_kind,
                    target.target_key,
                    target.agent_id,
                    target.source_child_session_id,
                    target.display_nickname,
                    target.role_name,
                    target.model.provider,
                    target.model.model,
                    target.model.variant,
                    now,
                ],
            )?;
            recipients.push(BtwRecipientRecord {
                id: recipient_id,
                batch_id: submit_id.to_string(),
                ordinal: i64::try_from(ordinal).unwrap_or(i64::MAX),
                target_kind: target.target_kind.clone(),
                target_key: target.target_key.clone(),
                agent_id: target.agent_id.clone(),
                source_child_session_id: target.source_child_session_id.clone(),
                display_nickname: target.display_nickname.clone(),
                role_name: target.role_name.clone(),
                model_provider: target.model.provider.clone(),
                model_id: target.model.model.clone(),
                variant: target.model.variant.clone(),
                assistant_message_id: None,
                status: "queued".to_string(),
                error_code: None,
            });
        }
        let answerers = target_keys(targets);
        transaction.execute(
            r#"UPDATE agent_btw_thread
               SET answerer_set_json = ?1, updated_at_ms = ?2
               WHERE external_id = ?3"#,
            params![json_array(&answerers), now, thread.id],
        )?;
        transaction.execute(
            "UPDATE agent_session SET updated_at_ms = ?1, last_message_at_ms = ?1 WHERE external_id = ?2",
            params![now, thread.child_session_id],
        )?;
        transaction.commit()?;
        Ok(BtwAccepted {
            thread,
            batch_id: submit_id.to_string(),
            user_message_id,
            recipients,
        })
    }

    /// 认领一个排队回答者：原子创建 streaming assistant 并转入 running。
    ///
    /// 返回 None 表示该回答者已被其他执行流认领或已终态。
    pub fn start_btw_recipient(
        &self,
        workspace: &Path,
        recipient_id: &str,
    ) -> Result<Option<BtwRecipientStart>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let row: Option<RecipientStartRow> = transaction
            .query_row(
                r#"SELECT r.status, b.thread_id, t.child_session_id,
                          r.model_provider, r.model_id, r.variant,
                          t.parent_session_id, t.parent_user_message_id, b.user_message_id,
                          r.display_nickname, r.agent_id, r.source_child_session_id,
                          r.target_kind,
                          COALESCE((
                              SELECT p.content FROM agent_message_part p
                              WHERE p.message_id = b.user_message_id
                              ORDER BY p.position LIMIT 1
                          ), '')
                   FROM agent_btw_recipient r
                   JOIN agent_btw_batch b ON b.external_id = r.batch_id
                   JOIN agent_btw_thread t ON t.external_id = b.thread_id
                   JOIN agent_session s ON s.external_id = t.parent_session_id
                   WHERE r.external_id = ?1 AND s.workspace_id = ?2"#,
                params![recipient_id, workspace_external],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                        row.get(11)?,
                        row.get(12)?,
                        row.get(13)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            status,
            thread_id,
            child_session_id,
            provider,
            model,
            variant,
            parent_session_id,
            qa_message_id,
            user_message_id,
            display_nickname,
            agent_id,
            source_child_session_id,
            target_kind,
            question,
        )) = row
        else {
            return Ok(None);
        };
        if status != "queued" {
            return Ok(None);
        }
        let turn_index: i64 = transaction.query_row(
            r#"SELECT m.turn_index FROM agent_message m
               JOIN agent_btw_batch b ON b.user_message_id = m.external_id
               WHERE b.external_id = (SELECT batch_id FROM agent_btw_recipient WHERE external_id = ?1)"#,
            params![recipient_id],
            |row| row.get(0),
        )?;
        let sequence = next_session_sequence(&transaction, &child_session_id)?;
        let assistant_message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status,
                    model_provider, model_id, variant, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'assistant', 'streaming', ?5, ?6, ?7, ?8)"#,
            params![
                assistant_message_id,
                child_session_id,
                sequence,
                turn_index,
                provider,
                model,
                variant,
                now,
            ],
        )?;
        let updated = transaction.execute(
            r#"UPDATE agent_btw_recipient
               SET status = 'running', assistant_message_id = ?1, completed_at_ms = NULL
               WHERE external_id = ?2 AND status = 'queued'"#,
            params![assistant_message_id, recipient_id],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        transaction.execute(
            "UPDATE agent_btw_batch SET status = 'running' WHERE external_id = (SELECT batch_id FROM agent_btw_recipient WHERE external_id = ?1)",
            params![recipient_id],
        )?;
        transaction.commit()?;
        Ok(Some(BtwRecipientStart {
            assistant_message_id,
            child_session_id,
            thread_id,
            parent_session_id,
            qa_message_id,
            user_message_id,
            question,
            display_nickname,
            target_kind,
            agent_id,
            source_child_session_id,
            model: ModelSnapshot {
                provider,
                model,
                variant,
            },
        }))
    }

    /// 结算单个回答者：assistant 终态/用量、回答者终态与批次聚合。
    pub fn finish_btw_recipient(
        &self,
        workspace: &Path,
        recipient_id: &str,
        assistant_message_id: &str,
        parts: Option<&[super::ChatPart]>,
        outcome: &TurnOutcome,
        usage: Usage,
        billing: Usage,
    ) -> Result<BtwRecipientCompletion> {
        self.finish_with_optional_parts(
            workspace,
            assistant_message_id,
            parts,
            outcome,
            usage,
            billing,
        )?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let (status, error_code) = match outcome {
            TurnOutcome::Complete { .. } => ("completed", None),
            TurnOutcome::Interrupted => ("interrupted", None),
            TurnOutcome::Failed { error_code } => ("failed", Some(error_code.as_str())),
        };
        let (batch_id, thread_id): (String, String) = transaction.query_row(
            r#"SELECT r.batch_id, b.thread_id
               FROM agent_btw_recipient r
               JOIN agent_btw_batch b ON b.external_id = r.batch_id
               WHERE r.external_id = ?1"#,
            params![recipient_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        transaction.execute(
            r#"UPDATE agent_btw_recipient
               SET status = ?1, error_code = ?2, completed_at_ms = ?3
               WHERE external_id = ?4"#,
            params![status, error_code, now, recipient_id],
        )?;
        let statuses: Vec<String> = {
            let mut statement = transaction
                .prepare("SELECT status FROM agent_btw_recipient WHERE batch_id = ?1")?;
            statement
                .query_map(params![batch_id], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let batch_terminal = statuses
            .iter()
            .all(|status| !matches!(status.as_str(), "queued" | "running"));
        if batch_terminal {
            let aggregate = aggregate_batch_status(&statuses);
            transaction.execute(
                "UPDATE agent_btw_batch SET status = ?1, completed_at_ms = ?2 WHERE external_id = ?3",
                params![aggregate, now, batch_id],
            )?;
            transaction.execute(
                "UPDATE agent_btw_thread SET updated_at_ms = ?1 WHERE external_id = ?2",
                params![now, thread_id],
            )?;
        }
        transaction.commit()?;
        Ok(BtwRecipientCompletion {
            thread_id,
            batch_id,
            batch_terminal,
            recipient_status: status.to_string(),
        })
    }

    /// 取消一批尚未开始的回答者（运行中的由事件层取消执行流后自行结算）。
    pub fn cancel_btw_queued(&self, batch_id: &str) -> Result<()> {
        let now = now_ms();
        self.conn.execute(
            "UPDATE agent_btw_recipient SET status = 'cancelled', completed_at_ms = ?1 WHERE batch_id = ?2 AND status = 'queued'",
            params![now, batch_id],
        )?;
        self.sync_btw_batch_status(batch_id)
    }

    /// 批次状态按当前回答者收敛（取消/中断后调用）。
    pub fn sync_btw_batch_status(&self, batch_id: &str) -> Result<()> {
        let mut statement = self
            .conn
            .prepare("SELECT status FROM agent_btw_recipient WHERE batch_id = ?1")?;
        let statuses: Vec<String> = statement
            .query_map(params![batch_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if !statuses
            .iter()
            .all(|status| !matches!(status.as_str(), "queued" | "running"))
        {
            return Ok(());
        }
        let aggregate = aggregate_batch_status(&statuses);
        self.conn.execute(
            "UPDATE agent_btw_batch SET status = ?1, completed_at_ms = ?2 WHERE external_id = ?3",
            params![aggregate, now_ms(), batch_id],
        )?;
        Ok(())
    }

    /// 删除线程与其 child session（级联清理消息、工具与附件）。
    pub fn delete_btw_thread(&self, workspace: &Path, thread_id: &str) -> Result<()> {
        let Some(thread) = self.btw_thread(workspace, thread_id)? else {
            return Ok(());
        };
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM agent_session WHERE external_id = ?1 AND source = 'btw'",
            params![thread.child_session_id],
        )?;
        transaction.execute(
            "DELETE FROM agent_btw_thread WHERE external_id = ?1",
            params![thread_id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 启动恢复：未完成的 BTW 回答标记 interrupted，保留已生成正文；不自动重发。
    pub fn recover_btw_startup(&self) -> Result<()> {
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            r#"UPDATE agent_message
               SET status = 'interrupted', completed_at_ms = ?1
               WHERE status = 'streaming' AND session_id IN (
                   SELECT external_id FROM agent_session WHERE source = 'btw'
               )"#,
            params![now],
        )?;
        transaction.execute(
            "UPDATE agent_btw_recipient SET status = 'interrupted', completed_at_ms = ?1 WHERE status IN ('queued', 'running')",
            params![now],
        )?;
        transaction.execute(
            "UPDATE agent_btw_batch SET status = 'interrupted', completed_at_ms = ?1 WHERE status IN ('queued', 'running')",
            params![now],
        )?;
        transaction.commit()?;
        Ok(())
    }

    fn btw_batch_by_submit(&self, submit_id: &str) -> Result<Option<BtwBatchRecord>> {
        let batch: Option<BtwBatchRecord> = self
            .conn
            .query_row(
                r#"SELECT external_id, thread_id, user_message_id, status, created_at_ms
                   FROM agent_btw_batch WHERE external_id = ?1"#,
                params![submit_id],
                |row| {
                    Ok(BtwBatchRecord {
                        id: row.get(0)?,
                        thread_id: row.get(1)?,
                        user_message_id: row.get(2)?,
                        status: row.get(3)?,
                        created_at_ms: row.get(4)?,
                        recipients: Vec::new(),
                    })
                },
            )
            .optional()?;
        let Some(mut batch) = batch else {
            return Ok(None);
        };
        let mut statement = self.conn.prepare(
            r#"SELECT external_id, batch_id, ordinal, target_kind, target_key, agent_id,
                      source_child_session_id, display_nickname, role_name,
                      model_provider, model_id, variant, assistant_message_id, status, error_code
               FROM agent_btw_recipient WHERE batch_id = ?1 ORDER BY ordinal"#,
        )?;
        batch.recipients = statement
            .query_map(params![submit_id], read_recipient_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Some(batch))
    }
}

fn read_thread_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BtwThreadRecord> {
    let answerers: String = row.get(5)?;
    Ok(BtwThreadRecord {
        id: row.get(0)?,
        parent_session_id: row.get(1)?,
        parent_user_message_id: row.get(2)?,
        child_session_id: row.get(3)?,
        title: row.get(4)?,
        answerers: serde_json::from_str(&answerers).unwrap_or_default(),
        created_at_ms: row.get(6)?,
        updated_at_ms: row.get(7)?,
    })
}

fn read_recipient_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BtwRecipientRecord> {
    Ok(BtwRecipientRecord {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        ordinal: row.get(2)?,
        target_kind: row.get(3)?,
        target_key: row.get(4)?,
        agent_id: row.get(5)?,
        source_child_session_id: row.get(6)?,
        display_nickname: row.get(7)?,
        role_name: row.get(8)?,
        model_provider: row.get(9)?,
        model_id: row.get(10)?,
        variant: row.get(11)?,
        assistant_message_id: row.get(12)?,
        status: row.get(13)?,
        error_code: row.get(14)?,
    })
}

fn target_keys(targets: &[BtwTargetSpec]) -> Vec<String> {
    targets
        .iter()
        .map(|target| target.target_key.clone())
        .collect()
}

fn target_set_json(targets: &[BtwTargetSpec]) -> String {
    json_array(&target_keys(targets))
}

fn json_array(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_string())
}

/// `start_btw_recipient` 的宽行：status/thread/child/model/parent/qa/user/身份/问题。
type RecipientStartRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
);

/// 回答者终态 → 批次聚合状态。
fn aggregate_batch_status(statuses: &[String]) -> &'static str {
    let completed = statuses
        .iter()
        .filter(|s| s.as_str() == "completed")
        .count();
    let failed = statuses
        .iter()
        .filter(|s| matches!(s.as_str(), "failed" | "interrupted"))
        .count();
    let cancelled = statuses
        .iter()
        .filter(|s| s.as_str() == "cancelled")
        .count();
    if completed > 0 && (failed > 0 || cancelled > 0) {
        "partial"
    } else if completed > 0 {
        "completed"
    } else if failed > 0 {
        "failed"
    } else if cancelled > 0 {
        "cancelled"
    } else {
        "failed"
    }
}

#[cfg(test)]
mod tests;

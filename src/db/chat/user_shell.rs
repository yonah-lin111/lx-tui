//! 用户本地命令的持久化：独立 `kind='user_shell'` 消息、checkpoint 与终态。
//!
//! 命令与普通聊天共用 `agent_message` 的 ID、sequence 与删除级联，但不产生 assistant
//! 行、不增加模型轮数/用量。执行事实只以 `user_shell_json` 为准；text part 仅保存
//! 用户命令，供标题与摘要使用。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::chat::process::ProcessState;

use super::super::{DbError, Result, new_external_id, now_ms};
use super::{ModelSnapshot, single_line_preview};

/// 单条命令输出的保存上限（与上下文回放同口径 64 KiB）。
pub const MAX_USER_SHELL_OUTPUT_BYTES: usize = 64 * 1024;

/// 一次本地命令的稳定记录（JSON 字段是持久化契约）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserShellRecord {
    /// 去模式前缀后的原始脚本。
    pub command: String,
    /// 实际执行目录。
    pub cwd: String,
    /// 实际使用的 shell。
    pub shell: String,
    /// 进程状态稳定值（`starting`/`running`/`exited`/…）。
    pub state: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    /// 受控输出（已剥离控制序列）。
    pub output: String,
    pub output_start: u64,
    pub output_end: u64,
    pub dropped_bytes: u64,
    pub truncated: bool,
    pub runtime_epoch: Option<String>,
    pub process_id: Option<u64>,
    /// 单调修订号：较旧 revision 与终态后的 Running 一律拒绝。
    pub revision: u64,
}

impl UserShellRecord {
    /// 新登记的运行记录（尚无进程 ID）。
    pub fn starting(command: &str, cwd: &str, shell: &str) -> Self {
        Self {
            command: command.to_string(),
            cwd: cwd.to_string(),
            shell: shell.to_string(),
            state: ProcessState::Starting.as_str().to_string(),
            exit_code: None,
            duration_ms: None,
            output: String::new(),
            output_start: 0,
            output_end: 0,
            dropped_bytes: 0,
            truncated: false,
            runtime_epoch: None,
            process_id: None,
            revision: 0,
        }
    }

    /// 终态判定（`starting`/`running` 之外的稳定值）。
    pub fn is_terminal(&self) -> bool {
        !matches!(self.state.as_str(), "starting" | "running")
    }

    /// 消息状态列：运行中保持 `streaming`；退出（含非零）为 `complete`。
    pub fn message_status(&self) -> &'static str {
        match self.state.as_str() {
            "starting" | "running" => "streaming",
            "exited" => "complete",
            "stopped" | "timed_out" | "interrupted" => "interrupted",
            _ => "failed",
        }
    }

    /// 输出是否被当前快照截断（尾部窗口丢失了更早内容）。
    pub fn output_truncated(&self) -> bool {
        self.truncated || self.dropped_bytes > 0
    }

    /// 稳定 JSON。
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// 解析 JSON；损坏记录返回 None（调用方显示明确降级，不执行其中内容）。
    pub fn from_json(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }
}

/// `begin_user_shell` 的提交结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserShellCommitted {
    pub session_id: String,
    pub message_id: String,
    pub turn_index: i64,
    pub sequence: i64,
    pub created_at_ms: i64,
}

impl super::Database {
    /// 预登记一条本地命令：必要时创建**无模型**会话，再写独立 user_shell 消息。
    ///
    /// sequence/turn_index 与普通聊天共用同一分配规则（全 kind 的 MAX+1）；
    /// 失败整体回滚，调用方不得 spawn。
    pub fn begin_user_shell(
        &self,
        workspace: &Path,
        existing_session: Option<&str>,
        record: &UserShellRecord,
    ) -> Result<UserShellCommitted> {
        if record.command.trim().is_empty() {
            return Err(DbError::InvalidDraft);
        }
        if record.command.len() > super::MAX_MESSAGE_BYTES {
            return Err(DbError::MessageTooLarge);
        }
        self.ensure_workspace(workspace)?;
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        let (session_id, sequence, turn_index) = match existing_session {
            Some(session_id) => {
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
                // sequence 与独立提示共用编号空间（`begin_turn` 同口径）：
                // 撤销删除消息后提示仍保留高水位，只查 agent_message 会撞号。
                let (sequence, turn_index): (i64, i64) = transaction.query_row(
                    r#"SELECT COALESCE(MAX(sequence) + 1, 0),
                              COALESCE(MAX(turn_index) + 1, 0)
                       FROM (
                           SELECT sequence, turn_index FROM agent_message WHERE session_id = ?1
                           UNION ALL
                           SELECT sequence, NULL FROM agent_session_notice WHERE session_id = ?1
                       )"#,
                    params![session_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                (session_id.to_string(), sequence, turn_index)
            }
            None => {
                let session_id = new_external_id("sess");
                let title = single_line_preview(&record.command, 60);
                // 本地命令不要求模型或 API Key：模型三列保持 NULL（不是空串）。
                transaction.execute(
                    r#"INSERT INTO agent_session
                           (external_id, workspace_id, source, title, first_user_message,
                            model_provider, model_id, variant, status, archived,
                            created_at_ms, updated_at_ms, last_message_at_ms)
                       VALUES (?1, ?2, 'native', ?3, ?4, NULL, NULL, NULL, 'active', 0,
                               ?5, ?5, ?5)"#,
                    params![
                        session_id,
                        workspace_external,
                        title,
                        single_line_preview(&record.command, super::PREVIEW_CHARS),
                        now,
                    ],
                )?;
                (session_id, 0, 0)
            }
        };
        let message_id = new_external_id("msg");
        transaction.execute(
            r#"INSERT INTO agent_message
                   (external_id, session_id, sequence, turn_index, role, status, kind,
                    user_shell_json, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, 'user', 'streaming', 'user_shell', ?5, ?6)"#,
            params![
                message_id,
                session_id,
                sequence,
                turn_index,
                record.to_json(),
                now,
            ],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_message_part
                   (external_id, message_id, position, kind, content, created_at_ms)
               VALUES (?1, ?2, 0, 'text', ?3, ?4)"#,
            params![new_external_id("part"), message_id, record.command, now],
        )?;
        transaction.execute(
            "UPDATE agent_session SET updated_at_ms = ?1, last_message_at_ms = ?1 WHERE external_id = ?2",
            params![now, session_id],
        )?;
        transaction.commit()?;
        Ok(UserShellCommitted {
            session_id,
            message_id,
            turn_index,
            sequence,
            created_at_ms: now,
        })
    }

    /// 幂等写入一次命令快照（输出 checkpoint 或终态）。
    ///
    /// - revision 必须不小于库里记录的 revision；
    /// - 已终态记录不接受任何 `starting`/`running` 覆盖；
    /// - 重复的同一终态返回 `Ok(false)`。
    ///
    /// 返回是否实际改动。
    pub fn update_user_shell(
        &self,
        workspace: &Path,
        message_id: &str,
        record: &UserShellRecord,
    ) -> Result<bool> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let existing: Option<(String, Option<String>)> = transaction
            .query_row(
                r#"SELECT m.status, m.user_shell_json
                   FROM agent_message m
                   JOIN agent_session s ON s.external_id = m.session_id
                   WHERE m.external_id = ?1 AND s.workspace_id = ?2
                     AND m.kind = 'user_shell'"#,
                params![message_id, workspace_external],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((status, previous_json)) = existing else {
            return Err(DbError::MessageNotStreaming);
        };
        let previous = previous_json
            .as_deref()
            .and_then(UserShellRecord::from_json);
        if let Some(previous) = &previous {
            if record.revision < previous.revision {
                return Ok(false);
            }
            if previous.is_terminal() && !record.is_terminal() {
                return Ok(false);
            }
        }
        let status_next = record.message_status();
        if status != "streaming" && status_next == "streaming" {
            return Ok(false);
        }
        let completed = record.is_terminal().then_some(now_ms());
        transaction.execute(
            r#"UPDATE agent_message
               SET status = ?1, user_shell_json = ?2, completed_at_ms = COALESCE(?3, completed_at_ms)
               WHERE external_id = ?4"#,
            params![status_next, record.to_json(), completed, message_id],
        )?;
        if let Some(completed) = completed {
            transaction.execute(
                r#"UPDATE agent_session SET updated_at_ms = ?1, last_message_at_ms = ?1
                   WHERE external_id = (SELECT session_id FROM agent_message WHERE external_id = ?2)"#,
                params![completed, message_id],
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    /// 读取某会话在 `after_sequence` 之后的本地命令消息（含运行中），按 sequence 升序。
    ///
    /// 供当前模型在下一次采样前增量吸收；只读，不执行命令。
    pub fn load_user_shells_since(
        &self,
        workspace: &Path,
        session_id: &str,
        after_sequence: i64,
    ) -> Result<Vec<super::ChatMessage>> {
        let Some(workspace_external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
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
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(
            r#"SELECT external_id, sequence, turn_index, role, status, error_code,
                      created_at_ms, completed_at_ms, user_shell_json
               FROM agent_message
               WHERE session_id = ?1 AND kind = 'user_shell' AND sequence > ?2
               ORDER BY sequence"#,
        )?;
        let rows = statement.query_map(params![session_id, after_sequence], |row| {
            let json: Option<String> = row.get(8)?;
            Ok(super::ChatMessage {
                external_id: row.get(0)?,
                sequence: row.get(1)?,
                turn_index: row.get(2)?,
                kind: "user_shell".to_string(),
                role: row.get(3)?,
                status: row.get(4)?,
                model_provider: None,
                model_id: None,
                variant: None,
                stop_reason: None,
                error_code: row.get(5)?,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
                created_at_ms: row.get(6)?,
                completed_at_ms: row.get(7)?,
                user_shell: json.as_deref().and_then(UserShellRecord::from_json),
                parts: Vec::new(),
                tools: Vec::new(),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 重启恢复：把遗留 `streaming` 的本地命令标为 interrupted（保留已保存输出）。
    ///
    /// 不重跑任何命令；重复调用幂等。
    pub fn recover_interrupted_user_shells(&self, sessions: &[String]) -> Result<usize> {
        let now = now_ms();
        let transaction = self.conn.unchecked_transaction()?;
        // 通用流恢复可能已把 status 改成 interrupted，但 JSON 仍写着 running；
        // 因此按 JSON 状态判断，而不是只看 status='streaming'。
        let rows: Vec<(String, String, Option<String>)> = {
            let mut statement = transaction.prepare(
                "SELECT external_id, session_id, user_shell_json FROM agent_message \
                 WHERE kind = 'user_shell' AND user_shell_json IS NOT NULL",
            )?;
            let mapped =
                statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
            mapped.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut total = 0usize;
        for (message_id, session_id, json) in rows {
            // 只结算本实例拿到 guard 的会话，其他实例仍在运行的命令不能被误标。
            if !sessions.iter().any(|session| session == &session_id) {
                continue;
            }
            let Some(mut record) = json.as_deref().and_then(UserShellRecord::from_json) else {
                continue;
            };
            if record.is_terminal() {
                continue;
            }
            record.state = ProcessState::Interrupted.as_str().to_string();
            record.revision = record.revision.saturating_add(1);
            transaction.execute(
                r#"UPDATE agent_message
                   SET status = 'interrupted', user_shell_json = ?1, completed_at_ms = ?2
                   WHERE external_id = ?3"#,
                params![record.to_json(), now, message_id],
            )?;
            total += 1;
        }
        transaction.commit()?;
        Ok(total)
    }
}

/// 会话模型选择：`agent_session` 的模型列可为 NULL（纯 shell 会话）。
pub(crate) fn session_model(
    connection: &Connection,
    workspace_external: &str,
    session_id: &str,
) -> Result<Option<ModelSnapshot>> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = connection
        .query_row(
            "SELECT model_provider, model_id, variant FROM agent_session \
             WHERE external_id = ?1 AND workspace_id = ?2",
            params![session_id, workspace_external],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    Ok(row.and_then(|(provider, model, variant)| {
        Some(ModelSnapshot {
            provider: provider?,
            model: model?,
            variant,
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Database, SCHEMA_VERSION};

    fn temp_path(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("lx-tui-shell-{label}-{nanos:x}"))
    }

    fn setup(label: &str) -> (Database, std::path::PathBuf) {
        let database = Database::open(&temp_path(label)).expect("open");
        let workspace = std::env::temp_dir().join(format!("lx-tui-shell-ws-{label}"));
        database.trust_workspace(&workspace, "ws").expect("trust");
        (database, workspace)
    }

    fn record(state: &str, revision: u64) -> UserShellRecord {
        UserShellRecord {
            state: state.into(),
            revision,
            ..UserShellRecord::starting("pwd", "/tmp", "/bin/zsh")
        }
    }

    #[test]
    fn shell_only_session_has_no_model_and_loads_typed_record() {
        let (database, workspace) = setup("only");
        let committed = database
            .begin_user_shell(
                &workspace,
                None,
                &UserShellRecord::starting("pwd", "/tmp", "/bin/zsh"),
            )
            .expect("begin");
        let messages = database
            .load_chat_session(&workspace, &committed.session_id)
            .expect("load")
            .expect("session");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].kind, "user_shell");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].status, "streaming");
        assert_eq!(messages[0].parts.len(), 1);
        assert_eq!(messages[0].parts[0].content, "pwd");
        assert_eq!(
            messages[0]
                .user_shell
                .as_ref()
                .map(|record| record.state.as_str()),
            Some("starting")
        );
        assert_eq!(
            database
                .chat_session_model(&workspace, &committed.session_id)
                .expect("model"),
            None,
            "纯命令会话没有模型"
        );
    }

    #[test]
    fn later_chat_turn_binds_model_to_shell_only_session() {
        let (database, workspace) = setup("later-chat");
        let shell = database
            .begin_user_shell(
                &workspace,
                None,
                &UserShellRecord::starting("ls", "/tmp", "/bin/zsh"),
            )
            .expect("shell");
        let model = ModelSnapshot {
            provider: "example".into(),
            model: "m".into(),
            variant: None,
        };
        let turn = database
            .begin_turn(&workspace, Some(&shell.session_id), "解释一下", &model)
            .expect("chat turn");
        assert_eq!(turn.turn_index, 1, "命令独占一个 turn_index");
        assert_eq!(
            database
                .chat_session_model(&workspace, &shell.session_id)
                .expect("model"),
            Some(model)
        );
    }

    #[test]
    fn snapshots_are_monotonic_and_terminal_is_sticky() {
        let (database, workspace) = setup("monotonic");
        let committed = database
            .begin_user_shell(
                &workspace,
                None,
                &UserShellRecord::starting("pwd", "/tmp", "/bin/zsh"),
            )
            .expect("begin");
        assert!(
            database
                .update_user_shell(&workspace, &committed.message_id, &record("running", 1))
                .expect("running")
        );
        assert!(
            !database
                .update_user_shell(&workspace, &committed.message_id, &record("running", 0))
                .expect("older"),
            "较旧 revision 被拒绝"
        );
        let terminal = UserShellRecord {
            exit_code: Some(0),
            duration_ms: Some(42),
            output: "hello\n".into(),
            ..record("exited", 2)
        };
        assert!(
            database
                .update_user_shell(&workspace, &committed.message_id, &terminal)
                .expect("terminal")
        );
        assert!(
            !database
                .update_user_shell(&workspace, &committed.message_id, &record("running", 3))
                .expect("late running"),
            "终态不能被迟到的 Running 覆盖"
        );
        let messages = database
            .load_chat_session(&workspace, &committed.session_id)
            .expect("load")
            .expect("session");
        assert_eq!(messages[0].status, "complete", "非零退出也算命令完成");
        let loaded = messages[0].user_shell.as_ref().expect("record");
        assert_eq!(loaded.state, "exited");
        assert_eq!(loaded.exit_code, Some(0));
        assert_eq!(loaded.duration_ms, Some(42));
        assert_eq!(loaded.output, "hello\n");
    }

    #[test]
    fn failed_and_stopped_map_to_failed_and_interrupted() {
        let (database, workspace) = setup("status");
        let committed = database
            .begin_user_shell(
                &workspace,
                None,
                &UserShellRecord::starting("x", "/tmp", "/bin/zsh"),
            )
            .expect("begin");
        database
            .update_user_shell(&workspace, &committed.message_id, &record("stopped", 1))
            .expect("stopped");
        let messages = database
            .load_chat_session(&workspace, &committed.session_id)
            .expect("load")
            .expect("session");
        assert_eq!(messages[0].status, "interrupted");
        let other = database
            .begin_user_shell(
                &workspace,
                Some(&committed.session_id),
                &UserShellRecord::starting("y", "/tmp", "/bin/zsh"),
            )
            .expect("second");
        database
            .update_user_shell(&workspace, &other.message_id, &record("failed", 1))
            .expect("failed");
        let messages = database
            .load_chat_session(&workspace, &committed.session_id)
            .expect("load")
            .expect("session");
        assert_eq!(messages[1].status, "failed");
    }

    #[test]
    fn load_since_and_recovery_mark_inflight_interrupted() {
        let (database, workspace) = setup("recover");
        let committed = database
            .begin_user_shell(
                &workspace,
                None,
                &UserShellRecord::starting("sleep 10", "/tmp", "/bin/zsh"),
            )
            .expect("begin");
        let since = database
            .load_user_shells_since(&workspace, &committed.session_id, -1)
            .expect("since");
        assert_eq!(since.len(), 1);
        assert_eq!(
            database
                .recover_interrupted_user_shells(std::slice::from_ref(&committed.session_id))
                .expect("recover"),
            1
        );
        let messages = database
            .load_chat_session(&workspace, &committed.session_id)
            .expect("load")
            .expect("session");
        assert_eq!(messages[0].status, "interrupted");
        let record = messages[0].user_shell.as_ref().expect("record");
        assert_eq!(record.state, "interrupted");
        assert!(record.revision > 0, "恢复也推进 revision");
        assert_eq!(
            database
                .recover_interrupted_user_shells(std::slice::from_ref(&committed.session_id))
                .expect("idempotent"),
            0
        );
    }

    #[test]
    fn schema_version_is_current() {
        let (database, _) = setup("version");
        let version: i64 = database
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(version, SCHEMA_VERSION);
    }
}

//! 会话中途模型切换（model switch）的持久化：写入切换事件、联合取号与时间线查询。
//!
//! 分层约束：
//! - 只接触 rusqlite，不做网络或文件系统 IO；
//! - `sequence` 与 `agent_message`、`agent_session_notice`、`agent_compact_checkpoint`
//!   共用同一编号空间（由 [`super::next_session_sequence`] 统一取号）；
//! - 不递增 `agent_session.revision`，避免打断在途 `/rewind` 或 `/compact` 的 CAS。

use std::path::Path;

use rusqlite::{OptionalExtension, Row, params};

use super::{Database, DbError, ModelSnapshot, Result, new_external_id, now_ms};

/// 切换记录的源模型行：`(sequence, provider, model, variant)`。
type SwitchOrigin = (i64, Option<String>, Option<String>, Option<String>);

/// 一条已持久化的模型切换时间线记录（供时间线渲染与上下文 `<system-update>` 注入读取）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSwitchRecord {
    pub id: String,
    pub sequence: i64,
    pub from_provider: Option<String>,
    pub from_model: Option<String>,
    pub from_variant: Option<String>,
    pub to_provider: String,
    pub to_model: String,
    pub to_variant: Option<String>,
    pub from_label: Option<String>,
    pub to_label: String,
    pub updated_sources: Vec<String>,
    pub injected_prompt: String,
    pub created_at_ms: i64,
}

impl ModelSwitchRecord {
    /// 是否仅切换了同一模型的思考等级（provider 与 model 未变）。
    pub fn is_variant_only(&self) -> bool {
        self.from_provider.as_deref() == Some(self.to_provider.as_str())
            && self.from_model.as_deref() == Some(self.to_model.as_str())
    }
}

/// 一次模型切换的写入请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSwitchCommit {
    pub session_id: String,
    pub model: ModelSnapshot,
    pub from_label: Option<String>,
    pub to_label: String,
    pub updated_sources: Vec<String>,
    pub injected_prompt: String,
    /// 发起切换的窗格实际工作目录：持久化层据此探测 worktree/子目录独有的 `AGENTS.md`。
    pub cwd: Option<std::path::PathBuf>,
    /// 切换后的模型身份：持久化层补齐指令来源后据此重新渲染权威 `injected_prompt`。
    pub to_identity: crate::chat::agent::ModelIdentity,
}

impl Database {
    /// 按 `sequence` 升序加载会话的全部模型切换记录（双条件校验工作区归属）。
    pub fn model_switches(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Vec<ModelSwitchRecord>> {
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
            r#"SELECT external_id, sequence, from_provider, from_model, from_variant,
                      to_provider, to_model, to_variant, from_label, to_label,
                      updated_sources_json, injected_prompt, created_at_ms
               FROM agent_model_switch
               WHERE session_id = ?1
               ORDER BY sequence ASC, id ASC"#,
        )?;
        let rows = statement.query_map(params![session_id], read_model_switch)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 在同一写事务内更新会话模型三列，并在会话已有消息且模型/思考等级实际变化时追加
    /// 一条 `agent_model_switch` 时间线记录。
    pub fn record_model_switch(
        &self,
        workspace: &Path,
        commit: &ModelSwitchCommit,
    ) -> Result<Option<ModelSwitchRecord>> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.write_transaction()?;
        let current: Option<(Option<String>, Option<String>, Option<String>)> = transaction
            .query_row(
                r#"SELECT model_provider, model_id, variant
                   FROM agent_session
                   WHERE external_id = ?1 AND workspace_id = ?2"#,
                params![commit.session_id, workspace_external],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((from_provider, from_model, from_variant)) = current else {
            return Err(DbError::SessionMissing);
        };

        let now = now_ms();
        super::update_session_selection(
            &transaction,
            &commit.session_id,
            &workspace_external,
            &commit.model,
            now,
        )?;

        let message_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM agent_message WHERE session_id = ?1",
            params![commit.session_id],
            |row| row.get(0),
        )?;
        let unchanged = from_provider.as_deref() == Some(commit.model.provider.as_str())
            && from_model.as_deref() == Some(commit.model.model.as_str())
            && from_variant == commit.model.variant;
        if message_count == 0 || unchanged {
            transaction.commit()?;
            return Ok(None);
        }

        let sequence = super::next_session_sequence(&transaction, &commit.session_id)?;
        let id = new_external_id("msw");
        let sources_json =
            serde_json::to_string(&commit.updated_sources).unwrap_or_else(|_| "[]".to_string());
        transaction.execute(
            r#"INSERT INTO agent_model_switch
                   (external_id, session_id, sequence, from_provider, from_model, from_variant,
                    to_provider, to_model, to_variant, from_label, to_label,
                    updated_sources_json, injected_prompt, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"#,
            params![
                id,
                commit.session_id,
                sequence,
                from_provider,
                from_model,
                from_variant,
                commit.model.provider,
                commit.model.model,
                commit.model.variant,
                commit.from_label,
                commit.to_label,
                sources_json,
                commit.injected_prompt,
                now,
            ],
        )?;
        transaction.commit()?;

        Ok(Some(ModelSwitchRecord {
            id,
            sequence,
            from_provider,
            from_model,
            from_variant,
            to_provider: commit.model.provider.clone(),
            to_model: commit.model.model.clone(),
            to_variant: commit.model.variant.clone(),
            from_label: commit.from_label.clone(),
            to_label: commit.to_label.clone(),
            updated_sources: commit.updated_sources.clone(),
            injected_prompt: commit.injected_prompt.clone(),
            created_at_ms: now,
        }))
    }

    /// 撤销最新一次模型切换：会话模型回滚到切换前，并删除该时间线条目。
    ///
    /// 只能撤销当前最新的一条切换；返回 `None` 表示切换前会话没有显式模型。
    pub fn revert_model_switch(
        &self,
        workspace: &Path,
        session_id: &str,
        switch_id: &str,
        expected_revision: i64,
        operation_id: &str,
    ) -> Result<Option<ModelSnapshot>> {
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.write_transaction()?;
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
        let revision: i64 = transaction.query_row(
            "SELECT revision FROM agent_session WHERE external_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        if revision != expected_revision {
            return Err(DbError::RewindTargetChanged);
        }
        let switch: Option<SwitchOrigin> = transaction
            .query_row(
                r#"SELECT sequence, from_provider, from_model, from_variant
                   FROM agent_model_switch
                   WHERE external_id = ?1 AND session_id = ?2"#,
                params![switch_id, session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((sequence, from_provider, from_model, from_variant)) = switch else {
            return Err(DbError::RewindTargetChanged);
        };
        // 只能撤销最新一条：更晚的切换存在时目标已过期。
        let later: Option<String> = transaction
            .query_row(
                r#"SELECT external_id FROM agent_model_switch
                   WHERE session_id = ?1 AND sequence > ?2
                   ORDER BY sequence DESC LIMIT 1"#,
                params![session_id, sequence],
                |row| row.get(0),
            )
            .optional()?;
        if later.is_some() {
            return Err(DbError::RewindTargetChanged);
        }
        let now = now_ms();
        transaction.execute(
            r#"UPDATE agent_session
               SET model_provider = ?1, model_id = ?2, variant = ?3,
                   revision = revision + 1, updated_at_ms = ?4
               WHERE external_id = ?5"#,
            params![from_provider, from_model, from_variant, now, session_id],
        )?;
        transaction.execute(
            "DELETE FROM agent_model_switch WHERE external_id = ?1 AND session_id = ?2",
            params![switch_id, session_id],
        )?;
        transaction.execute(
            r#"INSERT INTO agent_rewind_operation
                   (external_id, operation_id, session_id, action, target_user_message_id,
                    expected_revision, phase, user_text, created_at_ms, updated_at_ms)
               VALUES (?1, ?2, ?3, 'restore_model', ?4, ?5, 'committed', '', ?6, ?6)"#,
            params![
                new_external_id("op"),
                operation_id,
                session_id,
                switch_id,
                expected_revision,
                now,
            ],
        )?;
        transaction.commit()?;
        Ok(match (from_provider, from_model) {
            (Some(provider), Some(model)) => Some(ModelSnapshot {
                provider,
                model,
                variant: from_variant,
            }),
            _ => None,
        })
    }
}

fn read_model_switch(row: &Row<'_>) -> rusqlite::Result<ModelSwitchRecord> {
    let sources_json: String = row.get(10)?;
    let updated_sources: Vec<String> = serde_json::from_str(&sources_json).unwrap_or_default();
    Ok(ModelSwitchRecord {
        id: row.get(0)?,
        sequence: row.get(1)?,
        from_provider: row.get(2)?,
        from_model: row.get(3)?,
        from_variant: row.get(4)?,
        to_provider: row.get(5)?,
        to_model: row.get(6)?,
        to_variant: row.get(7)?,
        from_label: row.get(8)?,
        to_label: row.get(9)?,
        updated_sources,
        injected_prompt: row.get(11)?,
        created_at_ms: row.get(12)?,
    })
}

#[cfg(test)]
mod tests;

/// 分叉会话时复制边界之前的模型切换记录。
pub(super) fn copy_model_switches_for_fork(
    conn: &rusqlite::Connection,
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
    #[allow(clippy::type_complexity)]
    let rows = statement
        .query_map(params![source_session, start_seq, boundary], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, i64>(11)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (
        sequence,
        from_provider,
        from_model,
        from_variant,
        to_provider,
        to_model,
        to_variant,
        from_label,
        to_label,
        updated_sources_json,
        injected_prompt,
        created_at_ms,
    ) in rows
    {
        conn.execute(
            r#"INSERT INTO agent_model_switch
                   (external_id, session_id, sequence, from_provider, from_model, from_variant,
                    to_provider, to_model, to_variant, from_label, to_label,
                    updated_sources_json, injected_prompt, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"#,
            params![
                new_external_id("msw"),
                new_session_id,
                sequence,
                from_provider,
                from_model,
                from_variant,
                to_provider,
                to_model,
                to_variant,
                from_label,
                to_label,
                updated_sources_json,
                injected_prompt,
                created_at_ms,
            ],
        )?;
    }
    Ok(())
}

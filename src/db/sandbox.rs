//! 会话级沙箱覆盖与审批规则（v31/v32）。

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Database, DbError, new_external_id, now_ms};

/// 一条已持久化的权限模式切换记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionSwitchRecord {
    pub id: String,
    pub sequence: i64,
    pub from_mode: String,
    pub to_mode: String,
    pub injected_prompt: String,
    pub created_at_ms: i64,
}

/// 一次权限模式切换的写入请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionSwitchCommit {
    pub session_id: String,
    pub from_mode: String,
    pub to_mode: String,
    pub injected_prompt: String,
}

/// 规则种类：`exact` 为整条命令全等，`prefix` 为 token 前缀。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxRuleKind {
    /// on-failure 的「本会话同命令自动重试」。
    Exact,
    /// 模型 `prefix_rule` 的「本会话批准该前缀」。
    Prefix,
}

impl SandboxRuleKind {
    /// 稳定存储值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Prefix => "prefix",
        }
    }

    /// 解析存储值。
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "exact" => Some(Self::Exact),
            "prefix" => Some(Self::Prefix),
            _ => None,
        }
    }
}

/// 一条会话审批规则。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSandboxRule {
    /// 规则种类。
    pub kind: SandboxRuleKind,
    /// 命令 token 序列。
    pub tokens: Vec<String>,
}

impl Database {
    /// 读取会话沙箱模式覆盖；会话不存在或未设置返回 None。
    pub fn session_sandbox_mode(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Option<String>, DbError> {
        let Some(workspace) = self.workspace_external_id(workspace)? else {
            return Ok(None);
        };
        self.conn
            .query_row(
                "SELECT sandbox_mode FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace],
                |row| row.get(0),
            )
            .optional()
            .map(|value| value.flatten())
            .map_err(DbError::from)
    }

    /// 写入会话沙箱模式覆盖（`None` = 跟随全局默认）。
    pub fn set_session_sandbox_mode(
        &self,
        workspace: &Path,
        session_id: &str,
        mode: Option<&str>,
    ) -> Result<(), DbError> {
        let workspace = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let changed = self.conn.execute(
            "UPDATE agent_session SET sandbox_mode = ?1 WHERE external_id = ?2 AND workspace_id = ?3",
            params![mode, session_id, workspace],
        )?;
        if changed == 0 {
            return Err(DbError::SessionMissing);
        }
        Ok(())
    }

    /// 加载会话审批规则。
    pub fn sandbox_rules(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Vec<SessionSandboxRule>, DbError> {
        let Some(workspace) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(
            "SELECT kind, prefix_json FROM agent_session_sandbox_rule WHERE session_id = ?1 ORDER BY id ASC",
        )?;
        let rows = statement.query_map(params![session_id], |row| {
            let kind: String = row.get(0)?;
            let tokens: String = row.get(1)?;
            Ok((kind, tokens))
        })?;
        let mut rules = Vec::new();
        for row in rows {
            let (kind, prefix_json) = row?;
            let Some(kind) = SandboxRuleKind::parse(&kind) else {
                continue;
            };
            let tokens = if prefix_json.trim_start().starts_with('[') {
                match serde_json::from_str::<Vec<String>>(&prefix_json) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        tracing::warn!(%error, "skipping malformed sandbox rule prefix_json");
                        continue;
                    }
                }
            } else if prefix_json.is_empty() {
                Vec::new()
            } else {
                // 兼容历史测试/旧库用 `\u{0}` 拼接的数据（保留空参数 token）。
                prefix_json.split('\u{0}').map(str::to_string).collect()
            };
            rules.push(SessionSandboxRule { kind, tokens });
        }
        Ok(rules)
    }

    /// 插入一条会话审批规则（重复规则幂等）。
    pub fn insert_sandbox_rule(
        &self,
        workspace: &Path,
        session_id: &str,
        rule: &SessionSandboxRule,
    ) -> Result<(), DbError> {
        let workspace = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        let prefix_json = serde_json::to_string(&rule.tokens).unwrap_or_else(|error| {
            tracing::warn!(%error, "failed to serialize sandbox rule tokens");
            "[]".to_string()
        });
        self.conn.execute(
            "INSERT OR IGNORE INTO agent_session_sandbox_rule (session_id, kind, prefix_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, rule.kind.as_str(), prefix_json, now_ms()],
        )?;
        Ok(())
    }

    /// 清空会话审批规则（`/permission` 切档）。
    pub fn clear_sandbox_rules(&self, session_id: &str) -> Result<(), DbError> {
        self.conn.execute(
            "DELETE FROM agent_session_sandbox_rule WHERE session_id = ?1",
            params![session_id],
        )?;
        Ok(())
    }

    /// 按 `sequence` 升序加载会话的全部权限切换记录（双条件校验工作区归属）。
    pub fn permission_switches(
        &self,
        workspace: &Path,
        session_id: &str,
    ) -> Result<Vec<PermissionSwitchRecord>, DbError> {
        let Some(workspace) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let owned: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![session_id, workspace],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(
            r#"SELECT external_id, sequence, from_mode, to_mode, injected_prompt, created_at_ms
               FROM agent_permission_switch
               WHERE session_id = ?1
               ORDER BY sequence ASC, id ASC"#,
        )?;
        let rows = statement.query_map(params![session_id], |row| {
            Ok(PermissionSwitchRecord {
                id: row.get(0)?,
                sequence: row.get(1)?,
                from_mode: row.get(2)?,
                to_mode: row.get(3)?,
                injected_prompt: row.get(4)?,
                created_at_ms: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 同事务写入会话 `sandbox_mode`，并在已有消息且模式真实变化时记录 `agent_permission_switch` 并清空旧会话规则。
    pub fn record_permission_switch(
        &self,
        workspace: &Path,
        commit: &PermissionSwitchCommit,
    ) -> Result<Option<PermissionSwitchRecord>, DbError> {
        let workspace = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let transaction = self.conn.unchecked_transaction()?;
        let owned: Option<i64> = transaction
            .query_row(
                "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
                params![commit.session_id, workspace],
                |row| row.get(0),
            )
            .optional()?;
        if owned.is_none() {
            return Err(DbError::SessionMissing);
        }
        transaction.execute(
            "UPDATE agent_session SET sandbox_mode = ?1 WHERE external_id = ?2 AND workspace_id = ?3",
            params![commit.to_mode, commit.session_id, workspace],
        )?;
        let changed = commit.from_mode != commit.to_mode;
        if changed {
            transaction.execute(
                "DELETE FROM agent_session_sandbox_rule WHERE session_id = ?1",
                params![commit.session_id],
            )?;
        }
        let message_count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM agent_message WHERE session_id = ?1",
            params![commit.session_id],
            |row| row.get(0),
        )?;
        if !changed || message_count == 0 {
            transaction.commit()?;
            return Ok(None);
        }
        let sequence = super::chat::next_session_sequence(&transaction, &commit.session_id)?;
        let id = new_external_id("psw");
        let created_at_ms = now_ms();
        transaction.execute(
            r#"INSERT INTO agent_permission_switch
                   (external_id, session_id, sequence, from_mode, to_mode, injected_prompt, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                id,
                commit.session_id,
                sequence,
                commit.from_mode,
                commit.to_mode,
                commit.injected_prompt,
                created_at_ms,
            ],
        )?;
        transaction.commit()?;
        Ok(Some(PermissionSwitchRecord {
            id,
            sequence,
            from_mode: commit.from_mode.clone(),
            to_mode: commit.to_mode.clone(),
            injected_prompt: commit.injected_prompt.clone(),
            created_at_ms,
        }))
    }
}

/// 分叉会话时复制边界之前的权限切换记录。
pub(super) fn copy_permission_switches_for_fork(
    conn: &Connection,
    source_session: &str,
    new_session_id: &str,
    start_seq: i64,
    boundary: i64,
) -> Result<(), DbError> {
    let mut statement = conn.prepare(
        r#"SELECT sequence, from_mode, to_mode, injected_prompt, created_at_ms
           FROM agent_permission_switch
           WHERE session_id = ?1 AND sequence >= ?2 AND sequence < ?3
           ORDER BY sequence"#,
    )?;
    let rows = statement
        .query_map(params![source_session, start_seq, boundary], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (sequence, from_mode, to_mode, injected_prompt, created_at_ms) in rows {
        conn.execute(
            r#"INSERT INTO agent_permission_switch
                   (external_id, session_id, sequence, from_mode, to_mode, injected_prompt, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
            params![
                new_external_id("psw"),
                new_session_id,
                sequence,
                from_mode,
                to_mode,
                injected_prompt,
                created_at_ms,
            ],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

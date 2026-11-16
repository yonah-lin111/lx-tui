//! 会话级沙箱覆盖与审批规则（v30）。

use std::path::Path;

use rusqlite::{OptionalExtension, params};

use super::{Database, DbError, now_ms};

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
            let (kind, tokens) = row?;
            let Some(kind) = SandboxRuleKind::parse(&kind) else {
                continue;
            };
            let tokens = tokens
                .split('\u{0}')
                .filter(|token| !token.is_empty())
                .map(str::to_string)
                .collect();
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
        let tokens = rule.tokens.join("\u{0}");
        self.conn.execute(
            "INSERT OR IGNORE INTO agent_session_sandbox_rule (session_id, kind, prefix_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4)",
            params![session_id, rule.kind.as_str(), tokens, now_ms()],
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
}

#[cfg(test)]
mod tests;

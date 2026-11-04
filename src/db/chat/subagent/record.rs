//! 子 agent 记录类型与行读取：元数据行、逐轮报告行、spawn 回执与报告载荷。

use rusqlite::{Connection, OptionalExtension, params};

use crate::chat::agent::{AgentRole, BaseRole};
use crate::db::{DbError, Result};

/// 一个 subagent 的元数据行（列表用；不含消息流）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRecord {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 可选唯一昵称（如 `"Turing"`；旧记录为 None）。
    pub nickname: Option<String>,
    /// 子会话 external id。
    pub child_session_id: String,
    /// 父会话 external id。
    pub parent_session_id: String,
    /// spawn 调用所在的主 agent assistant 消息 id。
    pub spawn_message_id: Option<String>,
    /// 角色。
    pub role: AgentRole,
    /// 子轮冻结的 provider；旧记录为空串。
    pub model_provider: String,
    /// 子轮冻结的 model id；旧记录为空串。
    pub model_id: String,
    /// 子轮冻结的 variant（如 `high`）。
    pub model_variant: Option<String>,
    /// `pending` / `running` / `completed` / `failed` / `cancelled`。
    pub status: String,
    /// 面向用户的短标题；v17 之前的旧记录为空。
    pub title: String,
    /// 创建时刻（UTC 毫秒）。
    pub created_at_ms: i64,
    /// 完成时刻（未结束为 None）。
    pub completed_at_ms: Option<i64>,
    /// 最近一条报告 JSON（未生成或已关闭为 None）。
    pub report_json: Option<String>,
    /// 报告是否已被模型全部领取。
    pub report_delivered: bool,
}

/// 一条逐轮报告记录（`agent_subagent_report`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentReportRecord {
    /// 稳定报告 ID（`rep-...`）。
    pub report_id: String,
    /// 所属子 agent ID。
    pub agent_id: String,
    /// 对应子轮的用户消息 ID。
    pub child_user_message_id: Option<String>,
    /// `completed` / `failed`。
    pub status: String,
    /// 序列化后的 `SubagentReportView` JSON。
    pub report_json: String,
    /// 完成时刻（UTC 毫秒）。
    pub completed_at_ms: i64,
    /// 投递时刻（未投递为 None）。
    pub delivered_at_ms: Option<i64>,
}

/// spawn 的提交结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentCommitted {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 子会话 external id。
    pub child_session_id: String,
    /// 子 assistant 消息 id（工具归属锚点）。
    pub child_assistant_id: String,
    /// 子 user 消息 id（文件 mutation 归属）。
    pub child_user_message_id: String,
    /// 创建时刻（UTC 毫秒）。
    pub created_at_ms: i64,
}

/// 报告正文与尾巴（DB 在此组装成 `SubagentReportView` 的 JSON）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportPayload {
    /// 面向用户的短标题。
    pub title: String,
    /// 已按上限截断的正文。
    pub body: String,
    /// 去重后的改动文件数。
    pub files_changed: usize,
    /// 子 turn 耗时（毫秒）。
    pub duration_ms: Option<i64>,
}

/// 读取一行 `SubagentRecord` 与其原始角色字符串。
///
/// 列顺序固定为：`external_id, child_session_id, parent_session_id, spawn_message_id,
/// role, status, title, created_at_ms, completed_at_ms, report_json, report_delivered,
/// nickname, agent_path, model_provider, model_id, variant`。
pub(in crate::db::chat) fn read_subagent_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(SubagentRecord, Option<String>)> {
    let role: Option<String> = row.get(4)?;
    let nickname: Option<String> = row.get(11)?;
    let agent_path: Option<String> = row.get(12)?;
    let base_role = agent_path.as_deref().and_then(BaseRole::parse);
    Ok((
        SubagentRecord {
            agent_id: row.get(0)?,
            nickname: nickname
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            child_session_id: row.get(1)?,
            parent_session_id: row.get(2)?,
            spawn_message_id: row.get(3)?,
            role: role
                .as_deref()
                .and_then(|r| AgentRole::from_name_and_base(r, base_role))
                .unwrap_or(AgentRole::Explorer),
            model_provider: row
                .get::<_, Option<String>>(13)?
                .unwrap_or_default()
                .trim()
                .to_string(),
            model_id: row
                .get::<_, Option<String>>(14)?
                .unwrap_or_default()
                .trim()
                .to_string(),
            model_variant: row
                .get::<_, Option<String>>(15)?
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            status: row.get(5)?,
            title: row.get(6)?,
            created_at_ms: row.get(7)?,
            completed_at_ms: row.get(8)?,
            report_json: row.get(9)?,
            report_delivered: row.get::<_, i64>(10)? != 0,
        },
        role,
    ))
}

/// 同步 `agent_subagent.report_delivered`：当且仅当该 agent 已无未投递报告时置 1。
pub(super) fn sync_agent_report_delivered(
    transaction: &Connection,
    agent_id: &str,
    parent_session: &str,
    now: i64,
) -> Result<()> {
    transaction.execute(
        r#"UPDATE agent_subagent
           SET report_delivered = CASE
                   WHEN EXISTS (
                       SELECT 1 FROM agent_subagent_report r
                       WHERE r.agent_id = ?1 AND r.delivered_at_ms IS NULL
                   ) THEN 0
                   ELSE 1
               END,
               updated_at_ms = ?2
           WHERE external_id = ?1 AND parent_session_id = ?3"#,
        params![agent_id, now, parent_session],
    )?;
    Ok(())
}

/// 将直接写 `agent_subagent.report_json`（如旧测试/旧记录）的未投递报告同步到 `agent_subagent_report`。
pub(super) fn sync_legacy_reports_for_parent(
    conn: &Connection,
    parent_session: &str,
    workspace_external: &str,
) -> Result<()> {
    conn.execute(
        r#"INSERT OR IGNORE INTO agent_subagent_report
               (external_id, agent_id, child_user_message_id, status, report_json,
                completed_at_ms, delivered_at_ms)
           SELECT
               'rep-legacy-' || a.external_id,
               a.external_id,
               (SELECT m.external_id FROM agent_message m
                WHERE m.session_id = a.child_session_id AND m.role = 'user'
                ORDER BY m.sequence DESC LIMIT 1),
               CASE WHEN a.status = 'failed' THEN 'failed' ELSE 'completed' END,
               a.report_json,
               COALESCE(a.completed_at_ms, a.updated_at_ms, a.created_at_ms),
               CASE WHEN a.report_delivered <> 0
                    THEN COALESCE(a.completed_at_ms, a.updated_at_ms, a.created_at_ms)
                    ELSE NULL
               END
           FROM agent_subagent a
           JOIN agent_session p ON p.external_id = a.parent_session_id
           WHERE a.parent_session_id = ?1 AND p.workspace_id = ?2
             AND a.report_json IS NOT NULL AND a.report_json <> ''
             AND NOT EXISTS (
                 SELECT 1 FROM agent_subagent_report r WHERE r.agent_id = a.external_id
             )"#,
        params![parent_session, workspace_external],
    )?;
    Ok(())
}

/// 会话必须属于该工作区，否则按不存在处理。
pub(in crate::db::chat) fn ensure_session_owned(
    transaction: &Connection,
    session_id: &str,
    workspace_external: &str,
) -> Result<()> {
    let owned: Option<i64> = transaction
        .query_row(
            "SELECT id FROM agent_session WHERE external_id = ?1 AND workspace_id = ?2",
            params![session_id, workspace_external],
            |row| row.get(0),
        )
        .optional()?;
    owned.map(|_| ()).ok_or(DbError::SessionMissing)
}

//! 全局 prompt 历史的持久化：去重置顶写入、按最近使用载入与上限裁剪。
//!
//! 表结构见 `migration::SCHEMA_V10`：`prompt_history(text UNIQUE, updated_at_ms)`。
//! 历史是全局的（跨窗格、跨工作区、跨会话），因此这里不接收 workspace 或 session 参数。
//! 所有写入都在单事务内完成，调用方只需判断成功/失败。

use rusqlite::{OptionalExtension, params};

use super::{Database, DbError, Result, now_ms};

/// 单条历史文本字节上限；与 `app::state::PROMPT_HISTORY_MAX_BYTES` 保持一致。
///
/// DB 层独立校验：即使调用方漏检，也不让超长内容进入历史表。
const MAX_HISTORY_TEXT_BYTES: usize = 32 * 1024;

impl Database {
    /// 载入最近 `limit` 条历史，返回**旧 → 新**顺序（末尾为最近使用）。
    ///
    /// `limit` 为 0 时返回空。同毫秒按 `id` 决胜，保证顺序稳定。
    pub fn load_prompt_history(&self, limit: usize) -> Result<Vec<String>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut statement = self.conn.prepare(
            r#"SELECT text FROM prompt_history
               ORDER BY updated_at_ms DESC, id DESC
               LIMIT ?1"#,
        )?;
        let rows = statement.query_map(params![limit as i64], |row| row.get::<_, String>(0))?;
        let mut entries = Vec::with_capacity(limit);
        for row in rows {
            entries.push(row?);
        }
        // 查询按"最近在前"取出，调用方需要"末尾最新"，这里反转一次。
        entries.reverse();
        Ok(entries)
    }

    /// 记录一条已发送 prompt：去重置顶并按 `limit` 裁剪最旧条目。
    ///
    /// 已存在相同文本时只更新 `updated_at_ms`（最近使用置顶），否则插入新行；
    /// 随后删除超出 `limit` 的行。空内容或超长内容返回错误，不写库。
    pub fn record_prompt_history(&self, text: &str, limit: usize) -> Result<()> {
        if text.trim().is_empty() {
            return Err(DbError::InvalidDraft);
        }
        if text.len() > MAX_HISTORY_TEXT_BYTES {
            return Err(DbError::MessageTooLarge);
        }
        if limit == 0 {
            return Ok(());
        }
        let transaction = self.conn.unchecked_transaction()?;
        // 先删同文本旧行再插入：`id` 单调递增，可作同毫秒内的排序决胜键
        // （`updated_at_ms` 只有毫秒精度，快速连续记录会撞同一时刻）。
        transaction.execute("DELETE FROM prompt_history WHERE text = ?1", params![text])?;
        transaction.execute(
            "INSERT INTO prompt_history (text, updated_at_ms) VALUES (?1, ?2)",
            params![text, now_ms()],
        )?;
        // 裁剪：保留最近使用的前 limit 条，删除其余（含并列同毫秒的旧行）。
        transaction.execute(
            r#"DELETE FROM prompt_history WHERE id NOT IN (
                   SELECT id FROM prompt_history
                   ORDER BY updated_at_ms DESC, id DESC
                   LIMIT ?1)"#,
            params![limit as i64],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 历史条目总数（测试与诊断用）。
    pub fn prompt_history_count(&self) -> Result<usize> {
        let count: Option<i64> = self
            .conn
            .query_row("SELECT COUNT(*) FROM prompt_history", [], |row| row.get(0))
            .optional()?;
        Ok(count.unwrap_or(0) as usize)
    }
}

#[cfg(test)]
mod tests;

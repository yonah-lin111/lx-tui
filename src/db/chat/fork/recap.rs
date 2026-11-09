//! fork 时复制源会话的回顾行：只复制已随 fork 复制过去的消息。

use std::collections::HashMap;

use rusqlite::{Connection, params};

use crate::db::{Result, new_external_id};

/// 复制源会话的回顾行；只复制 `message_ids` 里已随 fork 复制过去的消息。
pub(super) fn copy_recaps(
    conn: &Connection,
    new_session_id: &str,
    message_ids: &HashMap<String, String>,
    source_session: &str,
) -> Result<()> {
    let mut statement = conn.prepare(
        r#"SELECT message_id, summary_text, next_action, model_provider, model_id,
                  variant, model_label, end_turn, end_sequence, created_at_ms
           FROM agent_message_recap
           WHERE session_id = ?1"#,
    )?;
    let rows = statement
        .query_map(params![source_session], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, i64>(9)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for (old_message_id, summary, next_action, provider, model, variant, label, turn, seq, at) in
        rows
    {
        let Some(new_message_id) = message_ids.get(&old_message_id) else {
            continue;
        };
        conn.execute(
            r#"INSERT INTO agent_message_recap
                   (external_id, session_id, message_id, summary_text, next_action,
                    model_provider, model_id, variant, model_label, end_turn,
                    end_sequence, created_at_ms)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            params![
                new_external_id("recap"),
                new_session_id,
                new_message_id,
                summary,
                next_action,
                provider,
                model,
                variant,
                label,
                turn,
                seq,
                at,
            ],
        )?;
    }
    Ok(())
}

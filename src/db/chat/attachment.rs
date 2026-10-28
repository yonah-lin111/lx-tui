//! 聊天消息附件（可见引用与图片副本）的数据库存储与查询。

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use std::collections::HashMap;

use crate::db::{DbError, Result, new_external_id};

/// 附件类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageAttachmentKind {
    File,
    Folder,
    Image,
}

impl MessageAttachmentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
            Self::Image => "image",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "folder" => Some(Self::Folder),
            "image" => Some(Self::Image),
            _ => None,
        }
    }
}

/// 消息附件元数据（不含图片 BLOB）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageAttachmentMeta {
    pub id: String,
    pub position: i64,
    pub kind: MessageAttachmentKind,
    pub label: String,
    pub marker_start: Option<usize>,
    pub marker_end: Option<usize>,
    pub source_path: Option<String>,
    pub mime: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// 发送/写入时的附件输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageAttachmentInput {
    pub kind: MessageAttachmentKind,
    pub label: String,
    pub marker_start: Option<usize>,
    pub marker_end: Option<usize>,
    pub source_path: Option<String>,
    pub mime: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub image_bytes: Option<Vec<u8>>,
}

/// 在当前事务中写入某条消息的一组附件，并返回其元数据列表。
pub fn insert_attachments(
    transaction: &Transaction<'_>,
    message_id: &str,
    attachments: &[MessageAttachmentInput],
) -> Result<Vec<MessageAttachmentMeta>> {
    let mut stmt = transaction.prepare_cached(
        r#"INSERT INTO agent_message_attachment
               (external_id, message_id, position, kind, label, marker_start, marker_end,
                source_path, mime, width, height, image_bytes)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
    )?;
    let mut metas = Vec::with_capacity(attachments.len());
    for (position, attachment) in attachments.iter().enumerate() {
        let external_id = new_external_id("att");
        stmt.execute(params![
            external_id,
            message_id,
            position as i64,
            attachment.kind.as_str(),
            attachment.label,
            attachment.marker_start.map(|s| s as i64),
            attachment.marker_end.map(|e| e as i64),
            attachment.source_path,
            attachment.mime,
            attachment.width.map(|w| w as i64),
            attachment.height.map(|h| h as i64),
            attachment.image_bytes,
        ])?;
        metas.push(MessageAttachmentMeta {
            id: external_id,
            position: position as i64,
            kind: attachment.kind,
            label: attachment.label.clone(),
            marker_start: attachment.marker_start,
            marker_end: attachment.marker_end,
            source_path: attachment.source_path.clone(),
            mime: attachment.mime.clone(),
            width: attachment.width,
            height: attachment.height,
        });
    }
    Ok(metas)
}

/// 按会话批量加载附件元数据（不查 BLOB）。
pub fn load_attachments_by_session(
    conn: &Connection,
    session_id: &str,
) -> Result<HashMap<String, Vec<MessageAttachmentMeta>>> {
    let mut statement = conn.prepare_cached(
        r#"SELECT external_id, message_id, position, kind, label, marker_start, marker_end,
                  source_path, mime, width, height
           FROM agent_message_attachment
           WHERE message_id IN (
               SELECT external_id FROM agent_message WHERE session_id = ?1
           )
           ORDER BY message_id, position"#,
    )?;
    let mut by_message: HashMap<String, Vec<MessageAttachmentMeta>> = HashMap::new();
    let rows = statement.query_map(params![session_id], |row| {
        let kind_str: String = row.get(3)?;
        let kind = MessageAttachmentKind::parse(&kind_str).unwrap_or(MessageAttachmentKind::File);
        let marker_start: Option<i64> = row.get(5)?;
        let marker_end: Option<i64> = row.get(6)?;
        let width: Option<i64> = row.get(9)?;
        let height: Option<i64> = row.get(10)?;
        Ok((
            row.get::<_, String>(1)?,
            MessageAttachmentMeta {
                id: row.get(0)?,
                position: row.get(2)?,
                kind,
                label: row.get(4)?,
                marker_start: marker_start.and_then(|v| usize::try_from(v).ok()),
                marker_end: marker_end.and_then(|v| usize::try_from(v).ok()),
                source_path: row.get(7)?,
                mime: row.get(8)?,
                width: width.and_then(|v| u32::try_from(v).ok()),
                height: height.and_then(|v| u32::try_from(v).ok()),
            },
        ))
    })?;
    for row in rows {
        let (message_id, meta) = row?;
        by_message.entry(message_id).or_default().push(meta);
    }
    Ok(by_message)
}

/// 按会话和附件 ID 加载图片原始 BLOB（仅返回本会话所属且存在的附件字节）。
pub fn load_image_payloads(
    conn: &Connection,
    workspace_external: &str,
    session_id: &str,
    attachment_ids: &[&str],
) -> Result<HashMap<String, Vec<u8>>> {
    if attachment_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut map = HashMap::new();
    let mut stmt = conn.prepare_cached(
        r#"SELECT a.external_id, a.image_bytes
           FROM agent_message_attachment a
           JOIN agent_message m ON a.message_id = m.external_id
           JOIN agent_session s ON m.session_id = s.external_id
           WHERE s.external_id = ?1 AND s.workspace_id = ?2 AND a.external_id = ?3"#,
    )?;
    for id in attachment_ids {
        let row: Option<(String, Option<Vec<u8>>)> = stmt
            .query_row(params![session_id, workspace_external, id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        let Some((att_id, bytes)) = row else {
            return Err(DbError::AttachmentMissing);
        };
        let Some(bytes) = bytes else {
            return Err(DbError::AttachmentMissing);
        };
        map.insert(att_id, bytes);
    }
    Ok(map)
}

#[cfg(test)]
mod tests;

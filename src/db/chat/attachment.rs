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

/// 发送/写入时的附件输入（图片本体存储于本地文件系统，数据库仅记录元数据与路径）。
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
}

/// 在当前事务中写入某条消息的一组附件（图片自动归档至 `~/.lx-t[-dev]/images/<session_id>/`），并返回其元数据列表。
pub fn insert_attachments(
    transaction: &Transaction<'_>,
    session_id: &str,
    message_id: &str,
    attachments: &[MessageAttachmentInput],
) -> Result<Vec<MessageAttachmentMeta>> {
    let mut stmt = transaction.prepare_cached(
        r#"INSERT INTO agent_message_attachment
               (external_id, message_id, position, kind, label, marker_start, marker_end,
                source_path, mime, width, height)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
    )?;
    let mut metas = Vec::with_capacity(attachments.len());
    for (position, attachment) in attachments.iter().enumerate() {
        let external_id = new_external_id("att");
        let persisted_source_path = if attachment.kind == MessageAttachmentKind::Image
            && let Some(ref src) = attachment.source_path
        {
            let src_path = std::path::Path::new(src);
            if src_path.is_file() {
                match crate::platform::archive_session_image(session_id, src_path) {
                    Ok(archived) => Some(archived.to_string_lossy().into_owned()),
                    Err(_) => Some(src.clone()),
                }
            } else {
                Some(src.clone())
            }
        } else {
            attachment.source_path.clone()
        };
        stmt.execute(params![
            external_id,
            message_id,
            position as i64,
            attachment.kind.as_str(),
            attachment.label,
            attachment.marker_start.map(|s| s as i64),
            attachment.marker_end.map(|e| e as i64),
            persisted_source_path,
            attachment.mime,
            attachment.width.map(|w| w as i64),
            attachment.height.map(|h| h as i64),
        ])?;
        metas.push(MessageAttachmentMeta {
            id: external_id,
            position: position as i64,
            kind: attachment.kind,
            label: attachment.label.clone(),
            marker_start: attachment.marker_start,
            marker_end: attachment.marker_end,
            source_path: persisted_source_path,
            mime: attachment.mime.clone(),
            width: attachment.width,
            height: attachment.height,
        });
    }
    Ok(metas)
}

/// 按单条消息 ID 加载其附件元数据（供 Rewind 删除消息前获取受管图片路径）。
pub fn load_attachments_for_message(
    conn: &Connection,
    message_id: &str,
) -> Result<Vec<MessageAttachmentMeta>> {
    let mut statement = conn.prepare_cached(
        r#"SELECT external_id, position, kind, label, marker_start, marker_end,
                  source_path, mime, width, height
           FROM agent_message_attachment
           WHERE message_id = ?1
           ORDER BY position"#,
    )?;
    let rows = statement.query_map(params![message_id], |row| {
        let kind_str: String = row.get(2)?;
        let kind = MessageAttachmentKind::parse(&kind_str).unwrap_or(MessageAttachmentKind::File);
        let marker_start: Option<i64> = row.get(4)?;
        let marker_end: Option<i64> = row.get(5)?;
        let width: Option<i64> = row.get(8)?;
        let height: Option<i64> = row.get(9)?;
        Ok(MessageAttachmentMeta {
            id: row.get(0)?,
            position: row.get(1)?,
            kind,
            label: row.get(3)?,
            marker_start: marker_start.and_then(|v| usize::try_from(v).ok()),
            marker_end: marker_end.and_then(|v| usize::try_from(v).ok()),
            source_path: row.get(6)?,
            mime: row.get(7)?,
            width: width.and_then(|v| u32::try_from(v).ok()),
            height: height.and_then(|v| u32::try_from(v).ok()),
        })
    })?;
    let mut metas = Vec::new();
    for row in rows {
        metas.push(row?);
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

/// 按会话和附件 ID 从本地磁盘路径加载并编码图片数据（仅返回本会话所属且存在的图片字节）。
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
        r#"SELECT a.external_id, a.source_path
           FROM agent_message_attachment a
           JOIN agent_message m ON a.message_id = m.external_id
           JOIN agent_session s ON m.session_id = s.external_id
           WHERE s.external_id = ?1 AND s.workspace_id = ?2 AND a.external_id = ?3"#,
    )?;
    for id in attachment_ids {
        let row: Option<(String, Option<String>)> = stmt
            .query_row(params![session_id, workspace_external, id], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        let Some((att_id, Some(source_path))) = row else {
            return Err(DbError::AttachmentMissing);
        };
        let processed =
            crate::platform::load_and_process_image_file(std::path::Path::new(&source_path))
                .map_err(|_| DbError::AttachmentMissing)?;
        map.insert(att_id, processed.bytes);
    }
    Ok(map)
}

/// 查询数据库中所有仍被消息附件引用的本地文件路径集合（供启动时剪贴板目录 GC 使用）。
pub fn load_referenced_attachment_paths(
    conn: &Connection,
) -> Result<std::collections::HashSet<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT source_path FROM agent_message_attachment WHERE source_path IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut set = std::collections::HashSet::new();
    for row in rows {
        set.insert(row?);
    }
    Ok(set)
}

impl crate::db::Database {
    /// 读取单条消息的全部附件元数据（供 Rewind 删除消息前获取受管图片路径）。
    pub fn load_message_attachments(&self, message_id: &str) -> Result<Vec<MessageAttachmentMeta>> {
        load_attachments_for_message(&self.conn, message_id)
    }
}

#[cfg(test)]
mod tests;

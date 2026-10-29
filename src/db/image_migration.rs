//! 会话图片目录搬迁：`<data>/images/<sid>/<file>` → `<data>/session/<sid>/image/<file>`。
//!
//! 搬迁挂在「旧目录仍存在 / 数据库仍引用旧路径」上，而不是 `user_version`：
//! 文件 rename 与数据库改写无法同事务，进程崩溃后必须在下次启动补齐。
//! 每个文件独立处理，失败只保留旧路径并记日志，不影响其他文件。

use std::path::{Path, PathBuf};

use crate::platform::sessions_root_dir;

use super::{Database, Result};

/// 执行一次补偿 + 搬迁，返回成功搬移/修正的行数。
pub fn migrate_legacy_session_images(db: &Database) -> usize {
    let Ok(sessions_root) = sessions_root_dir() else {
        return 0;
    };
    let Some(data_root) = sessions_root.parent() else {
        return 0;
    };
    migrate_legacy_session_images_in(db, data_root)
}

/// 指定数据根执行搬迁（测试用）。
pub fn migrate_legacy_session_images_in(db: &Database, data_root: &Path) -> usize {
    let sessions_root = data_root.join("session");
    let legacy_root = data_root.join("images");
    let Ok(paths) = db.managed_image_paths() else {
        return 0;
    };
    let legacy_paths: Vec<&String> = paths
        .iter()
        .filter(|path| Path::new(path).starts_with(&legacy_root))
        .collect();
    if legacy_paths.is_empty() && !legacy_root.exists() {
        return 0;
    }

    let mut changed = 0;
    // 1. 崩溃窗口补偿：旧路径文件已被搬走、数据库仍指向旧路径。
    for old in &legacy_paths {
        let old = Path::new(old.as_str());
        if old.exists() {
            continue;
        }
        let Some(new) = mapped_path(&legacy_root, &sessions_root, old) else {
            continue;
        };
        if new.is_file()
            && db
                .rewrite_managed_image_path(&old.to_string_lossy(), &new.to_string_lossy())
                .is_ok()
        {
            changed += 1;
        }
    }

    // 2. 搬移仍在旧目录的文件，成功后才改写数据库。
    if let Ok(entries) = std::fs::read_dir(&legacy_root) {
        for session in entries.flatten() {
            let session_path = session.path();
            if !session_path.is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(&session_path) else {
                continue;
            };
            for file in files.flatten() {
                let source = file.path();
                if !source.is_file() {
                    continue;
                }
                let Some(target) = mapped_path(&legacy_root, &sessions_root, &source) else {
                    continue;
                };
                let Some(parent) = target.parent() else {
                    continue;
                };
                if std::fs::create_dir_all(parent).is_err() {
                    continue;
                }
                let _ = crate::platform::restrict_dir_permissions(parent);
                match std::fs::rename(&source, &target) {
                    Ok(()) => {
                        if db
                            .rewrite_managed_image_path(
                                &source.to_string_lossy(),
                                &target.to_string_lossy(),
                            )
                            .is_ok()
                        {
                            changed += 1;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "lx::db",
                            %error,
                            path = %source.display(),
                            "skip legacy image migration"
                        );
                    }
                }
            }
            let _ = std::fs::remove_dir(&session_path);
        }
        let _ = std::fs::remove_dir(&legacy_root);
    }
    if changed > 0 {
        tracing::info!(target: "lx::db", changed, "migrated session images");
    }
    changed
}

/// 旧路径 → 新路径；不在旧根内返回 None。
fn mapped_path(legacy_root: &Path, sessions_root: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(legacy_root).ok()?;
    let mut components = relative.components();
    let session = components.next()?.as_os_str();
    let file = components.next()?.as_os_str();
    if components.next().is_some() {
        return None;
    }
    Some(sessions_root.join(session).join("image").join(file))
}

impl Database {
    /// 全部受管图片路径（附件图片 + 子 agent 输入图片）。
    fn managed_image_paths(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare_cached(
            r#"SELECT DISTINCT source_path FROM agent_message_attachment
               WHERE kind = 'image' AND source_path IS NOT NULL
               UNION
               SELECT DISTINCT archive_path FROM agent_subagent_input_item
               WHERE archive_path IS NOT NULL"#,
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut paths = Vec::new();
        for row in rows {
            paths.push(row?);
        }
        Ok(paths)
    }

    /// 把仍指向旧路径的行改写到新路径。
    fn rewrite_managed_image_path(&self, old: &str, new: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE agent_message_attachment SET source_path = ?2
             WHERE kind = 'image' AND source_path = ?1",
            rusqlite::params![old, new],
        )?;
        self.conn.execute(
            "UPDATE agent_subagent_input_item SET archive_path = ?2 WHERE archive_path = ?1",
            rusqlite::params![old, new],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

//! 工作区表（`workspace`）读写：信任状态、历史记录、重命名与级联删除。

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, path_string};

/// 数据库中的工作区历史记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub name: String,
    pub path: PathBuf,
}

impl Database {
    /// 路径是否已信任（`is_trusted = 1`）。
    pub fn is_trusted(&self, path: &Path) -> Result<bool> {
        let trusted: i64 = self
            .conn
            .query_row(
                "SELECT is_trusted FROM workspace WHERE path = ?1",
                params![path_string(path)],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(trusted == 1)
    }

    /// 记录工作区信任：按 path upsert `is_trusted = 1`，已存在时保留原自定义名称。
    pub fn trust_workspace(&self, path: &Path, name: &str) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, 1, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO UPDATE SET
                   is_trusted = 1,
                   updated_at = datetime('now')"#,
            params![new_external_id("ws"), name, path_string(path)],
        )?;
        Ok(())
    }

    /// 确保工作区行存在（提示词/会话读写前置）；已存在时不覆盖名称、不修改信任位与打开时间。
    pub fn ensure_workspace(&self, path: &Path) -> Result<()> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_string(path));
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, 0, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO NOTHING"#,
            params![new_external_id("ws"), name, path_string(path)],
        )?;
        Ok(())
    }

    /// 记录工作区显式创建/打开：新路径写入 `default_name`（不隐式授予信任），
    /// 已有路径保留数据库中的名称与信任位并刷新 `updated_at`；
    /// 返回数据库中保存的名称（供打开时恢复自定义名称）。
    pub fn record_workspace_open(&self, path: &Path, default_name: &str) -> Result<String> {
        let path_str = path_string(path);
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, 0, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO UPDATE SET
                   updated_at = datetime('now')"#,
            params![new_external_id("ws"), default_name, path_str],
        )?;
        let saved_name: String = self.conn.query_row(
            "SELECT name FROM workspace WHERE path = ?1",
            params![path_str],
            |row| row.get(0),
        )?;
        Ok(saved_name)
    }

    /// 显式重命名工作区：按 path upsert 新名称。
    pub fn rename_workspace(&self, path: &Path, name: &str) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, 0, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO UPDATE SET
                   name = excluded.name"#,
            params![new_external_id("ws"), name, path_string(path)],
        )?;
        Ok(())
    }

    /// 按最近打开时间倒序返回全部工作区历史记录。
    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceRecord>> {
        let mut statement = self.conn.prepare(
            r#"SELECT name, path
               FROM workspace
               WHERE path IS NOT NULL AND path != ''
               ORDER BY updated_at DESC, id DESC"#,
        )?;
        let rows = statement.query_map([], |row| {
            let name: String = row.get(0)?;
            let raw_path: String = row.get(1)?;
            Ok(WorkspaceRecord {
                name,
                path: PathBuf::from(raw_path),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 删除指定路径的工作区行；外键 `ON DELETE CASCADE` 自动级联删除该工作区下的
    /// 文件夹、提示词、分页、引用目录与全部聊天会话及子表数据。
    pub fn delete_workspace(&self, path: &Path) -> Result<()> {
        self.conn.execute(
            "DELETE FROM workspace WHERE path = ?1",
            params![path_string(path)],
        )?;
        Ok(())
    }

    /// 工作区外部标识；未登记时 None。
    pub(crate) fn workspace_external_id(&self, path: &Path) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT external_id FROM workspace WHERE path = ?1",
                params![path_string(path)],
                |row| row.get(0),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::db::chat;

    #[test]
    fn trust_is_scoped_per_workspace_path() {
        let db = Database::open_in_memory().expect("in-memory db");
        let trusted = PathBuf::from("/tmp/lx-test-ws-trusted");
        let other = PathBuf::from("/tmp/lx-test-ws-other");
        assert!(!db.is_trusted(&trusted).expect("query ok"));
        db.trust_workspace(&trusted, "trusted").expect("trust ok");
        assert!(db.is_trusted(&trusted).expect("query ok"));
        assert!(!db.is_trusted(&other).expect("query ok"));
        db.trust_workspace(&trusted, "renamed")
            .expect("trust again ok");
        assert!(db.is_trusted(&trusted).expect("query ok"));
    }

    #[test]
    fn record_workspace_open_deduplicates_and_preserves_custom_name() {
        let db = Database::open_in_memory().expect("in-memory db");
        let alpha = PathBuf::from("/tmp/lx-ws-alpha");
        let beta = PathBuf::from("/tmp/lx-ws-beta");

        let saved_alpha = db
            .record_workspace_open(&alpha, "alpha")
            .expect("record alpha");
        assert_eq!(saved_alpha, "alpha");
        let saved_beta = db
            .record_workspace_open(&beta, "beta")
            .expect("record beta");
        assert_eq!(saved_beta, "beta");

        // 重命名 alpha 后，再次调用 ensure_workspace 或 record_workspace_open 不会覆盖自定义名称。
        db.rename_workspace(&alpha, "Custom Alpha")
            .expect("rename alpha");
        db.ensure_workspace(&alpha).expect("ensure alpha");
        let reopened_alpha = db
            .record_workspace_open(&alpha, "alpha")
            .expect("reopen alpha");
        assert_eq!(reopened_alpha, "Custom Alpha");

        let list = db.list_workspaces().expect("list workspaces");
        assert_eq!(list.len(), 2);
        let alpha_rec = list
            .iter()
            .find(|item| item.path == alpha)
            .expect("alpha present");
        assert_eq!(alpha_rec.name, "Custom Alpha");
    }

    #[test]
    fn delete_workspace_cascades_folders_prompts_referenced_and_sessions() {
        let db = Database::open_in_memory().expect("in-memory db");
        let ws = PathBuf::from("/tmp/lx-ws-cascade-del");
        db.record_workspace_open(&ws, "cascade").expect("record ws");

        let folder_id = db.create_folder(&ws, None, "folder-a").expect("folder");
        let _prompt_id = db
            .create_prompt(
                &ws,
                Some(folder_id),
                "prompt-a",
                &[("Page 1".into(), "body".into())],
            )
            .expect("prompt");
        db.add_referenced_folder(&ws, "/tmp/ref-a")
            .expect("ref folder");
        let _turn = db
            .begin_turn(
                &ws,
                None,
                "hello",
                &chat::ModelSnapshot {
                    provider: "openai".into(),
                    model: "gpt-4o".into(),
                    variant: None,
                },
            )
            .expect("begin chat turn");

        assert_eq!(db.list_folders(&ws).expect("folders").len(), 1);
        assert_eq!(db.list_prompts(&ws).expect("prompts").len(), 1);
        assert_eq!(db.list_referenced_folders(&ws).expect("refs").len(), 1);
        assert_eq!(
            db.list_chat_sessions(&ws, 0, 10).expect("sessions").len(),
            1
        );

        db.delete_workspace(&ws).expect("delete workspace");

        assert!(db.list_workspaces().expect("workspaces").is_empty());
        assert!(db.list_folders(&ws).expect("folders").is_empty());
        assert!(db.list_prompts(&ws).expect("prompts").is_empty());
        assert!(db.list_referenced_folders(&ws).expect("refs").is_empty());
        assert!(
            db.list_chat_sessions(&ws, 0, 10)
                .expect("sessions")
                .is_empty()
        );
    }
}

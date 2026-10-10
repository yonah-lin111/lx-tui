//! SQLite 持久层：工作区信任、提示词与文件夹的受控读写；唯一接触 rusqlite 的模块。
//!
//! 数据库路径按环境区分：生产 `~/.lx-t/db/lx.db`，开发 `~/.lx-t-dev/db/lx.db`；
//! `LX_TUI_ENV` / `LX_ENV` 环境变量优先（`dev` 走开发路径，其余取值视为生产），
//! 缺省回退 `cfg!(debug_assertions)` 门控；`LX_TUI_DB_PATH` 可显式覆盖（测试隔离用）。
//! Schema 迁移使用 SQLite 原生 `PRAGMA user_version` 版本控制。

pub mod service;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

/// 当前 Schema 版本。
const SCHEMA_VERSION: i64 = 1;

/// 首个版本的建表语句（workspace 语义，对齐 lx-agent 的 external_id + 级联外键设计）。
const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS workspace (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    "type" TEXT NOT NULL DEFAULT 'filesystem',
    path TEXT UNIQUE,
    is_trusted INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_folder (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    parent_folder_id TEXT,
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (parent_folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_folder_workspace_id ON workspace_folder(workspace_id);
CREATE INDEX IF NOT EXISTS idx_workspace_folder_parent ON workspace_folder(parent_folder_id);

CREATE TABLE IF NOT EXISTS workspace_prompt (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    folder_id TEXT,
    name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'in_progress', 'completed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_prompt_workspace_id ON workspace_prompt(workspace_id);
CREATE INDEX IF NOT EXISTS idx_workspace_prompt_folder_id ON workspace_prompt(folder_id);
"#;

/// 持久层错误；仅事件层记录日志，不直接面向用户。
#[derive(Debug)]
pub enum DbError {
    /// HOME 缺失，无法确定数据库路径。
    HomeMissing,
    /// 目录创建、路径探测等 IO 错误。
    Io(std::io::Error),
    /// SQLite 错误。
    Sqlite(rusqlite::Error),
    /// 目标工作区在库中不存在（调用方应先信任/登记）。
    WorkspaceMissing,
    /// 目标文件夹不存在。
    FolderMissing,
    /// 文件夹嵌套超过 2 层约束。
    DepthExceeded,
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::WorkspaceMissing => write!(formatter, "workspace not registered"),
            Self::FolderMissing => write!(formatter, "folder not found"),
            Self::DepthExceeded => write!(formatter, "folder nesting depth exceeded"),
        }
    }
}

impl std::error::Error for DbError {}

impl From<rusqlite::Error> for DbError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<std::io::Error> for DbError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// 持久层结果。
pub type Result<T> = std::result::Result<T, DbError>;

/// 文件夹记录：`parent_id` 为 integer id（TEXT 外键在查询中联查解析）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRecord {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
}

/// 提示词记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRecord {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub content: String,
    /// 状态稳定值：`todo` / `in_progress` / `completed`。
    pub status: String,
}

/// SQLite 连接包装：打开、迁移与领域读写。
pub struct Database {
    conn: Connection,
}

impl Database {
    /// 打开（必要时创建）数据库并完成迁移。
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        // 多实例并发（多个 lx 进程/测试）时等待短暂锁，而不是立刻失败。
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        let database = Self { conn };
        database.migrate()?;
        Ok(database)
    }

    /// 按环境探测路径打开默认数据库；`LX_TUI_DB_PATH` 显式覆盖优先（测试隔离用）。
    pub fn open_default() -> Result<Self> {
        if let Some(overridden) = std::env::var_os("LX_TUI_DB_PATH").filter(|path| !path.is_empty())
        {
            return Self::open(Path::new(&overridden));
        }
        Self::open(&database_path(is_dev_environment())?)
    }

    /// 迁移到当前 Schema 版本；幂等。
    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < SCHEMA_VERSION {
            self.conn.execute_batch(SCHEMA_V1)?;
            self.conn
                .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"))?;
        }
        Ok(())
    }

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

    /// 记录工作区信任：按 path upsert `is_trusted = 1`。
    pub fn trust_workspace(&self, path: &Path, name: &str) -> Result<()> {
        self.ensure_workspace_row(path, name, true)
    }

    /// 确保工作区行存在（提示词读写前置）；已存在时不修改信任位。
    pub fn ensure_workspace(&self, path: &Path) -> Result<()> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path_string(path));
        self.ensure_workspace_row(path, &name, false)
    }

    /// 工作区行 upsert；`trust` 为真时同时提升信任位。
    fn ensure_workspace_row(&self, path: &Path, name: &str, trust: bool) -> Result<()> {
        let trusted = i64::from(trust);
        self.conn.execute(
            r#"INSERT INTO workspace (external_id, name, "type", path, is_trusted, created_at, updated_at)
               VALUES (?1, ?2, 'filesystem', ?3, ?4, datetime('now'), datetime('now'))
               ON CONFLICT(path) DO UPDATE SET
                   name = excluded.name,
                   is_trusted = MAX(workspace.is_trusted, excluded.is_trusted),
                   updated_at = datetime('now')"#,
            params![new_external_id("ws"), name, path_string(path), trusted],
        )?;
        Ok(())
    }

    /// 工作区外部标识；未登记时 None。
    fn workspace_external_id(&self, path: &Path) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT external_id FROM workspace WHERE path = ?1",
                params![path_string(path)],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// 工作区内全部文件夹（按创建顺序）。
    pub fn list_folders(&self, workspace: &Path) -> Result<Vec<FolderRecord>> {
        let Some(external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT f.id, p.id, f.name
               FROM workspace_folder f
               LEFT JOIN workspace_folder p ON f.parent_folder_id = p.external_id
               WHERE f.workspace_id = ?1
               ORDER BY f.id"#,
        )?;
        let rows = statement.query_map(params![external], |row| {
            Ok(FolderRecord {
                id: row.get(0)?,
                parent_id: row.get(1)?,
                name: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 工作区内全部提示词（按创建顺序）。
    pub fn list_prompts(&self, workspace: &Path) -> Result<Vec<PromptRecord>> {
        let Some(external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT p.id, f.id, p.name, p.content, p.status
               FROM workspace_prompt p
               LEFT JOIN workspace_folder f ON p.folder_id = f.external_id
               WHERE p.workspace_id = ?1
               ORDER BY p.id"#,
        )?;
        let rows = statement.query_map(params![external], |row| {
            Ok(PromptRecord {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                name: row.get(2)?,
                content: row.get(3)?,
                status: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 新建文件夹；`parent_id` 必须为根层文件夹（目录嵌套最多 2 层）。
    pub fn create_folder(
        &self,
        workspace: &Path,
        parent_id: Option<i64>,
        name: &str,
    ) -> Result<i64> {
        self.ensure_workspace(workspace)?;
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let parent_external = match parent_id {
            Some(id) => {
                let row: Option<(String, Option<String>)> = self
                    .conn
                    .query_row(
                        "SELECT external_id, parent_folder_id FROM workspace_folder WHERE id = ?1 AND workspace_id = ?2",
                        params![id, workspace_external],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let Some((external, nested_parent)) = row else {
                    return Err(DbError::FolderMissing);
                };
                if nested_parent.is_some() {
                    return Err(DbError::DepthExceeded);
                }
                Some(external)
            }
            None => None,
        };
        self.conn.execute(
            r#"INSERT INTO workspace_folder (external_id, workspace_id, parent_folder_id, name, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))"#,
            params![new_external_id("folder"), workspace_external, parent_external, name],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 重命名文件夹。
    pub fn rename_folder(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_folder SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    /// 删除文件夹；子文件夹与其中提示词按外键级联删除。
    pub fn delete_folder(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM workspace_folder WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 新建提示词；`folder_id` 为 None 时挂在根层；`content` 为初始正文。
    pub fn create_prompt(
        &self,
        workspace: &Path,
        folder_id: Option<i64>,
        name: &str,
        content: &str,
    ) -> Result<i64> {
        self.ensure_workspace(workspace)?;
        let workspace_external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let folder_external: Option<String> = match folder_id {
            Some(id) => {
                let external: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT external_id FROM workspace_folder WHERE id = ?1 AND workspace_id = ?2",
                        params![id, workspace_external],
                        |row| row.get(0),
                    )
                    .optional()?;
                Some(external.ok_or(DbError::FolderMissing)?)
            }
            None => None,
        };
        self.conn.execute(
            r#"INSERT INTO workspace_prompt (external_id, workspace_id, folder_id, name, content, status, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, 'todo', datetime('now'), datetime('now'))"#,
            params![
                new_external_id("prompt"),
                workspace_external,
                folder_external,
                name,
                content
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 重命名提示词。
    pub fn rename_prompt(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_prompt SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    /// 更新提示词正文。
    pub fn update_prompt_content(&self, id: i64, content: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_prompt SET content = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![content, id],
        )?;
        Ok(())
    }

    /// 更新提示词状态（`todo` / `in_progress` / `completed`）。
    pub fn update_prompt_status(&self, id: i64, status: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_prompt SET status = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![status, id],
        )?;
        Ok(())
    }

    /// 删除提示词。
    pub fn delete_prompt(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM workspace_prompt WHERE id = ?1", params![id])?;
        Ok(())
    }
}

/// 数据库目录：生产 `~/.lx-t/db`，开发 `~/.lx-t-dev/db`。
pub fn database_dir(is_dev: bool) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or(DbError::HomeMissing)?;
    let root = if is_dev { ".lx-t-dev" } else { ".lx-t" };
    Ok(home.join(root).join("db"))
}

/// 数据库文件路径：`database_dir` 下的 `lx.db`。
pub fn database_path(is_dev: bool) -> Result<PathBuf> {
    Ok(database_dir(is_dev)?.join("lx.db"))
}

/// 环境判定：`LX_TUI_ENV` / `LX_ENV` 约定值优先，缺省回退 debug 门控。
pub fn is_dev_environment() -> bool {
    let explicit = std::env::var("LX_TUI_ENV")
        .or_else(|_| std::env::var("LX_ENV"))
        .ok();
    dev_environment(explicit.as_deref())
}

/// 纯环境判定：`dev` / `development` / `local` 视为开发；None 回退编译门控。
fn dev_environment(explicit: Option<&str>) -> bool {
    match explicit {
        Some(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "dev" | "development" | "local"
        ),
        None => cfg!(debug_assertions),
    }
}

/// 外部标识：时间戳纳秒 + 进程内自增序号，保证唯一且无需引入 uuid 依赖。
fn new_external_id(prefix: &str) -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{nanos:x}-{sequence:x}")
}

/// 路径稳定字符串形式；展示用 lossy 转换。
fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests;

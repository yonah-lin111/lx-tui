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

/// 当前 Schema 版本；版本号保留自管理截图资产的开发期结构，避免旧库被判为过新。
const SCHEMA_VERSION: i64 = 6;

/// 旧正文迁移为唯一第一页时使用的默认页名。
const FIRST_PAGE_NAME: &str = "Page 1";

/// 建表语句（workspace 语义，对齐 lx-agent 的 external_id + 级联外键设计）。
const SCHEMA: &str = r#"
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
    active_page_id INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_prompt_workspace_id ON workspace_prompt(workspace_id);
CREATE INDEX IF NOT EXISTS idx_workspace_prompt_folder_id ON workspace_prompt(folder_id);

CREATE TABLE IF NOT EXISTS workspace_referenced_folder (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    path TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    UNIQUE (workspace_id, path)
);

CREATE INDEX IF NOT EXISTS idx_workspace_referenced_folder_workspace_id
    ON workspace_referenced_folder(workspace_id);
"#;

/// v4 分页建表语句：页表以 `(prompt_id, position)` 唯一约束保证创建顺序；
/// `workspace_prompt.content` 保留为第一页镜像（兼容既有读取者）。
const SCHEMA_V4: &str = r#"
CREATE TABLE IF NOT EXISTS workspace_prompt_page (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    prompt_id INTEGER NOT NULL,
    position INTEGER NOT NULL,
    name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (prompt_id, position),
    FOREIGN KEY (prompt_id) REFERENCES workspace_prompt(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_workspace_prompt_page_prompt_id ON workspace_prompt_page(prompt_id);
"#;

/// v5 迁移的提示词表重建语句：移除 `status` 列（SQLite 不支持带 CHECK 的 DROP COLUMN）。
/// 重建必须在关闭外键的独立事务中执行（见 `rebuild_prompt_without_status`）。
const SCHEMA_V5_PROMPT: &str = r#"
CREATE TABLE workspace_prompt_v5 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    workspace_id TEXT NOT NULL,
    folder_id TEXT,
    name TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    active_page_id INTEGER,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(external_id) ON DELETE CASCADE,
    FOREIGN KEY (folder_id) REFERENCES workspace_folder(external_id) ON DELETE CASCADE
);

INSERT INTO workspace_prompt_v5 (id, external_id, workspace_id, folder_id, name, content, active_page_id, created_at, updated_at)
    SELECT id, external_id, workspace_id, folder_id, name, content, active_page_id, created_at, updated_at
    FROM workspace_prompt;

DROP TABLE workspace_prompt;

ALTER TABLE workspace_prompt_v5 RENAME TO workspace_prompt;

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
    /// 目标提示词页不存在。
    PageMissing,
    /// 文件夹嵌套超过 2 层约束。
    DepthExceeded,
    /// 迁移重建后外键校验发现悬空引用。
    ForeignKeyViolation,
    /// 数据库版本高于当前程序支持值（不能用旧代码覆盖新结构）。
    UnsupportedSchema(i64),
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::WorkspaceMissing => write!(formatter, "workspace not registered"),
            Self::FolderMissing => write!(formatter, "folder not found"),
            Self::PageMissing => write!(formatter, "prompt page not found"),
            Self::DepthExceeded => write!(formatter, "folder nesting depth exceeded"),
            Self::ForeignKeyViolation => write!(formatter, "foreign key violation after rebuild"),
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "database schema v{version} is newer than supported"
                )
            }
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

/// 提示词页记录；`position` 为创建顺序（删除留空、不重排）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptPageRecord {
    pub id: i64,
    pub position: i64,
    pub name: String,
    pub content: String,
}

/// 引用文件夹记录：项目级共享的磁盘目录引用；删除只移除记录、不动磁盘内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencedFolderRecord {
    pub id: i64,
    /// 规范化后的绝对路径；稳定身份与复制值。
    pub path: String,
}

/// 提示词记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptRecord {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    /// 第一页正文镜像；正文真源为 `pages`，读取当前页不得使用此列。
    pub content: String,
    /// 上次打开的页 id；失效或为空时由应用层回退第一页。
    pub active_page_id: Option<i64>,
    /// 全部页（按 `position` 升序；迁移后至少一页）。
    pub pages: Vec<PromptPageRecord>,
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
        let mut database = Self { conn };
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

    /// 迁移到当前 Schema：建表与版本推进在同一事务中完成，幂等。
    ///
    /// 开发期的资产/附件表已随截图粘贴功能移除，老库中遗留的这些表不再被读写；
    /// 已有数据（工作区、提示词）原样保留。版本高于程序支持值时返回错误。
    ///
    /// v4 迁移把每条旧提示词的正文原样复制为唯一 `Page 1` 并指向当前页；
    /// v5 迁移重建提示词表移除状态列（在关闭外键的独立事务中完成，不删页表数据）。
    /// 已升级库不再回填（版本号门控），空库得到同一结构。
    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(DbError::UnsupportedSchema(version));
        }
        if version < SCHEMA_VERSION {
            if version < 4 {
                let transaction = self.conn.transaction()?;
                transaction.execute_batch(SCHEMA)?;
                transaction.execute_batch(SCHEMA_V4)?;
                // 老库的 workspace_prompt 由历史 SCHEMA 建立，缺 active_page_id 列。
                if !column_exists(&transaction, "workspace_prompt", "active_page_id")? {
                    transaction.execute_batch(
                        "ALTER TABLE workspace_prompt ADD COLUMN active_page_id INTEGER;",
                    )?;
                }
                // 旧正文逐字进入唯一 Page 1；已有页的提示词（重复迁移）不重复生成。
                transaction.execute(
                    r#"INSERT INTO workspace_prompt_page
                           (prompt_id, position, name, content, created_at, updated_at)
                       SELECT p.id, 0, ?1, p.content, datetime('now'), datetime('now')
                       FROM workspace_prompt p
                       WHERE NOT EXISTS (
                           SELECT 1 FROM workspace_prompt_page pg WHERE pg.prompt_id = p.id
                       )"#,
                    params![FIRST_PAGE_NAME],
                )?;
                transaction.execute(
                    r#"UPDATE workspace_prompt
                       SET active_page_id = (
                           SELECT pg.id FROM workspace_prompt_page pg
                           WHERE pg.prompt_id = workspace_prompt.id
                           ORDER BY pg.position LIMIT 1
                       )
                       WHERE active_page_id IS NULL"#,
                    [],
                )?;
                transaction.pragma_update(None, "user_version", 4)?;
                transaction.commit()?;
            }
            // 旧库（v1..v4）带状态列：整表重建去列，必须在事务外关闭外键，
            // 否则 `DROP TABLE workspace_prompt` 会级联删除页表数据。
            if column_exists(&self.conn, "workspace_prompt", "status")? {
                self.rebuild_prompt_without_status()?;
            }
            let transaction = self.conn.transaction()?;
            transaction.execute_batch(SCHEMA)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
        }
        Ok(())
    }

    /// v5：重建 `workspace_prompt` 去 `status` 列（页表外键指向重建后的同名表）。
    ///
    /// `DROP TABLE` 在打开外键时会级联清空页表，因此重建必须在关闭外键的
    /// 独立事务内完成；失败不推进版本，下次打开会重试（结构幂等）。
    fn rebuild_prompt_without_status(&mut self) -> Result<()> {
        self.conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let result = (|| -> Result<()> {
            let transaction = self.conn.transaction()?;
            transaction.execute_batch(SCHEMA_V5_PROMPT)?;
            let violations: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM pragma_foreign_key_check",
                [],
                |row| row.get(0),
            )?;
            if violations != 0 {
                return Err(DbError::ForeignKeyViolation);
            }
            transaction.commit()?;
            Ok(())
        })();
        self.conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        result
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

    /// 工作区内全部提示词（按创建顺序）；每条的页集合一次批量读取。
    pub fn list_prompts(&self, workspace: &Path) -> Result<Vec<PromptRecord>> {
        let Some(external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT p.id, p.external_id, f.id, p.name, p.content, p.active_page_id
               FROM workspace_prompt p
               LEFT JOIN workspace_folder f ON p.folder_id = f.external_id
               WHERE p.workspace_id = ?1
               ORDER BY p.id"#,
        )?;
        let mut prompts: Vec<PromptRecord> = statement
            .query_map(params![external], |row| {
                Ok(PromptRecord {
                    id: row.get(0)?,
                    folder_id: row.get(2)?,
                    name: row.get(3)?,
                    content: row.get(4)?,
                    active_page_id: row.get(5)?,
                    pages: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut pages_statement = self.conn.prepare(
            r#"SELECT pg.prompt_id, pg.id, pg.position, pg.name, pg.content
               FROM workspace_prompt_page pg
               JOIN workspace_prompt p ON p.id = pg.prompt_id
               WHERE p.workspace_id = ?1
               ORDER BY pg.prompt_id, pg.position"#,
        )?;
        let mut pages: std::collections::HashMap<i64, Vec<PromptPageRecord>> =
            std::collections::HashMap::new();
        let rows = pages_statement.query_map(params![external], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                PromptPageRecord {
                    id: row.get(1)?,
                    position: row.get(2)?,
                    name: row.get(3)?,
                    content: row.get(4)?,
                },
            ))
        })?;
        for row in rows {
            let (prompt_id, page) = row?;
            pages.entry(prompt_id).or_default().push(page);
        }
        for prompt in &mut prompts {
            prompt.pages = pages.remove(&prompt.id).unwrap_or_default();
        }
        Ok(prompts)
    }

    /// 项目内全部引用文件夹（按添加顺序 `id ASC`）；工作区未登记时返回空表。
    pub fn list_referenced_folders(&self, workspace: &Path) -> Result<Vec<ReferencedFolderRecord>> {
        let Some(external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT id, path FROM workspace_referenced_folder
               WHERE workspace_id = ?1
               ORDER BY id"#,
        )?;
        let rows = statement.query_map(params![external], |row| {
            Ok(ReferencedFolderRecord {
                id: row.get(0)?,
                path: row.get(1)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 添加引用文件夹（路径须为调用方规范化后的绝对路径）。
    ///
    /// 唯一约束 `(workspace_id, path)` 使重复添加幂等：命中已有记录时返回既有 id。
    pub fn add_referenced_folder(&self, workspace: &Path, path: &str) -> Result<i64> {
        self.ensure_workspace(workspace)?;
        let external = self
            .workspace_external_id(workspace)?
            .ok_or(DbError::WorkspaceMissing)?;
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM workspace_referenced_folder WHERE workspace_id = ?1 AND path = ?2",
                params![external, path],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
        self.conn.execute(
            r#"INSERT INTO workspace_referenced_folder (workspace_id, path, created_at)
               VALUES (?1, ?2, datetime('now'))"#,
            params![external, path],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 删除引用文件夹记录；以记录 id 与项目键双条件约束，避免跨项目误删。
    pub fn delete_referenced_folder(&self, workspace: &Path, id: i64) -> Result<()> {
        let Some(external) = self.workspace_external_id(workspace)? else {
            return Ok(());
        };
        self.conn.execute(
            "DELETE FROM workspace_referenced_folder WHERE id = ?1 AND workspace_id = ?2",
            params![id, external],
        )?;
        Ok(())
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

    /// 新建提示词与全部页；`folder_id` 为 None 时挂在根层。
    ///
    /// `pages` 为（页名，正文）序列（临时提示词升格时携带全部页）；
    /// 为空时建立唯一的空 `Page 1`。第一页正文写入镜像列，当前页指向第一页。
    /// 同一事务完成，成功后才由调用方移除临时对象。
    pub fn create_prompt(
        &self,
        workspace: &Path,
        folder_id: Option<i64>,
        name: &str,
        pages: &[(String, String)],
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
        let default_page = (FIRST_PAGE_NAME.to_string(), String::new());
        let pages: Vec<&(String, String)> = if pages.is_empty() {
            vec![&default_page]
        } else {
            pages.iter().collect()
        };
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            r#"INSERT INTO workspace_prompt (external_id, workspace_id, folder_id, name, content, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'), datetime('now'))"#,
            params![
                new_external_id("prompt"),
                workspace_external,
                folder_external,
                name,
                pages[0].1
            ],
        )?;
        let prompt_id = transaction.last_insert_rowid();
        let mut first_page_id = 0;
        for (position, (page_name, content)) in pages.iter().enumerate() {
            transaction.execute(
                r#"INSERT INTO workspace_prompt_page (prompt_id, position, name, content, created_at, updated_at)
                   VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'))"#,
                params![prompt_id, position as i64, page_name, content],
            )?;
            if position == 0 {
                first_page_id = transaction.last_insert_rowid();
            }
        }
        transaction.execute(
            "UPDATE workspace_prompt SET active_page_id = ?1 WHERE id = ?2",
            params![first_page_id, prompt_id],
        )?;
        transaction.commit()?;
        Ok(prompt_id)
    }

    /// 重命名提示词。
    pub fn rename_prompt(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_prompt SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    /// 更新一页正文；该页为第一页（最小 `position`）时同一事务同步镜像列。
    pub fn update_page_content(&self, id: i64, content: &str) -> Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        let updated = transaction.execute(
            "UPDATE workspace_prompt_page SET content = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![content, id],
        )?;
        if updated == 0 {
            return Err(DbError::PageMissing);
        }
        transaction.execute(
            r#"UPDATE workspace_prompt SET content = ?1, updated_at = datetime('now')
               WHERE id = (SELECT prompt_id FROM workspace_prompt_page WHERE id = ?2)
                 AND (SELECT position FROM workspace_prompt_page WHERE id = ?2) = (
                     SELECT MIN(position) FROM workspace_prompt_page
                     WHERE prompt_id = (SELECT prompt_id FROM workspace_prompt_page WHERE id = ?2)
                 )"#,
            params![content, id],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// 追加一页到末尾（`position` 取最大加一）并设为当前页，返回新页 id。
    pub fn create_prompt_page(&self, prompt_id: i64, name: &str) -> Result<i64> {
        let transaction = self.conn.unchecked_transaction()?;
        let exists: Option<i64> = transaction
            .query_row(
                "SELECT id FROM workspace_prompt WHERE id = ?1",
                params![prompt_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(DbError::WorkspaceMissing);
        }
        let position: i64 = transaction.query_row(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM workspace_prompt_page WHERE prompt_id = ?1",
            params![prompt_id],
            |row| row.get(0),
        )?;
        transaction.execute(
            r#"INSERT INTO workspace_prompt_page (prompt_id, position, name, content, created_at, updated_at)
               VALUES (?1, ?2, ?3, '', datetime('now'), datetime('now'))"#,
            params![prompt_id, position, name],
        )?;
        let id = transaction.last_insert_rowid();
        transaction.execute(
            "UPDATE workspace_prompt SET active_page_id = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![id, prompt_id],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    /// 重命名一页。
    pub fn rename_prompt_page(&self, id: i64, name: &str) -> Result<()> {
        let updated = self.conn.execute(
            "UPDATE workspace_prompt_page SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![name, id],
        )?;
        if updated == 0 {
            return Err(DbError::PageMissing);
        }
        Ok(())
    }

    /// 删除一页；删除当前页时同事务选择后继页（优先下一页、其次前一页）。
    ///
    /// 返回删除后应显示的页 id；删除的不是当前页时为 None。
    pub fn delete_prompt_page(&self, id: i64) -> Result<Option<i64>> {
        let transaction = self.conn.unchecked_transaction()?;
        let row: Option<(i64, i64)> = transaction
            .query_row(
                "SELECT prompt_id, position FROM workspace_prompt_page WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((prompt_id, position)) = row else {
            return Err(DbError::PageMissing);
        };
        transaction.execute(
            "DELETE FROM workspace_prompt_page WHERE id = ?1",
            params![id],
        )?;
        let current: Option<i64> = transaction
            .query_row(
                "SELECT active_page_id FROM workspace_prompt WHERE id = ?1",
                params![prompt_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if current != Some(id) {
            transaction.commit()?;
            return Ok(None);
        }
        let next: Option<i64> = transaction
            .query_row(
                "SELECT id FROM workspace_prompt_page WHERE prompt_id = ?1 AND position > ?2 ORDER BY position LIMIT 1",
                params![prompt_id, position],
                |row| row.get(0),
            )
            .optional()?
            .or(transaction
                .query_row(
                    "SELECT id FROM workspace_prompt_page WHERE prompt_id = ?1 AND position < ?2 ORDER BY position DESC LIMIT 1",
                    params![prompt_id, position],
                    |row| row.get(0),
                )
                .optional()?);
        transaction.execute(
            "UPDATE workspace_prompt SET active_page_id = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![next, prompt_id],
        )?;
        transaction.commit()?;
        Ok(next)
    }

    /// 记录提示词的当前页；页不属于该提示词时拒绝。
    pub fn set_prompt_active_page(&self, prompt_id: i64, page_id: i64) -> Result<()> {
        let updated = self.conn.execute(
            r#"UPDATE workspace_prompt SET active_page_id = ?1, updated_at = datetime('now')
               WHERE id = ?2
                 AND EXISTS (SELECT 1 FROM workspace_prompt_page WHERE id = ?1 AND prompt_id = ?2)"#,
            params![page_id, prompt_id],
        )?;
        if updated == 0 {
            return Err(DbError::PageMissing);
        }
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

/// 表是否已有指定列（`PRAGMA table_info`）；仅迁移期使用。
fn column_exists(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut names = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(names.any(|name| name.is_ok_and(|name| name == column)))
}

#[cfg(test)]
mod tests;

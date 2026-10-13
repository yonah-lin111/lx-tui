//! SQLite 持久层：工作区信任、提示词与文件夹的受控读写；唯一接触 rusqlite 的模块。
//!
//! 数据库路径按环境区分：生产 `~/.lx-t/db/lx.db`，开发 `~/.lx-t-dev/db/lx.db`；
//! `LX_TUI_ENV` / `LX_ENV` 环境变量优先（`dev` 走开发路径，其余取值视为生产），
//! 缺省回退 `cfg!(debug_assertions)` 门控；`LX_TUI_DB_PATH` 可显式覆盖（测试隔离用）。
//! Schema 迁移使用 SQLite 原生 `PRAGMA user_version` 版本控制。

pub mod service;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

/// 当前 Schema 版本（v2 增加 prompt 附件与 managed 资产登记）。
const SCHEMA_VERSION: i64 = 2;

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

/// v2 迁移：prompt 附件表与应用管理的截图资产登记表。
///
/// 约定与 v1 一致：内部 `id INTEGER`、对外 `external_id TEXT UNIQUE`；`prompt_id` 引用
/// `workspace_prompt.external_id`，`managed_asset_id` 引用资产 `external_id`；
/// `managed_asset_id` 与 `external_path` 恰一非空，managed 资产只允许图片。
const SCHEMA_V2: &str = r#"
CREATE TABLE IF NOT EXISTS managed_attachment_asset (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS workspace_prompt_attachment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    external_id TEXT NOT NULL UNIQUE,
    prompt_id TEXT NOT NULL,
    managed_asset_id TEXT,
    external_path TEXT,
    position INTEGER NOT NULL,
    "type" TEXT NOT NULL CHECK ("type" IN ('file', 'image')),
    name TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (prompt_id) REFERENCES workspace_prompt(external_id) ON DELETE CASCADE,
    FOREIGN KEY (managed_asset_id) REFERENCES managed_attachment_asset(external_id) ON DELETE RESTRICT,
    CHECK (
        (managed_asset_id IS NULL AND external_path IS NOT NULL)
        OR (managed_asset_id IS NOT NULL AND external_path IS NULL AND "type" = 'image')
    ),
    UNIQUE (prompt_id, position)
);

CREATE INDEX IF NOT EXISTS idx_prompt_attachment_prompt ON workspace_prompt_attachment(prompt_id);
CREATE INDEX IF NOT EXISTS idx_prompt_attachment_asset ON workspace_prompt_attachment(managed_asset_id);
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
            Self::DepthExceeded => write!(formatter, "folder nesting depth exceeded"),
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

/// 提示词附件记录（加载）：managed 附件路径来自资产表 JOIN，外部附件来自 `external_path`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRecord {
    /// 附件行稳定标识（`workspace_prompt_attachment.external_id`）。
    pub id: String,
    /// 解析后的绝对路径（managed 走资产表）。
    pub path: PathBuf,
    /// `file` / `image`。
    pub kind: String,
    pub name: String,
    /// managed 附件的资产外部标识；外部附件为 None。
    pub managed_asset_id: Option<String>,
}

/// 附件写入参数：`managed_asset_id` 与 `external_path` 恰一非空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentWrite {
    /// 附件稳定标识；由业务层生成。
    pub id: String,
    pub managed_asset_id: Option<String>,
    pub external_path: Option<PathBuf>,
    /// `file` / `image`。
    pub kind: String,
    pub name: String,
}

/// 应用管理的截图资产登记记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedAssetRecord {
    pub external_id: String,
    pub path: PathBuf,
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
    /// 附件（按保存顺序）。
    pub attachments: Vec<AttachmentRecord>,
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

    /// 逐版本迁移到当前 Schema；每步在建表与版本推进的同一事务中完成，幂等。
    ///
    /// v1 建表保留为历史步骤；v2 在此基础上新增附件表与资产登记表，
    /// 旧 v1 数据不改正文、附件为空。版本高于程序支持值时返回错误。
    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(DbError::UnsupportedSchema(version));
        }
        if version < 1 {
            let transaction = self.conn.transaction()?;
            transaction.execute_batch(SCHEMA_V1)?;
            transaction.pragma_update(None, "user_version", 1)?;
            transaction.commit()?;
        }
        if version < 2 {
            let transaction = self.conn.transaction()?;
            transaction.execute_batch(SCHEMA_V2)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            transaction.commit()?;
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

    /// 工作区内全部提示词（按创建顺序）；附件按 `position` 随记录加载。
    pub fn list_prompts(&self, workspace: &Path) -> Result<Vec<PromptRecord>> {
        let Some(external) = self.workspace_external_id(workspace)? else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            r#"SELECT p.id, p.external_id, f.id, p.name, p.content, p.status
               FROM workspace_prompt p
               LEFT JOIN workspace_folder f ON p.folder_id = f.external_id
               WHERE p.workspace_id = ?1
               ORDER BY p.id"#,
        )?;
        let mut prompts: Vec<(String, PromptRecord)> = statement
            .query_map(params![external], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    PromptRecord {
                        id: row.get(0)?,
                        folder_id: row.get(2)?,
                        name: row.get(3)?,
                        content: row.get(4)?,
                        status: row.get(5)?,
                        attachments: Vec::new(),
                    },
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut attachments = self.prompt_attachments(&external)?;
        for (prompt_external, record) in &mut prompts {
            if let Some(loaded) = attachments.remove(prompt_external) {
                record.attachments = loaded;
            }
        }
        Ok(prompts.into_iter().map(|(_, record)| record).collect())
    }

    /// 工作区内全部附件，按 prompt 外部标识分组并保持 `position` 顺序。
    fn prompt_attachments(
        &self,
        workspace_external: &str,
    ) -> Result<HashMap<String, Vec<AttachmentRecord>>> {
        let mut statement = self.conn.prepare(
            r#"SELECT a.prompt_id, a.external_id,
                      COALESCE(m.path, a.external_path), a."type", a.name, a.managed_asset_id
               FROM workspace_prompt_attachment a
               LEFT JOIN managed_attachment_asset m ON a.managed_asset_id = m.external_id
               WHERE a.prompt_id IN (
                   SELECT external_id FROM workspace_prompt WHERE workspace_id = ?1
               )
               ORDER BY a.prompt_id, a.position"#,
        )?;
        let rows = statement.query_map(params![workspace_external], |row| {
            let path: Option<String> = row.get(2)?;
            Ok((
                row.get::<_, String>(0)?,
                AttachmentRecord {
                    id: row.get(1)?,
                    path: path.map(PathBuf::from).unwrap_or_default(),
                    kind: row.get(3)?,
                    name: row.get(4)?,
                    managed_asset_id: row.get(5)?,
                },
            ))
        })?;
        let mut grouped: HashMap<String, Vec<AttachmentRecord>> = HashMap::new();
        for row in rows {
            let (prompt_id, record) = row?;
            grouped.entry(prompt_id).or_default().push(record);
        }
        Ok(grouped)
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

    /// 新建提示词；正文与附件在同一事务中写入。
    ///
    /// `folder_id` 为 None 时挂在根层；`attachments` 按数组顺序编号保存。
    pub fn create_prompt(
        &self,
        workspace: &Path,
        folder_id: Option<i64>,
        name: &str,
        content: &str,
        attachments: &[AttachmentWrite],
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
        let prompt_external = new_external_id("prompt");
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            r#"INSERT INTO workspace_prompt (external_id, workspace_id, folder_id, name, content, status, created_at, updated_at)
               VALUES (?1, ?2, ?3, ?4, ?5, 'todo', datetime('now'), datetime('now'))"#,
            params![
                prompt_external,
                workspace_external,
                folder_external,
                name,
                content
            ],
        )?;
        // 提示词行 id 必须在附件插入前取得（附件插入会刷新 last_insert_rowid）。
        let id = transaction.last_insert_rowid();
        insert_attachments(&transaction, &prompt_external, attachments)?;
        transaction.commit()?;
        Ok(id)
    }

    /// 重命名提示词。
    pub fn rename_prompt(&self, id: i64, name: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE workspace_prompt SET name = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![name, id],
        )?;
        Ok(())
    }

    /// 更新提示词正文与附件：同一事务内整体替换附件行，避免只成功一半。
    pub fn update_prompt_document(
        &self,
        id: i64,
        content: &str,
        attachments: &[AttachmentWrite],
    ) -> Result<()> {
        let prompt_external: Option<String> = self
            .conn
            .query_row(
                "SELECT external_id FROM workspace_prompt WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(prompt_external) = prompt_external else {
            return Err(DbError::WorkspaceMissing);
        };
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "UPDATE workspace_prompt SET content = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![content, id],
        )?;
        transaction.execute(
            "DELETE FROM workspace_prompt_attachment WHERE prompt_id = ?1",
            params![prompt_external],
        )?;
        insert_attachments(&transaction, &prompt_external, attachments)?;
        transaction.commit()?;
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

    /// 登记应用管理的截图资产；`external_id` 与文件名 stem 一致。
    pub fn register_managed_asset(&self, external_id: &str, path: &Path) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO managed_attachment_asset (external_id, path, created_at, updated_at)
               VALUES (?1, ?2, datetime('now'), datetime('now'))"#,
            params![external_id, path_string(path)],
        )?;
        Ok(())
    }

    /// 全部 managed 资产登记（用于垃圾回收）。
    pub fn list_managed_assets(&self) -> Result<Vec<ManagedAssetRecord>> {
        let mut statement = self
            .conn
            .prepare("SELECT external_id, path FROM managed_attachment_asset ORDER BY id")?;
        let rows = statement.query_map([], |row| {
            Ok(ManagedAssetRecord {
                external_id: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 删除 managed 资产登记（文件清理由调用方在引用检查后进行）。
    pub fn delete_managed_asset(&self, external_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM managed_attachment_asset WHERE external_id = ?1",
            params![external_id],
        )?;
        Ok(())
    }

    /// 全部已保存附件引用的 managed 资产 ID。
    pub fn managed_attachment_ids(&self) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
            "SELECT DISTINCT managed_asset_id FROM workspace_prompt_attachment WHERE managed_asset_id IS NOT NULL",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }

    /// 全部已保存提示词正文（managed 引用扫描用）。
    pub fn all_prompt_contents(&self) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare("SELECT content FROM workspace_prompt")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(DbError::from)
    }
}

/// 在事务内按顺序写入附件行。
fn insert_attachments(
    transaction: &Transaction<'_>,
    prompt_external: &str,
    attachments: &[AttachmentWrite],
) -> Result<()> {
    let mut statement = transaction.prepare(
        r#"INSERT INTO workspace_prompt_attachment
               (external_id, prompt_id, managed_asset_id, external_path, position, "type", name, created_at, updated_at)
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now'))"#,
    )?;
    for (position, attachment) in attachments.iter().enumerate() {
        statement.execute(params![
            attachment.id,
            prompt_external,
            attachment.managed_asset_id,
            attachment.external_path.as_deref().map(path_string),
            position as i64,
            attachment.kind,
            attachment.name,
        ])?;
    }
    Ok(())
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

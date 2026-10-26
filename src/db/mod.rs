//! SQLite 持久层：工作区信任、提示词、聊天会话的受控读写；唯一接触 rusqlite 的模块。
//!
//! 数据库路径按环境区分：生产 `~/.lx-t/db/lx.db`，开发 `~/.lx-t-dev/db/lx.db`；
//! 开发环境在 linked worktree 内再按 worktree 隔离（`db/worktrees/<名字>-<哈希>/lx.db`，
//! 由入口传入 worktree 根），避免不同分支各自推进的 Schema 版本互相把库判废；
//! `LX_TUI_ENV` / `LX_ENV` 环境变量优先
//! （`dev` 走开发路径，其余取值视为生产），缺省回退 `cfg!(debug_assertions)` 门控；
//! `LX_TUI_DB_PATH` 可显式覆盖（测试隔离用）。
//! Schema 迁移使用 SQLite 原生 `PRAGMA user_version` 版本控制，迁移文本集中在
//! `migration` 子模块；聊天读写见 `chat`，工具调用见 `chat::tools`。

pub mod chat;
pub mod history;
mod migration;
pub mod service;
pub mod todo;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

/// 当前 Schema 版本；版本号保留自管理截图资产的开发期结构，避免旧库被判为过新。
const SCHEMA_VERSION: i64 = 15;

/// 当前 Schema 版本（测试断言用；不暴露可变入口）。
#[cfg(test)]
pub(crate) const SCHEMA_VERSION_FOR_TESTS: i64 = SCHEMA_VERSION;

/// 旧正文迁移为唯一第一页时使用的默认页名。
const FIRST_PAGE_NAME: &str = "Page 1";

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
    /// 目标提示词不存在（已被删除或从未落库）。
    PromptMissing,
    /// 目标提示词页不存在。
    PageMissing,
    /// 文件夹嵌套超过 2 层约束。
    DepthExceeded,
    /// 目标文件夹仍含子文件夹，不能整体移入另一个文件夹。
    FolderHasChildren,
    /// 迁移重建后外键校验发现悬空引用。
    ForeignKeyViolation,
    /// 数据库版本高于当前程序支持值（不能用旧代码覆盖新结构）。
    UnsupportedSchema(i64),
    /// 目标聊天会话不存在（或不在指定工作区内）。
    SessionMissing,
    /// 目标 assistant 消息不存在或已进入终态（checkpoint/finish 只作用于 streaming）。
    MessageNotStreaming,
    /// 会话正在被另一轮写入占用（同 session 的 begin_turn 串行化）。
    SessionBusy,
    /// 发送的草稿为空或仅空白；应用层应拦截，不落库。
    InvalidDraft,
    /// 单条消息内容超过上限；拒绝写入而非截断用户输入。
    MessageTooLarge,
    /// 目标工具调用不存在（或不在指定工作区/会话/消息内）。
    ToolCallMissing,
    /// 同会话内工具 call ID 已存在（重复 begin 被拒）。
    ToolCallConflict,
    /// 分配的时间线位置已被该消息的 part 或工具占用。
    PositionConflict,
    /// 工具调用当前状态不允许目标转移（如 pending 直接 finish）。
    InvalidToolTransition,
    /// 工具调用已落不同终态；不同结果不允许覆盖。
    ToolTerminalConflict,
    /// 工具参数 JSON 超过单次上限。
    ToolArgumentTooLarge,
    /// 工具结果 JSON 超过单次上限。
    ToolResultTooLarge,
    /// 同一运行代次内进程会话 ID 已存在（重复 begin 被拒）。
    ProcessConflict,
    /// 数据库不可用（无持久化模式）；调用方据此停止需要落库的流程。
    Unavailable,
    /// 该轮没有可用的文件快照（旧轮次 / 缺 scope）。
    RewindUnavailable,
    /// 快照超出单份或单轮限额；写入前拒绝。
    RewindLimit,
    /// 目标轮次或会话 revision 已变化。
    RewindTargetChanged,
    /// mutation 记录缺失或状态不允许该转移。
    MutationConflict,
    /// 撤销操作记录不存在。
    OperationMissing,
    /// 撤销存在冲突（文件被其他进程/轮次修改）。
    RewindConflict,
    /// 读后版本保护拒绝：修改已有文件缺 token，或 token 与快照内容不一致。
    StaleRead,
}

impl fmt::Display for DbError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::WorkspaceMissing => write!(formatter, "workspace not registered"),
            Self::FolderMissing => write!(formatter, "folder not found"),
            Self::PromptMissing => write!(formatter, "prompt not found"),
            Self::PageMissing => write!(formatter, "prompt page not found"),
            Self::DepthExceeded => write!(formatter, "folder nesting depth exceeded"),
            Self::FolderHasChildren => write!(formatter, "folder still contains subfolders"),
            Self::ForeignKeyViolation => write!(formatter, "foreign key violation after rebuild"),
            Self::UnsupportedSchema(version) => {
                write!(
                    formatter,
                    "database schema v{version} is newer than supported"
                )
            }
            Self::SessionMissing => write!(formatter, "chat session not found"),
            Self::MessageNotStreaming => write!(formatter, "assistant message is not streaming"),
            Self::SessionBusy => write!(formatter, "chat session is busy"),
            Self::InvalidDraft => write!(formatter, "empty draft"),
            Self::MessageTooLarge => write!(formatter, "message exceeds size limit"),
            Self::ToolCallMissing => write!(formatter, "tool call not found"),
            Self::ToolCallConflict => write!(formatter, "tool call already recorded"),
            Self::PositionConflict => write!(formatter, "message timeline position already used"),
            Self::InvalidToolTransition => write!(formatter, "invalid tool call transition"),
            Self::ToolTerminalConflict => {
                write!(formatter, "tool call already finished differently")
            }
            Self::ToolArgumentTooLarge => write!(formatter, "tool arguments exceed size limit"),
            Self::ToolResultTooLarge => write!(formatter, "tool result exceeds size limit"),
            Self::ProcessConflict => write!(formatter, "process already recorded"),
            Self::Unavailable => write!(formatter, "database unavailable"),
            Self::RewindUnavailable => write!(formatter, "rewind snapshot unavailable"),
            Self::RewindLimit => write!(formatter, "rewind snapshot exceeds limit"),
            Self::RewindTargetChanged => write!(formatter, "rewind target changed"),
            Self::MutationConflict => write!(formatter, "file mutation record conflict"),
            Self::OperationMissing => write!(formatter, "rewind operation missing"),
            Self::RewindConflict => write!(formatter, "rewind conflict"),
            Self::StaleRead => write!(formatter, "stale file version"),
        }
    }
}

impl std::error::Error for DbError {}

impl DbError {
    /// 稳定错误码（用于回传 agent loop 的错误码与 tracing，不含 SQL 原文/参数）。
    pub fn code(&self) -> &'static str {
        match self {
            Self::HomeMissing => "home missing",
            Self::Io(_) => "io",
            Self::Sqlite(_) => "sqlite",
            Self::WorkspaceMissing => "workspace missing",
            Self::FolderMissing => "folder missing",
            Self::PromptMissing => "prompt missing",
            Self::PageMissing => "page missing",
            Self::DepthExceeded => "depth exceeded",
            Self::FolderHasChildren => "folder has children",
            Self::ForeignKeyViolation => "foreign key violation",
            Self::UnsupportedSchema(_) => "unsupported schema",
            Self::SessionMissing => "session missing",
            Self::MessageNotStreaming => "message not streaming",
            Self::SessionBusy => "session busy",
            Self::InvalidDraft => "invalid draft",
            Self::MessageTooLarge => "message too large",
            Self::ToolCallMissing => "tool call missing",
            Self::ToolCallConflict => "tool call conflict",
            Self::PositionConflict => "position conflict",
            Self::InvalidToolTransition => "invalid tool transition",
            Self::ToolTerminalConflict => "tool terminal conflict",
            Self::ToolArgumentTooLarge => "tool argument too large",
            Self::ToolResultTooLarge => "tool result too large",
            Self::ProcessConflict => "process conflict",
            Self::Unavailable => "database unavailable",
            Self::RewindUnavailable => "rewind unavailable",
            Self::RewindLimit => "rewind limit",
            Self::RewindTargetChanged => "rewind target changed",
            Self::MutationConflict => "mutation conflict",
            Self::OperationMissing => "operation missing",
            Self::RewindConflict => "rewind conflict",
            Self::StaleRead => "stale read",
        }
    }
}
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
    ///
    /// `linked_worktree` 由入口传入：开发环境在 linked worktree 内使用独立库文件，
    /// 避免不同分支的 Schema 互相判废。
    pub fn open_default(linked_worktree: Option<&Path>) -> Result<Self> {
        if let Some(overridden) = std::env::var_os("LX_TUI_DB_PATH").filter(|path| !path.is_empty())
        {
            return Self::open(Path::new(&overridden));
        }
        Self::open(&database_path(is_dev_environment(), linked_worktree)?)
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

    /// 移动提示词到指定文件夹（None 回根层）；文件夹须属同一工作区。
    ///
    /// 只改容器，不改顺序（无 `sort_order` 列，顺序仍由 `id` 决定）。
    pub fn move_prompt(&self, id: i64, folder_id: Option<i64>) -> Result<()> {
        // 提示词可能已被另一实例删除：两个分支都要先确认它存在。
        let workspace_external: Option<String> = self
            .conn
            .query_row(
                "SELECT workspace_id FROM workspace_prompt WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let workspace_external = workspace_external.ok_or(DbError::PromptMissing)?;
        let folder_external: Option<String> = match folder_id {
            Some(folder) => {
                let external: Option<String> = self
                    .conn
                    .query_row(
                        "SELECT external_id FROM workspace_folder WHERE id = ?1 AND workspace_id = ?2",
                        params![folder, workspace_external],
                        |row| row.get(0),
                    )
                    .optional()?;
                Some(external.ok_or(DbError::FolderMissing)?)
            }
            None => None,
        };
        let changed = self.conn.execute(
            "UPDATE workspace_prompt SET folder_id = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![folder_external, id],
        )?;
        if changed == 0 {
            return Err(DbError::PromptMissing);
        }
        Ok(())
    }

    /// 移动文件夹（None 回根层）。层数约束：目标必须是根层文件夹，
    /// 被拖文件夹必须是根层且无子文件夹（对齐 `create_folder` 的 2 层上限）。
    ///
    /// 只改容器，不改顺序（无 `sort_order` 列，顺序仍由 `id` 决定）。
    pub fn move_folder(&self, id: i64, parent_id: Option<i64>) -> Result<()> {
        let dragged: Option<(String, String, Option<String>)> = self
            .conn
            .query_row(
                "SELECT external_id, workspace_id, parent_folder_id FROM workspace_folder WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((dragged_external, workspace_external, dragged_parent)) = dragged else {
            return Err(DbError::FolderMissing);
        };
        let parent_external: Option<String> = match parent_id {
            Some(parent) => {
                if parent == id {
                    return Err(DbError::DepthExceeded);
                }
                // 带子文件夹时移入另一文件夹会形成第 3 层：整体拒绝（回根层不受影响）。
                let children: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM workspace_folder WHERE parent_folder_id = ?1",
                    params![dragged_external],
                    |row| row.get(0),
                )?;
                if children > 0 {
                    return Err(DbError::FolderHasChildren);
                }
                let target: Option<(String, Option<String>)> = self
                    .conn
                    .query_row(
                        "SELECT external_id, parent_folder_id FROM workspace_folder WHERE id = ?1 AND workspace_id = ?2",
                        params![parent, workspace_external],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let Some((target_external, target_parent)) = target else {
                    return Err(DbError::FolderMissing);
                };
                if target_parent.is_some() || dragged_parent.is_some() {
                    return Err(DbError::DepthExceeded);
                }
                Some(target_external)
            }
            None => None,
        };
        self.conn.execute(
            "UPDATE workspace_folder SET parent_folder_id = ?1, updated_at = datetime('now') WHERE id = ?2",
            params![parent_external, id],
        )?;
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
///
/// 开发环境在 linked worktree 内按 worktree 再分一层（`worktrees/<名字>-<哈希>/lx.db`）：
/// 不同分支的构建可能各自推进 `SCHEMA_VERSION`，共用同一文件会让先启动的一方把库迁到
/// 新版本，其余 checkout 打开时被判为 `UnsupportedSchema` 而失去持久化。隔离根由入口
/// 传入，持久层不依赖 git；生产环境忽略该参数。
pub fn database_path(is_dev: bool, linked_worktree: Option<&Path>) -> Result<PathBuf> {
    let dir = database_dir(is_dev)?;
    if is_dev && let Some(scope) = linked_worktree.and_then(worktree_scope_for) {
        return Ok(dir.join("worktrees").join(scope).join("lx.db"));
    }
    Ok(dir.join("lx.db"))
}

/// 作用域目录名：清洗后的 worktree 目录名 + 路径哈希（同名目录不互相覆盖）。
fn worktree_scope_for(root: &Path) -> Option<String> {
    let name = root.file_name()?.to_str().map(scope_name)?;
    Some(format!("{name}-{:08x}", path_hash(root) as u32))
}

/// 目录名清洗：保留 ASCII 字母数字与 `-_.`，其余替换为 `-`；空结果回退 `worktree`。
fn scope_name(raw: &str) -> String {
    let name: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('.');
    if name.is_empty() {
        "worktree".to_string()
    } else {
        name.to_string()
    }
}

/// FNV-1a 64 位；作用域只需稳定，不用于安全判定。
fn path_hash(path: &Path) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
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
pub(crate) fn new_external_id(prefix: &str) -> String {
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

/// 当前 UTC 毫秒时间戳；时钟异常时回退 0。
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// 测试辅助：执行一条返回单个整数的查询（只在测试里用；生产读写都有具名入口）。
#[cfg(test)]
impl Database {
    /// 执行 `sql`（必须是单行单列整数查询）。
    pub(crate) fn query_count(&self, sql: &str) -> Result<i64> {
        Ok(self.conn.query_row(sql, [], |row| row.get(0))?)
    }
}

#[cfg(test)]
mod tests;

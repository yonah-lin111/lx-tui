//! 工作区上下文解析、SQLite 数据库打开与跨进程会话锁辅助。

use std::path::{Path, PathBuf};

use crate::db::{self, Database};
use crate::git;
use crate::platform::FileGuard;

/// 工作区与数据库解析上下文。
#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    /// 实际执行目录（`-C` 或当前进程 `cwd`，已 canonicalize）。
    pub cwd: PathBuf,
    /// 项目主工作区根路径（Git 主 checkout 根；非 Git 目录则等于 `cwd`）。
    pub workspace: PathBuf,
    /// 开发环境下的 linked worktree 根路径（用于隔离开发数据库）。
    pub effective_worktree: Option<PathBuf>,
}

impl WorkspaceContext {
    /// 解析工作目录、Git 主仓库根与开发态 worktree 数据库隔离路径。
    pub fn resolve(
        cwd_override: Option<&Path>,
        entry_linked_worktree: Option<&Path>,
    ) -> Result<Self, String> {
        let base = std::env::current_dir()
            .map_err(|e| format!("failed to read current directory: {e}"))?;
        let target = match cwd_override {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => base.join(path),
            None => base,
        };
        let meta = std::fs::metadata(&target).map_err(|e| {
            format!(
                "working directory '{}' is not accessible: {e}",
                target.display()
            )
        })?;
        if !meta.is_dir() {
            return Err(format!(
                "working directory '{}' is not a directory",
                target.display()
            ));
        }
        let cwd = std::fs::canonicalize(&target).unwrap_or(target);
        let workspace = git::list_worktrees(&cwd)
            .map(|list| list.repo_root)
            .unwrap_or_else(|| cwd.clone());
        let effective_worktree = if cwd_override.is_some() {
            if db::is_dev_environment() {
                git::linked_worktree_root(&cwd)
            } else {
                None
            }
        } else {
            entry_linked_worktree.map(Path::to_path_buf).or_else(|| {
                db::is_dev_environment()
                    .then(|| git::linked_worktree_root(&cwd))
                    .flatten()
            })
        };
        Ok(Self {
            cwd,
            workspace,
            effective_worktree,
        })
    }

    /// 当前所在 checkout 根：linked worktree 返回其自身路径，否则回退工作区根。
    pub fn checkout_path(&self) -> PathBuf {
        git::list_worktrees(&self.cwd)
            .and_then(|list| git::checkout(&list, &self.cwd))
            .map(|checkout| checkout.checkout_path)
            .unwrap_or_else(|| self.workspace.clone())
    }

    /// 项目级写操作的统一信任门控：cwd、当前 checkout 与工作区根任一已信任即可。
    pub fn is_trusted(&self, db: &Database) -> bool {
        db.is_trusted(&self.cwd).unwrap_or(false)
            || db.is_trusted(&self.checkout_path()).unwrap_or(false)
            || db.is_trusted(&self.workspace).unwrap_or(false)
    }

    /// 打开默认 SQLite 数据库。
    pub fn open_db(&self) -> Result<Database, String> {
        Database::open_default(self.effective_worktree.as_deref())
            .map_err(|e| format!("failed to open database: {}", e.code()))
    }

    /// 返回当前生效的默认数据库文件路径。
    pub fn db_path(&self) -> Result<PathBuf, String> {
        db::database_path(db::is_dev_environment(), self.effective_worktree.as_deref())
            .map_err(|e| format!("failed to resolve database path: {}", e.code()))
    }
}

/// 打开全局（非特定工作区）默认 SQLite 数据库（如 `todo` 与 `workspace list` 使用）。
pub fn open_global_db(
    cwd_override: Option<&Path>,
    entry_linked_worktree: Option<&Path>,
) -> Result<Database, String> {
    if cwd_override.is_some() {
        let ctx = WorkspaceContext::resolve(cwd_override, entry_linked_worktree)?;
        return ctx.open_db();
    }
    let effective = entry_linked_worktree.map(Path::to_path_buf).or_else(|| {
        db::is_dev_environment()
            .then(|| git::linked_worktree_root(Path::new(".")))
            .flatten()
    });
    Database::open_default(effective.as_deref())
        .map_err(|e| format!("failed to open database: {}", e.code()))
}

/// 解析 `<SESSION_ID> | --last` 目标会话（支持精确匹配或唯一前缀匹配）。
pub fn resolve_target_session(
    db: &Database,
    workspace: &Path,
    session_id: Option<&str>,
    last: bool,
) -> Result<String, String> {
    if last && session_id.is_some() {
        return Err("cannot specify both a session ID and '--last'".to_string());
    }
    if !last && session_id.is_none() {
        return Err("specify a <SESSION_ID> or pass '--last'".to_string());
    }
    // 分页拉取全部会话：单页上限 200，避免超出后无法按 ID/前缀解析；
    // 空标题会话同样参与匹配。
    let mut sessions = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = db
            .list_chat_sessions(workspace, offset, 200)
            .map_err(|e| format!("failed to list sessions: {}", e.code()))?;
        let count = page.len();
        sessions.extend(page);
        if count < 200 {
            break;
        }
        offset = offset.saturating_add(count);
    }
    if last {
        return sessions
            .into_iter()
            .next()
            .map(|s| s.external_id)
            .ok_or_else(|| format!("no sessions found in workspace '{}'", workspace.display()));
    }
    let query = session_id.unwrap_or_default().trim();
    if query.is_empty() {
        return Err("session ID must not be empty".to_string());
    }
    if let Some(exact) = sessions.iter().find(|s| s.external_id == query) {
        return Ok(exact.external_id.clone());
    }
    let prefix_matches: Vec<&str> = sessions
        .iter()
        .filter(|s| s.external_id.starts_with(query))
        .map(|s| s.external_id.as_str())
        .collect();
    match prefix_matches.as_slice() {
        [single] => Ok((*single).to_string()),
        [] => Err(format!(
            "session '{query}' not found in workspace '{}'",
            workspace.display()
        )),
        many => Err(format!(
            "session prefix '{query}' is ambiguous (matches: {})",
            many.join(", ")
        )),
    }
}

/// 获取跨进程会话独占锁；被 TUI 或其它 `exec` 占用时立即返回可读英文错误。
pub fn acquire_session_lock(session_id: &str) -> Result<FileGuard, String> {
    match crate::platform::try_lock_session(session_id) {
        Ok(Some(guard)) => Ok(guard),
        Ok(None) => Err(format!(
            "session '{session_id}' is currently active in another lx-tui process; stop the active turn first"
        )),
        Err(err) => Err(format!("failed to lock session '{session_id}': {err}")),
    }
}

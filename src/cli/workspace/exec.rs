//! `lx-tui workspace` 子命令执行与历史列表操作。

use super::*;

mod list;
use list::run as list;

pub fn run(
    cli: WorkspaceCli,
    linked_worktree: Option<&Path>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let WorkspaceCli { global, command } = cli;
    let ctx = WorkspaceContext::resolve(global.cwd.as_deref(), linked_worktree)?;
    let out = Output::new(global.json, global.color);
    let db = ctx.open_db()?;
    match command {
        WorkspaceCommand::List { raw, search } => list(&db, raw, search.as_deref(), &out)?,
        WorkspaceCommand::Status => status(&ctx, &db, &out)?,
        WorkspaceCommand::Add { dir, name, trust } => {
            add(&ctx, &db, dir.as_deref(), name.as_deref(), trust, &out)?
        }
        WorkspaceCommand::Rename { new_name } => rename(&ctx, &db, &new_name, &out)?,
        WorkspaceCommand::Delete {
            dir,
            include_worktrees,
        } => delete(&ctx, &db, dir.as_deref(), include_worktrees, &out)?,
        WorkspaceCommand::Reorder { paths } => reorder(&ctx, &db, &paths, &out)?,
        WorkspaceCommand::Trust { name } => trust(&ctx, &db, name.as_deref(), &out)?,
        WorkspaceCommand::Worktrees => worktrees(&ctx, &out)?,
    }
    Ok(EXIT_OK)
}

fn status(ctx: &WorkspaceContext, db: &Database, out: &Output) -> Result<(), String> {
    let target = ctx.checkout_path();
    let records = db
        .list_workspaces()
        .map_err(|e| format!("failed to list workspaces: {}", e.code()))?;
    let record = find_record(&records, &target).or_else(|| find_record(&records, &ctx.workspace));
    let hidden = db
        .hidden_workspace_paths()
        .map_err(|e| format!("failed to read hidden workspaces: {}", e.code()))?;
    let hidden = is_hidden(&hidden, &target);
    let trusted = ctx.is_trusted(db);
    let git =
        crate::git::list_worktrees(&ctx.cwd).and_then(|list| crate::git::checkout(&list, &ctx.cwd));
    let db_path = database_file(ctx)?;
    let value = json!({
        "path": target, "cwd": ctx.cwd, "name": record.map(|r| r.name.clone()),
        "position": record.map(|r| r.position), "trusted": trusted, "hidden": hidden,
        "exists": target.is_dir(),
        "git": git.as_ref().map(|c| json!({ "repo_root": c.repo_root, "checkout_path": c.checkout_path, "is_linked": c.is_linked, "branch": c.branch, "main_branch": c.main_branch })),
        "database": db_path,
    });
    out.emit(&value, |out, writer| {
        writeln!(writer, "{} {}", out.bold("Workspace:"), target.display())?;
        writeln!(
            writer,
            "{} {}",
            out.bold("Name:"),
            record.map(|r| r.name.as_str()).unwrap_or("-")
        )?;
        writeln!(writer, "{} {}", out.bold("Trusted:"), trusted)?;
        writeln!(writer, "{} {}", out.bold("Hidden:"), hidden)?;
        writeln!(writer, "{} {}", out.bold("Exists:"), target.is_dir())?;
        match &git {
            Some(c) => {
                writeln!(
                    writer,
                    "{} {}",
                    out.bold("Branch:"),
                    c.branch.as_deref().unwrap_or("-")
                )?;
                writeln!(
                    writer,
                    "{} {} (repo {})",
                    out.bold("Linked worktree:"),
                    c.is_linked,
                    c.repo_root.display()
                )?;
            }
            None => writeln!(writer, "{} none", out.bold("Git:"))?,
        }
        writeln!(writer, "{} {}", out.bold("Database:"), db_path.display())
    })
}
fn add(
    ctx: &WorkspaceContext,
    db: &Database,
    dir: Option<&Path>,
    name: Option<&str>,
    trust: bool,
    out: &Output,
) -> Result<(), String> {
    let target = resolve_path(ctx, dir);
    if !target.is_dir() {
        return Err(format!(
            "workspace path '{}' does not exist or is not a directory",
            target.display()
        ));
    }
    let default_name = name
        .map(str::to_string)
        .or_else(|| file_name(&target))
        .unwrap_or_else(|| "workspace".to_string());
    let saved = db
        .record_workspace_open(&target, &default_name)
        .map_err(|e| format!("failed to record workspace: {}", e.code()))?;
    db.unhide_workspace(&target)
        .map_err(|e| format!("failed to unhide workspace: {}", e.code()))?;
    if trust {
        db.trust_workspace(&target, &saved)
            .map_err(|e| format!("failed to trust workspace: {}", e.code()))?;
    }
    let trusted = db.is_trusted(&target).unwrap_or(trust);
    let value = json!({ "added": true, "path": target, "name": saved, "trusted": trusted });
    out.emit(&value, |out, writer| {
        writeln!(
            writer,
            "{} workspace '{}' registered as '{}'{}",
            out.green("OK"),
            target.display(),
            saved,
            if trusted { " (trusted)" } else { "" }
        )
    })
}
fn rename(
    ctx: &WorkspaceContext,
    db: &Database,
    new_name: &str,
    out: &Output,
) -> Result<(), String> {
    let records = db
        .list_workspaces()
        .map_err(|e| format!("failed to list workspaces: {}", e.code()))?;
    let target = resolve_path(ctx, None);
    if find_record(&records, &target).is_none() {
        return Err(format!(
            "workspace '{}' is not registered in the history list",
            target.display()
        ));
    }
    db.rename_workspace(&target, new_name)
        .map_err(|e| format!("failed to rename workspace: {}", e.code()))?;
    let value = json!({ "renamed": true, "path": target, "name": new_name });
    out.emit(&value, |out, writer| {
        writeln!(
            writer,
            "{} workspace '{}' renamed to '{}'",
            out.green("OK"),
            target.display(),
            new_name
        )
    })
}
fn delete(
    ctx: &WorkspaceContext,
    db: &Database,
    dir: Option<&Path>,
    include_worktrees: bool,
    out: &Output,
) -> Result<(), String> {
    let target = resolve_path(ctx, dir);
    let records = db
        .list_workspaces()
        .map_err(|e| format!("failed to list workspaces: {}", e.code()))?;
    if find_record(&records, &target).is_none() {
        let history = db
            .list_workspace_history_entries()
            .map_err(|e| format!("failed to list workspace history: {}", e.code()))?;
        if !history.iter().any(|entry| same_path(&entry.path, &target)) {
            return Err(format!(
                "workspace '{}' is not registered in the history list",
                target.display()
            ));
        }
    }
    // 只有 target 本身是主 checkout 时才展开同仓库 linked worktree；
    // 仓库子目录或 linked worktree 绝不能把主仓库/兄弟 worktree 带上。
    let is_main_root =
        crate::git::list_worktrees(&target).is_some_and(|list| list.repo_root == target);
    let mut targets = vec![target.clone()];
    if include_worktrees || is_main_root {
        targets.extend(linked_worktrees(db, &target)?);
    }
    let (mut deleted, mut hidden) = (Vec::new(), Vec::new());
    for candidate in targets {
        let (guards, sessions) = lock_workspace_sessions(db, &candidate)?;
        if candidate.join(".git").is_file() {
            db.hide_workspace(&candidate)
                .map_err(|e| format!("failed to hide workspace: {}", e.code()))?;
            hidden.push(candidate.clone());
        }
        db.delete_workspace(&candidate)
            .map_err(|e| format!("failed to delete workspace: {}", e.code()))?;
        for session in &sessions {
            crate::platform::remove_session_images(session);
        }
        drop(guards);
        deleted.push(candidate);
    }
    let value = json!({ "deleted": true, "workspaces": &deleted, "hidden": &hidden, "include_worktrees": include_worktrees });
    out.emit(&value, |out, writer| {
        for path in &deleted {
            writeln!(
                writer,
                "{} deleted workspace '{}'",
                out.green("OK"),
                path.display()
            )?;
        }
        Ok(())
    })
}

/// 同仓库下需要一并清理的 linked worktree（当前存在的 + DB 里仍留存的）。
fn linked_worktrees(db: &Database, target: &Path) -> Result<Vec<PathBuf>, String> {
    let Some(list) = crate::git::list_worktrees(target) else {
        return Ok(Vec::new());
    };
    if list.repo_root != target {
        return Ok(Vec::new());
    }
    let mut candidates: Vec<PathBuf> = list
        .entries
        .iter()
        .filter(|entry| entry.path != list.repo_root)
        .map(|entry| entry.path.clone())
        .collect();
    let records = db
        .list_workspaces()
        .map_err(|e| format!("failed to list workspaces: {}", e.code()))?;
    for record in records {
        if record.path != target && gitdir_points_into(&record.path, target) {
            candidates.push(record.path);
        }
    }
    let mut seen = HashSet::new();
    Ok(candidates
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect())
}

/// `.git` 文件的 `gitdir:` 是否指向 `repo_root/.git/worktrees/`（排除嵌套子模块）。
fn gitdir_points_into(worktree: &Path, repo_root: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(worktree.join(".git")) else {
        return false;
    };
    let Some(raw) = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:").map(str::trim))
    else {
        return false;
    };
    let path = Path::new(raw);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        worktree.join(path)
    };
    let resolved = std::fs::canonicalize(&resolved).unwrap_or(resolved);
    let expected = repo_root.join(".git").join("worktrees");
    let expected = std::fs::canonicalize(&expected).unwrap_or(expected);
    resolved.starts_with(expected)
}

fn same_path(left: &Path, right: &Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .is_ok_and(|left| std::fs::canonicalize(right).is_ok_and(|right| left == right))
}

/// 目标工作区存在运行中的会话时拒绝删除；返回的 guard 必须活到删除完成，
/// 同时返回该工作区全部会话 ID 供删除后清理图片。
fn lock_workspace_sessions(
    db: &Database,
    workspace: &Path,
) -> Result<(Vec<FileGuard>, Vec<String>), String> {
    let mut guards = Vec::new();
    let mut session_ids = Vec::new();
    let mut offset = 0;
    loop {
        let sessions = db
            .list_chat_sessions(workspace, offset, 200)
            .map_err(|e| format!("failed to list sessions: {}", e.code()))?;
        if sessions.is_empty() {
            break;
        }
        let count = sessions.len();
        for session in &sessions {
            match crate::platform::try_lock_session(&session.external_id) {
                Ok(Some(guard)) => guards.push(guard),
                Ok(None) => {
                    return Err(format!(
                        "workspace '{}' has an active session '{}'; stop it before deleting",
                        workspace.display(),
                        session.external_id
                    ));
                }
                Err(error) => {
                    return Err(format!(
                        "failed to check session '{}': {error}",
                        session.external_id
                    ));
                }
            }
            session_ids.push(session.external_id.clone());
        }
        if count < 200 {
            break;
        }
        offset += count;
    }
    Ok((guards, session_ids))
}
fn reorder(
    ctx: &WorkspaceContext,
    db: &Database,
    paths: &[PathBuf],
    out: &Output,
) -> Result<(), String> {
    let records = db
        .list_workspaces()
        .map_err(|e| format!("failed to list workspaces: {}", e.code()))?;
    let mut order: Vec<(PathBuf, String)> = Vec::new();
    for path in paths {
        let resolved = resolve_path(ctx, Some(path.as_path()));
        let Some(record) = find_record(&records, &resolved) else {
            return Err(format!(
                "workspace '{}' is not registered in the history list",
                resolved.display()
            ));
        };
        if !order.iter().any(|(existing, _)| existing == &record.path) {
            order.push((record.path.clone(), record.name.clone()));
        }
    }
    // 未显式列出的工作区保持当前相对顺序追加，避免 position 重复或记录丢失。
    for record in &records {
        if !order.iter().any(|(existing, _)| existing == &record.path) {
            order.push((record.path.clone(), record.name.clone()));
        }
    }
    db.save_workspace_order(&order)
        .map_err(|e| format!("failed to save workspace order: {}", e.code()))?;
    let value = json!({ "reordered": true, "workspaces": order.iter().map(|(path, name)| json!({ "path": path, "name": name })).collect::<Vec<_>>() });
    out.emit(&value, |out, writer| {
        writeln!(writer, "{} workspace order saved", out.green("OK"))?;
        for (index, (path, _)) in order.iter().enumerate() {
            writeln!(writer, "  {}. {}", index + 1, path.display())?;
        }
        Ok(())
    })
}
fn trust(
    ctx: &WorkspaceContext,
    db: &Database,
    name: Option<&str>,
    out: &Output,
) -> Result<(), String> {
    let target = ctx.checkout_path();
    let name = name
        .map(str::to_string)
        .or_else(|| file_name(&target))
        .unwrap_or_else(|| "workspace".to_string());
    db.trust_workspace(&target, &name)
        .map_err(|e| format!("failed to trust workspace: {}", e.code()))?;
    db.unhide_workspace(&target)
        .map_err(|e| format!("failed to unhide workspace: {}", e.code()))?;
    let value = json!({ "trusted": true, "path": target, "name": name });
    out.emit(&value, |out, writer| {
        writeln!(
            writer,
            "{} workspace '{}' trusted",
            out.green("OK"),
            target.display()
        )
    })
}
fn worktrees(ctx: &WorkspaceContext, out: &Output) -> Result<(), String> {
    let list = crate::git::list_worktrees(&ctx.cwd)
        .ok_or_else(|| format!("'{}' is not inside a Git repository", ctx.cwd.display()))?;
    let current_checkout = crate::git::checkout(&list, &ctx.cwd).map(|c| c.checkout_path);
    let rows = list
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            json!({
                "path": entry.path, "branch": entry.branch, "head": short_head(&entry.path),
                "bare": entry.is_bare, "main": index == 0,
                "current": current_checkout.as_deref() == Some(entry.path.as_path()),
            })
        })
        .collect::<Vec<_>>();
    let value = json!({ "repo_root": list.repo_root, "worktrees": &rows });
    out.emit(&value, |out, writer| {
        let table = rows
            .iter()
            .map(|row| {
                vec![
                    row["path"].as_str().unwrap_or("-").to_string(),
                    row["branch"].as_str().unwrap_or("-").to_string(),
                    row["head"].as_str().unwrap_or("-").to_string(),
                    row["bare"].to_string(),
                    row["main"].to_string(),
                    row["current"].to_string(),
                ]
            })
            .collect::<Vec<_>>();
        out.print_table(
            writer,
            &["PATH", "BRANCH", "HEAD", "BARE", "MAIN", "CURRENT"],
            &table,
        )
    })
}

fn short_head(path: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|sha| sha.trim().to_string())
        .filter(|sha| !sha.is_empty())
        .unwrap_or_else(|| "-".to_string())
}

fn resolve_path(ctx: &WorkspaceContext, path: Option<&Path>) -> PathBuf {
    let target = match path {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => ctx.cwd.join(path),
        None => ctx.checkout_path(),
    };
    std::fs::canonicalize(&target).unwrap_or(target)
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

fn find_record<'a>(records: &'a [WorkspaceRecord], path: &Path) -> Option<&'a WorkspaceRecord> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    records.iter().find(|record| {
        record.path == path || std::fs::canonicalize(&record.path).is_ok_and(|c| c == target)
    })
}

fn is_hidden(hidden: &HashSet<PathBuf>, path: &Path) -> bool {
    hidden.contains(path) || std::fs::canonicalize(path).is_ok_and(|c| hidden.contains(&c))
}

/// 与 `Database::open_default` 相同的路径解析（含 `LX_TUI_DB_PATH` 覆盖）。
fn database_file(ctx: &WorkspaceContext) -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(overridden) = crate::platform::test_db_override() {
        return Ok(overridden);
    }
    if let Some(overridden) = std::env::var_os("LX_TUI_DB_PATH").filter(|path| !path.is_empty()) {
        return Ok(PathBuf::from(overridden));
    }
    ctx.db_path()
}

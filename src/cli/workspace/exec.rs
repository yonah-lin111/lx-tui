//! `lx-tui workspace` 子命令执行与历史列表操作。

use super::*;

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

fn list(db: &Database, raw: bool, search: Option<&str>, out: &Output) -> Result<(), String> {
    if raw {
        let records = db
            .list_workspaces()
            .map_err(|e| format!("failed to list workspaces: {}", e.code()))?;
        let records = records
            .into_iter()
            .filter(|r| matches_search(&r.name, &r.path, search))
            .collect::<Vec<_>>();
        let rows = records.iter().map(|r| json!({ "name": r.name, "path": r.path, "position": r.position, "trusted": r.trusted })).collect::<Vec<_>>();
        let value = json!({ "raw": true, "workspaces": &rows });
        return out.emit(&value, |out, writer| {
            let table = rows
                .iter()
                .map(|r| {
                    vec![
                        r["name"].as_str().unwrap_or("-").to_string(),
                        r["path"].as_str().unwrap_or("-").to_string(),
                        r["position"].to_string(),
                        r["trusted"].to_string(),
                    ]
                })
                .collect::<Vec<_>>();
            out.print_table(writer, &["NAME", "PATH", "POSITION", "TRUSTED"], &table)
        });
    }
    let entries = db
        .list_workspace_history_entries()
        .map_err(|e| format!("failed to list workspace history: {}", e.code()))?;
    let entries = entries
        .into_iter()
        .filter(|entry| {
            matches_search(&entry.name, &entry.path, search)
                || entry.git.as_ref().is_some_and(|g| {
                    matches_search("", &g.repo_root, search)
                        || g.branch.as_deref().is_some_and(|b| {
                            search.is_some_and(|q| b.to_lowercase().contains(&q.to_lowercase()))
                        })
                })
        })
        .collect::<Vec<_>>();
    let rows = entries.iter().map(|e| json!({
        "name": e.name, "display_name": e.display_label(false), "path": e.path, "exists": e.exists,
        "name_is_manual": e.name_is_manual, "position": e.position, "trusted": e.trusted,
        "git": e.git.as_ref().map(|g| json!({ "repo_root": g.repo_root, "checkout_path": g.checkout_path, "is_linked": g.is_linked, "branch": g.branch, "main_branch": g.main_branch })),
    })).collect::<Vec<_>>();
    let value = json!({ "raw": false, "workspaces": &rows });
    out.emit(&value, |out, writer| {
        if entries.is_empty() {
            return writeln!(writer, "{}", out.muted("No workspaces found."));
        }
        out.print_table(
            writer,
            &[
                "WORKSPACE",
                "PATH",
                "BRANCH",
                "EXISTS",
                "TRUSTED",
                "POSITION",
            ],
            &history_rows(&entries),
        )
    })
}

fn matches_search(name: &str, path: &Path, search: Option<&str>) -> bool {
    let Some(query) = search else { return true };
    let query = query.to_lowercase();
    name.to_lowercase().contains(&query) || path.to_string_lossy().to_lowercase().contains(&query)
}

/// 历史列表树形排版：Git 主 checkout 父项 + linked worktree 子项。
fn history_rows(entries: &[crate::app::state::WorkspaceHistoryEntry]) -> Vec<Vec<String>> {
    let mut groups: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let key = entry
            .git
            .as_ref()
            .map(|g| g.repo_root.clone())
            .unwrap_or_else(|| entry.path.clone());
        match groups.iter().position(|(existing, _)| *existing == key) {
            Some(group) => groups[group].1.push(index),
            None => groups.push((key, vec![index])),
        }
    }
    let mut rows = Vec::new();
    for (repo_root, indices) in groups {
        let parent = entries
            .iter()
            .position(|e| e.path == repo_root)
            .filter(|i| indices.contains(i));
        let mut children: Vec<usize> = indices.into_iter().filter(|i| Some(*i) != parent).collect();
        if let Some(parent) = parent {
            rows.push(entry_row(&entries[parent], None));
            let count = children.len();
            for (position, index) in children.drain(..).enumerate() {
                rows.push(entry_row(
                    &entries[index],
                    Some(if position + 1 == count {
                        "└─ "
                    } else {
                        "├─ "
                    }),
                ));
            }
        } else {
            rows.extend(
                children
                    .into_iter()
                    .map(|index| entry_row(&entries[index], None)),
            );
        }
    }
    rows
}
fn entry_row(
    entry: &crate::app::state::WorkspaceHistoryEntry,
    connector: Option<&str>,
) -> Vec<String> {
    let label = match connector {
        Some(prefix) => format!("{prefix}{}", entry.display_label(true)),
        None => entry.name.clone(),
    };
    let branch = entry
        .git
        .as_ref()
        .and_then(|g| g.short_branch())
        .unwrap_or("-")
        .to_string();
    vec![
        label,
        entry.path.display().to_string(),
        branch,
        entry.exists.to_string(),
        entry.trusted.to_string(),
        entry.position.to_string(),
    ]
}
fn status(ctx: &WorkspaceContext, db: &Database, out: &Output) -> Result<(), String> {
    let records = db
        .list_workspaces()
        .map_err(|e| format!("failed to list workspaces: {}", e.code()))?;
    let record = find_record(&records, &ctx.workspace);
    let hidden = db
        .hidden_workspace_paths()
        .map_err(|e| format!("failed to read hidden workspaces: {}", e.code()))?;
    let hidden = is_hidden(&hidden, &ctx.workspace);
    let trusted = record
        .map(|r| r.trusted)
        .unwrap_or_else(|| db.is_trusted(&ctx.workspace).unwrap_or(false));
    let git =
        crate::git::list_worktrees(&ctx.cwd).and_then(|list| crate::git::checkout(&list, &ctx.cwd));
    let db_path = database_file(ctx)?;
    let value = json!({
        "path": ctx.workspace, "cwd": ctx.cwd, "name": record.map(|r| r.name.clone()),
        "position": record.map(|r| r.position), "trusted": trusted, "hidden": hidden,
        "exists": ctx.workspace.is_dir(),
        "git": git.as_ref().map(|c| json!({ "repo_root": c.repo_root, "checkout_path": c.checkout_path, "is_linked": c.is_linked, "branch": c.branch, "main_branch": c.main_branch })),
        "database": db_path,
    });
    out.emit(&value, |out, writer| {
        writeln!(
            writer,
            "{} {}",
            out.bold("Workspace:"),
            ctx.workspace.display()
        )?;
        writeln!(
            writer,
            "{} {}",
            out.bold("Name:"),
            record.map(|r| r.name.as_str()).unwrap_or("-")
        )?;
        writeln!(writer, "{} {}", out.bold("Trusted:"), trusted)?;
        writeln!(writer, "{} {}", out.bold("Hidden:"), hidden)?;
        writeln!(writer, "{} {}", out.bold("Exists:"), ctx.workspace.is_dir())?;
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
    let value = json!({ "added": true, "path": target, "name": saved, "trusted": trust });
    out.emit(&value, |out, writer| {
        writeln!(
            writer,
            "{} workspace '{}' registered as '{}'{}",
            out.green("OK"),
            target.display(),
            saved,
            if trust { " (trusted)" } else { "" }
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
    // 已登记的 linked worktree 优先重命名 cwd；否则重命名仓库主工作区。
    let target = if ctx.cwd != ctx.workspace && find_record(&records, &ctx.cwd).is_some() {
        ctx.cwd.clone()
    } else {
        ctx.workspace.clone()
    };
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
        return Err(format!(
            "workspace '{}' is not registered in the history list",
            target.display()
        ));
    }
    let mut targets = vec![target.clone()];
    if include_worktrees && !target.join(".git").is_file() {
        targets.extend(linked_worktrees(&target, &records));
    }
    let (mut deleted, mut hidden) = (Vec::new(), Vec::new());
    for candidate in targets {
        let _guards = ensure_no_active_sessions(db, &candidate)?;
        if candidate.join(".git").is_file() {
            db.hide_workspace(&candidate)
                .map_err(|e| format!("failed to hide workspace: {}", e.code()))?;
            hidden.push(candidate.clone());
        }
        db.delete_workspace(&candidate)
            .map_err(|e| format!("failed to delete workspace: {}", e.code()))?;
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
fn linked_worktrees(target: &Path, records: &[WorkspaceRecord]) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = crate::git::list_worktrees(target)
        .map(|list| {
            list.entries
                .into_iter()
                .map(|entry| entry.path)
                .filter(|path| path != target)
                .collect()
        })
        .unwrap_or_default();
    for record in records {
        if record.path.as_path() != target
            && record.path.starts_with(target)
            && record.path.join(".git").is_file()
        {
            candidates.push(record.path.clone());
        }
    }
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// 目标工作区存在运行中的会话时拒绝删除；返回的 guard 必须活到删除完成。
fn ensure_no_active_sessions(db: &Database, workspace: &Path) -> Result<Vec<FileGuard>, String> {
    let mut guards = Vec::new();
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
        }
        if count < 200 {
            break;
        }
        offset += count;
    }
    Ok(guards)
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
    let order = paths
        .iter()
        .map(|path| {
            let resolved = resolve_path(ctx, Some(path.as_path()));
            let name = find_record(&records, &resolved)
                .map(|r| r.name.clone())
                .or_else(|| file_name(&resolved))
                .unwrap_or_else(|| "workspace".to_string());
            (resolved, name)
        })
        .collect::<Vec<_>>();
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
    let target = ctx.workspace.clone();
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
    let current = std::fs::canonicalize(&ctx.cwd).unwrap_or_else(|_| ctx.cwd.clone());
    let rows = list
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            json!({
                "path": entry.path, "branch": entry.branch, "bare": entry.is_bare,
                "main": index == 0, "current": current.starts_with(&entry.path),
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
                    row["bare"].to_string(),
                    row["main"].to_string(),
                    row["current"].to_string(),
                ]
            })
            .collect::<Vec<_>>();
        out.print_table(
            writer,
            &["PATH", "BRANCH", "BARE", "MAIN", "CURRENT"],
            &table,
        )
    })
}

fn resolve_path(ctx: &WorkspaceContext, path: Option<&Path>) -> PathBuf {
    let target = match path {
        Some(path) if path.is_absolute() => path.to_path_buf(),
        Some(path) => ctx.cwd.join(path),
        None => ctx.workspace.clone(),
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

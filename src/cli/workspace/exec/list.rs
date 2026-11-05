//! `workspace list`：树形历史列表、`--raw` 扁平表与搜索过滤。

use super::*;

pub(super) fn run(
    db: &Database,
    raw: bool,
    search: Option<&str>,
    out: &Output,
) -> Result<(), String> {
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

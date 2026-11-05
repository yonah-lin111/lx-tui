//! 工作区表（`workspace`）读写：信任状态、历史记录、重命名与级联删除。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, params};

use super::{Database, DbError, Result, new_external_id, path_string};
use crate::app::state::{WorkspaceGit, WorkspaceHistoryEntry, home_dir, workspace_label};

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

    /// 组装工作区历史列表（含 Git 仓库主工作区归位、全部 `git worktree` 子节点合并与历史命名污染自愈）。
    pub fn list_workspace_history_entries(&self) -> Result<Vec<WorkspaceHistoryEntry>> {
        let records = self.list_workspaces()?;
        let home = home_dir();
        let home_ref = home.as_deref();

        let mut db_names: HashMap<PathBuf, String> = HashMap::new();
        for record in &records {
            db_names
                .entry(record.path.clone())
                .or_insert_with(|| record.name.clone());
            if let Ok(canon) = std::fs::canonicalize(&record.path) {
                db_names.entry(canon).or_insert_with(|| record.name.clone());
            }
        }

        let mut repo_lists: HashMap<PathBuf, crate::git::WorktreeList> = HashMap::new();
        let mut path_to_repo: HashMap<PathBuf, PathBuf> = HashMap::new();
        for record in &records {
            if path_to_repo.contains_key(&record.path) || !record.path.is_dir() {
                continue;
            }
            if let Some(list) = crate::git::list_worktrees(&record.path) {
                let root = list.repo_root.clone();
                path_to_repo.insert(record.path.clone(), root.clone());
                path_to_repo.insert(root.clone(), root.clone());
                for wt in &list.entries {
                    path_to_repo.insert(wt.path.clone(), root.clone());
                }
                repo_lists.entry(root).or_insert(list);
            }
        }
        // 已删除的历史 worktree 路径：若挂在已知 Git 仓库根路径之下，归入该仓库分组。
        let known_roots: Vec<PathBuf> = repo_lists.keys().cloned().collect();
        for record in &records {
            if path_to_repo.contains_key(&record.path) {
                continue;
            }
            if let Some(root) = known_roots
                .iter()
                .filter(|root| record.path != **root && record.path.starts_with(root))
                .max_by_key(|root| root.as_os_str().len())
            {
                path_to_repo.insert(record.path.clone(), root.clone());
            }
        }

        let mut seen_repos: HashSet<PathBuf> = HashSet::new();
        let mut seen_paths: HashSet<PathBuf> = HashSet::new();
        let mut out = Vec::with_capacity(records.len());

        for record in &records {
            if let Some(repo_root) = path_to_repo.get(&record.path).cloned() {
                if !seen_repos.insert(repo_root.clone()) {
                    continue;
                }
                let Some(wt_list) = repo_lists.get(&repo_root) else {
                    continue;
                };
                let main_branch = wt_list
                    .entries
                    .first()
                    .and_then(|entry| entry.branch.clone());
                let default_repo_name = workspace_label(&repo_root, home_ref);
                let is_polluted = |saved: &str| -> bool {
                    saved != default_repo_name
                        && (wt_list
                            .entries
                            .iter()
                            .skip(1)
                            .any(|wt| workspace_label(&wt.path, home_ref) == saved)
                            || records.iter().any(|r| {
                                r.path != repo_root
                                    && r.path.starts_with(&repo_root)
                                    && workspace_label(&r.path, home_ref) == saved
                            }))
                };
                let (repo_name, repo_manual) = match db_names.get(&repo_root) {
                    Some(saved) if is_polluted(saved) => {
                        let _ = self.rename_workspace(&repo_root, &default_repo_name);
                        (default_repo_name, false)
                    }
                    Some(saved) if !saved.trim().is_empty() => {
                        let manual = saved != &default_repo_name;
                        (saved.clone(), manual)
                    }
                    _ => (default_repo_name, false),
                };
                seen_paths.insert(repo_root.clone());
                out.push(WorkspaceHistoryEntry {
                    name: repo_name,
                    path: repo_root.clone(),
                    exists: repo_root.is_dir(),
                    name_is_manual: repo_manual,
                    git: Some(WorkspaceGit {
                        repo_root: repo_root.clone(),
                        checkout_path: repo_root.clone(),
                        is_linked: false,
                        branch: main_branch.clone(),
                        main_branch: main_branch.clone(),
                    }),
                });

                for wt in wt_list.entries.iter().skip(1) {
                    if wt.is_bare || wt.path == repo_root {
                        continue;
                    }
                    seen_paths.insert(wt.path.clone());
                    let default_wt_name = workspace_label(&wt.path, home_ref);
                    let (wt_name, wt_manual) = match db_names
                        .get(&wt.path)
                        .filter(|saved| !saved.trim().is_empty())
                    {
                        Some(saved) => (saved.clone(), saved != &default_wt_name),
                        None => (default_wt_name, false),
                    };
                    out.push(WorkspaceHistoryEntry {
                        name: wt_name,
                        path: wt.path.clone(),
                        exists: wt.path.is_dir(),
                        name_is_manual: wt_manual,
                        git: Some(WorkspaceGit {
                            repo_root: repo_root.clone(),
                            checkout_path: wt.path.clone(),
                            is_linked: true,
                            branch: wt.branch.clone(),
                            main_branch: main_branch.clone(),
                        }),
                    });
                }

                for r in &records {
                    let canon_r = std::fs::canonicalize(&r.path).unwrap_or_else(|_| r.path.clone());
                    if path_to_repo.get(&r.path) == Some(&repo_root)
                        && !seen_paths.contains(&r.path)
                        && !seen_paths.contains(&canon_r)
                    {
                        seen_paths.insert(r.path.clone());
                        let default_wt_name = workspace_label(&r.path, home_ref);
                        let wt_manual = r.name != default_wt_name && !r.name.trim().is_empty();
                        out.push(WorkspaceHistoryEntry {
                            name: r.name.clone(),
                            path: r.path.clone(),
                            exists: r.path.is_dir(),
                            name_is_manual: wt_manual,
                            git: Some(WorkspaceGit {
                                repo_root: repo_root.clone(),
                                checkout_path: r.path.clone(),
                                is_linked: true,
                                branch: None,
                                main_branch: main_branch.clone(),
                            }),
                        });
                    }
                }
            } else {
                let canon =
                    std::fs::canonicalize(&record.path).unwrap_or_else(|_| record.path.clone());
                if !seen_paths.insert(record.path.clone())
                    || (canon != record.path && !seen_paths.insert(canon))
                {
                    continue;
                }
                let default_name = workspace_label(&record.path, home_ref);
                let name_is_manual = record.name != default_name && !record.name.trim().is_empty();
                out.push(WorkspaceHistoryEntry {
                    name: record.name.clone(),
                    path: record.path.clone(),
                    exists: record.path.is_dir(),
                    name_is_manual,
                    git: None,
                });
            }
        }
        Ok(out)
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

    #[test]
    fn list_workspace_history_entries_discovers_git_worktrees_and_heals_main_repo_name() {
        use std::process::Command;

        if !Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|out| out.status.success())
        {
            return;
        }
        let base = std::env::temp_dir().join(format!(
            "lx-ws-git-tree-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let repo = base.join("lx-tui");
        let wt1 = repo.join(".worktrees").join("workspace-history-list");
        std::fs::create_dir_all(&repo).expect("repo dir");

        let run_git = |args: &[&str]| {
            let out = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(args)
                .output()
                .expect("run git");
            assert!(out.status.success());
        };
        run_git(&["init", "--quiet", "-b", "dev"]);
        run_git(&[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=Test",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "init",
        ]);
        run_git(&[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feature/workspace-history-list",
            wt1.to_str().expect("utf8"),
        ]);

        let canon_repo = std::fs::canonicalize(&repo).expect("canon repo");
        let canon_wt1 = std::fs::canonicalize(&wt1).expect("canon wt1");

        let db = Database::open_in_memory().expect("in-memory db");
        // 模拟旧版 startup_trust 将主工作区路径错误写入子 worktree 名字，并登记子 worktree 与已删除历史 worktree。
        db.trust_workspace(&canon_repo, "workspace-history-list")
            .expect("trust polluted");
        db.record_workspace_open(&canon_wt1, "workspace-history-list")
            .expect("record wt1");
        let deleted_wt = canon_repo.join(".worktrees").join("deleted-feature");
        db.record_workspace_open(&deleted_wt, "deleted-feature")
            .expect("record deleted wt");

        let entries = db
            .list_workspace_history_entries()
            .expect("history entries");
        assert_eq!(entries.len(), 3);
        // 父节点必须恢复为主工作区名称 lx-tui（而非子工作区名称 workspace-history-list）。
        assert_eq!(entries[0].path, canon_repo);
        assert_eq!(entries[0].name, "lx-tui");
        assert!(!entries[0].name_is_manual);
        assert!(entries[0].git.as_ref().is_some_and(|g| !g.is_linked));

        // 实时扫描到的 git worktree 子节点：
        assert_eq!(entries[1].path, canon_wt1);
        assert!(entries[1].exists);
        assert!(entries[1].git.as_ref().is_some_and(|g| g.is_linked));
        assert_eq!(
            entries[1].display_label(true),
            "feature/workspace-history-list"
        );

        // 已删除的历史 worktree 子节点（标红且归属主仓库之下）：
        assert_eq!(entries[2].path, deleted_wt);
        assert!(!entries[2].exists);
        assert!(entries[2].git.as_ref().is_some_and(|g| g.is_linked));

        let _ = std::fs::remove_dir_all(&base);
    }
}

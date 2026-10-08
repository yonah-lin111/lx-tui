//! Git 查询：`git worktree list --porcelain` 的执行与解析。
//!
//! 唯一执行 git 命令的模块；由事件层在后台线程调用，app 只接收转换后的纯数据。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 一条 worktree 记录；`path` 已尽力规范化（canonicalize）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    /// 分支短名（`refs/heads/` 前缀已剥离）；detached 或 bare 为 None。
    pub branch: Option<String>,
    pub is_bare: bool,
}

/// 一个仓库的全部 worktree；首项为主 checkout（git 保证输出顺序）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeList {
    pub repo_root: PathBuf,
    pub entries: Vec<WorktreeEntry>,
}

/// cwd 所在 checkout 的元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub repo_root: PathBuf,
    pub checkout_path: PathBuf,
    pub is_linked: bool,
    pub branch: Option<String>,
    /// 仓库主 checkout（列表首项）的分支；linked worktree 的状态栏展示用。
    pub main_branch: Option<String>,
}

/// 查询 cwd 所属仓库的全部 worktree；非仓库、git 缺失或命令失败返回 None。
pub fn list_worktrees(cwd: &Path) -> Option<WorktreeList> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let mut entries = parse_worktree_list(&text);
    for entry in &mut entries {
        entry.path = canonical_or_original(&entry.path);
    }
    let repo_root = entries.first()?.path.clone();
    Some(WorktreeList { repo_root, entries })
}

/// cwd 落在列表中的哪个 checkout；取路径前缀最长者（防御嵌套）。
pub fn checkout(list: &WorktreeList, cwd: &Path) -> Option<Checkout> {
    let mut best: Option<(usize, &WorktreeEntry)> = None;
    for (index, entry) in list.entries.iter().enumerate() {
        if !cwd.starts_with(&entry.path) {
            continue;
        }
        let longer = best.is_none_or(|(_, current)| {
            entry.path.as_os_str().len() > current.path.as_os_str().len()
        });
        if longer {
            best = Some((index, entry));
        }
    }
    let (index, entry) = best?;
    Some(Checkout {
        repo_root: list.repo_root.clone(),
        checkout_path: entry.path.clone(),
        is_linked: index > 0,
        branch: entry.branch.clone(),
        main_branch: list.entries.first().and_then(|main| main.branch.clone()),
    })
}

/// 解析 porcelain 输出；空行分隔记录，路径保持原文。
pub fn parse_worktree_list(text: &str) -> Vec<WorktreeEntry> {
    let mut entries: Vec<WorktreeEntry> = Vec::new();
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            entries.push(WorktreeEntry {
                path: PathBuf::from(path),
                branch: None,
                is_bare: false,
            });
            continue;
        }
        let Some(entry) = entries.last_mut() else {
            continue;
        };
        if let Some(reference) = line.strip_prefix("branch ") {
            entry.branch = reference
                .strip_prefix("refs/heads/")
                .map(str::to_string)
                .or_else(|| (!reference.is_empty()).then(|| reference.to_string()));
        } else if line == "bare" {
            entry.is_bare = true;
        }
    }
    entries
}

/// 尽力规范化路径；失败保留原路径。
fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests;

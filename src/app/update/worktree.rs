//! worktree 相关行为：git 元数据查询登记、打开已有 worktree 对话框与工作区创建/切换。

use std::path::{Path, PathBuf};

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::overlay::{Overlay, TextInput, WorktreeOpen, WorktreeOpenEntry};
use crate::app::state::{
    AppState, FocusedContainer, Workspace, WorkspaceGit, home_dir, unique_workspace_name,
    workspace_label,
};
use crate::app::trust::TrustTrigger;
use crate::git;

use super::{clear_selection, swap_display_prompt, switch_workspace};

/// 登记一次工作区 git 元数据查询；同一路径去重。
pub fn request_git_refresh(state: &mut AppState, cwd: &Path) {
    if state.git_requests.iter().any(|pending| pending == cwd) {
        return;
    }
    state.git_requests.push(cwd.to_path_buf());
}

/// 取走待查询的 cwd 列表；事件层在后台执行后回投结果。
pub fn take_git_requests(state: &mut AppState) -> Vec<PathBuf> {
    std::mem::take(&mut state.git_requests)
}

/// worktree 列表结果 → prompt 绑定候选：排除 bare 与 detached（无分支）项。
///
/// 主 checkout 为首项（git 输出顺序），linked 条目按原顺序保留；同名分支由
/// 解析侧判为歧义。
pub fn worktree_candidates(
    entries: Vec<git::WorktreeEntry>,
) -> Vec<crate::app::markdown::WorktreeCandidate> {
    entries
        .into_iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            let branch = entry.branch?;
            if entry.is_bare {
                return None;
            }
            Some(crate::app::markdown::WorktreeCandidate {
                branch,
                path: entry.path,
                is_main: index == 0,
            })
        })
        .collect()
}

/// 查询结果落地：cwd 与工作区当前 cwd 精确匹配时更新元数据（可能命中多个）；
/// 工作区首次识别为 linked worktree 时自动补开主 checkout，保持树形结构。
pub fn apply_git_refresh(state: &mut AppState, cwd: &Path, git: Option<WorkspaceGit>) {
    let mut first_linked = Vec::new();
    for (index, workspace) in state.workspaces.iter_mut().enumerate() {
        if workspace.cwd.as_deref() != Some(cwd) {
            continue;
        }
        let first_metadata = workspace.git.is_none();
        workspace.git = git.clone();
        if first_metadata && git.as_ref().is_some_and(|git| git.is_linked) {
            first_linked.push(index);
        }
    }
    for index in first_linked {
        ensure_main_workspace(state, index);
    }
}

/// 若 linked 工作区所属仓库的主 checkout 尚未打开，自动在其仓库首个成员前方插入主工作区，
/// 保证父项始终位于全部子项之前；激活索引随插入平移。返回插入的主工作区索引。
pub fn ensure_main_workspace(state: &mut AppState, index: usize) -> Option<usize> {
    let child = state.workspaces.get(index)?;
    let git = child.git.as_ref()?;
    if !git.is_linked {
        return None;
    }
    let repo_root = git.repo_root.clone();
    if open_workspace_for_checkout(state, &repo_root).is_some() {
        return None;
    }
    let main_branch = git.main_branch.clone().or_else(|| git.branch.clone());
    let insert_at = state
        .workspaces
        .iter()
        .position(|workspace| {
            workspace
                .git
                .as_ref()
                .is_some_and(|git| git.repo_root == repo_root)
        })
        .unwrap_or(index);
    let base = workspace_label(&repo_root, home_dir().as_deref());
    let name = unique_workspace_name(&base, |candidate| {
        state
            .workspaces
            .iter()
            .any(|workspace| workspace.name == candidate)
    });
    let mut workspace = Workspace::single_terminal(name, Some(repo_root.clone()));
    workspace.git = Some(WorkspaceGit {
        repo_root: repo_root.clone(),
        checkout_path: repo_root,
        is_linked: false,
        branch: main_branch.clone(),
        main_branch,
    });
    state.workspaces.insert(insert_at, workspace);
    if state.active_workspace >= insert_at {
        state.active_workspace += 1;
    }
    if let Some(pinned) = state.prompt_pinned
        && pinned >= insert_at
    {
        state.prompt_pinned = Some(pinned + 1);
    }
    Some(insert_at)
}

/// 打开已有 worktree 对话框（loading 态）；源工作区必须已有 git 元数据。
pub fn open_worktree_dialog(state: &mut AppState, source: usize) {
    let Some(workspace) = state.workspaces.get(source) else {
        return;
    };
    let Some(git) = workspace.git.as_ref() else {
        return;
    };
    // 打开模态时结束可能残留的滚动条拖拽。
    state.workspace_scroll_drag = None;
    state.overlay = Some(Overlay::WorktreeOpen(WorktreeOpen {
        source,
        repo_root: git.repo_root.clone(),
        entries: Vec::new(),
        selected: 0,
        query: TextInput::new(""),
        loading: true,
        failed: false,
    }));
}

/// 列表结果落地：填充对话框条目并标记已打开项；对话框已关闭或换了仓库时忽略。
pub fn apply_worktree_list(
    state: &mut AppState,
    repo_root: &Path,
    entries: Vec<git::WorktreeEntry>,
    failed: bool,
) {
    let targeted = matches!(
        &state.overlay,
        Some(Overlay::WorktreeOpen(dialog)) if dialog.repo_root == repo_root
    );
    if !targeted {
        return;
    }
    let converted: Vec<WorktreeOpenEntry> = entries
        .into_iter()
        .enumerate()
        .map(|(index, entry)| WorktreeOpenEntry {
            already_open: open_workspace_for_checkout(state, &entry.path),
            path: entry.path,
            branch: entry.branch,
            is_bare: entry.is_bare,
            is_linked: index > 0,
        })
        .collect();
    if let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_mut() {
        dialog.entries = converted;
        dialog.selected = 0;
        dialog.loading = false;
        dialog.failed = failed;
        dialog.normalize_selection();
    }
}

/// 对话框按键：编辑搜索、移动选择、打开或关闭。
pub fn apply_worktree_open_key(state: &mut AppState, key: OverlayKey) {
    match key {
        OverlayKey::Esc => state.overlay = None,
        OverlayKey::Enter => commit_worktree_open(state),
        _ => {
            let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_mut() else {
                return;
            };
            match key {
                OverlayKey::Up => dialog.select_previous(),
                OverlayKey::Down => dialog.select_next(),
                OverlayKey::Char(ch) => {
                    dialog.query.insert_char(ch);
                    dialog.normalize_selection();
                }
                OverlayKey::Clear => {
                    dialog.query.clear();
                    dialog.normalize_selection();
                }
                OverlayKey::Backspace => {
                    dialog.query.backspace();
                    dialog.normalize_selection();
                }
                OverlayKey::Delete => {
                    dialog.query.delete();
                    dialog.normalize_selection();
                }
                OverlayKey::Left => dialog.query.move_left(),
                OverlayKey::Right => dialog.query.move_right(),
                OverlayKey::Home => dialog.query.move_home(),
                OverlayKey::End => dialog.query.move_end(),
                OverlayKey::Esc
                | OverlayKey::Enter
                | OverlayKey::EnterFolder
                | OverlayKey::Back
                | OverlayKey::Copy
                | OverlayKey::Remove
                | OverlayKey::PageUp
                | OverlayKey::PageDown => {}
            }
        }
    }
}

/// 悬停/点击条目：设置对话框高亮；越界忽略；返回是否变化。
pub fn set_worktree_open_selection(state: &mut AppState, index: usize) -> bool {
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_mut() else {
        return false;
    };
    if index >= dialog.entries.len() || dialog.selected == index {
        return false;
    }
    dialog.selected = index;
    true
}

/// 打开选中项：已在工作区中打开则直接切换（无信任检查）；否则先对仓库主 checkout
/// 做信任检查，通过后创建新工作区并激活；打开的若是 linked worktree 且主 checkout
/// 未打开，自动补开主工作区。
pub fn commit_worktree_open(state: &mut AppState) {
    let Some(Overlay::WorktreeOpen(dialog)) = state.overlay.as_ref() else {
        return;
    };
    let Some(entry_index) = dialog.selected_entry_index() else {
        return;
    };
    let Some(entry) = dialog.entries.get(entry_index).cloned() else {
        return;
    };
    let repo_root = dialog.repo_root.clone();
    let source = dialog.source;
    state.overlay = None;

    if let Some(index) = open_workspace_for_checkout(state, &entry.path) {
        switch_workspace(state, index);
        ensure_main_workspace(state, index);
        return;
    }
    // 源工作区的元数据带有仓库主 checkout 分支，供 linked worktree 的状态栏展示。
    let main_branch = state
        .workspaces
        .get(source)
        .and_then(|workspace| workspace.git.as_ref())
        .and_then(|git| git.main_branch.clone());
    state.effects.push(Effect::CheckTrust {
        path: entry.path.clone(),
        root: Some(repo_root.clone()),
        trigger: TrustTrigger::OpenWorktree {
            entry,
            repo_root,
            main_branch,
        },
    });
}

/// 信任通过后打开 worktree checkout：按 checkout 创建新工作区并激活；
/// 打开的若是 linked worktree 且主 checkout 未打开，自动补开主工作区。
pub(super) fn open_worktree_checkout(
    state: &mut AppState,
    entry: WorktreeOpenEntry,
    repo_root: PathBuf,
    main_branch: Option<String>,
) {
    if let Some(index) = open_workspace_for_checkout(state, &entry.path) {
        switch_workspace(state, index);
        ensure_main_workspace(state, index);
        return;
    }
    let base = workspace_label(&entry.path, home_dir().as_deref());
    let name = unique_workspace_name(&base, |candidate| {
        state
            .workspaces
            .iter()
            .any(|workspace| workspace.name == candidate)
    });
    let mut workspace = Workspace::single_terminal(name, Some(entry.path.clone()));
    workspace.git = Some(WorkspaceGit {
        repo_root,
        checkout_path: entry.path.clone(),
        is_linked: entry.is_linked,
        branch: entry.branch.clone(),
        main_branch,
    });
    state.workspaces.push(workspace);
    let child = state.workspaces.len().saturating_sub(1);
    let from = state.active_workspace;
    state.active_workspace = child;
    if state.prompt_pinned.is_none() {
        swap_display_prompt(state, from, child);
    }
    clear_selection(state);
    state.set_focus(FocusedContainer::Panes);
    ensure_main_workspace(state, child);
}

/// 切换工作区分组折叠：按仓库根路径记录；仅组父项调用。
pub fn toggle_workspace_group(state: &mut AppState, index: usize) {
    let Some(workspace) = state.workspaces.get(index) else {
        return;
    };
    let Some(git) = workspace.git.as_ref() else {
        return;
    };
    let key = git.repo_root.clone();
    if let Some(position) = state
        .collapsed_groups
        .iter()
        .position(|group| *group == key)
    {
        state.collapsed_groups.remove(position);
    } else {
        state.collapsed_groups.push(key);
    }
}

/// 已打开为工作区的 checkout 索引：优先按 git 元数据匹配，未查询到时按 cwd 兜底。
fn open_workspace_for_checkout(state: &AppState, path: &Path) -> Option<usize> {
    state.workspaces.iter().position(|workspace| {
        workspace
            .git
            .as_ref()
            .is_some_and(|git| git.checkout_path == path)
            || workspace.cwd.as_deref() == Some(path)
    })
}

#[cfg(test)]
mod candidate_tests {
    use super::*;

    /// 构造 worktree 记录。
    fn entry(path: &str, branch: Option<&str>, bare: bool) -> git::WorktreeEntry {
        git::WorktreeEntry {
            path: PathBuf::from(path),
            branch: branch.map(str::to_string),
            is_bare: bare,
        }
    }

    #[test]
    fn candidates_exclude_detached_and_bare_and_keep_order() {
        let candidates = worktree_candidates(vec![
            entry("/repo", Some("main"), false),
            entry("/repo/.wt/detached", None, false),
            entry("/repo/.wt/feat", Some("feature/x"), false),
            entry("/repo/bare.git", Some("bare-branch"), true),
        ]);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].branch, "main");
        assert!(candidates[0].is_main);
        assert_eq!(candidates[1].branch, "feature/x");
        assert!(!candidates[1].is_main);
    }
}

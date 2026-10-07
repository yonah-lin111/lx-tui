//! worktree 相关行为：git 元数据查询登记、打开已有 worktree 对话框与工作区创建/切换。

use std::path::{Path, PathBuf};

use crate::app::actions::OverlayKey;
use crate::app::overlay::{Overlay, TextInput, WorktreeOpen, WorktreeOpenEntry};
use crate::app::state::{
    AppState, Workspace, WorkspaceGit, home_dir, unique_workspace_name, workspace_label,
};
use crate::git;

use super::{clear_selection, switch_workspace};

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

/// 查询结果落地：cwd 与工作区当前 cwd 精确匹配时更新元数据（可能命中多个）。
pub fn apply_git_refresh(state: &mut AppState, cwd: &Path, git: Option<WorkspaceGit>) {
    for workspace in &mut state.workspaces {
        if workspace.cwd.as_deref() == Some(cwd) {
            workspace.git = git.clone();
        }
    }
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
                OverlayKey::Esc | OverlayKey::Enter => {}
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

/// 打开选中项：已在工作区中打开则切换，否则按 checkout 创建新工作区并激活。
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
    state.overlay = None;

    if let Some(index) = open_workspace_for_checkout(state, &entry.path) {
        switch_workspace(state, index);
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
    });
    state.workspaces.push(workspace);
    state.active_workspace = state.workspaces.len().saturating_sub(1);
    clear_selection(state);
    state.prompt_focused = false;
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

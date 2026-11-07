//! 工作区历史条目级联删除：单条删除与一键清理全部失效条目。

use std::path::PathBuf;

use crate::app::effects::Effect;
use crate::app::state::AppState;
use crate::app::update::workspace::close_workspace;
use crate::app::update::{on_session_deleted, on_session_removed, on_turn_failed};

/// 级联删除单条工作区历史条目（若为主仓库则一并级联删除同仓库全部 linked worktree）。
pub fn delete_workspace_history_entry(state: &mut AppState, entry_index: usize) -> bool {
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    let cascade = state
        .workspace_history
        .cascade_linked_worktree_indices(entry_index);
    let mut paths_to_delete = Vec::with_capacity(cascade.len() + 1);
    let mut linked_paths = Vec::with_capacity(cascade.len() + 1);
    if entry.git.as_ref().is_some_and(|git| git.is_linked) {
        linked_paths.push(entry.path.clone());
    }
    paths_to_delete.push(entry.path.clone());
    for child_idx in cascade {
        if let Some(child) = state.workspace_history.entries.get(child_idx) {
            paths_to_delete.push(child.path.clone());
            linked_paths.push(child.path.clone());
        }
    }
    for path in linked_paths {
        if !state.workspace_history.hidden_worktrees.contains(&path) {
            state.workspace_history.hidden_worktrees.push(path);
        }
    }

    purge_workspace_history_paths(state, paths_to_delete);
    true
}

/// 一键级联删除确认快照中路径失效且当前仍不存在的工作区历史条目；返回实际删除的条目数。
///
/// 只处理确认弹窗展示时已标记 `!exists` 且此刻磁盘上仍不存在的条目：
/// 弹窗打开后才失效的目录不在此列（防止 TOCTOU 扩大删除集合），
/// 快照中已恢复的目录也会被跳过并刷新存在性。
/// 严格只清理磁盘上已不存在的条目本身，绝不连带删除磁盘上仍存在的同仓库主工作区或子 worktree。
pub fn delete_missing_workspace_history_entries(state: &mut AppState) -> usize {
    let missing: Vec<(PathBuf, bool)> = state
        .workspace_history
        .entries
        .iter()
        .filter(|entry| !entry.exists && !entry.path.is_dir())
        .map(|entry| {
            let linked = entry.git.as_ref().is_some_and(|git| git.is_linked);
            (entry.path.clone(), linked)
        })
        .collect();
    // 快照中已被跳过（目录恢复或弹窗后才失效）的条目同步回真实存在态。
    state.workspace_history.refresh_existence();
    if missing.is_empty() {
        return 0;
    }
    let mut paths_to_delete = Vec::with_capacity(missing.len());
    for (path, linked) in missing {
        if linked && !state.workspace_history.hidden_worktrees.contains(&path) {
            state.workspace_history.hidden_worktrees.push(path.clone());
        }
        paths_to_delete.push(path);
    }
    let count = paths_to_delete.len();
    purge_workspace_history_paths(state, paths_to_delete);
    count
}

/// 对给定的待删工作区路径集合执行统一清理：
/// 中断运行中 Agent/子 Agent/Shell、关闭已打开工作区、移除内存历史条目并派发 `Effect::DeleteWorkspaceHistory`。
fn purge_workspace_history_paths(state: &mut AppState, paths_to_delete: Vec<PathBuf>) {
    // 中断所有归属待删路径的运行中 Agent、子 Agent 与本地 Shell，并关闭已打开的对应活跃工作区。
    let matching_ws: Vec<usize> = state
        .workspaces
        .iter()
        .enumerate()
        .filter(|(_, ws)| {
            ws.cwd
                .as_ref()
                .is_some_and(|cwd| paths_to_delete.contains(cwd))
                || ws
                    .git
                    .as_ref()
                    .is_some_and(|git| paths_to_delete.contains(&git.checkout_path))
        })
        .map(|(idx, _)| idx)
        .collect();

    for &ws_idx in matching_ws.iter().rev() {
        let pane_ids: Vec<crate::layout::PaneId> = state
            .workspaces
            .get(ws_idx)
            .map(|ws| {
                ws.tabs
                    .iter()
                    .flat_map(|tab| tab.layout.pane_ids())
                    .collect()
            })
            .unwrap_or_default();
        for pane_id in pane_ids {
            state
                .effects
                .push(Effect::CancelUserShells { pane: pane_id });
            let (streaming_key, committing_gen, session_id) = state
                .pane_anywhere(pane_id)
                .map(|pane| {
                    let streaming = match &pane.lx.chat.request {
                        crate::app::state::RequestState::Streaming { key, .. } => Some(key.clone()),
                        _ => None,
                    };
                    let committing = match &pane.lx.chat.request {
                        crate::app::state::RequestState::Committing { generation, .. } => {
                            Some(*generation)
                        }
                        _ => None,
                    };
                    (streaming, committing, pane.lx.chat.session_id.clone())
                })
                .unwrap_or((None, None, None));
            if let Some(key) = streaming_key {
                state.effects.push(Effect::CancelChatStream { key });
            }
            if let Some(generation) = committing_gen {
                on_turn_failed(state, pane_id, generation);
            }
            if let Some(sid) = session_id {
                state.effects.push(Effect::CancelSubagents {
                    session_id: sid.clone(),
                    agent_id: None,
                });
                on_session_deleted(state, pane_id, &sid);
                on_session_removed(state, pane_id, &sid);
            }
        }
        if state.workspaces.len() > 1 {
            close_workspace(state, ws_idx);
        } else if let Some(ws) = state.workspaces.get_mut(ws_idx) {
            // 最后一个工作区无法关闭：清空内存中的提示词树/引用，数据库行由服务层
            // 以“已信任 + 隐藏”方式重建，避免后续保存把历史条目以未信任状态复活。
            ws.prompt_panel.tree.replace(Vec::new(), Vec::new());
            state.referenced.reset(None);
        }
    }

    state
        .workspace_history
        .entries
        .retain(|item| !paths_to_delete.contains(&item.path));
    state.workspace_history.row_click = None;
    let total = state.workspace_history.visible_rows().len();
    if total == 0 {
        state.workspace_history.selected = 0;
        state.workspace_history.scroll = 0;
    } else {
        state.workspace_history.selected = state.workspace_history.selected.min(total - 1);
        state.workspace_history.scroll =
            state.workspace_history.scroll.min(total.saturating_sub(1));
    }
    state.workspace_history.reveal_selected = true;
    for del_path in paths_to_delete {
        state
            .effects
            .push(Effect::DeleteWorkspaceHistory { path: del_path });
    }
}

#[cfg(test)]
mod tests;

//! 侧栏工作区历史列表交互：模式切换、Git 树形折叠、搜索过滤、上下导航、双击打开与失效条目级联删除。

use std::time::{Duration, Instant};

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::state::{
    AppState, Workspace, WorkspaceHistoryEntry, WorkspaceMode, home_dir, unique_workspace_name,
    workspace_label,
};
use crate::app::toast::{Toast, ToastKind};
use crate::ui::text;

use super::editor::swap_display_prompt;
use super::interaction::show_toast;
use super::selection::clear_selection;
use super::workspace::switch_workspace;
use super::worktree::{ensure_main_workspace, request_git_refresh};

/// 历史列表同行双击时间窗口（对齐 Prompt 树与页列表的 400ms）。
pub const WORKSPACE_HISTORY_DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(400);

/// 侧栏上半区是否处于工作区历史列表模式。
pub fn workspace_history_mode(state: &AppState) -> bool {
    state.workspace_history.mode == WorkspaceMode::History
}

/// 切换侧栏上半区模式（活跃工作区列表 ↔ 工作区历史列表）。
pub fn toggle_workspace_mode(state: &mut AppState) -> bool {
    match state.workspace_history.mode {
        WorkspaceMode::Active => show_workspace_history(state),
        WorkspaceMode::History => exit_workspace_history(state),
    }
}

/// 进入工作区历史列表视图：刷新路径存在性、请求数据库最新快照并聚焦侧栏搜索框。
pub fn show_workspace_history(state: &mut AppState) -> bool {
    state.workspace_drag = None;
    state.workspace_dragging = false;
    state.workspace_scroll_drag = None;
    state.workspace_hover = None;
    state.workspace_history.mode = WorkspaceMode::History;
    state.workspace_history.hover = None;
    state.workspace_history.row_click = None;
    state.workspace_history.scroll_drag = None;
    state.workspace_history.search_drag = false;
    state.workspace_history.refresh_existence();
    // 默认将选中行对齐到当前激活工作区对应的历史项（若存在）。
    if let Some(active_cwd) = state.active_workspace().cwd.as_deref() {
        let filtered = state.workspace_history.filtered_indices();
        if let Some(pos) = filtered.iter().position(|&idx| {
            state
                .workspace_history
                .entries
                .get(idx)
                .is_some_and(|entry| entry.path == active_cwd)
        }) {
            state.workspace_history.selected = pos;
        }
    }
    state.effects.push(Effect::LoadWorkspaceHistory);
    super::focus::focus_sidebar(state);
    true
}

/// 退出工作区历史列表视图，返回默认活跃工作区列表。
pub fn exit_workspace_history(state: &mut AppState) -> bool {
    if state.workspace_history.mode == WorkspaceMode::Active {
        return false;
    }
    state.workspace_history.mode = WorkspaceMode::Active;
    state.workspace_history.hover = None;
    state.workspace_history.row_click = None;
    state.workspace_history.scroll_drag = None;
    state.workspace_history.search_drag = false;
    state.workspace_history.search.clear_selection();
    true
}

/// 切换工作区历史列表中 Git 仓库父节点的折叠/展开状态。
pub fn toggle_workspace_history_group(state: &mut AppState, visible_row: usize) -> bool {
    let Some(row) = state
        .workspace_history
        .visible_rows()
        .get(visible_row)
        .copied()
    else {
        return false;
    };
    if !row.parent {
        return false;
    }
    let Some(repo_root) = state
        .workspace_history
        .entries
        .get(row.index)
        .and_then(|entry| entry.git.as_ref())
        .map(|git| git.repo_root.clone())
    else {
        return false;
    };
    state.workspace_history.toggle_repo_collapsed(&repo_root);
    state.workspace_history.row_click = None;
    let total = state.workspace_history.visible_rows().len();
    if total == 0 {
        state.workspace_history.selected = 0;
        state.workspace_history.scroll = 0;
    } else {
        state.workspace_history.selected = state.workspace_history.selected.min(total - 1);
    }
    true
}

/// 接收数据库加载的工作区历史记录并更新状态（同时将数据库保存的自定义名称回填到活跃工作区）。
pub fn apply_workspace_history(
    state: &mut AppState,
    mut new_entries: Vec<WorkspaceHistoryEntry>,
) -> bool {
    let selected_path = state
        .workspace_history
        .entry_at_visible_row(state.workspace_history.selected)
        .map(|(_, entry)| entry.path.clone());

    let mut ws_renamed = false;
    new_entries.retain(|entry| {
        !state
            .workspace_history
            .hidden_worktrees
            .contains(&entry.path)
    });
    for entry in &mut new_entries {
        entry.exists = entry.path.is_dir();
        if entry.name_is_manual {
            for ws in &mut state.workspaces {
                if !ws.name_is_manual && ws.cwd.as_deref() == Some(entry.path.as_path()) {
                    ws.name = entry.name.clone();
                    ws.name_is_manual = true;
                    ws_renamed = true;
                }
            }
        }
    }

    let entries_changed = state.workspace_history.entries != new_entries;
    let was_loaded = state.workspace_history.loaded;
    state.workspace_history.entries = new_entries;
    state.workspace_history.loaded = true;

    if let Some(target_path) = selected_path {
        let filtered = state.workspace_history.filtered_indices();
        if let Some(pos) = filtered.iter().position(|&idx| {
            state
                .workspace_history
                .entries
                .get(idx)
                .is_some_and(|entry| entry.path == target_path)
        }) {
            state.workspace_history.selected = pos;
        }
    }
    let total = state.workspace_history.visible_rows().len();
    if total == 0 {
        state.workspace_history.selected = 0;
        state.workspace_history.scroll = 0;
    } else {
        state.workspace_history.selected = state.workspace_history.selected.min(total - 1);
    }

    entries_changed || !was_loaded || ws_renamed
}

/// 上下移动历史列表选中项并保持在视口内；返回是否变化。
pub fn move_workspace_history_selection(
    state: &mut AppState,
    delta: isize,
    visible_rows: usize,
) -> bool {
    let total = state.workspace_history.visible_rows().len();
    if total == 0 || delta == 0 {
        return false;
    }
    let current = state.workspace_history.selected.min(total - 1) as isize;
    let next = (current + delta).clamp(0, (total - 1) as isize) as usize;
    let prev_scroll = state.workspace_history.scroll;
    let changed = next != state.workspace_history.selected;
    state.workspace_history.selected = next;
    state
        .workspace_history
        .scroll_selected_into_view(visible_rows);
    changed || state.workspace_history.scroll != prev_scroll
}

/// 选中指定可见行下标。
pub fn select_workspace_history_row(state: &mut AppState, visible_row: usize) -> bool {
    let total = state.workspace_history.visible_rows().len();
    if visible_row >= total {
        return false;
    }
    let changed = state.workspace_history.selected != visible_row;
    state.workspace_history.selected = visible_row;
    changed
}

/// 滚轮滚动工作区历史列表；返回是否变化。
pub fn scroll_workspace_history(state: &mut AppState, delta: isize, visible_rows: usize) -> bool {
    let total = state.workspace_history.visible_rows().len();
    let max = total.saturating_sub(visible_rows.max(1));
    let target = (state.workspace_history.scroll.min(max) as isize).saturating_add(delta);
    let offset = target.clamp(0, max as isize) as usize;
    if offset == state.workspace_history.scroll {
        return false;
    }
    state.workspace_history.scroll = offset;
    true
}

/// 直接设置工作区历史列表滚动偏移（滚动条点击/拖拽）。
pub fn set_workspace_history_scroll(
    state: &mut AppState,
    offset: usize,
    visible_rows: usize,
) -> bool {
    let total = state.workspace_history.visible_rows().len();
    let max = total.saturating_sub(visible_rows.max(1));
    let next = offset.min(max);
    if next == state.workspace_history.scroll {
        return false;
    }
    state.workspace_history.scroll = next;
    true
}

/// 更新工作区历史列表悬停行；返回是否变化。
pub fn set_workspace_history_hover(state: &mut AppState, hover: Option<usize>) -> bool {
    if state.workspace_history.hover == hover {
        return false;
    }
    state.workspace_history.hover = hover;
    true
}

/// 登记左键行点击并判定是否构成 400ms 内同行双击；双击后复位点击记录。
pub fn workspace_history_row_double_click(
    state: &mut AppState,
    visible_row: usize,
    now: Instant,
) -> bool {
    let double = state
        .workspace_history
        .row_click
        .is_some_and(|(last_row, at)| {
            last_row == visible_row
                && now.saturating_duration_since(at) <= WORKSPACE_HISTORY_DOUBLE_CLICK_WINDOW
        });
    state.workspace_history.row_click = (!double).then_some((visible_row, now));
    double
}

/// 打开当前选中的历史工作区（回车触发）。
pub fn activate_workspace_history_selected(state: &mut AppState) -> bool {
    let visible_row = state.workspace_history.selected;
    open_workspace_history_row(state, visible_row)
}

/// 按过滤后的可见行下标打开历史工作区（双击触发）。
pub fn open_workspace_history_row(state: &mut AppState, visible_row: usize) -> bool {
    let Some(row) = state
        .workspace_history
        .visible_rows()
        .get(visible_row)
        .copied()
    else {
        return false;
    };
    open_workspace_history_entry(state, row.index)
}

/// 按 `entries` 中的原始下标打开历史工作区：
/// - 路径不存在时刷新为失效态（红字）并弹出 Danger Toast，留在历史列表；
/// - 路径已在活跃列表中打开时直接切换过去（若为 linked worktree 则自动补开主工作区）并退回活跃视图；
/// - 路径未打开时按保存的名称、路径与 Git 元数据创建并激活工作区（若为 linked worktree 则自动补开主工作区），退回活跃视图。
pub fn open_workspace_history_entry(state: &mut AppState, entry_index: usize) -> bool {
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    if !entry.path.is_dir() {
        if let Some(slot) = state.workspace_history.entries.get_mut(entry_index) {
            slot.exists = false;
        }
        let toast = Toast::new(
            ToastKind::Danger,
            text::TOAST_DIRECTORY_NOT_FOUND,
            None,
            Instant::now(),
        )
        .with_title(text::TOAST_WORKSPACE_TITLE);
        show_toast(state, toast);
        return true;
    }
    state
        .workspace_history
        .hidden_worktrees
        .retain(|hidden| hidden != &entry.path);
    if let Some(existing) = state.workspaces.iter().position(|ws| {
        ws.cwd.as_deref() == Some(entry.path.as_path())
            || ws
                .git
                .as_ref()
                .is_some_and(|git| git.checkout_path == entry.path)
    }) {
        switch_workspace(state, existing);
        ensure_main_workspace(state, existing);
        state.effects.push(Effect::RecordWorkspace {
            path: entry.path,
            name: entry.name,
        });
        exit_workspace_history(state);
        return true;
    }

    let default_base = workspace_label(&entry.path, home_dir().as_deref());
    let is_manual = entry.name_is_manual && !entry.name.trim().is_empty();
    let base = if is_manual {
        entry.name.as_str()
    } else {
        default_base.as_str()
    };
    let name = unique_workspace_name(base, |candidate| {
        state
            .workspaces
            .iter()
            .any(|workspace| workspace.name == candidate)
    });
    let mut workspace = Workspace::single_terminal(name, Some(entry.path.clone()));
    workspace.name_is_manual = is_manual;
    workspace.git = entry.git.clone();
    state.workspaces.push(workspace);
    let child = state.workspaces.len().saturating_sub(1);
    let from = state.active_workspace;
    state.active_workspace = child;
    if state.prompt_pinned.is_none() {
        swap_display_prompt(state, from, state.active_workspace);
    }
    ensure_main_workspace(state, child);
    request_git_refresh(state, &entry.path);
    state.effects.push(Effect::RecordWorkspace {
        path: entry.path,
        name: entry.name,
    });
    clear_selection(state);
    exit_workspace_history(state);
    super::focus::focus_panes(state);
    true
}

/// 级联删除工作区历史条目：
/// - 若存在正在运行的 Agent 流、子 Agent 或本地命令，立即发出中断/取消效应并解绑会话；
/// - 若该工作区已在活跃列表中打开，将其关闭（若仅剩最后一个工作区则清空其会话与提示词树）；
/// - 登记 `Effect::DeleteWorkspaceHistory` 从 SQLite 级联删除该工作区及全部关联表数据。
pub fn delete_workspace_history_entry(state: &mut AppState, entry_index: usize) -> bool {
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    let mut paths_to_delete = vec![entry.path.clone()];
    if entry.git.as_ref().is_some_and(|git| !git.is_linked) {
        for other in &state.workspace_history.entries {
            if other.path != entry.path
                && other
                    .git
                    .as_ref()
                    .is_some_and(|git| git.repo_root == entry.path)
            {
                paths_to_delete.push(other.path.clone());
            }
        }
    } else if entry.git.as_ref().is_some_and(|git| git.is_linked)
        && !state
            .workspace_history
            .hidden_worktrees
            .contains(&entry.path)
    {
        state
            .workspace_history
            .hidden_worktrees
            .push(entry.path.clone());
    }

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
                super::on_turn_failed(state, pane_id, generation);
            }
            if let Some(sid) = session_id {
                state.effects.push(Effect::CancelSubagents {
                    session_id: sid.clone(),
                    agent_id: None,
                });
                super::on_session_deleted(state, pane_id, &sid);
                super::on_session_removed(state, pane_id, &sid);
            }
        }
        if state.workspaces.len() > 1 {
            super::workspace::close_workspace(state, ws_idx);
        } else if let Some(ws) = state.workspaces.get_mut(ws_idx) {
            ws.prompt_panel.tree.replace(Vec::new(), Vec::new());
            state.referenced.reset(None);
        }
    }

    state
        .workspace_history
        .entries
        .retain(|item| !paths_to_delete.contains(&item.path));
    let total = state.workspace_history.visible_rows().len();
    if total == 0 {
        state.workspace_history.selected = 0;
        state.workspace_history.scroll = 0;
    } else {
        state.workspace_history.selected = state.workspace_history.selected.min(total - 1);
    }
    for del_path in paths_to_delete {
        state
            .effects
            .push(Effect::DeleteWorkspaceHistory { path: del_path });
    }
    true
}

/// 在历史列表（`ws list`）视图下新建工作区：仅登记入库、清空搜索过滤并选中该新建项，
/// 保持停留在历史列表视图而不直接打开激活该工作区（对齐 Prompt List 新建节点行为）。
pub fn record_workspace_in_history_list(state: &mut AppState, cwd: std::path::PathBuf) -> bool {
    let base = workspace_label(&cwd, home_dir().as_deref());
    state.workspace_history.search.clear();
    state.workspace_history.row_click = None;
    state
        .workspace_history
        .hidden_worktrees
        .retain(|hidden| hidden != &cwd);
    if !state
        .workspace_history
        .entries
        .iter()
        .any(|entry| entry.path == cwd)
    {
        let exists = cwd.is_dir();
        state.workspace_history.entries.insert(
            0,
            WorkspaceHistoryEntry {
                name: base.clone(),
                path: cwd.clone(),
                exists,
                name_is_manual: false,
                git: None,
            },
        );
    }
    let filtered = state.workspace_history.filtered_indices();
    if let Some(pos) = filtered.iter().position(|&idx| {
        state
            .workspace_history
            .entries
            .get(idx)
            .is_some_and(|entry| entry.path == cwd)
    }) {
        state.workspace_history.selected = pos;
    }
    state.effects.push(Effect::RecordWorkspace {
        path: cwd,
        name: base,
    });
    super::focus::focus_sidebar(state);
    true
}

/// 处理历史列表模式下的键盘按键（上下选择、回车打开、Esc 退出、搜索框编辑）。
pub fn apply_workspace_history_key(
    state: &mut AppState,
    key: OverlayKey,
    visible_rows: usize,
) -> bool {
    match key {
        OverlayKey::Esc => exit_workspace_history(state),
        OverlayKey::Up => move_workspace_history_selection(state, -1, visible_rows),
        OverlayKey::Down => move_workspace_history_selection(state, 1, visible_rows),
        OverlayKey::Enter => activate_workspace_history_selected(state),
        _ => {
            let before = state.workspace_history.search.text().to_string();
            let edited = state.workspace_history.search.apply_edit_key(key);
            if state.workspace_history.search.text() != before {
                state.workspace_history.row_click = None;
                if state.workspace_history.search.text().trim().is_empty() {
                    state.workspace_history.clamp(visible_rows);
                } else {
                    state.workspace_history.selected = 0;
                    state.workspace_history.clamp(visible_rows);
                    state
                        .workspace_history
                        .scroll_selected_into_view(visible_rows);
                }
            }
            edited
        }
    }
}

/// 清空历史列表搜索框内容。
pub fn clear_workspace_history_search(state: &mut AppState, visible_rows: usize) -> bool {
    if state.workspace_history.search.text().is_empty() {
        return false;
    }
    state.workspace_history.search.clear();
    state.workspace_history.row_click = None;
    state.workspace_history.clamp(visible_rows);
    true
}

/// 剪切历史列表搜索框选区文本。
pub fn cut_workspace_history_search(state: &mut AppState, visible_rows: usize) -> Option<String> {
    let cut = state.workspace_history.search.cut_selected()?;
    state.workspace_history.row_click = None;
    if state.workspace_history.search.text().trim().is_empty() {
        state.workspace_history.clamp(visible_rows);
    } else {
        state.workspace_history.selected = 0;
        state.workspace_history.clamp(visible_rows);
        state
            .workspace_history
            .scroll_selected_into_view(visible_rows);
    }
    Some(cut)
}

/// 将粘贴文本写入历史列表搜索框（压平换行）。
pub fn apply_workspace_history_paste(
    state: &mut AppState,
    text: &str,
    visible_rows: usize,
) -> bool {
    if !workspace_history_mode(state) {
        return false;
    }
    let clean: String = text
        .chars()
        .map(|ch| if matches!(ch, '\n' | '\r') { ' ' } else { ch })
        .collect();
    if clean.is_empty() {
        return false;
    }
    state.workspace_history.search.insert_str(&clean);
    state.workspace_history.row_click = None;
    state.workspace_history.selected = 0;
    state.workspace_history.clamp(visible_rows);
    state
        .workspace_history
        .scroll_selected_into_view(visible_rows);
    true
}

#[cfg(test)]
mod tests;

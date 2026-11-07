//! 侧栏工作区历史列表交互：模式切换、Git 树形分组与折叠、搜索过滤、上下导航、双击打开与拖拽排序；
//! 级联删除见子模块 `delete`。

mod delete;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::state::{
    AppState, Workspace, WorkspaceHistoryEntry, WorkspaceMode, home_dir, unique_workspace_name,
    workspace_label,
};
use crate::app::toast::{Toast, ToastKind};
use crate::app::trust::TrustTrigger;
use crate::ui::text;

use super::editor::swap_display_prompt;
use super::interaction::show_toast;
use super::selection::clear_selection;
use super::workspace::switch_workspace;
use super::worktree::{ensure_main_workspace, request_git_refresh};

pub use delete::{delete_missing_workspace_history_entries, delete_workspace_history_entry};

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
    state.workspace_drag_moved = false;
    state.workspace_scroll_drag = None;
    state.workspace_hover = None;
    state.workspace_history.mode = WorkspaceMode::History;
    state.workspace_history.hover = None;
    state.workspace_history.row_click = None;
    state.workspace_history.scroll_drag = None;
    state.workspace_history.search_drag = false;
    state.workspace_history.drag = None;
    state.workspace_history.dragging = false;
    state.workspace_history.drag_moved = false;
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
    state.workspace_history.reveal_selected = true;
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
    state.workspace_history.drag = None;
    state.workspace_history.dragging = false;
    state.workspace_history.drag_moved = false;
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

/// 归一化历史条目为完整树形顺序（Git 父项在前、子项紧随）；返回是否有变化。
pub fn normalize_workspace_history_entries(state: &mut AppState) -> bool {
    state.workspace_history.normalize()
}

/// 开始拖拽历史条目：按路径重新定位并归一化树序；搜索过滤中不支持拖拽。
pub fn begin_workspace_history_drag(state: &mut AppState, visible_row: usize) -> bool {
    if state.workspace_history.mode != WorkspaceMode::History
        || !state.workspace_history.search.text().trim().is_empty()
    {
        return false;
    }
    let Some((_, entry)) = state.workspace_history.entry_at_visible_row(visible_row) else {
        return false;
    };
    let path = entry.path.clone();
    normalize_workspace_history_entries(state);
    let Some(row) = state
        .workspace_history
        .visible_rows()
        .iter()
        .position(|row| {
            state
                .workspace_history
                .entries
                .get(row.index)
                .is_some_and(|entry| entry.path == path)
        })
    else {
        return false;
    };
    state.workspace_history.selected = row;
    state.workspace_history.drag = state
        .workspace_history
        .visible_rows()
        .get(row)
        .map(|row| row.index);
    state.workspace_history.dragging = false;
    state.workspace_history.drag_moved = false;
    state.workspace_history.hover = None;
    true
}

/// 拖拽历史条目：以 Git 分组块为单位移动到目标可见行；同步活跃列表顺序。
pub fn drag_workspace_history_to(state: &mut AppState, target_visible_row: usize) -> bool {
    let Some(from) = state.workspace_history.drag else {
        return false;
    };
    let len = state.workspace_history.entries.len();
    if from >= len || !state.workspace_history.search.text().trim().is_empty() {
        return false;
    }
    let Some(target) = state
        .workspace_history
        .visible_rows()
        .get(target_visible_row)
        .map(|row| row.index)
    else {
        return false;
    };
    if target >= len {
        return false;
    }
    state.workspace_history.dragging = true;
    let block = state.workspace_history.drag_block(from);
    let hovered = state.workspace_history.drag_block(target);
    if hovered.iter().any(|member| block.contains(member)) {
        return false;
    }
    // 落点坐标：向下拖插到目标块之后，向上拖插到目标块之前。
    let at = if target > from {
        hovered.last().map_or(len, |last| last.saturating_add(1))
    } else {
        hovered.first().copied().unwrap_or(0)
    };
    let removed_before =
        |position: usize| block.iter().filter(|member| **member < position).count();
    let insert_at = at - removed_before(at);
    let offset = block.iter().position(|member| *member == from).unwrap_or(0);
    let kept: Vec<usize> = (0..len).filter(|index| !block.contains(index)).collect();
    let mut order = kept;
    for (slot, member) in block.iter().enumerate() {
        order.insert(insert_at + slot, *member);
    }
    if !state.workspace_history.reorder_entries(&order) {
        return false;
    }
    state.workspace_history.drag_moved = true;
    state.workspace_history.row_click = None;
    state.workspace_history.drag = Some(insert_at + offset);
    state.workspace_history.selected = state
        .workspace_history
        .visible_rows()
        .iter()
        .position(|row| row.index == insert_at + offset)
        .unwrap_or(0);
    sync_active_order_to_history(state);
    true
}

/// 结束历史列表拖拽；真正移动过时持久化全局顺序。
pub fn end_workspace_history_drag(state: &mut AppState) {
    let persist = state.workspace_history.drag_moved;
    state.workspace_history.drag = None;
    state.workspace_history.dragging = false;
    state.workspace_history.drag_moved = false;
    if persist {
        persist_workspace_order(state);
    }
}

/// 将当前历史条目顺序登记为持久化效应。
pub fn persist_workspace_order(state: &mut AppState) {
    if state.workspace_history.entries.is_empty() {
        return;
    }
    let entries = state
        .workspace_history
        .entries
        .iter()
        .map(|entry| (entry.path.clone(), entry.name.clone()))
        .collect();
    state.effects.push(Effect::SaveWorkspaceOrder { entries });
}

/// 按历史列表顺序稳定重排已打开工作区；返回是否有变化。
///
/// 历史条目之外的活跃工作区排在末尾并保持原有相对顺序；激活项与钉住的
/// prompt 索引随重排一并重映射。
pub fn sync_active_order_to_history(state: &mut AppState) -> bool {
    let len = state.workspaces.len();
    if len < 2 {
        return false;
    }
    let entries = &state.workspace_history.entries;
    let ranks: Vec<Option<usize>> = state
        .workspaces
        .iter()
        .map(|workspace| {
            entries.iter().position(|entry| {
                workspace.cwd.as_deref() == Some(entry.path.as_path())
                    || workspace
                        .git
                        .as_ref()
                        .is_some_and(|git| git.checkout_path == entry.path)
            })
        })
        .collect();
    let mut matched: Vec<usize> = (0..len).filter(|&index| ranks[index].is_some()).collect();
    if matched.is_empty() {
        return false;
    }
    matched.sort_by_key(|&index| ranks[index].unwrap_or(usize::MAX));
    // 未出现在历史列表中的工作区保持原槽位，只重排已匹配项；避免 Sync cwd 后
    // 工作区每次 reload 都被挤到列表末尾。
    let mut next = matched.into_iter();
    let order: Vec<usize> = (0..len)
        .map(|index| {
            if ranks[index].is_some() {
                next.next().unwrap_or(index)
            } else {
                index
            }
        })
        .collect();
    if order.iter().copied().eq(0..len) {
        return false;
    }
    let active = state.active_workspace;
    let pinned = state.prompt_pinned;
    let new_active = order.iter().position(|&old| old == active);
    let new_pinned = pinned.and_then(|pinned| order.iter().position(|&old| old == pinned));
    let mut slots: Vec<Option<Workspace>> = state.workspaces.drain(..).map(Some).collect();
    state.workspaces = order
        .into_iter()
        .filter_map(|old| slots.get_mut(old).and_then(Option::take))
        .collect();
    if let Some(index) = new_active {
        state.active_workspace = index;
    }
    state.prompt_pinned = new_pinned;
    true
}

/// 活跃列表拖拽后，把已打开工作区的相对顺序回写到历史条目。
///
/// 未打开的历史条目保持原槽位不动（稳定交错），只替换已打开条目的先后次序；
/// 随后归一化为树形顺序。返回是否有变化。
pub fn sync_history_entries_from_active(state: &mut AppState) -> bool {
    if state.workspace_history.entries.is_empty() {
        return false;
    }
    let open_paths: Vec<PathBuf> = state
        .workspaces
        .iter()
        .filter_map(|workspace| {
            workspace
                .cwd
                .clone()
                .or_else(|| workspace.git.as_ref().map(|git| git.checkout_path.clone()))
        })
        .collect();
    if open_paths.is_empty() {
        return false;
    }
    let panel = &mut state.workspace_history;
    let index_by_path: HashMap<PathBuf, usize> = panel
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.path.clone(), index))
        .collect();
    let mut pending: VecDeque<WorkspaceHistoryEntry> = VecDeque::new();
    let mut open_set: HashSet<PathBuf> = HashSet::new();
    for path in open_paths {
        if let Some(&index) = index_by_path.get(&path)
            && open_set.insert(path)
        {
            pending.push_back(panel.entries[index].clone());
        }
    }
    if pending.is_empty() {
        return false;
    }
    let mut changed = false;
    let mut rebuilt = Vec::with_capacity(panel.entries.len());
    for entry in panel.entries.drain(..) {
        if open_set.contains(&entry.path) {
            if let Some(next) = pending.pop_front() {
                changed |= next != entry;
                rebuilt.push(next);
            } else {
                rebuilt.push(entry);
            }
        } else {
            rebuilt.push(entry);
        }
    }
    panel.entries = rebuilt;
    changed |= panel.normalize();
    changed
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
    state.workspace_history.row_click = None;
    // 数据库按 position 返回，但历史条目中的 Git 树必须父项在前、子项紧随；
    // 归一化后 raw entries 顺序与渲染树序一致，活跃列表可直接按它重排。
    let normalized = state.workspace_history.normalize();

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
        state.workspace_history.scroll =
            state.workspace_history.scroll.min(total.saturating_sub(1));
    }
    // 恢复/首次加载的选中项由事件循环在有视口后滚入可见区。
    state.workspace_history.reveal_selected = true;

    // 两个列表共用同一顺序：数据库快照到达后，已打开工作区按历史顺序稳定重排。
    let ws_reordered = state.workspace_history.drag.is_none()
        && state.workspace_drag.is_none()
        && sync_active_order_to_history(state);

    entries_changed || normalized || !was_loaded || ws_renamed || ws_reordered
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
    state.workspace_history.row_click = None;
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
    state.workspace_history.row_click = None;
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

/// 登记左键行点击并判定是否构成 400ms 内同一条目双击；双击后复位点击记录。
///
/// 以条目路径而非可见行号作身份，滚动/过滤/重排后不会把同一屏幕行误判为同一条目。
pub fn workspace_history_row_double_click(
    state: &mut AppState,
    visible_row: usize,
    now: Instant,
) -> bool {
    let path = state
        .workspace_history
        .visible_rows()
        .get(visible_row)
        .and_then(|row| state.workspace_history.entries.get(row.index))
        .map(|entry| entry.path.clone());
    let Some(path) = path else {
        state.workspace_history.row_click = None;
        return false;
    };
    let double = state
        .workspace_history
        .row_click
        .as_ref()
        .is_some_and(|(last, at)| {
            last == &path
                && now.saturating_duration_since(*at) <= WORKSPACE_HISTORY_DOUBLE_CLICK_WINDOW
        });
    state.workspace_history.row_click = (!double).then_some((path, now));
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
/// - 路径已信任时按保存的名称、路径与 Git 元数据创建并激活工作区（若为 linked worktree 则自动补开主工作区），退回活跃视图；
/// - 路径未信任时先走 `CheckTrust`，授予信任后由 `open_workspace_history_trusted` 续作。
pub fn open_workspace_history_entry(state: &mut AppState, entry_index: usize) -> bool {
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    if !entry.path.is_dir() {
        mark_history_entry_missing(state, entry_index);
        return true;
    }
    if let Some(existing) = open_history_workspace_index(state, &entry) {
        open_existing_history_workspace(state, existing, &entry);
        return true;
    }
    if entry.trusted {
        open_new_history_workspace(state, entry);
        return true;
    }
    state.effects.push(Effect::CheckTrust {
        path: entry.path.clone(),
        root: entry.git.as_ref().map(|git| git.repo_root.clone()),
        trigger: TrustTrigger::OpenWorkspaceHistory {
            path: entry.path,
            name: entry.name,
        },
    });
    true
}

/// 信任通过后打开历史工作区：按路径重新定位条目，避免信任浮层期间列表变化导致下标漂移。
pub fn open_workspace_history_trusted(
    state: &mut AppState,
    path: std::path::PathBuf,
    _name: String,
) -> bool {
    let Some(entry_index) = state
        .workspace_history
        .entries
        .iter()
        .position(|entry| entry.path == path)
    else {
        return false;
    };
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    if !entry.path.is_dir() {
        mark_history_entry_missing(state, entry_index);
        return false;
    }
    if let Some(existing) = open_history_workspace_index(state, &entry) {
        open_existing_history_workspace(state, existing, &entry);
    } else {
        open_new_history_workspace(state, entry);
    }
    true
}

/// 失效路径：刷新条目红字态并弹出 Danger Toast，返回 false 表示未打开。
fn mark_history_entry_missing(state: &mut AppState, entry_index: usize) {
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
}

/// 已有同路径工作区时返回其活跃列表下标（cwd 或 git checkout 路径任一匹配）。
fn open_history_workspace_index(state: &AppState, entry: &WorkspaceHistoryEntry) -> Option<usize> {
    state.workspaces.iter().position(|workspace| {
        workspace
            .cwd
            .as_deref()
            .is_some_and(|cwd| same_workspace_path(cwd, &entry.path))
            || workspace
                .git
                .as_ref()
                .is_some_and(|git| same_workspace_path(&git.checkout_path, &entry.path))
    })
}

/// 路径文本不同但指向同一目录时视为同一工作区（兼容旧库/终端 cwd 的非 canonical 文本）。
fn same_workspace_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

/// 打开已在活跃列表中的历史工作区：切换并补开主 checkout，退回活跃视图并交回主区焦点。
fn open_existing_history_workspace(
    state: &mut AppState,
    existing: usize,
    entry: &WorkspaceHistoryEntry,
) {
    switch_workspace(state, existing);
    ensure_main_workspace(state, existing);
    state.effects.push(Effect::RecordWorkspace {
        path: entry.path.clone(),
        name: entry.name.clone(),
    });
    exit_workspace_history(state);
    super::focus::focus_panes(state);
}

/// 创建并激活历史工作区（调用方已完成存在性与信任检查）。
fn open_new_history_workspace(state: &mut AppState, entry: WorkspaceHistoryEntry) {
    state
        .workspace_history
        .hidden_worktrees
        .retain(|hidden| hidden != &entry.path);
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
    // 立即按历史顺序重排已打开工作区（不必等待数据库回执）。
    sync_active_order_to_history(state);
    state.effects.push(Effect::RecordWorkspace {
        path: entry.path,
        name: entry.name,
    });
    clear_selection(state);
    exit_workspace_history(state);
    super::focus::focus_panes(state);
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
        let position = state
            .workspace_history
            .entries
            .iter()
            .map(|entry| entry.position)
            .max()
            .map_or(0, |position| position.saturating_add(1));
        state.workspace_history.entries.push(WorkspaceHistoryEntry {
            name: base.clone(),
            path: cwd.clone(),
            exists,
            name_is_manual: false,
            position,
            trusted: true,
            git: None,
        });
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
    state.workspace_history.reveal_selected = true;
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
    if key == OverlayKey::Esc && super::escape_clear_selection(state) {
        return true;
    }
    if key == OverlayKey::SelectAll {
        super::clear_selection(state);
    }
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
                    state.workspace_history.reveal_selected = true;
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
    state.workspace_history.reveal_selected = true;
    true
}

/// 剪切历史列表搜索框选区文本。
pub fn cut_workspace_history_search(state: &mut AppState, visible_rows: usize) -> Option<String> {
    let cut = state.workspace_history.search.cut_selected()?;
    state.workspace_history.row_click = None;
    if state.workspace_history.search.text().trim().is_empty() {
        state.workspace_history.clamp(visible_rows);
        state.workspace_history.reveal_selected = true;
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

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

/// 删除路径已不存在的历史工作区条目，并登记数据库级联删除效应。
pub fn delete_workspace_history_entry(state: &mut AppState, entry_index: usize) -> bool {
    let Some(entry) = state.workspace_history.entries.get(entry_index).cloned() else {
        return false;
    };
    if entry.path.is_dir() {
        if let Some(slot) = state.workspace_history.entries.get_mut(entry_index) {
            slot.exists = true;
        }
        return true;
    }
    state.workspace_history.entries.remove(entry_index);
    let total = state.workspace_history.visible_rows().len();
    if total == 0 {
        state.workspace_history.selected = 0;
        state.workspace_history.scroll = 0;
    } else {
        state.workspace_history.selected = state.workspace_history.selected.min(total - 1);
    }
    state
        .effects
        .push(Effect::DeleteWorkspaceHistory { path: entry.path });
    true
}

/// 在历史列表（`ws list`）视图下新建工作区：仅登记入库、清空搜索过滤并选中该新建项，
/// 保持停留在历史列表视图而不直接打开激活该工作区（对齐 Prompt List 新建节点行为）。
pub fn record_workspace_in_history_list(state: &mut AppState, cwd: std::path::PathBuf) -> bool {
    let base = workspace_label(&cwd, home_dir().as_deref());
    state.workspace_history.search.clear();
    state.workspace_history.row_click = None;
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
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::app::overlay::{MenuCommand, Overlay, OverlayTarget, RenameTarget};
    use crate::app::state::{FocusedContainer, WorkspaceGit};
    use crate::app::update::{
        activate_menu, apply_overlay_key, open_workspace_history_menu, open_workspace_menu,
    };

    fn plain_entry(name: &str, path: PathBuf, manual: bool) -> WorkspaceHistoryEntry {
        let exists = path.is_dir();
        WorkspaceHistoryEntry {
            name: name.into(),
            path,
            exists,
            name_is_manual: manual,
            git: None,
        }
    }

    #[test]
    fn toggle_workspace_mode_switches_and_focuses_sidebar() {
        let mut state = AppState::demo();
        assert!(!workspace_history_mode(&state));

        assert!(toggle_workspace_mode(&mut state));
        assert!(workspace_history_mode(&state));
        assert_eq!(state.focus(), FocusedContainer::Sidebar);
        assert!(
            state
                .effects
                .iter()
                .any(|eff| matches!(eff, Effect::LoadWorkspaceHistory))
        );

        assert!(toggle_workspace_mode(&mut state));
        assert!(!workspace_history_mode(&state));
    }

    #[test]
    fn search_filtering_and_navigation_in_workspace_history() {
        let mut state = AppState::demo();
        let existing_dir = std::env::temp_dir();
        let missing_dir = PathBuf::from("/tmp/lx-nonexistent-history-dir-999999");
        apply_workspace_history(
            &mut state,
            vec![
                plain_entry("Alpha Project", existing_dir, true),
                plain_entry("Missing Project", missing_dir, true),
            ],
        );
        assert_eq!(state.workspace_history.entries.len(), 2);
        assert!(state.workspace_history.entries[0].exists);
        assert!(!state.workspace_history.entries[1].exists);

        show_workspace_history(&mut state);
        assert_eq!(state.workspace_history.filtered_indices(), vec![0, 1]);

        // Down / Up 移动选中项。
        assert!(move_workspace_history_selection(&mut state, 1, 5));
        assert_eq!(state.workspace_history.selected, 1);
        assert!(move_workspace_history_selection(&mut state, -1, 5));
        assert_eq!(state.workspace_history.selected, 0);

        // 搜索过滤（不区分大小写，匹配名称和路径）。
        for ch in "missing".chars() {
            apply_workspace_history_key(&mut state, OverlayKey::Char(ch), 5);
        }
        assert_eq!(state.workspace_history.filtered_indices(), vec![1]);
        assert_eq!(state.workspace_history.selected, 0);

        // 清空搜索恢复全部条目。
        assert!(clear_workspace_history_search(&mut state, 5));
        assert_eq!(state.workspace_history.filtered_indices(), vec![0, 1]);

        // Esc 退出历史模式。
        assert!(apply_workspace_history_key(&mut state, OverlayKey::Esc, 5));
        assert!(!workspace_history_mode(&state));
    }

    #[test]
    fn git_tree_grouping_collapse_and_opening_linked_worktree_auto_opens_main_parent() {
        let mut state = AppState::demo();
        let repo_root = std::env::temp_dir();
        let wt_path = std::env::temp_dir().join(".");
        apply_workspace_history(
            &mut state,
            vec![
                WorkspaceHistoryEntry {
                    name: "lx-tui".into(),
                    path: repo_root.clone(),
                    exists: true,
                    name_is_manual: false,
                    git: Some(WorkspaceGit {
                        repo_root: repo_root.clone(),
                        checkout_path: repo_root.clone(),
                        is_linked: false,
                        branch: Some("dev".into()),
                        main_branch: Some("dev".into()),
                    }),
                },
                WorkspaceHistoryEntry {
                    name: "workspace-history-list".into(),
                    path: wt_path.clone(),
                    exists: true,
                    name_is_manual: false,
                    git: Some(WorkspaceGit {
                        repo_root: repo_root.clone(),
                        checkout_path: wt_path.clone(),
                        is_linked: true,
                        branch: Some("feature/workspace-history-list".into()),
                        main_branch: Some("dev".into()),
                    }),
                },
            ],
        );

        let rows = state.workspace_history.visible_rows();
        assert_eq!(rows.len(), 2);
        assert!(rows[0].parent && !rows[0].collapsed);
        assert!(rows[1].child && rows[1].last_child);
        assert_eq!(
            state.workspace_history.entries[0].display_label(rows[0].child),
            "lx-tui"
        );
        assert_eq!(
            state.workspace_history.entries[1].display_label(rows[1].child),
            "feature/workspace-history-list"
        );

        // 点击父节点折叠/展开子工作区。
        assert!(toggle_workspace_history_group(&mut state, 0));
        let collapsed_rows = state.workspace_history.visible_rows();
        assert_eq!(collapsed_rows.len(), 1);
        assert!(collapsed_rows[0].parent && collapsed_rows[0].collapsed);

        assert!(toggle_workspace_history_group(&mut state, 0));
        assert_eq!(state.workspace_history.visible_rows().len(), 2);
    }

    #[test]
    fn double_click_opens_existing_or_shows_toast_on_missing_directory() {
        let mut state = AppState::demo();
        let dir_a = std::env::temp_dir();
        let missing = PathBuf::from("/tmp/lx-nonexistent-history-dir-888888");
        apply_workspace_history(
            &mut state,
            vec![
                plain_entry("TempWS", dir_a.clone(), true),
                plain_entry("GoneWS", missing, true),
            ],
        );
        show_workspace_history(&mut state);

        // 双击第 1 行（路径不存在）：弹 Danger Toast 并留在历史列表模式。
        let now = Instant::now();
        assert!(!workspace_history_row_double_click(&mut state, 1, now));
        assert!(workspace_history_row_double_click(
            &mut state,
            1,
            now + Duration::from_millis(100)
        ));
        assert!(open_workspace_history_row(&mut state, 1));
        assert!(
            workspace_history_mode(&state),
            "路径不存在时保持在历史列表视图"
        );
        let toast = state.toast.as_ref().expect("danger toast shown");
        assert_eq!(toast.kind, ToastKind::Danger);
        assert_eq!(toast.message, text::TOAST_DIRECTORY_NOT_FOUND);

        // 双击第 0 行（路径存在且尚未在 state.workspaces 打开）：新建并激活该工作区，退回活跃列表。
        let now2 = now + Duration::from_secs(1);
        assert!(!workspace_history_row_double_click(&mut state, 0, now2));
        assert!(workspace_history_row_double_click(
            &mut state,
            0,
            now2 + Duration::from_millis(120)
        ));
        assert!(open_workspace_history_row(&mut state, 0));
        assert!(!workspace_history_mode(&state));
        assert_eq!(
            state.active_workspace().cwd.as_deref(),
            Some(dir_a.as_path())
        );
        assert_eq!(state.active_workspace().name, "TempWS");
        let count_after_first_open = state.workspaces.len();

        // 再次进入历史列表打开同一个已打开的路径：只切换过去，不重复创建。
        show_workspace_history(&mut state);
        assert!(open_workspace_history_row(&mut state, 0));
        assert!(!workspace_history_mode(&state));
        assert_eq!(state.workspaces.len(), count_after_first_open);
    }

    #[test]
    fn context_menu_offers_delete_only_for_missing_path_and_cascades_on_confirm() {
        let mut state = AppState::demo();
        let existing_dir = std::env::temp_dir();
        let missing_dir = PathBuf::from("/tmp/lx-nonexistent-history-dir-777777");
        apply_workspace_history(
            &mut state,
            vec![
                plain_entry("ValidWS", existing_dir, true),
                plain_entry("BrokenWS", missing_dir.clone(), true),
            ],
        );
        show_workspace_history(&mut state);

        // 有效路径条目菜单：仅 Open 与 Rename，不含 Delete。
        open_workspace_history_menu(&mut state, 0, (5, 5));
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("menu expected");
        };
        assert_eq!(menu.target, OverlayTarget::WorkspaceHistory(0));
        assert_eq!(
            menu.commands,
            vec![
                MenuCommand::OpenWorkspaceHistory,
                MenuCommand::RenameWorkspace
            ]
        );

        // 失效路径条目菜单：包含 Open、Rename 与 Delete。
        open_workspace_history_menu(&mut state, 1, (5, 6));
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("menu expected");
        };
        assert_eq!(menu.target, OverlayTarget::WorkspaceHistory(1));
        assert_eq!(
            menu.commands,
            vec![
                MenuCommand::OpenWorkspaceHistory,
                MenuCommand::RenameWorkspace,
                MenuCommand::DeleteWorkspaceHistory
            ]
        );

        // 选中 Delete：第一次激活进入行内二次确认，第二次激活执行删除并登记级联删除效应。
        if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
            menu.selected = 2;
        }
        state.effects.clear();
        activate_menu(&mut state);
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("menu stays open for inline confirmation");
        };
        assert!(menu.confirming);
        assert_eq!(state.workspace_history.entries.len(), 2);

        activate_menu(&mut state);
        assert!(state.overlay.is_none());
        assert_eq!(state.workspace_history.entries.len(), 1);
        assert_eq!(
            state.effects,
            vec![Effect::DeleteWorkspaceHistory { path: missing_dir }]
        );
    }

    #[test]
    fn rename_syncs_between_active_workspace_and_history_list() {
        let mut state = AppState::demo();
        let path = PathBuf::from("/tmp/lx-rename-sync-test");
        state.workspaces[0].cwd = Some(path.clone());
        apply_workspace_history(
            &mut state,
            vec![plain_entry("OldName", path.clone(), false)],
        );

        // 从历史列表重命名：同步更新活跃工作区名称并登记 RenameWorkspace 效应。
        show_workspace_history(&mut state);
        open_workspace_history_menu(&mut state, 0, (4, 4));
        if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
            menu.selected = 1;
        }
        activate_menu(&mut state);
        let Some(Overlay::Rename(rename)) = state.overlay.as_mut() else {
            panic!("rename overlay expected");
        };
        assert_eq!(rename.target, RenameTarget::WorkspaceHistory(0));
        rename.input.clear();
        rename.input.insert_str("NewHistoryName");
        state.effects.clear();
        apply_overlay_key(&mut state, OverlayKey::Enter);

        assert_eq!(state.workspace_history.entries[0].name, "NewHistoryName");
        assert!(state.workspace_history.entries[0].name_is_manual);
        assert_eq!(state.workspaces[0].name, "NewHistoryName");
        assert!(state.workspaces[0].name_is_manual);
        assert_eq!(
            state.effects,
            vec![Effect::RenameWorkspace {
                path: path.clone(),
                name: "NewHistoryName".into(),
            }]
        );

        // 从活跃列表重命名：同步更新历史列表条目名称并登记 RenameWorkspace 效应。
        exit_workspace_history(&mut state);
        open_workspace_menu(&mut state, 0, (4, 4));
        if let Some(Overlay::Menu(menu)) = state.overlay.as_mut() {
            menu.selected = menu
                .commands
                .iter()
                .position(|cmd| *cmd == MenuCommand::RenameWorkspace)
                .unwrap_or(0);
        }
        activate_menu(&mut state);
        if let Some(Overlay::Rename(rename)) = state.overlay.as_mut() {
            rename.input.clear();
            rename.input.insert_str("ActiveRenamed");
        }
        state.effects.clear();
        apply_overlay_key(&mut state, OverlayKey::Enter);

        assert_eq!(state.workspaces[0].name, "ActiveRenamed");
        assert_eq!(state.workspace_history.entries[0].name, "ActiveRenamed");
        assert_eq!(
            state.effects,
            vec![Effect::RenameWorkspace {
                path,
                name: "ActiveRenamed".into(),
            }]
        );
    }

    #[test]
    fn creating_workspace_in_history_mode_records_and_selects_without_opening() {
        let mut state = AppState::demo();
        let initial_ws_count = state.workspaces.len();
        let target_dir = std::env::temp_dir();
        apply_workspace_history(
            &mut state,
            vec![plain_entry(
                "OldEntry",
                PathBuf::from("/tmp/lx-old-entry"),
                true,
            )],
        );
        show_workspace_history(&mut state);
        state.workspace_history.search.insert_str("old");
        assert_eq!(state.workspace_history.selected, 0);

        // 在历史列表模式下打开新建工作区弹窗并提交路径（经过信任放行 resume_trust）。
        crate::app::update::open_new_workspace_dialog(&mut state);
        if let Some(Overlay::NewWorkspace(dialog)) = state.overlay.as_mut() {
            dialog.input.insert_str(&target_dir.to_string_lossy());
        }
        state.effects.clear();
        apply_overlay_key(&mut state, OverlayKey::Enter);
        let Some(Effect::CheckTrust { trigger, .. }) = state.effects.first().cloned() else {
            panic!("CheckTrust expected");
        };
        state.effects.clear();
        crate::app::update::resume_trust(&mut state, trigger);

        // 保持停留在历史列表模式，不直接向活跃工作区列表添加/打开新工作区。
        assert!(
            workspace_history_mode(&state),
            "在 ws list 中新建后保持停留在历史列表"
        );
        assert_eq!(
            state.workspaces.len(),
            initial_ws_count,
            "不直接打开新建工作区"
        );
        assert!(
            state.workspace_history.search.text().is_empty(),
            "新建后自动清空搜索过滤以展示新条目"
        );
        let (_, selected_entry) = state
            .workspace_history
            .entry_at_visible_row(state.workspace_history.selected)
            .expect("selected entry");
        let canon_target = std::fs::canonicalize(&target_dir).unwrap_or(target_dir);
        assert_eq!(selected_entry.path, canon_target);
        assert!(state.effects.iter().any(
            |eff| matches!(eff, Effect::RecordWorkspace { path, .. } if *path == canon_target)
        ));
    }
}

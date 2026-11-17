use super::*;
use crate::app::state::{AppState, CompactPanel, CompactRestore, FocusedContainer, RightTab};

fn compact_state(state: &mut AppState) {
    let config = Config::default();
    update_sync(state, &config);
}

fn update_sync(state: &mut AppState, config: &Config) {
    crate::app::update::sync_compact_layout(state, 100, config);
}

#[test]
fn threshold_is_inclusive_at_160() {
    let config = Config::default();
    assert_eq!(config.narrow_width, 160);
    assert!(is_compact(160, &config));
    assert!(!is_compact(161, &config));
    assert!(!is_compact(0, &config));
}

#[test]
fn sync_entering_snapshots_and_expands_without_moving_focus() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.sidebar_collapsed = true;
    state.prompt_collapsed = true;
    state.panes_collapsed = true;
    state.set_focus(FocusedContainer::Sidebar);
    crate::app::update::sync_compact_layout(&mut state, 100, &config);
    assert_eq!(
        state.compact_restore,
        Some(CompactRestore {
            sidebar_collapsed: true,
            prompt_collapsed: true,
            panes_collapsed: true,
        })
    );
    assert!(!state.sidebar_collapsed);
    assert!(!state.prompt_collapsed);
    assert!(!state.panes_collapsed);
    assert_eq!(state.focus(), FocusedContainer::Sidebar);
    assert_eq!(state.compact_panel(), Some(CompactPanel::Left));
}

#[test]
fn sync_exit_restores_snapshot_when_focus_panes() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.compact_restore = Some(CompactRestore {
        sidebar_collapsed: true,
        prompt_collapsed: true,
        panes_collapsed: true,
    });
    state.set_focus(FocusedContainer::Panes);
    crate::app::update::sync_compact_layout(&mut state, 161, &config);
    assert_eq!(state.compact_restore, None);
    assert!(state.sidebar_collapsed);
    assert!(state.prompt_collapsed);
    assert!(state.panes_collapsed);
}

#[test]
fn sync_exit_keeps_panel_expanded_when_focus_prompt() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.compact_restore = Some(CompactRestore {
        sidebar_collapsed: false,
        prompt_collapsed: true,
        panes_collapsed: false,
    });
    state.prompt_collapsed = false;
    state.set_focus(FocusedContainer::Prompt);
    state.right_tab = RightTab::Prompt;
    crate::app::update::sync_compact_layout(&mut state, 161, &config);
    assert_eq!(state.compact_restore, None);
    assert!(!state.prompt_collapsed, "正在编辑的面板不能被重新折叠");
    assert_eq!(state.focus(), FocusedContainer::Prompt);
}

#[test]
fn sync_exit_moves_focus_off_collapsed_sidebar() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.compact_restore = Some(CompactRestore {
        sidebar_collapsed: true,
        prompt_collapsed: false,
        panes_collapsed: false,
    });
    state.set_focus(FocusedContainer::Sidebar);
    crate::app::update::sync_compact_layout(&mut state, 161, &config);
    assert!(state.sidebar_collapsed);
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn toggle_left_opens_and_closes_fullscreen_sidebar() {
    let mut state = AppState::demo();
    compact_state(&mut state);
    crate::app::update::toggle_compact_left(&mut state);
    assert_eq!(state.focus(), FocusedContainer::Sidebar);
    assert!(!state.sidebar_collapsed, "紧凑左栏必须展开");
    crate::app::update::toggle_compact_left(&mut state);
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn toggle_right_opens_current_tab_and_closes() {
    let mut state = AppState::demo();
    compact_state(&mut state);
    state.right_tab = RightTab::Todo;
    crate::app::update::toggle_compact_right(&mut state);
    assert_eq!(state.focus(), FocusedContainer::Todo);
    crate::app::update::toggle_compact_right(&mut state);
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn back_to_chat_clears_prompt_transient_panel() {
    let mut state = AppState::demo();
    compact_state(&mut state);
    state.set_focus(FocusedContainer::Prompt);
    crate::app::update::back_to_compact_chat(&mut state);
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn sync_exit_keeps_panes_expanded_when_prompt_left_and_panes_focused() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.prompt_left = true;
    state.compact_restore = Some(CompactRestore {
        sidebar_collapsed: false,
        prompt_collapsed: false,
        panes_collapsed: true,
    });
    state.set_focus(FocusedContainer::Panes);
    crate::app::update::sync_compact_layout(&mut state, 161, &config);
    assert!(!state.panes_collapsed, "当前停留在主区时不得重新折叠主区");
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn sync_exit_from_collapsed_sidebar_moves_focus_to_right_when_panes_collapsed() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.prompt_left = true;
    state.right_tab = RightTab::Todo;
    state.compact_restore = Some(CompactRestore {
        sidebar_collapsed: true,
        prompt_collapsed: false,
        panes_collapsed: true,
    });
    state.set_focus(FocusedContainer::Sidebar);
    crate::app::update::sync_compact_layout(&mut state, 161, &config);
    assert!(state.sidebar_collapsed);
    assert!(state.panes_collapsed);
    assert_eq!(state.focus(), FocusedContainer::Todo);
}

#[test]
fn compact_transition_clears_active_resizing_drag_and_hover() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.resizing_sidebar = true;
    state.sidebar_hover = true;
    state.resizing_prompt = true;
    state.prompt_hover = true;
    state.workspace_drag = Some(0);
    state.workspace_dragging = true;
    crate::app::update::sync_compact_layout(&mut state, 100, &config);
    assert!(!state.resizing_sidebar);
    assert!(!state.sidebar_hover);
    assert!(!state.resizing_prompt);
    assert!(!state.prompt_hover);
    assert_eq!(state.workspace_drag, None);
    assert!(!state.workspace_dragging);
}

#[test]
fn toggle_actions_are_ignored_during_compact() {
    let mut state = AppState::demo();
    compact_state(&mut state);
    crate::app::update::apply(crate::app::actions::Action::ToggleSidebar, &mut state);
    crate::app::update::apply(crate::app::actions::Action::TogglePrompt, &mut state);
    crate::app::update::apply(crate::app::actions::Action::TogglePanes, &mut state);
    assert!(!state.sidebar_collapsed);
    assert!(!state.prompt_collapsed);
    assert!(!state.panes_collapsed);
}

#[test]
fn focus_prompt_commits_in_progress_todo_edit() {
    let mut state = AppState::demo();
    state.right_tab = RightTab::Todo;
    state.set_focus(FocusedContainer::Todo);
    crate::app::update::apply_todo_button(&mut state, crate::app::tooltip::TodoButton::New);
    if let Some(input) = state.todo.edit_input_mut() {
        input.insert_str("finish compact fix");
    }
    crate::app::update::focus_prompt(&mut state);
    assert_eq!(state.right_tab, RightTab::Prompt);
    assert_eq!(state.focus(), FocusedContainer::Prompt);
    assert!(state.todo.editing().is_none(), "离开 Todo 时必须提交编辑");
    assert_eq!(state.todo.items().len(), 1);
}

#[test]
fn open_existing_worktree_from_compact_left_panel_returns_to_chat() {
    let mut state = AppState::demo();
    let dir = std::env::temp_dir().join("lx-tui-compact-wt-test");
    state.workspaces[0].cwd = Some(dir.clone());
    state.workspaces[0].git = Some(crate::app::state::WorkspaceGit {
        repo_root: dir.clone(),
        checkout_path: dir.clone(),
        is_linked: false,
        branch: Some("main".into()),
        main_branch: Some("main".into()),
    });
    compact_state(&mut state);
    state.set_focus(FocusedContainer::Sidebar);
    crate::app::update::open_worktree_dialog(&mut state, 0);
    crate::app::update::apply_worktree_list(
        &mut state,
        &dir,
        vec![crate::git::WorktreeEntry {
            path: dir.clone(),
            branch: Some("main".into()),
            is_bare: false,
        }],
        false,
    );
    crate::app::update::commit_worktree_open(&mut state);
    assert!(state.overlay.is_none());
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

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

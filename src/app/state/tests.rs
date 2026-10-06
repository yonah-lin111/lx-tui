//! 单元测试；仅测试构建编译。

use super::*;
use std::path::Path;

#[test]
fn demo_state_has_expected_shape() {
    let state = AppState::demo();
    assert_eq!(state.workspaces.len(), 1);
    assert_eq!(state.active_workspace().name, workspace_name());
    assert_eq!(state.active_tab().title, "shell");
    assert_eq!(state.active_tab().layout.pane_ids().len(), 1);
}

#[test]
fn workspace_label_takes_last_segment_and_home() {
    assert_eq!(workspace_label(Path::new("/a/b/lx-tui"), None), "lx-tui");
    assert_eq!(
        workspace_label(Path::new("/Users/me"), Some(Path::new("/Users/me"))),
        "~"
    );
    assert_eq!(workspace_label(Path::new("/"), None), "/");
}

#[test]
fn unique_workspace_name_appends_smallest_free_suffix() {
    let taken = |candidate: &str| candidate == "lx-tui";
    assert_eq!(unique_workspace_name("api", taken), "api");
    assert_eq!(unique_workspace_name("lx-tui", taken), "lx-tui 2");
    let taken = |candidate: &str| candidate == "lx-tui" || candidate == "lx-tui 2";
    assert_eq!(unique_workspace_name("lx-tui", taken), "lx-tui 3");
}

#[test]
fn demo_active_tab_is_terminal() {
    let state = AppState::demo();
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    assert!(
        tab.pane(focus)
            .is_some_and(|pane| pane.kind == PaneKind::Terminal)
    );
}

#[test]
fn prompt_starts_empty_and_is_editable() {
    let mut state = AppState::demo();
    let id = state.prompt.id();
    assert!(state.pane_anywhere(id).is_none());
    assert_eq!(state.prompt.text(), "");
    state.prompt.insert_str("hello");
    assert_eq!(state.prompt.text(), "hello");
}

#[test]
fn all_pane_ids_exclude_global_prompt() {
    let state = AppState::demo();
    let layout_ids: usize = state
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.tabs.iter())
        .map(|tab| tab.layout.pane_ids().len())
        .sum();
    assert_eq!(state.all_pane_ids().len(), layout_ids);
    assert!(!state.all_pane_ids().contains(&state.prompt.id()));
}

#[test]
fn pane_lookup_does_not_reach_prompt() {
    let mut state = AppState::demo();
    let id = state.prompt.id();
    assert!(state.pane_mut_anywhere(id).is_none());
}

#[test]
fn demo_starts_without_overlay_or_scroll() {
    let state = AppState::demo();
    assert!(state.overlay.is_none());
    assert_eq!(state.workspace_scroll, 0);
    assert!(state.workspace_scroll_drag.is_none());
    assert!(!state.workspaces[0].name_is_manual);
    assert!(state.workspaces[0].cwd.is_some());
}

#[test]
fn single_terminal_workspace_is_auto_named_with_root_pane() {
    let workspace = Workspace::single_terminal("workspace 7".to_string(), None);
    assert_eq!(workspace.name, "workspace 7");
    assert_eq!(workspace.active_tab, 0);
    assert_eq!(workspace.tabs.len(), 1);
    assert_eq!(workspace.tabs[0].title, "shell");
    assert_eq!(workspace.tabs[0].layout.pane_ids().len(), 1);
    assert!(!workspace.name_is_manual);
    assert_eq!(
        workspace.root_pane(),
        Some(workspace.tabs[0].layout.focus())
    );
}

//! 单元测试；仅测试构建编译。

use super::*;
use std::path::Path;

#[test]
fn demo_state_has_expected_shape() {
    let state = AppState::demo();
    assert_eq!(state.workspaces.len(), 1);
    assert_eq!(state.active_workspace().name, workspace_name());
    assert_eq!(state.active_workspace().tabs.len(), 1);
    assert_eq!(state.active_tab().name, None);
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
fn tab_label_uses_custom_name_or_position() {
    assert_eq!(tab_label(0, None), "tab 1");
    assert_eq!(tab_label(4, None), "tab 5");
    assert_eq!(tab_label(1, Some("dev")), "dev");
    assert_eq!(tab_label(1, Some("  ")), "tab 2");
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
    assert_eq!(state.tab_scroll, 0);
    assert_eq!(state.workspace_scroll, 0);
    assert!(state.workspace_scroll_drag.is_none());
    assert!(state.workspace_drag.is_none());
    assert!(!state.workspaces[0].name_is_manual);
    assert!(state.workspaces[0].cwd.is_some());
    assert!(state.workspaces[0].is_initial);
    assert_eq!(state.sidebar_width, DEFAULT_SIDEBAR_WIDTH);
    assert!(!state.resizing_sidebar);
    assert!(!state.sidebar_hover);
}

#[test]
fn single_terminal_workspace_is_auto_named_with_root_pane() {
    let workspace = Workspace::single_terminal("workspace 7".to_string(), None);
    assert_eq!(workspace.name, "workspace 7");
    assert_eq!(workspace.active_tab, 0);
    assert_eq!(workspace.tabs.len(), 1);
    assert_eq!(workspace.tabs[0].name, None);
    assert_eq!(workspace.tabs[0].layout.pane_ids().len(), 1);
    assert!(!workspace.name_is_manual);
    assert!(!workspace.is_initial);
    assert_eq!(
        workspace.root_pane(),
        Some(workspace.tabs[0].layout.focus())
    );
}

#[test]
fn panes_default_to_lx_view_and_report_visibility() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    assert_eq!(
        state.pane_anywhere(pane).expect("pane exists").view,
        PaneView::Lx
    );
    assert!(state.lx_visible());

    for workspace in &mut state.workspaces {
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(target) = tab.pane_mut(id) {
                    target.view = PaneView::Terminal;
                }
            }
        }
    }
    assert!(!state.lx_visible());
}

#[test]
fn split_pane_inherits_view_and_focuses_new_pane() {
    let mut state = AppState::demo();
    let source = state.active_tab().layout.focus();
    state.pane_mut_anywhere(source).expect("pane exists").view = PaneView::Terminal;

    let tab = state.active_tab_mut();
    let new_id = tab
        .split_pane(source, Direction::Horizontal)
        .expect("split succeeds");
    assert_eq!(tab.layout.pane_ids().len(), 2);
    assert_eq!(tab.layout.focus(), new_id);
    assert_eq!(tab.pane(new_id).expect("new pane").view, PaneView::Terminal);
    assert_eq!(tab.pane(source).expect("source").view, PaneView::Terminal);
}

#[test]
fn split_pane_rejects_unknown_source() {
    let mut state = AppState::demo();
    let foreign = PaneId::alloc();
    let tab = state.active_tab_mut();
    assert!(tab.split_pane(foreign, Direction::Vertical).is_none());
    assert_eq!(tab.layout.pane_ids().len(), 1);
}

#[test]
fn remove_pane_keeps_at_least_one() {
    let mut state = AppState::demo();
    let only = state.active_tab().layout.focus();
    let tab = state.active_tab_mut();
    assert!(!tab.remove_pane(only));
    assert_eq!(tab.layout.pane_ids(), vec![only]);
    assert!(tab.pane(only).is_some());
}

#[test]
fn remove_pane_drops_payload_and_promotes_sibling() {
    let mut state = AppState::demo();
    let tab = state.active_tab_mut();
    let first = tab.layout.focus();
    let second = tab
        .split_pane(first, Direction::Horizontal)
        .expect("split succeeds");

    assert!(tab.remove_pane(second));
    assert_eq!(tab.layout.pane_ids(), vec![first]);
    assert_eq!(tab.layout.focus(), first);
    assert!(tab.pane(second).is_none());
}

#[test]
fn workspace_cwd_for_pane_finds_owning_workspace_only() {
    let state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    assert_eq!(
        state.workspace_cwd_for_pane(pane),
        state.workspaces[0].cwd.clone()
    );
    assert_eq!(state.workspace_cwd_for_pane(state.prompt.id()), None);
    assert_eq!(state.workspace_cwd_for_pane(PaneId::alloc()), None);
}

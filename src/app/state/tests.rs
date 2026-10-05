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
fn workspace_name_uses_current_path_last_segment() {
    assert_eq!(workspace_name_from(Path::new("/a/b/lx-tui")), "lx-tui");
    assert_eq!(workspace_name_from(Path::new("/")), FALLBACK_WORKSPACE_NAME);
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
fn prompt_is_global_and_has_selectable_content() {
    let mut state = AppState::demo();
    let id = state.prompt.id();
    let pane = state.pane_anywhere(id);
    assert!(pane.is_some_and(|pane| pane.kind == PaneKind::Prompt));
    let Some(pane) = state.pane_mut_anywhere(id) else {
        return;
    };
    let text = pane
        .terminal
        .text_in_range((0, 0), (0, 3))
        .unwrap_or_default();
    assert_eq!(text, "Drag");
}

#[test]
fn all_pane_ids_include_global_prompt() {
    let state = AppState::demo();
    let layout_ids: usize = state
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.tabs.iter())
        .map(|tab| tab.layout.pane_ids().len())
        .sum();
    assert_eq!(state.all_pane_ids().len(), layout_ids + 1);
    assert!(state.all_pane_ids().contains(&state.prompt.id()));
}

#[test]
fn pane_mut_anywhere_reaches_prompt() {
    let mut state = AppState::demo();
    let id = state.prompt.id();
    let pane = state.pane_mut_anywhere(id);
    assert!(pane.is_some());
}

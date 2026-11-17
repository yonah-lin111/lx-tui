use super::*;
use crate::app::overlay::Overlay;
use crate::app::state::{AppState, CompactRow, CompactSection, FocusedContainer};
use crate::app::update;
use crate::config::Config;
use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};

fn set_agent(state: &mut AppState, workspace: usize, agent_state: AgentState) {
    let pane = state.workspaces[workspace]
        .tabs
        .first()
        .and_then(|tab| tab.root_pane());
    let Some(pane) = pane else {
        return;
    };
    if let Some(target) = state.workspaces[workspace]
        .tabs
        .first_mut()
        .and_then(|tab| tab.pane_mut(pane))
    {
        target.agent = Some(PaneAgentSnapshot {
            kind: AgentKind::Claude,
            state: agent_state,
            title: None,
        });
    }
}

fn switcher(state: &AppState) -> &CompactSwitcher {
    match state.overlay.as_ref() {
        Some(Overlay::CompactSwitcher(switcher)) => switcher,
        _ => panic!("switcher is not open"),
    }
}

fn select_row(state: &mut AppState, predicate: impl Fn(&CompactRow) -> bool) {
    let index = switcher(state)
        .rows
        .iter()
        .position(predicate)
        .expect("row");
    update::select_compact_switcher_row(state, index);
}

#[test]
fn sync_entering_snapshots_and_expands() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.prompt_collapsed = true;
    state.panes_collapsed = true;
    state.set_focus(FocusedContainer::Sidebar);
    update::sync_compact_layout(&mut state, 60, 24, &config);
    assert_eq!(
        state.compact_restore,
        Some(CompactRestore {
            prompt_collapsed: true,
            panes_collapsed: true,
        })
    );
    assert!(!state.prompt_collapsed);
    assert!(!state.panes_collapsed);
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn sync_exit_restores_snapshot_when_focus_panes() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.compact_restore = Some(CompactRestore {
        prompt_collapsed: true,
        panes_collapsed: true,
    });
    state.prompt_collapsed = false;
    state.panes_collapsed = false;
    state.set_focus(FocusedContainer::Panes);
    update::sync_compact_layout(&mut state, 100, 30, &config);
    assert_eq!(state.compact_restore, None);
    assert!(state.prompt_collapsed);
    assert!(state.panes_collapsed);
}

#[test]
fn sync_exit_keeps_panel_expanded_when_focus_prompt() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.compact_restore = Some(CompactRestore {
        prompt_collapsed: true,
        panes_collapsed: false,
    });
    state.prompt_collapsed = false;
    state.set_focus(FocusedContainer::Prompt);
    state.right_tab = RightTab::Prompt;
    update::sync_compact_layout(&mut state, 100, 30, &config);
    assert_eq!(state.compact_restore, None);
    assert!(!state.prompt_collapsed, "正在编辑的面板不能被重新折叠");
    assert_eq!(state.focus(), FocusedContainer::Prompt);
}

#[test]
fn sync_exit_closes_switcher() {
    let mut state = AppState::demo();
    let config = Config::default();
    state.compact_restore = Some(CompactRestore {
        prompt_collapsed: false,
        panes_collapsed: false,
    });
    update::open_compact_switcher(&mut state, 10);
    update::sync_compact_layout(&mut state, 100, 30, &config);
    assert!(state.overlay.is_none());
}

#[test]
fn activate_agent_switches_workspace_tab_and_pane() {
    let mut state = AppState::demo();
    set_agent(&mut state, 0, AgentState::Working);
    let target = state.workspaces[0].tabs[0].layout.focus();
    update::create_workspace(&mut state);
    assert_eq!(state.active_workspace, 1);

    update::open_compact_switcher(&mut state, 10);
    select_row(&mut state, |row| matches!(row, CompactRow::Agent(_)));
    assert_eq!(state.active_workspace, 0);
    assert_eq!(state.active_tab().layout.focus(), target);
    assert_eq!(state.focus(), FocusedContainer::Panes);
    assert!(state.overlay.is_none());
}

#[test]
fn activate_panel_enters_fullscreen_and_back_returns() {
    let mut state = AppState::demo();
    update::open_compact_switcher(&mut state, 10);
    select_row(&mut state, |row| {
        matches!(row, CompactRow::Panel(RightTab::Prompt))
    });
    assert_eq!(state.focus(), FocusedContainer::Prompt);
    assert_eq!(state.right_tab, RightTab::Prompt);
    assert!(state.overlay.is_none());

    update::back_to_compact_chat(&mut state);
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn new_tab_row_creates_tab() {
    let mut state = AppState::demo();
    let before = state.active_workspace().tabs.len();
    update::open_compact_switcher(&mut state, 10);
    select_row(&mut state, |row| matches!(row, CompactRow::NewTab));
    assert_eq!(state.active_workspace().tabs.len(), before + 1);
    assert!(state.overlay.is_none());
}

#[test]
fn new_workspace_row_opens_dialog() {
    let mut state = AppState::demo();
    update::open_compact_switcher(&mut state, 10);
    select_row(&mut state, |row| matches!(row, CompactRow::NewWorkspace));
    assert!(matches!(state.overlay, Some(Overlay::NewWorkspace(_))));
}

#[test]
fn navigation_skips_section_rows() {
    let mut state = AppState::demo();
    set_agent(&mut state, 0, AgentState::Working);
    update::open_compact_switcher(&mut state, 10);
    for _ in 0..20 {
        assert!(switcher(&state).selected_row().is_some());
        update::move_compact_switcher(&mut state, 1);
    }
    assert!(switcher(&state).selected_row().is_some());
}

#[test]
fn escape_closes_switcher() {
    let mut state = AppState::demo();
    update::open_compact_switcher(&mut state, 10);
    assert!(update::apply_compact_switcher_key(
        &mut state,
        OverlayKey::Esc
    ));
    assert!(state.overlay.is_none());
}

#[test]
fn page_moves_by_viewport() {
    let mut state = AppState::demo();
    for _ in 0..6 {
        update::create_tab(&mut state);
    }
    update::open_compact_switcher(&mut state, 3);
    let before = switcher(&state).selected;
    update::page_compact_switcher(&mut state, 1);
    let after = switcher(&state).selected;
    assert!(after > before);
    assert!(switcher(&state).selected_row().is_some());
}

#[test]
fn workspace_row_switches_workspace() {
    let mut state = AppState::demo();
    update::create_workspace(&mut state);
    update::open_compact_switcher(&mut state, 10);
    select_row(&mut state, |row| matches!(row, CompactRow::Workspace(0)));
    assert_eq!(state.active_workspace, 0);
}

#[test]
fn compact_rows_snapshot_includes_sections_in_order() {
    let mut state = AppState::demo();
    set_agent(&mut state, 0, AgentState::Idle);
    let rows = state.compact_rows();
    let sections: Vec<_> = rows
        .iter()
        .filter_map(|row| match row {
            CompactRow::Section(section) => Some(*section),
            _ => None,
        })
        .collect();
    assert_eq!(
        sections,
        vec![
            CompactSection::Agents,
            CompactSection::Spaces,
            CompactSection::Tabs,
            CompactSection::Panels,
            CompactSection::Actions,
        ]
    );
}

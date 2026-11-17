use super::*;
use crate::app::state::{FocusedContainer, RightTab};
use crate::app::update;
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

#[test]
fn compact_panel_follows_focus() {
    let mut state = AppState::demo();
    assert_eq!(state.compact_panel(), None);
    state.set_focus(FocusedContainer::Sidebar);
    assert_eq!(state.compact_panel(), Some(CompactPanel::Left));
    state.set_focus(FocusedContainer::Prompt);
    assert_eq!(state.compact_panel(), Some(CompactPanel::Right));
    state.set_focus(FocusedContainer::Todo);
    assert_eq!(state.compact_panel(), Some(CompactPanel::Right));
    state.right_tab = RightTab::Todo;
    assert_eq!(state.right_tab, RightTab::Todo);
}

#[test]
fn agent_entries_stay_in_active_workspace() {
    let mut state = AppState::demo();
    set_agent(&mut state, 0, AgentState::Working);
    assert_eq!(state.agent_entries().len(), 1);

    update::create_workspace(&mut state);
    assert_eq!(state.active_workspace, 1);
    assert!(state.agent_entries().is_empty());
    assert_eq!(state.agent_entries_for_workspace(0).len(), 1);
}

#[test]
fn global_agent_states_span_workspaces() {
    let mut state = AppState::demo();
    set_agent(&mut state, 0, AgentState::Blocked);
    update::create_workspace(&mut state);
    set_agent(&mut state, 1, AgentState::Working);

    let states = state.global_agent_states();
    assert_eq!(states, vec![AgentState::Blocked, AgentState::Working]);
}

#[test]
fn global_agent_states_empty_without_agents() {
    let state = AppState::demo();
    assert!(state.global_agent_states().is_empty());
}

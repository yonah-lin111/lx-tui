use super::*;
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
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    assert_eq!(state.compact_panel(), Some(RightTab::Prompt));
    state.set_focus(crate::app::state::FocusedContainer::Todo);
    assert_eq!(state.compact_panel(), Some(RightTab::Todo));
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
fn global_agent_entries_span_workspaces() {
    let mut state = AppState::demo();
    set_agent(&mut state, 0, AgentState::Blocked);
    update::create_workspace(&mut state);
    set_agent(&mut state, 1, AgentState::Working);

    let entries = state.global_agent_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].workspace, 0);
    assert_eq!(entries[0].entry.state, AgentState::Blocked);
    assert_eq!(entries[1].workspace, 1);
    assert_eq!(entries[1].entry.state, AgentState::Working);
}

#[test]
fn compact_rows_order_and_empty_agents_are_skipped() {
    let mut state = AppState::demo();
    let rows = state.compact_rows();
    assert!(matches!(
        rows.first(),
        Some(CompactRow::Section(CompactSection::Spaces))
    ));
    assert!(
        rows.iter()
            .any(|row| matches!(row, CompactRow::Workspace(0)))
    );
    assert!(rows.iter().any(|row| matches!(row, CompactRow::Tab(0))));
    assert!(rows.iter().any(|row| matches!(
        row,
        CompactRow::Panel(RightTab::Prompt) | CompactRow::Panel(RightTab::Todo)
    )));
    assert!(matches!(rows.last(), Some(CompactRow::NewTab)));

    set_agent(&mut state, 0, AgentState::Working);
    let rows = state.compact_rows();
    assert!(matches!(
        rows.first(),
        Some(CompactRow::Section(CompactSection::Agents))
    ));
    assert!(matches!(rows.get(1), Some(CompactRow::Agent(_))));
}

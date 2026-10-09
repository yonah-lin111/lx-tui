//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::state::AppState;
use crate::app::update;
use crate::config::Config;
use crate::detect::{AgentKind, AgentState};
use crate::layout::PaneId;
use crate::ui::render;
use ratatui::Terminal as RatatuiTerminal;
use ratatui::backend::TestBackend;

fn view_for(state: &AppState) -> layout::ViewLayout {
    layout::compute(
        Rect::new(0, 0, 100, 24),
        &Config::default(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
    )
}

fn render_lines(state: &AppState) -> Vec<String> {
    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

fn agent_snapshot(kind: AgentKind, state: AgentState) -> PaneAgentSnapshot {
    PaneAgentSnapshot {
        kind,
        state,
        title: None,
    }
}

/// 给当前标签的焦点窗格挂一个 Agent 快照；返回窗格标识。
fn add_agent(state: &mut AppState, kind: AgentKind, agent_state: AgentState) -> PaneId {
    let pane = state.active_tab().layout.focus();
    update::update_pane_agent(state, pane, Some(agent_snapshot(kind, agent_state)));
    pane
}

/// 追加 `count` 个各带一个 Agent 的标签。
fn add_agent_tabs(state: &mut AppState, count: usize) {
    for _ in 0..count {
        update::create_tab(state);
        add_agent(state, AgentKind::Codex, AgentState::Idle);
    }
}

#[test]
fn agents_section_renders_empty_state() {
    let state = AppState::demo();
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let lines = render_lines(&state);
    let row = &lines[sections.agents.y as usize];
    assert!(
        row.contains(text::AGENTS_EMPTY),
        "empty agents section must show hint: {row:?}"
    );
}

#[test]
fn agent_item_shows_dot_name_and_location() {
    let mut state = AppState::demo();
    let pane = add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let lines = render_lines(&state);
    let row = &lines[sections.agents.y as usize];
    assert!(
        row.contains(&format!(
            "{} claude {}",
            text::AGENT_STATUS_DOT,
            text::agent_location(0, pane)
        )),
        "agent row must show dot, name and location: {row:?}"
    );
}

#[test]
fn agent_dot_follows_state() {
    let mut state = AppState::demo();
    let pane = add_agent(&mut state, AgentKind::Codex, AgentState::Idle);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let idle = render_lines(&state)[sections.agents.y as usize].clone();
    assert!(
        idle.contains(text::AGENT_STATUS_RING),
        "idle agent uses ring dot: {idle:?}"
    );

    let snapshot = agent_snapshot(AgentKind::Codex, AgentState::Blocked);
    update::update_pane_agent(&mut state, pane, Some(snapshot));
    let blocked = render_lines(&state)[sections.agents.y as usize].clone();
    assert!(
        blocked.contains(text::AGENT_STATUS_DOT),
        "blocked agent uses filled dot: {blocked:?}"
    );
}

#[test]
fn agent_items_follow_tabs_and_pane_order() {
    let mut state = AppState::demo();
    let first = add_agent(&mut state, AgentKind::Claude, AgentState::Idle);
    update::create_tab(&mut state);
    let second = add_agent(&mut state, AgentKind::OpenCode, AgentState::Working);
    let items = agent_items(&state);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].pane_id, first);
    assert_eq!(items[0].tab_index, 0);
    assert_eq!(items[1].pane_id, second);
    assert_eq!(items[1].tab_index, 1);
}

#[test]
fn agent_item_at_hits_content_only() {
    let mut state = AppState::demo();
    let pane = add_agent(&mut state, AgentKind::Pi, AgentState::Working);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    assert_eq!(
        agent_item_at(&view, &state, sections.agents.x, sections.agents.y),
        Some(pane)
    );
    assert_eq!(
        agent_item_at(&view, &state, sections.agents.x, sections.divider.y),
        None
    );
    assert_eq!(
        agent_item_at(&view, &state, sections.agents.x, sections.agents.bottom()),
        None
    );
}

#[test]
fn agent_item_at_respects_scroll_offset() {
    let mut state = AppState::demo();
    add_agent_tabs(&mut state, 20);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    assert!(agent_scrollbar(&view, &state).is_some(), "overflow scrolls");
    state.agent_scroll = 1;
    let slot = agent_item_index_at(&view, &state, sections.agents.x, sections.agents.y);
    assert_eq!(slot, Some(1), "first visible row maps to slot 1");
    let pane = agent_item_at(&view, &state, sections.agents.x, sections.agents.y);
    assert_eq!(pane, Some(state.agent_panes()[1].1));
}

#[test]
fn agent_scrollbar_column_is_not_hittable() {
    let mut state = AppState::demo();
    add_agent_tabs(&mut state, 20);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let bar = agent_scrollbar(&view, &state).expect("scrollbar is visible");
    assert_eq!(bar.track.x, sections.agents.right() - 1);
    assert_eq!(
        agent_item_at(&view, &state, bar.track.x, sections.agents.y),
        None
    );
}

#[test]
fn collapsed_agents_section_is_not_hittable() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Kimi, AgentState::Idle);
    state.agents_collapsed = true;
    let view = view_for(&state);
    assert_eq!(agent_list_rows(&view, &state), None);
    assert_eq!(agent_item_at(&view, &state, view.sidebar.x + 2, 20), None);
    assert!(!agents_section_at(&view, &state, view.sidebar.x + 2, 20));
}

#[test]
fn hover_row_gets_selection_background() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Gemini, AgentState::Working);
    state.agent_hover = Some(0);
    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    assert_eq!(
        buffer[(sections.agents.x, sections.agents.y)].style().bg,
        style::selection().bg,
        "hovered agent row must use selection background"
    );
}

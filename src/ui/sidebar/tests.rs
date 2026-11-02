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
        state.prompt_left,
        state.panes_collapsed,
    )
}

fn render_buffer(state: &AppState) -> ratatui::buffer::Buffer {
    let config = Config::default();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    terminal.backend().buffer().clone()
}

fn render_lines(state: &AppState) -> Vec<String> {
    let buffer = render_buffer(state);
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

/// Agents 分区首行的内容文本（去侧栏边框与内边距）。
fn agents_row_text(state: &AppState) -> String {
    agents_row_text_at(state, 0)
}

/// Agents 分区指定行偏移的内容文本（去侧栏边框与内边距）。
fn agents_row_text_at(state: &AppState, row_offset: u16) -> String {
    let view = view_for(state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    render_lines(state)[usize::from(sections.agents.y + row_offset)]
        .chars()
        .skip(view.sidebar.x as usize + 1)
        .take(view.sidebar.width as usize - 2)
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
fn agent_item_shows_dot_name_and_right_aligned_location() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    let row = agents_row_text(&state);
    let location = text::agent_location(0);
    assert!(
        row.starts_with(&format!(
            "{} {} ",
            text::AGENT_SELECTED_MARKER,
            text::AGENT_STATUS_DOT
        )),
        "focused agent row must show marker and dot before the name: {row:?}"
    );
    assert!(row.contains("clau"), "agent name is rendered: {row:?}");
    assert!(
        row.ends_with(&location),
        "location must be right aligned: {row:?}"
    );
}

#[test]
fn selected_agent_row_uses_symbol_marker_without_color() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    let row = agents_row_text(&state);
    assert!(
        row.starts_with(&format!("{} ", text::AGENT_SELECTED_MARKER)),
        "focused pane marks its agent item: {row:?}"
    );

    let buffer = render_buffer(&state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    let marker = &buffer[(sections.agents.x, sections.agents.y)];
    assert_eq!(
        marker.fg,
        ratatui::style::Color::Reset,
        "marker must keep the default foreground, not a highlight color"
    );
    assert_eq!(
        marker.bg,
        ratatui::style::Color::Reset,
        "marker has no background"
    );

    // 切换到其他容器（Prompt / Todo / Sidebar）时，当前激活窗格的 Agent 仍保留行首 `❯` 标记。
    for container in [
        crate::app::state::FocusedContainer::Prompt,
        crate::app::state::FocusedContainer::Todo,
        crate::app::state::FocusedContainer::Sidebar,
    ] {
        state.set_focus(container);
        let row = agents_row_text(&state);
        assert!(
            row.starts_with(&format!("{} ", text::AGENT_SELECTED_MARKER)),
            "active pane agent must keep marker when {container:?} is focused: {row:?}"
        );
    }

    // 切换到其它无该 Agent 的标签/窗格时，非当前聚焦窗格的条目让出标记列为空格。
    update::create_tab(&mut state);
    let row = agents_row_text(&state);
    assert!(
        !row.contains(text::AGENT_SELECTED_MARKER),
        "unfocused pane must not mark its agent item: {row:?}"
    );
    assert!(
        row.starts_with(&format!("  {} ", text::AGENT_STATUS_DOT)),
        "marker column stays reserved: {row:?}"
    );

    // 切回原标签后，该标签的焦点窗格重新显示 `❯` 标记。
    update::switch_tab(&mut state, 0);
    let row = agents_row_text(&state);
    assert!(
        row.starts_with(&format!("{} ", text::AGENT_SELECTED_MARKER)),
        "switching back to agent tab restores marker: {row:?}"
    );
}

#[test]
fn split_pane_agent_marker_tracks_active_pane_across_container_focus() {
    let mut state = AppState::demo();
    let first = add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    let second = state
        .active_tab_mut()
        .split_pane(first, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    assert_eq!(state.active_tab().layout.focus(), second);

    // 焦点在同标签的无 Agent 兄弟分屏上时，首个分屏的 Agent 不标 `❯`。
    for container in [
        crate::app::state::FocusedContainer::Panes,
        crate::app::state::FocusedContainer::Prompt,
    ] {
        state.set_focus(container);
        let row = agents_row_text_at(&state, 0);
        assert!(
            !row.contains(text::AGENT_SELECTED_MARKER),
            "sibling non-agent pane focus must not mark first agent ({container:?}): {row:?}"
        );
    }

    // 第二个分屏也挂载 Agent：仅当前焦点分屏（second）带 `❯`，且跨容器失焦后保持。
    update::update_pane_agent(
        &mut state,
        second,
        Some(agent_snapshot(AgentKind::OpenCode, AgentState::Idle)),
    );
    for container in [
        crate::app::state::FocusedContainer::Panes,
        crate::app::state::FocusedContainer::Prompt,
        crate::app::state::FocusedContainer::Todo,
        crate::app::state::FocusedContainer::Sidebar,
    ] {
        state.set_focus(container);
        let first_row = agents_row_text_at(&state, 0);
        let second_row = agents_row_text_at(&state, 1);
        assert!(
            !first_row.contains(text::AGENT_SELECTED_MARKER),
            "first pane is not focused ({container:?}): {first_row:?}"
        );
        assert!(
            second_row.starts_with(&format!("{} ", text::AGENT_SELECTED_MARKER)),
            "second pane keeps marker ({container:?}): {second_row:?}"
        );
    }

    // 焦点切回第一个分屏后再点击其他容器，标记同步切回第一个分屏并保持。
    update::focus_pane(&mut state, first);
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    let first_row = agents_row_text_at(&state, 0);
    let second_row = agents_row_text_at(&state, 1);
    assert!(
        first_row.starts_with(&format!("{} ", text::AGENT_SELECTED_MARKER)),
        "first pane keeps marker after focus switch: {first_row:?}"
    );
    assert!(
        !second_row.contains(text::AGENT_SELECTED_MARKER),
        "second pane loses marker after focus switch: {second_row:?}"
    );
}

#[test]
fn agent_location_stays_visible_on_narrow_sidebar() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::OpenCode, AgentState::Idle);
    state.sidebar_width = 18;
    let row = agents_row_text(&state);
    assert!(
        row.ends_with(&text::agent_location(0)),
        "location must survive a narrow sidebar: {row:?}"
    );
    assert!(
        !row.contains("opencode"),
        "name is truncated first to keep location visible: {row:?}"
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
fn focused_pane_agent_row_has_no_selected_background() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    let buffer = render_buffer(&state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    assert_ne!(
        buffer[(sections.agents.x, sections.agents.y)].style().bg,
        style::selected_item().bg,
        "pane focus alone must not fill the agent row"
    );
}

#[test]
fn agent_hover_keeps_status_dot_color() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    state.agent_hover = Some(0);
    let buffer = render_buffer(&state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    // 行首 `❯ ` 标记之后是状态圆点格：状态色保留，仅叠加 hover 背景。
    let dot_x = sections.agents.x + text::AGENT_SELECTED_MARKER.chars().count() as u16 + 1;
    let cell = &buffer[(dot_x, sections.agents.y)];
    assert_eq!(cell.symbol(), text::AGENT_STATUS_DOT);
    assert_eq!(
        cell.fg,
        style::agent_status(AgentState::Working)
            .fg
            .expect("working fg"),
        "hover 不覆盖运行中状态色"
    );
    assert_eq!(
        cell.bg,
        style::selection().bg.expect("selection bg"),
        "hover 背景叠加在状态色之下"
    );
}

#[test]
fn hover_row_gets_selection_background() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Gemini, AgentState::Working);
    state.agent_hover = Some(0);
    let buffer = render_buffer(&state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.sidebar, false).expect("sections are visible");
    assert_eq!(
        buffer[(sections.agents.x, sections.agents.y)].style().bg,
        style::selection().bg,
        "hovered agent row must use selection background"
    );
}

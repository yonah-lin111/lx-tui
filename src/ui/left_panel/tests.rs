//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::state::AppState;
use crate::app::update;
use crate::config::Config;
use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};
use crate::tiling::PaneId;
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
    render_lines(state)[usize::from(sections.agents.y + row_offset)]
        .chars()
        .skip(view.left_panel.x as usize + 1)
        .take(view.left_panel.width as usize - 2)
        .collect()
}

fn agent_snapshot(kind: AgentKind, state: AgentState) -> PaneAgentSnapshot {
    PaneAgentSnapshot {
        kind,
        state,
        title: None,
    }
}

/// 给当前标签的焦点窗格挂一个终端 Agent 快照并切到终端视图；返回窗格标识。
fn add_agent(state: &mut AppState, kind: AgentKind, agent_state: AgentState) -> PaneId {
    let pane = state.active_tab().layout.focus();
    if state
        .active_tab()
        .pane(pane)
        .is_some_and(|target| target.view == PaneView::Lx)
    {
        update::toggle_pane_view(state, pane);
    }
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
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
    assert_eq!(
        agent_item_at(&view, &state, view.left_panel.x + 2, 20),
        None
    );
    assert!(!agents_section_at(&view, &state, view.left_panel.x + 2, 20));
}

#[test]
fn focused_pane_agent_row_has_no_selected_background() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    let buffer = render_buffer(&state);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
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
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
    assert_eq!(
        buffer[(sections.agents.x, sections.agents.y)].style().bg,
        style::selection().bg,
        "hovered agent row must use selection background"
    );
}

#[test]
fn lx_entry_only_appears_in_agents_when_session_is_open_and_renders_mode_labels() {
    use crate::app::state::{AiWebSite, LxEntry};

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    assert!(
        agent_items(&state).is_empty(),
        "welcome page without session must not appear in agents list"
    );

    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = Some("ses-1".to_string());
    }
    let cases = [
        (LxEntry::Native, AiWebSite::Deepseek, "lx-native"),
        (LxEntry::Hybrid, AiWebSite::Deepseek, "lx-hybrid"),
        (LxEntry::Automation, AiWebSite::Deepseek, "lx-automation"),
        (LxEntry::AiWeb, AiWebSite::Deepseek, "lx-web-deepseek"),
        (LxEntry::AiWeb, AiWebSite::Gemini, "lx-web-gemini"),
        (LxEntry::AiWeb, AiWebSite::Grok, "lx-web-grok"),
    ];
    for (entry, site, expected_label) in cases {
        if let Some(target) = state.active_tab_mut().pane_mut(pane) {
            target.lx.entry = entry;
            target.lx.ai_web_site = site;
        }
        let items = agent_items(&state);
        assert_eq!(items.len(), 1);
        assert_eq!(text::sidebar_agent_label(items[0].kind), expected_label);
        let row = agents_row_text(&state);
        assert!(
            row.starts_with(&format!(
                "{} {} ",
                text::AGENT_SELECTED_MARKER,
                text::AGENT_STATUS_RING
            )),
            "focused lx session row must show marker and idle ring: {row:?}"
        );
    }

    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = None;
    }
    assert!(
        agent_items(&state).is_empty(),
        "closing/resetting session removes lx item from agents list"
    );
}

#[test]
fn single_pane_renders_both_lx_and_terminal_agent_rows_and_marker_follows_view() {
    let mut state = AppState::demo();
    let pane = add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = Some("ses-dual".to_string());
    }

    let items = agent_items(&state);
    assert_eq!(
        items.len(),
        2,
        "same pane with lx session and claude yields 2 rows"
    );
    assert_eq!(text::sidebar_agent_label(items[0].kind), "lx-native");
    assert_eq!(text::sidebar_agent_label(items[1].kind), "claude");

    // 当前在 Terminal 视图：第 0 行 lx-native 不带 ❯，第 1 行 claude 带 ❯。
    let row0_terminal = agents_row_text_at(&state, 0);
    let row1_terminal = agents_row_text_at(&state, 1);
    assert!(
        row0_terminal.starts_with("  ○ lx-native"),
        "background lx row has no marker when viewing terminal: {row0_terminal:?}"
    );
    assert!(
        row1_terminal.starts_with(&format!(
            "{} {} clau",
            text::AGENT_SELECTED_MARKER,
            text::AGENT_STATUS_DOT
        )),
        "visible terminal agent row has marker: {row1_terminal:?}"
    );

    // 切回 Lx 视图：第 0 行 lx-native 带 ❯，第 1 行 claude 不带 ❯。
    update::toggle_pane_view(&mut state, pane);
    let row0_lx = agents_row_text_at(&state, 0);
    let row1_lx = agents_row_text_at(&state, 1);
    assert!(
        row0_lx.starts_with(&format!(
            "{} {} lx-native",
            text::AGENT_SELECTED_MARKER,
            text::AGENT_STATUS_RING
        )),
        "visible lx row has marker when viewing lx: {row0_lx:?}"
    );
    assert!(
        row1_lx.starts_with("  ● clau"),
        "background terminal agent row has no marker when viewing lx: {row1_lx:?}"
    );

    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
    assert_eq!(
        agent_target_at(&view, &state, sections.agents.x, sections.agents.y),
        Some((pane, PaneView::Lx))
    );
    assert_eq!(
        agent_target_at(&view, &state, sections.agents.x, sections.agents.y + 1),
        Some((pane, PaneView::Terminal))
    );
}

#[test]
fn lx_agent_dot_tracks_idle_working_and_blocked_states() {
    use crate::app::state::{AskState, RequestKey, RequestState};

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    // 首次提交阶段（Committing，尚未落库生成 session_id）：收录并显示 Working。
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.request = RequestState::Committing {
            generation: 1,
            draft: "hello".to_string(),
        };
    }
    assert_eq!(agent_items(&state).len(), 1);
    assert_eq!(agent_items(&state)[0].state, AgentState::Working);

    // `/resume` 加载中（loading_session）：收录并显示 Working。
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.request = RequestState::Idle;
        target.lx.chat.loading_session = true;
    }
    assert_eq!(agent_items(&state).len(), 1);
    assert_eq!(agent_items(&state)[0].state, AgentState::Working);

    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.loading_session = false;
        target.lx.chat.session_id = Some("ses-status".to_string());
    }
    assert_eq!(agent_items(&state)[0].state, AgentState::Idle);

    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.status.streaming = true;
    }
    assert_eq!(agent_items(&state)[0].state, AgentState::Working);

    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.ask = Some(AskState::new(
            RequestKey {
                pane,
                session: "ses-status".to_string(),
                generation: 1,
            },
            "call-1".to_string(),
            Vec::new(),
        ));
    }
    assert_eq!(
        agent_items(&state)[0].state,
        AgentState::Blocked,
        "pending ask takes priority as Blocked"
    );
}

#[test]
fn slash_new_and_site_switch_while_streaming_immediately_remove_lx_agent_row() {
    use crate::app::state::{AiWebSite, LxEntry, RequestKey, RequestState};

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = Some("sess-1".to_string());
        target.lx.chat.request = RequestState::Streaming {
            key: RequestKey {
                pane,
                session: "sess-1".to_string(),
                generation: 1,
            },
            assistant_id: "a1".to_string(),
        };
        target.lx.chat.status.streaming = true;
    }
    assert_eq!(agent_items(&state).len(), 1);
    assert_eq!(agent_items(&state)[0].state, AgentState::Working);

    // 流式中执行 `/new`：旧请求仍在后台取消收尾，但侧栏条目必须立即移除。
    assert!(update::new_session(&mut state, pane));
    assert!(
        agent_items(&state).is_empty(),
        "slash /new while streaming must immediately remove lx agent item"
    );
    assert_eq!(state.agent_entry_count(), 0);

    // 若此时加载另一个空闲会话 `sess-2`，旧会话 `sess-1` 的后台 Streaming 不得把 `sess-2` 污染为 Working。
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = Some("sess-2".to_string());
    }
    assert_eq!(agent_items(&state).len(), 1);
    assert_eq!(
        agent_items(&state)[0].state,
        AgentState::Idle,
        "unbound old streaming request must not mark newly loaded session as Working"
    );

    // 加载 AI Web 云端会话时自动同步 `entry = LxEntry::AiWeb` 与站点名称，切站点后立即移除。
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.entry = LxEntry::Native;
        target.lx.chat.loading_session = true;
    }
    assert!(update::on_ai_web_session_loaded(
        &mut state,
        pane,
        "ai-web-grok-42".to_string(),
        Some("Grok Chat".to_string()),
        Vec::new(),
    ));
    let items = agent_items(&state);
    assert_eq!(items.len(), 1);
    assert_eq!(text::sidebar_agent_label(items[0].kind), "lx-web-grok");

    update::set_ai_web_site(&mut state, pane, AiWebSite::Gemini);
    assert!(
        agent_items(&state).is_empty(),
        "switching ai web site resets session and removes lx agent item"
    );
}

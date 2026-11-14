//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::markdown::TemplateStatus;
use crate::app::prompt::{PromptItem, PromptPage, PromptPageId, PromptTree};
use crate::app::state::{AgentPromptRef, AgentTitleOverride, AppState, Workspace};
use crate::app::update;
use crate::config::Config;
use crate::detect::{AgentKind, AgentState, PaneAgentSnapshot};
use crate::tiling::PaneId;
use crate::ui::render;
use crate::ui::test_support::{git_info, push_git_workspace};
use ratatui::Terminal as RatatuiTerminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

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

/// Agents 分区指定行偏移的内容文本（去侧栏边框与内边距；宽字符跳过续行空格）。
fn agents_row_text_at(state: &AppState, row_offset: u16) -> String {
    use unicode_width::UnicodeWidthStr;

    let view = view_for(state);
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
    let buffer = render_buffer(state);
    let y = sections.agents.y + row_offset;
    let end = view
        .left_panel
        .x
        .saturating_add(view.left_panel.width)
        .saturating_sub(1);
    let mut text = String::new();
    let mut column = view.left_panel.x + 1;
    while column < end {
        let symbol = buffer[(column, y)].symbol();
        text.push_str(symbol);
        column += u16::try_from(symbol.width().max(1)).expect("symbol width fits u16");
    }
    text
}

/// Agents 分区第 `item_index` 个条目的第 `line` 行（0 = 名称/归属行，1 = 标题行）。
fn agent_item_line(state: &AppState, item_index: usize, line: usize) -> String {
    let row_offset = u16::try_from(item_index * 2 + line).expect("test row fits u16");
    agents_row_text_at(state, row_offset)
}

/// 条目应显示的归属标签：T 为 1 基标签序，P 为 1 基窗格树序。
fn item_location(item: &AgentListItem<'_>) -> String {
    format!("[T{} P{}]", item.tab_index + 1, item.pane_index + 1)
}

/// 绑定 Recent 页的侧栏状态色测试状态：页正文按 `&&&` 结束行设置模板状态。
fn state_with_bound_title(status: Option<TemplateStatus>) -> AppState {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane exists")
        .lx
        .chat
        .session_id = Some("ses-title-color".to_string());
    let content = match status {
        Some(TemplateStatus::Todo) => {
            "&&& addTemplate --start\n{title: Auth Flow}\n- body\n&&& addTemplate --end"
        }
        Some(TemplateStatus::InProgress) => {
            "&&& addTemplate --start\n{title: Auth Flow}\n- body\n&&& addTemplate --end in_progress"
        }
        Some(TemplateStatus::Done) => {
            "&&& addTemplate --start\n{title: Auth Flow}\n- body\n&&& addTemplate --end done"
        }
        None => "plain body",
    };
    let page = PromptPage::saved_at(9, "Auth Flow", content, 100);
    let item = PromptItem {
        id: 7,
        folder_id: None,
        name: "Backend Prompt".to_string(),
        active_page: page.id,
        pages: vec![page],
    };
    let panel = &mut state.active_workspace_mut().prompt_panel;
    panel.tree = PromptTree::new();
    panel.tree.replace(Vec::new(), vec![item]);
    let kind = agent_items(&state)[0].kind;
    state
        .pane_mut_anywhere(pane)
        .expect("pane exists")
        .set_agent_title_override(
            kind,
            Some(AgentTitleOverride {
                text: "Auth Flow".to_string(),
                prompt: Some(AgentPromptRef {
                    item_id: 7,
                    page_id: PromptPageId::Saved(9),
                }),
            }),
        );
    state
}

/// 第一行 Agents 条目标题首字符的单元格样式。
fn agent_title_cell(state: &AppState) -> (Color, Modifier, Color) {
    use unicode_width::UnicodeWidthStr;

    let view = view_for(state);
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
    let buffer = render_buffer(state);
    let x = view
        .left_panel
        .x
        .saturating_add(1)
        .saturating_add(u16::try_from(text::agent::TREE_LEAF.width()).expect("prefix width"));
    let cell = &buffer[(x, sections.agents.y + 1)];
    (cell.fg, cell.modifier, cell.bg)
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
    let items = agent_items(&state);
    assert_eq!(items.len(), 1);
    let location = item_location(&items[0]);
    assert_eq!(
        location, "[T1 P1]",
        "第一个标签的第一个窗格必须从 T1 P1 开始"
    );
    let row = agent_item_line(&state, 0, 0);
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
    let items = agent_items(&state);
    assert_eq!(items.len(), 2);
    assert_eq!(item_location(&items[0]), "[T1 P1]");
    assert_eq!(item_location(&items[1]), "[T1 P2]");
    for container in [
        crate::app::state::FocusedContainer::Panes,
        crate::app::state::FocusedContainer::Prompt,
        crate::app::state::FocusedContainer::Todo,
        crate::app::state::FocusedContainer::Sidebar,
    ] {
        state.set_focus(container);
        let first_row = agent_item_line(&state, 0, 0);
        let second_row = agent_item_line(&state, 1, 0);
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
    let first_row = agent_item_line(&state, 0, 0);
    let second_row = agent_item_line(&state, 1, 0);
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
    let items = agent_items(&state);
    let location = item_location(&items[0]);
    let row = agent_item_line(&state, 0, 0);
    assert!(
        row.ends_with(&location),
        "location must survive a narrow sidebar: {row:?}"
    );
    assert!(
        !row.contains("opencode") && row.contains('…'),
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
fn agent_rows_end_with_location_matching_tab_and_pane_indices() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Claude, AgentState::Idle);
    update::create_tab(&mut state);
    let source = state.active_tab().layout.focus();
    let second_pane = state
        .active_tab_mut()
        .split_pane(source, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    update::update_pane_agent(
        &mut state,
        second_pane,
        Some(agent_snapshot(AgentKind::Codex, AgentState::Idle)),
    );

    let items = agent_items(&state);
    assert_eq!(items.len(), 2);
    assert_eq!((items[0].tab_index, items[0].pane_index), (0, 0));
    assert_eq!((items[1].tab_index, items[1].pane_index), (1, 1));
    assert_eq!(item_location(&items[0]), "[T1 P1]");
    assert_eq!(item_location(&items[1]), "[T2 P2]");
    for (index, item) in items.iter().enumerate() {
        let row = agent_item_line(&state, index, 0);
        assert!(
            row.ends_with(&item_location(item)),
            "条目 {index} 的归属标签必须与实际 T/P 一致: {row:?}"
        );
    }
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
fn agent_name_and_title_lines_hit_the_same_item() {
    let mut state = AppState::demo();
    let pane = add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
    for offset in [0u16, 1] {
        assert_eq!(
            agent_item_at(&view, &state, sections.agents.x, sections.agents.y + offset),
            Some(pane),
            "条目第 {offset} 行必须命中同一条目"
        );
    }
}

#[test]
fn odd_agents_height_rounds_down_items_and_ignores_blank_row() {
    let mut state = AppState::demo();
    add_agent_tabs(&mut state, 5);
    let view = layout::ViewLayout {
        left_panel: Rect::new(0, 0, 40, 21),
        ..view_for(&state)
    };
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
    assert_eq!(sections.agents.height, 9, "构造奇数高度的 Agents 区");
    assert_eq!(
        agent_list_rows(&view, &state),
        Some(4),
        "可见条目数按高度 / 2 向下取整"
    );
    assert_eq!(
        agent_item_index_at(
            &view,
            &state,
            sections.agents.x,
            sections.agents.y + sections.agents.height - 2
        ),
        Some(3),
        "最后一行完整条目的标题行仍可命中"
    );
    let blank_row = sections.agents.bottom() - 1;
    assert_eq!(
        agent_item_index_at(&view, &state, sections.agents.x, blank_row),
        None,
        "底部不足一个条目的留空行不参与命中"
    );
    assert!(
        agents_section_at(&view, &state, sections.agents.x, blank_row),
        "留空行仍属于 Agents 分区，滚轮可继续滚动"
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
    assert_eq!(
        agent_item_index_at(&view, &state, sections.agents.x, sections.agents.y + 1),
        Some(1),
        "条目第二行与首行映射到同一槽位"
    );
    assert_eq!(
        agent_item_index_at(&view, &state, sections.agents.x, sections.agents.y + 2),
        Some(2),
        "滚动偏移按条目而不是按行累加"
    );
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
    for offset in [0, 1] {
        assert_eq!(
            buffer[(sections.agents.x, sections.agents.y + offset)]
                .style()
                .bg,
            style::selection().bg,
            "条目的名称行与标题行共享悬停背景（offset {offset}）"
        );
    }
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
    let location = item_location(&items[0]);
    assert_eq!(location, "[T1 P1]");
    assert_eq!(
        item_location(&items[1]),
        location,
        "同一窗格的 lx 与终端条目必须共享同一个归属标签"
    );

    // 当前在 Terminal 视图：条目 0（lx）不带 ❯，条目 1（claude）带 ❯；
    // 每个条目在本行之后紧跟一行标题。
    let row0_terminal = agent_item_line(&state, 0, 0);
    let row1_terminal = agent_item_line(&state, 1, 0);
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
    assert!(row0_terminal.ends_with(&location), "{row0_terminal:?}");
    assert!(row1_terminal.ends_with(&location), "{row1_terminal:?}");

    // 切回 Lx 视图：条目 0 带 ❯，条目 1 不带 ❯。
    update::toggle_pane_view(&mut state, pane);
    let row0_lx = agent_item_line(&state, 0, 0);
    let row1_lx = agent_item_line(&state, 1, 0);
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
        Some((pane, PaneView::Lx)),
        "标题行与名称行命中同一条目"
    );
    assert_eq!(
        agent_target_at(&view, &state, sections.agents.x, sections.agents.y + 2),
        Some((pane, PaneView::Terminal))
    );
    assert_eq!(
        agent_target_at(&view, &state, sections.agents.x, sections.agents.y + 3),
        Some((pane, PaneView::Terminal)),
        "终端条目标题行同样命中该条目"
    );
}

#[test]
fn agent_title_line_starts_with_tree_leaf_and_shows_placeholder() {
    let mut state = AppState::demo();
    add_agent(&mut state, AgentKind::Claude, AgentState::Idle);
    let items = agent_items(&state);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, None, "无 OSC 标题的终端条目标题为 None");

    let title_line = agent_item_line(&state, 0, 1);
    let expected = format!("{}{}", text::agent::TREE_LEAF, text::AGENT_NO_TITLE);
    assert!(
        title_line.starts_with(&expected),
        "第二行必须以树连接符开头并显示无标题占位: {title_line:?}"
    );
    assert_eq!(
        title_line.trim_end(),
        expected,
        "占位符之后的列全部留空: {title_line:?}"
    );
}

#[test]
fn lx_agent_title_ignores_session_title_and_uses_custom_override() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = Some("ses-title".to_string());
        target.lx.chat.session_title = Some("Fix login bug".to_string());
    }

    let kind = agent_items(&state)[0].kind;
    assert_eq!(
        agent_items(&state)[0].title,
        None,
        "会话标题不再作为侧栏标题来源"
    );
    assert!(agent_item_line(&state, 0, 1).contains(text::AGENT_NO_TITLE));

    state
        .pane_mut_anywhere(pane)
        .expect("pane exists")
        .set_agent_title_override(kind, Some(AgentTitleOverride::custom("Sidebar label")));
    assert_eq!(agent_items(&state)[0].title, Some("Sidebar label"));
    let line = agent_item_line(&state, 0, 1);
    assert!(line.contains("Sidebar label"), "{line:?}");
    assert!(
        !line.contains("Fix login bug"),
        "侧栏标题不得回退会话标题: {line:?}"
    );
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|pane| pane.lx.chat.session_title.as_deref()),
        Some("Fix login bug"),
        "侧栏覆盖不得修改会话标题本身"
    );

    state
        .pane_mut_anywhere(pane)
        .expect("pane exists")
        .set_agent_title_override(kind, None);
    assert_eq!(
        agent_items(&state)[0].title,
        None,
        "清除覆盖后不回退会话标题"
    );
    let line = agent_item_line(&state, 0, 1);
    assert!(line.contains(text::AGENT_NO_TITLE), "{line:?}");
    assert!(!line.contains("Sidebar label"), "{line:?}");
}

#[test]
fn terminal_agent_title_ignores_osc_and_clears_override_on_kind_change() {
    let mut state = AppState::demo();
    let pane = add_agent(&mut state, AgentKind::Claude, AgentState::Working);
    update::update_pane_agent(
        &mut state,
        pane,
        Some(PaneAgentSnapshot {
            kind: AgentKind::Claude,
            state: AgentState::Working,
            title: Some("Claude Code".to_string()),
        }),
    );
    assert_eq!(
        agent_items(&state)[0].title,
        None,
        "终端 OSC 标题不再作为侧栏标题来源"
    );
    assert!(agent_item_line(&state, 0, 1).contains(text::AGENT_NO_TITLE));

    let kind = agent_items(&state)[0].kind;
    state
        .pane_mut_anywhere(pane)
        .expect("pane exists")
        .set_agent_title_override(kind, Some(AgentTitleOverride::custom("My claude")));
    assert_eq!(agent_items(&state)[0].title, Some("My claude"));
    let line = agent_item_line(&state, 0, 1);
    assert!(line.contains("My claude"), "{line:?}");
    assert!(
        !line.contains("Claude Code"),
        "侧栏标题不得回退 OSC 标题: {line:?}"
    );

    // 同种类仅状态/OSC 标题更新：覆盖保留。
    update::update_pane_agent(
        &mut state,
        pane,
        Some(PaneAgentSnapshot {
            kind: AgentKind::Claude,
            state: AgentState::Idle,
            title: Some("Claude Code".to_string()),
        }),
    );
    assert_eq!(agent_items(&state)[0].title, Some("My claude"));

    // AgentKind 变化：覆盖清空，不回退新种类的 OSC 标题。
    update::update_pane_agent(
        &mut state,
        pane,
        Some(PaneAgentSnapshot {
            kind: AgentKind::Codex,
            state: AgentState::Working,
            title: Some("Codex CLI".to_string()),
        }),
    );
    assert_eq!(agent_items(&state)[0].title, None);
    let line = agent_item_line(&state, 0, 1);
    assert!(line.contains(text::AGENT_NO_TITLE), "{line:?}");
    assert!(
        !line.contains("My claude") && !line.contains("Codex CLI"),
        "AgentKind 变化必须清空终端标题覆盖且不回退 OSC: {line:?}"
    );
}

#[test]
fn agent_title_uses_recent_template_status_colors() {
    let cases = [
        (Some(TemplateStatus::Todo), Color::Reset, false),
        (Some(TemplateStatus::InProgress), Color::Yellow, false),
        (Some(TemplateStatus::Done), Color::Green, false),
        (None, Color::Reset, true),
    ];
    for (status, fg, dim) in cases {
        let state = state_with_bound_title(status);
        let (actual_fg, modifier, _) = agent_title_cell(&state);
        assert_eq!(actual_fg, fg, "{status:?} 标题前景色");
        assert_eq!(modifier.contains(Modifier::DIM), dim, "{status:?} DIM");
    }
}

#[test]
fn sidebar_title_status_follows_live_edited_page() {
    let mut state = state_with_bound_title(Some(TemplateStatus::Todo));
    let panel = &mut state.active_workspace_mut().prompt_panel;
    panel.mode = crate::app::prompt::PromptMode::Editor;
    panel.editing = Some(crate::app::prompt::PromptEditing::Item(7));
    state.prompt.set_text(
        "&&& addTemplate --start\n{title: Auth Flow}\n- body\n&&& addTemplate --end in_progress",
    );

    let (fg, _, _) = agent_title_cell(&state);
    assert_eq!(fg, Color::Yellow, "侧栏应读取正在编辑页的实时状态");
}

#[test]
fn one_row_agents_area_has_no_hit_slot() {
    assert_eq!(agent_content_rect(Rect::new(0, 0, 20, 1)).height, 0);
    assert_eq!(agent_content_rect(Rect::new(0, 0, 20, 2)).height, 2);
    let state = AppState::demo();
    assert_eq!(
        agent_scrollbar_for(Rect::new(0, 0, 20, 1), &state, 3),
        None,
        "没有完整条目槽位时不画滚动条"
    );
}

#[test]
fn hovered_agent_title_keeps_status_color_over_selection() {
    let mut state = state_with_bound_title(Some(TemplateStatus::InProgress));
    state.agent_hover = Some(0);

    let (fg, _, bg) = agent_title_cell(&state);
    assert_eq!(fg, Color::Yellow, "悬停不得覆盖状态前景色");
    assert_eq!(
        bg,
        style::selection().bg.expect("selection background"),
        "悬停底色保留"
    );
}

#[test]
fn cjk_agent_title_is_ellipsized_within_content_width() {
    use unicode_width::UnicodeWidthStr;

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let title = "修复登录问题并审查权限路径中的潜在越权风险";
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = Some("ses-cjk".to_string());
    }
    let kind = agent_items(&state)[0].kind;
    state
        .pane_mut_anywhere(pane)
        .expect("pane exists")
        .set_agent_title_override(kind, Some(AgentTitleOverride::custom(title)));

    let view = view_for(&state);
    let sections = layout::sidebar_sections(view.left_panel, false).expect("sections are visible");
    assert_eq!(agent_items(&state).len(), 1, "单条目不出现滚动条");
    let width = usize::from(sections.agents.width);
    let budget = width - text::agent::TREE_LEAF.width();
    let expected = text::ellipsize_width(title, budget);
    assert!(expected.ends_with('…'), "超宽标题必须截断: {expected:?}");
    assert!(
        expected.width() <= budget,
        "截断后按显示列计不得超过预算: {} > {budget}",
        expected.width()
    );

    let line = agent_item_line(&state, 0, 1);
    assert!(
        line.starts_with(&format!("{}{expected}", text::agent::TREE_LEAF)),
        "标题行必须按显示宽度截断: {line:?}"
    );
    assert_eq!(line.width(), width, "标题行铺满内容列且不越界: {line:?}");
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

/// 非分组父项/子项的普通顶层项一律在分组箭头列显示 `•`，与 git 分支无关；名字列不位移。
#[test]
fn sidebar_marks_flat_items_at_group_arrow_column() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "plain".to_string();
    state.workspaces[0].git = None;
    push_git_workspace(
        &mut state,
        "repo",
        "/repo",
        git_info("/repo", "/repo", false, Some("main")),
    );
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    push_git_workspace(
        &mut state,
        "headless",
        "/repo/headless",
        git_info("/repo", "/repo/headless", false, None),
    );
    push_git_workspace(
        &mut state,
        "flat-repo",
        "/flat",
        git_info("/flat", "/flat", false, Some("main")),
    );
    state
        .workspaces
        .push(Workspace::single_terminal("scratch".to_string(), None));

    let lines = render_lines(&state);
    let row = |needle: &str| {
        lines
            .iter()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} row missing: {lines:?}"))
    };
    let icon = text::WORKSPACE_ITEM_ICON;
    let plain = row("plain");
    let parent = row("▾ repo");
    let child = row("feature/x");
    let headless = row("headless");
    let flat_repo = row("flat-repo");
    let scratch = row("scratch");

    for (line, label) in [
        (plain, "plain"),
        (headless, "headless"),
        (flat_repo, "flat-repo"),
        (scratch, "scratch"),
    ] {
        assert!(
            line.contains(&format!("{icon} {label}")),
            "普通顶层项必须在名字左侧显示标记: {line:?}"
        );
    }
    assert!(
        !parent.contains(icon),
        "分组父项不得显示普通顶层标记: {parent:?}"
    );
    assert!(
        child.contains(&format!("└─ {icon} feature/x")),
        "分组子项在树连接符后显示标记: {child:?}"
    );

    let char_col =
        |line: &str, needle: &str| line.find(needle).map(|byte| line[..byte].chars().count());
    let name_col = char_col(plain, "plain");
    assert_eq!(
        char_col(parent, "repo"),
        name_col,
        "父项名字仍与普通顶层项同列"
    );
    assert_eq!(char_col(headless, "headless"), name_col);
    assert_eq!(char_col(flat_repo, "flat-repo"), name_col);
    assert_eq!(char_col(scratch, "scratch"), name_col);
    assert_eq!(
        char_col(plain, icon),
        char_col(parent, "▾"),
        "普通顶层标记必须与分组箭头同列"
    );
}

/// 父项折叠箭头不叠加 `•`；子项保留树连接符并在其后显示 `•`。
#[test]
fn sidebar_parent_never_shows_item_icon() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "main".to_string();
    state.workspaces[0].git = Some(git_info("/repo", "/repo", false, None));
    push_git_workspace(
        &mut state,
        "feat",
        "/repo/.worktrees/feat",
        git_info("/repo", "/repo/.worktrees/feat", true, Some("feature/x")),
    );
    push_git_workspace(
        &mut state,
        "detached",
        "/repo/.worktrees/detached",
        git_info("/repo", "/repo/.worktrees/detached", true, None),
    );

    let lines = render_lines(&state);
    let row = |needle: &str| {
        lines
            .iter()
            .find(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} row missing: {lines:?}"))
    };
    let parent = row("▾ main");
    let child = row("detached");
    assert!(
        child.contains(&format!("└─ {} detached", text::WORKSPACE_ITEM_ICON)),
        "子项应保留树连接符并在其后显示标记: {child:?}"
    );
    assert!(
        !parent.contains(text::WORKSPACE_ITEM_ICON),
        "父项不得显示条目标记: {parent:?}"
    );
}

/// 普通顶层标记只在激活工作区用正文色，其余压暗；hover 只叠背景、不覆盖前景。
#[test]
fn flat_item_icon_is_muted_until_workspace_is_active() {
    let mut state = AppState::demo();
    state.workspaces[0].name = "alpha-ws".to_string();
    state.workspaces[0].git = None;
    state.active_workspace = 0;
    state
        .workspaces
        .push(Workspace::single_terminal("beta-ws".to_string(), None));

    let lines = render_lines(&state);
    let buffer = render_buffer(&state);
    let view = view_for(&state);
    let icon_position = |needle: &str| {
        let y = lines
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("{needle} row missing: {lines:?}"));
        let sidebar: String = lines[y]
            .chars()
            .take(usize::from(view.left_panel.right()))
            .collect();
        let byte = sidebar
            .find(text::WORKSPACE_ITEM_ICON)
            .expect("flat item icon");
        (sidebar[..byte].chars().count() as u16, y as u16)
    };

    let (active_x, active_y) = icon_position("alpha-ws");
    let (inactive_x, inactive_y) = icon_position("beta-ws");
    let active = &buffer[(active_x, active_y)];
    assert_eq!(active.fg, ratatui::style::Color::Reset);
    assert!(!active.modifier.contains(ratatui::style::Modifier::DIM));
    assert!(!active.modifier.contains(ratatui::style::Modifier::BOLD));
    let inactive = &buffer[(inactive_x, inactive_y)];
    assert_eq!(inactive.fg, ratatui::style::Color::Reset);
    assert!(inactive.modifier.contains(ratatui::style::Modifier::DIM));
    assert!(!inactive.modifier.contains(ratatui::style::Modifier::BOLD));

    // hover 只叠背景：selection 的白色前景不得覆盖显式 Reset + dim。
    state.workspace_hover = Some(1);
    let hovered = render_buffer(&state);
    let cell = &hovered[(inactive_x, inactive_y)];
    assert_eq!(cell.fg, ratatui::style::Color::Reset);
    assert!(cell.modifier.contains(ratatui::style::Modifier::DIM));
    assert_eq!(cell.bg, ratatui::style::Color::Indexed(240));
}

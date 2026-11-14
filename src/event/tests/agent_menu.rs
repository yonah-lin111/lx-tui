use super::test_support::*;
use super::*;

use crate::app::overlay::{MenuCommand, OverlayTarget};
use crate::app::state::{SidebarAgentEntry, SidebarAgentKind};
use crate::detect::{AgentKind, AgentState};
use crate::event::session::Geometry;
use crate::tiling::PaneId;

/// 同一窗格同时有 lx 会话与终端 Agent 的场景；条目顺序固定为 `[lx, terminal]`。
fn dual_agent_state() -> (AppState, Config, Geometry, PaneId) {
    let mut state = demo_terminal();
    let pane = mount_agent(&mut state, AgentKind::Claude, AgentState::Working);
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.lx.chat.session_id = Some("ses-agent-menu".to_string());
    }
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 100, 24));
    (state, config, geo, pane)
}

/// 侧栏 Agents 分区矩形。
fn agents_section(geo: &Geometry, state: &AppState) -> Rect {
    ui::layout::sidebar_sections(geo.view.left_panel, state.agents_collapsed)
        .expect("sections are visible")
        .agents
}

/// 投递一次鼠标事件并返回是否置脏。
fn send_mouse(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    kind: MouseEventKind,
    column: u16,
    row: u16,
) -> bool {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(kind, column, row),
        state,
        &mut sessions,
        geo,
        config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    dirty
}

/// 断言当前菜单精确指向给定 Agent 条目，命令与锚点符合预期。
fn assert_agent_menu(
    state: &AppState,
    entry: SidebarAgentEntry,
    anchor: (u16, u16),
    commands: &[MenuCommand],
) {
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("agent menu overlay expected");
    };
    assert_eq!(
        menu.target,
        OverlayTarget::Agent {
            workspace: 0,
            tab: entry.tab_index,
            pane: entry.pane_id,
            kind: entry.kind,
        },
        "右键 Agents 条目必须命中 Agent 目标，不得误开工作区/Tab/Pane 菜单"
    );
    assert_eq!(menu.anchor, anchor);
    assert_eq!(menu.commands.as_slice(), commands);
    assert!(!menu.confirming, "菜单打开时不处于确认态");
}

#[test]
fn right_click_agent_first_row_opens_agent_menu() {
    let (mut state, config, geo, pane) = dual_agent_state();
    let agents = agents_section(&geo, &state);
    let expected = state.agent_entries()[0];
    assert_eq!(expected.pane_id, pane);
    assert!(matches!(expected.kind, SidebarAgentKind::Lx { .. }));
    let anchor = (agents.x + 1, agents.y);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );

    assert!(dirty);
    assert_agent_menu(
        &state,
        expected,
        anchor,
        &[MenuCommand::EditAgentTitle, MenuCommand::CloseAgent],
    );
}

#[test]
fn right_click_agent_second_row_opens_same_agent_menu() {
    let (mut state, config, geo, _pane) = dual_agent_state();
    let agents = agents_section(&geo, &state);
    let expected = state.agent_entries()[0];
    let anchor = (agents.x + 1, agents.y + 1);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );

    assert!(dirty);
    assert_agent_menu(
        &state,
        expected,
        anchor,
        &[MenuCommand::EditAgentTitle, MenuCommand::CloseAgent],
    );
}

#[test]
fn right_click_terminal_agent_opens_agent_menu_not_pane_menu() {
    let (mut state, config, _geo, pane) = dual_agent_state();
    // 拆出第二个窗格，使终端 Agent 条目可以关闭，同时验证目标仍精确落在 Agent 上。
    state
        .active_tab_mut()
        .split_pane(pane, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    let geo = geometry(&state, &config, Rect::new(0, 0, 100, 24));
    let agents = agents_section(&geo, &state);
    let expected = state.agent_entries()[1];
    assert_eq!(expected.pane_id, pane);
    assert!(matches!(
        expected.kind,
        SidebarAgentKind::Terminal(AgentKind::Claude)
    ));
    let anchor = (agents.x + 1, agents.y + 2);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );

    assert!(dirty);
    assert_agent_menu(
        &state,
        expected,
        anchor,
        &[MenuCommand::EditAgentTitle, MenuCommand::CloseAgent],
    );
    assert_eq!(state.active_workspace, 0);
    assert_eq!(state.active_workspace().active_tab, 0);
}

#[test]
fn terminal_agent_in_sole_pane_offers_only_edit_title() {
    let mut state = demo_terminal();
    let pane = mount_agent(&mut state, AgentKind::Codex, AgentState::Idle);
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 100, 24));
    let agents = agents_section(&geo, &state);
    let expected = state.agent_entries()[0];
    assert_eq!(state.active_tab().layout.pane_ids(), vec![pane]);
    let anchor = (agents.x + 1, agents.y);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );

    assert!(dirty);
    assert_agent_menu(&state, expected, anchor, &[MenuCommand::EditAgentTitle]);
}

#[test]
fn clicking_each_agent_row_focuses_pane_and_switches_view() {
    let (mut state, config, geo, pane) = dual_agent_state();
    let agents = agents_section(&geo, &state);
    let (column, top) = (agents.x + 1, agents.y);

    fn set_view(state: &mut AppState, pane: PaneId, view: PaneView) {
        if let Some(target) = state.active_tab_mut().pane_mut(pane) {
            target.view = view;
        }
    }

    // 条目 0 第一行：切到 lx 视图。
    assert_eq!(view_of(&state, pane), PaneView::Terminal);
    send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Left),
        column,
        top,
    );
    assert_eq!(view_of(&state, pane), PaneView::Lx);
    assert_eq!(state.active_tab().layout.focus(), pane);

    // 条目 0 第二行（标题行）：同一条目同样切到 lx 视图。
    set_view(&mut state, pane, PaneView::Terminal);
    send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Left),
        column,
        top + 1,
    );
    assert_eq!(view_of(&state, pane), PaneView::Lx);

    // 条目 1 第一行与第二行：都切到终端视图。
    for offset in [2, 3] {
        set_view(&mut state, pane, PaneView::Lx);
        send_mouse(
            &mut state,
            &geo,
            &config,
            MouseEventKind::Down(MouseButton::Left),
            column,
            top + offset,
        );
        assert_eq!(view_of(&state, pane), PaneView::Terminal);
        assert_eq!(state.active_tab().layout.focus(), pane);
    }
    assert!(!state.prompt_focused());
}

#[test]
fn wheel_scroll_then_hover_and_right_click_use_new_row_offset() {
    let mut state = demo_terminal();
    create_agent_tabs(&mut state, 20);
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 100, 24));
    let agents = agents_section(&geo, &state);

    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::ScrollDown,
        agents.x,
        agents.y,
    );
    assert!(dirty);
    assert_eq!(state.agent_scroll, 3);
    assert_eq!(state.agent_hover, Some(3), "滚动后按新偏移重算悬停");

    // 第二可见条目的第二行：新偏移下应命中索引 4。
    let row = agents.y + 3;
    send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Moved,
        agents.x + 1,
        row,
    );
    assert_eq!(state.agent_hover, Some(4));

    // 同一坐标右键也应命中索引 4 的条目。
    let expected = state.agent_entries()[4];
    let anchor = (agents.x + 1, row);
    let dirty = send_mouse(
        &mut state,
        &geo,
        &config,
        MouseEventKind::Down(MouseButton::Right),
        anchor.0,
        anchor.1,
    );
    assert!(dirty);
    assert_agent_menu(&state, expected, anchor, &[MenuCommand::EditAgentTitle]);
}

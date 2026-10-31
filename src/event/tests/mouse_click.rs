use super::test_support::*;
use super::*;

#[test]
fn mouse_down_on_swap_button_swaps_sides_without_resize() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let button = ui::swap_button(&geo.view).expect("swap button visible");
    // 屏幕 120、侧栏 24：可用 96，右列默认 38。
    let available = geo.view.panes.width + geo.view.prompt.width;
    let side = ui::layout::side_width(available, config.min_pane_width);

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_left);
    assert!(!state.resizing_prompt, "swap button must not start resize");
    assert_eq!(
        state.prompt_width,
        available - side,
        "right column resets to side width"
    );
    assert!(dirty);
}

#[test]
fn mouse_down_on_panes_collapse_button_collapses_panes() {
    let config = Config::default();
    let mut state = demo_terminal();
    update::swap_prompt_side(&mut state, 96, 38);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let button = ui::collapse_buttons(&geo.view, false)
        .into_iter()
        .find(|button| button.target == ui::CollapseTarget::Panes)
        .expect("panes collapse button visible");
    assert!(!button.collapsed);

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            button.area.x,
            button.area.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.panes_collapsed);

    // 折叠后窗格不参与几何：PTY 不重排，命中集只剩 prompt。
    let collapsed = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    assert!(
        collapsed
            .rects
            .iter()
            .all(|(id, _)| *id == state.prompt.id()),
        "collapsed panes leave the geometry"
    );
    assert!(
        ui::swap_button(&collapsed.view).is_some(),
        "swap button stays clickable"
    );
}

#[test]
fn mouse_click_switches_tab_and_add_creates_one() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::create_tab(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let (_, first) = bar.tabs[0];
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), first.x, first.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.active_workspace().active_tab, 0);
    assert!(dirty);

    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let add = bar.add.expect("add button is visible");
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), add.x, add.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.active_workspace().tabs.len(), 3);
    assert_eq!(state.active_workspace().active_tab, 2);
}

#[test]
fn mouse_click_scroll_buttons_scrolls_tab_bar() {
    let config = Config::default();
    let mut state = demo_terminal();
    for _ in 1..30 {
        crate::app::update::create_tab(&mut state);
    }
    crate::app::update::set_tab_scroll(&mut state, 0, usize::MAX);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    assert!(bar.max_scroll > 0);
    let right = bar.scroll_right.expect("right button is visible");
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), right.x, right.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.tab_scroll > 0);

    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let left = bar.scroll_left.expect("left button is visible");
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), left.x, left.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.tab_scroll, 0);
}

#[test]
fn toggle_button_click_flips_view_without_stealing_focus() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let button = toggle_button_of(&geo, pane);
    crate::app::update::focus_prompt(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty);
    assert!(state.prompt_focused(), "按钮是控件，不抢焦点");
    assert_eq!(view_of(&state, pane), PaneView::Terminal);

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(view_of(&state, pane), PaneView::Lx);
}

#[test]
fn mouse_motion_tracks_workspace_and_tab_hover() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::create_workspace(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        crate::ui::layout::sidebar_sections(geo.view.sidebar, false).expect("sections visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            sections.workspaces.x + 1,
            sections.workspaces.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_hover, Some(0));
    assert!(dirty, "悬停变化置脏重绘");

    dirty = false;
    let bar = crate::ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let (index, rect) = bar.tabs[0];
    handle_terminal_event(
        mouse(MouseEventKind::Moved, rect.x + 1, rect.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.tab_hover, Some(index));
    assert!(dirty);

    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.view.panes.x + 2,
            geo.view.panes.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_hover, None);
    assert_eq!(state.tab_hover, None);
    assert!(dirty, "离开条目区同样置脏");

    // 拖动排序期间不产生悬停高亮。
    crate::app::update::begin_workspace_drag(&mut state, 0);
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            sections.workspaces.x + 1,
            sections.workspaces.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_hover, None, "拖动反显优先于 hover");
}

#[test]
fn wheel_over_workspace_list_rescrolls_hover() {
    let config = Config::default();
    let mut state = demo_terminal();
    for _ in 1..30 {
        crate::app::update::create_workspace(&mut state);
    }
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        crate::ui::layout::sidebar_sections(geo.view.sidebar, false).expect("sections visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;
    state.workspace_hover = Some(0);

    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            sections.workspaces.x + 1,
            sections.workspaces.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_scroll, 3);
    assert_eq!(state.workspace_hover, Some(3), "滚动后按新偏移重算悬停");
}

#[test]
fn mouse_click_agent_item_switches_tab_and_terminal_view() {
    use crate::detect::{AgentKind, AgentState};

    let mut state = demo_terminal();
    let mut sessions = HashMap::new();
    let config = Config::default();
    let screen = Rect::new(0, 0, 100, 24);
    let first = mount_agent(&mut state, AgentKind::Claude, AgentState::Working);
    update::create_tab(&mut state);
    mount_agent(&mut state, AgentKind::Codex, AgentState::Idle);
    update::focus_prompt(&mut state);
    if let Some(pane) = state.active_tab_mut().pane_mut(first) {
        pane.view = PaneView::Lx;
    }
    let geo = geometry(&state, &config, screen);
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, false).expect("sections are visible");
    assert_eq!(state.active_workspace().active_tab, 1);

    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            sections.agents.x,
            sections.agents.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(dirty);
    assert_eq!(state.active_workspace().active_tab, 0);
    assert_eq!(state.active_tab().layout.focus(), first);
    assert_eq!(
        state.active_tab().pane(first).map(|pane| pane.view),
        Some(PaneView::Terminal)
    );
    assert!(!state.prompt_focused());
}

#[test]
fn mouse_hover_agent_item_tracks_hover() {
    use crate::detect::{AgentKind, AgentState};

    let mut state = demo_terminal();
    let mut sessions = HashMap::new();
    let config = Config::default();
    let screen = Rect::new(0, 0, 100, 24);
    mount_agent(&mut state, AgentKind::Gemini, AgentState::Blocked);
    let geo = geometry(&state, &config, screen);
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, false).expect("sections are visible");

    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, sections.agents.x, sections.agents.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.agent_hover, Some(0));

    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.view.panes.x + 1,
            geo.view.panes.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.agent_hover, None);
}

#[test]
fn wheel_over_agents_section_scrolls_list() {
    let mut state = demo_terminal();
    let mut sessions = HashMap::new();
    let config = Config::default();
    let screen = Rect::new(0, 0, 100, 24);
    create_agent_tabs(&mut state, 20);
    let geo = geometry(&state, &config, screen);
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, false).expect("sections are visible");

    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            sections.agents.x,
            sections.agents.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.agent_scroll, 3);
    assert_eq!(state.agent_hover, Some(3), "hover follows the scrolled row");
}

#[test]
fn mouse_drag_agent_scrollbar_thumb_scrolls_list() {
    let mut state = demo_terminal();
    let mut sessions = HashMap::new();
    let config = Config::default();
    let screen = Rect::new(0, 0, 100, 24);
    create_agent_tabs(&mut state, 20);
    let geo = geometry(&state, &config, screen);
    let bar = ui::agent_scrollbar(&geo.view, &state).expect("scrollbar is visible");

    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.thumb.x,
            bar.thumb.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.agent_scroll_drag, Some(0));

    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            bar.track.x,
            bar.track.bottom().saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.agent_scroll > 0, "dragging thumb scrolls the list");

    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            bar.track.x,
            bar.track.bottom().saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.agent_scroll_drag, None);
}

#[test]
fn poll_active_agents_ignores_plain_shell() {
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let session = PtySession::spawn(pane, 80, 24, None, |_| {}).expect("shell spawns");
    sessions.insert(pane, session);

    assert!(
        !poll_active_agents(&mut state, &sessions),
        "plain shell must not register an agent"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .and_then(|target| target.agent.as_ref())
            .is_none()
    );
}

#[test]
fn mouse_motion_over_button_arms_tooltip_and_leaving_clears() {
    use crate::app::tooltip::TooltipTarget;

    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let button = collapse_sidebar_area(&geo);

    handle_terminal_event(
        mouse(MouseEventKind::Moved, button.x + 1, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let tooltip = state.tooltip.as_ref().expect("tooltip armed");
    assert_eq!(tooltip.target, TooltipTarget::CollapseSidebar);
    assert!(!tooltip.is_visible(), "停留前只计时不显示");
    assert!(dirty, "悬停变化置脏重绘");

    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.view.panes.x + 2,
            geo.view.panes.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.tooltip.is_none(), "离开按钮立即清除");
    assert!(dirty);
}

#[test]
fn mouse_down_clears_tooltip() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let button = collapse_sidebar_area(&geo);

    handle_terminal_event(
        mouse(MouseEventKind::Moved, button.x + 1, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.tooltip.is_some());

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            geo.view.panes.x + 2,
            geo.view.panes.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.tooltip.is_none(), "左键按下清空提示");
}

#[test]
fn mouse_motion_during_workspace_drag_does_not_arm_tooltip() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::create_workspace(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    crate::app::update::begin_workspace_drag(&mut state, 0);
    let button = collapse_sidebar_area(&geo);

    handle_terminal_event(
        mouse(MouseEventKind::Moved, button.x + 1, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.tooltip.is_none(), "拖动排序期间不弹提示");
}

#[test]
fn mouse_motion_with_overlay_open_keeps_tooltip_clear() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let button = collapse_sidebar_area(&geo);

    handle_terminal_event(
        mouse(MouseEventKind::Moved, button.x + 1, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.tooltip.is_some());

    crate::app::update::open_tab_menu(&mut state, 0, (2, 2));
    handle_terminal_event(
        mouse(MouseEventKind::Moved, button.x + 1, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.tooltip.is_none(), "浮层打开时清除且不重新触发");
}

#[test]
fn mouse_motion_over_template_button_arms_tooltip() {
    use crate::app::tooltip::{TemplateButtonKind, TooltipTarget};

    let (mut state, config, geo, area) = template_button_state();
    let copy = crate::ui::prompt::template_buttons(&state.prompt, area)
        .into_iter()
        .find(|hit| hit.button == crate::ui::prompt::TemplateBlockButton::Copy)
        .expect("copy button visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Moved, copy.rect.x, copy.rect.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let tooltip = state.tooltip.as_ref().expect("template tooltip armed");
    assert_eq!(
        tooltip.target,
        TooltipTarget::TemplateButton {
            line: copy.line,
            button: TemplateButtonKind::Copy,
        }
    );
    assert!(!tooltip.is_visible(), "停留前只计时不显示");
    assert!(dirty);
}

#[test]
fn mouse_click_right_tab_switches_panel() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    state.right_tab_hover = Some(RightTab::Prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let todo = right_tab_rect(&geo, RightTab::Todo);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), todo.x, todo.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.right_tab, RightTab::Todo);
    assert!(!state.prompt_focused(), "切到 Todo 时键盘退回聚焦窗格");
    assert_eq!(state.right_tab_hover, None, "切换清空 tab 悬停");
    assert!(dirty);
}

#[test]
fn mouse_motion_tracks_right_tab_hover() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let todo = right_tab_rect(&geo, RightTab::Todo);
    handle_terminal_event(
        mouse(MouseEventKind::Moved, todo.x + 1, todo.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.right_tab_hover, Some(RightTab::Todo));
    assert!(dirty, "悬停变化置脏重绘");

    dirty = false;
    let text = layout::prompt_text_rect(geo.view.prompt);
    handle_terminal_event(
        mouse(MouseEventKind::Moved, text.x + 1, text.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.right_tab_hover, None, "离开 tab 清空悬停");
    assert!(dirty);

    // 折叠态：顶边框行不再是 tab 命中区，已有的悬停被清空。
    state.prompt_collapsed = true;
    state.right_tab_hover = Some(RightTab::Todo);
    let collapsed = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            collapsed.view.prompt.x,
            collapsed.view.prompt.y,
        ),
        &mut state,
        &mut sessions,
        &collapsed,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.right_tab_hover, None, "折叠态不产生 tab 悬停");
}

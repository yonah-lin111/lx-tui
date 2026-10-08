//! 单元测试；仅测试构建编译。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::app::overlay::ConfirmSwitchCwd;

use super::*;

fn geometry(state: &AppState, config: &Config, screen: Rect) -> Geometry {
    let view = ui::layout::compute(
        screen,
        config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let mut rects = layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );
    if !state.prompt_collapsed {
        rects.push((state.prompt.id(), view.prompt));
    }
    Geometry {
        screen,
        rects,
        view,
    }
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> TerminalEvent {
    TerminalEvent::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// demo 状态并把全部窗格置为终端视图：事件交互测试的默认前置。
fn demo_terminal() -> AppState {
    let mut state = AppState::demo();
    for workspace in &mut state.workspaces {
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    pane.view = PaneView::Terminal;
                }
            }
        }
    }
    state
}

#[test]
fn mouse_drag_resizes_sidebar() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let boundary = geo.view.sidebar.right() - 1;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.resizing_sidebar, "press on boundary starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary + 8, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.sidebar_width, boundary + 9);

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), boundary + 8, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!state.resizing_sidebar);
}

#[test]
fn mouse_drag_resizes_prompt() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let boundary = geo.view.prompt.x;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.resizing_prompt, "press on boundary starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary - 5, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt_width, geo.view.prompt.width + 5);
}

#[test]
fn mouse_drag_reorders_workspaces() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::create_workspace(&mut state);
    crate::app::update::create_workspace(&mut state);
    let names: Vec<String> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.clone())
        .collect();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections = crate::ui::layout::sidebar_sections(geo.view.sidebar, false)
        .expect("sidebar sections are visible");
    let list_y = sections.workspaces.y;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 2, list_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.workspace_drag, Some(0));

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), 2, list_y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.workspace_drag, Some(2));

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), 2, list_y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.workspace_drag.is_none());
    let order: Vec<&str> = state
        .workspaces
        .iter()
        .map(|workspace| workspace.name.as_str())
        .collect();
    assert_eq!(
        order,
        vec![names[1].as_str(), names[2].as_str(), names[0].as_str()]
    );
    assert_eq!(state.active_workspace, 2);
}

/// 让 prompt 文本溢出视口：按视口尺寸 resize 后写入两倍视口高度的行。
fn overflow_prompt(state: &mut AppState, view: &ui::layout::ViewLayout) {
    let text = crate::layout::prompt_text_rect(view.prompt);
    state.prompt.resize(text.width, text.height);
    for _ in 0..=usize::from(text.height) {
        state.prompt.insert_str("line\n");
    }
    crate::app::update::set_prompt_scroll(state, 0);
}

#[test]
fn mouse_track_click_prompt_scrollbar_scrolls_without_focus() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    overflow_prompt(&mut state, &geo.view);
    let bar = ui::prompt_scrollbar(&geo.view, &state).expect("scrollbar is visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt.scroll() > 0, "track click jumps down");
    assert!(
        !state.prompt_focused,
        "scrollbar press must not focus prompt"
    );
    assert!(state.prompt_scroll_drag.is_none());
    assert!(dirty);
}

#[test]
fn mouse_drag_prompt_scrollbar_thumb() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    overflow_prompt(&mut state, &geo.view);
    let bar = ui::prompt_scrollbar(&geo.view, &state).expect("scrollbar is visible");
    let max = ui::widgets::scrollbar::offset_from_drag_row(&bar, bar.track.bottom() - 1, 0);
    assert!(max > 0);
    let mut sessions = HashMap::new();
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
        &mut dirty,
    );
    assert_eq!(state.prompt_scroll_drag, Some(0));

    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.scroll(), max);

    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            bar.track.x,
            bar.track.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt_scroll_drag.is_none());
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
        &mut dirty,
    );
    assert_eq!(state.active_workspace().tabs.len(), 3);
    assert_eq!(state.active_workspace().active_tab, 2);
}

#[test]
fn mouse_right_click_tab_opens_tab_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::create_tab(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let (_, first) = bar.tabs[0];
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), first.x, first.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("tab menu overlay expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::Tab {
            workspace: 0,
            tab: 0
        }
    );
    assert!(dirty);
}

#[test]
fn mouse_right_click_pane_opens_pane_menu_at_pointer() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let (column, row) = (inner.x + 1, inner.y + 1);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("pane menu overlay expected");
    };
    assert_eq!(menu.anchor, (column, row));
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::Pane {
            workspace: 0,
            tab: 0,
            pane
        }
    );
    assert!(dirty);
}

#[test]
fn mouse_right_click_prompt_does_not_open_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let inner = layout::pane_inner_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    assert!(state.overlay.is_none());
    assert!(!dirty);
}

#[test]
fn mouse_right_click_other_pane_retargets_open_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let first = state.active_tab().layout.focus();
    let second = state
        .active_tab_mut()
        .split_pane(first, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for pane in [first, second] {
        let inner = pane_inner(&geo, pane);
        handle_terminal_event(
            mouse(
                MouseEventKind::Down(MouseButton::Right),
                inner.x + 1,
                inner.y + 1,
            ),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut dirty,
        );
        let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
            panic!("pane menu overlay expected");
        };
        assert_eq!(
            menu.target,
            crate::app::overlay::OverlayTarget::Pane {
                workspace: 0,
                tab: 0,
                pane
            }
        );
    }
}

#[test]
fn mouse_right_click_prompt_closes_open_pane_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.overlay.is_some());

    let prompt_inner = layout::pane_inner_rect(geo.view.prompt);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            prompt_inner.x + 1,
            prompt_inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    assert!(state.overlay.is_none());
}

#[test]
fn mouse_click_pane_menu_split_item_creates_pane() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("pane menu overlay expected");
    };
    let menu_layout = ui::overlay::menu_layout(&state, geo.screen, menu);
    let item = menu_layout.item_rects[0];
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), item.x + 1, item.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    assert!(state.overlay.is_none());
    assert_eq!(state.active_tab().layout.pane_ids().len(), 2);
}

/// 切换路径确认浮层的确认按钮矩形。
fn confirm_switch_button(state: &mut AppState, path: &str, confirm: bool) -> (Rect, Geometry) {
    let config = Config::default();
    let geo = geometry(state, &config, Rect::new(0, 0, 120, 30));
    let shell = ui::overlay::confirm_switch_cwd_shell(geo.screen, std::path::Path::new(path))
        .expect("dialog fits");
    let buttons = ui::widgets::modal::button_row(
        shell.inner,
        &[ui::text::BUTTON_CONFIRM, ui::text::BUTTON_CANCEL],
        2,
        1,
    );
    let index = usize::from(!confirm);
    (buttons[index], geo)
}

#[test]
fn confirm_switch_cwd_click_confirm_returns_cd_command() {
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: PathBuf::from("/tmp/target"),
    }));
    let (button, geo) = confirm_switch_button(&mut state, "/tmp/target", true);

    let action = handle_overlay_click(&mut state, geo.screen, button.x, button.y);

    assert_eq!(action, Some((pane, b"cd \"/tmp/target\"\n".to_vec())));
    assert!(state.overlay.is_none());
}

#[test]
fn confirm_switch_cwd_click_cancel_closes_without_command() {
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: PathBuf::from("/tmp/target"),
    }));
    let (button, geo) = confirm_switch_button(&mut state, "/tmp/target", false);

    let action = handle_overlay_click(&mut state, geo.screen, button.x, button.y);

    assert_eq!(action, None);
    assert!(state.overlay.is_none());
}

#[test]
fn mouse_click_confirm_switch_cwd_closes_overlay() {
    let config = Config::default();
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    state.overlay = Some(Overlay::ConfirmSwitchCwd(ConfirmSwitchCwd {
        workspace: 0,
        tab: 0,
        pane,
        path: PathBuf::from("/tmp/target"),
    }));
    let (button, geo) = confirm_switch_button(&mut state, "/tmp/target", true);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    assert!(state.overlay.is_none());
    assert!(dirty);
}

/// 目标窗格内容区矩形。
fn pane_inner(geo: &Geometry, pane: PaneId) -> Rect {
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .expect("pane rect");
    layout::pane_inner_rect(*rect)
}

/// 给窗格塞满回滚历史，并滚回顶部 5 行。
fn scrolled_pane(state: &mut AppState, pane: PaneId) {
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    for i in 0..40 {
        target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
    }
    assert!(target.terminal.scroll_display(5));
    assert!(target.terminal.display_offset() > 0);
}

#[test]
fn mouse_wheel_over_terminal_pane_scrolls_backlog() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    scrolled_pane(&mut state, pane);
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    let offset = target.terminal.display_offset();
    assert!(offset > 0, "wheel up scrolls into backlog");
    assert!(dirty);

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert!(target.terminal.display_offset() < offset);
}

#[test]
fn mouse_wheel_extends_in_progress_selection() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    for i in 0..40 {
        target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
    }
    // 本地选区只存在于非鼠标上报窗格；选中滚动优先于本地回滚。
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 2,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.terminal_selection, Some(pane));

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x + 2, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 3);
    let content = target.terminal.renderable_content();
    let range = content.selection.expect("selection range");
    // 锚点钉在网格行 0（鼠标按下的内容），终点随视口滚到网格行 -3。
    assert_eq!(range.start.line.0, -3);
    assert_eq!(range.end.line.0, 0);
    assert_eq!(range.start.column.0, 2);
    assert_eq!(range.end.column.0, 2);
    assert!(dirty);
}

#[test]
fn mouse_drag_to_pane_edge_arms_autoscroll() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        for i in 0..80 {
            target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
        }
    }
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 2,
            inner.y + 5,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            inner.x + 2,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        state.selection_autoscroll.is_some(),
        "edge drag arms autoscroll"
    );

    let before = state
        .pane_anywhere(pane)
        .expect("terminal pane")
        .terminal
        .display_offset();
    assert!(crate::app::update::tick(
        &mut state,
        Instant::now() + Duration::from_millis(31)
    ));
    let after = state
        .pane_anywhere(pane)
        .expect("terminal pane")
        .terminal
        .display_offset();
    assert!(after > before, "autoscroll tick scrolls the viewport");
}

#[test]
fn mouse_wheel_during_prompt_selection_scrolls_viewport() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    overflow_prompt(&mut state, &geo.view);
    let inner = crate::layout::prompt_text_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 1,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        state
            .selection
            .is_some_and(|selection| selection.is_dragging())
    );

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, inner.x + 1, inner.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt.scroll() > 0, "wheel scrolls prompt viewport");
    let range = state
        .selection
        .and_then(|selection| selection.range())
        .expect("selection range");
    assert_eq!(range.0.0, 0, "anchor stays pinned to its text line");
    assert!(
        range.1.0 > 0,
        "cursor follows the mouse cell in content rows"
    );
    assert!(dirty);
}

#[test]
fn mouse_wheel_in_mouse_report_mode_leaves_local_view_at_bottom() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    scrolled_pane(&mut state, pane);
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    target.terminal.feed(b"\x1b[?1000h\x1b[?1006h");
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 0);
    assert!(dirty);
}

#[test]
fn terminal_scrollbar_thumb_drag_moves_viewport() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        for i in 0..80 {
            target.terminal.feed(format!("line {i:02}\r\n").as_bytes());
        }
    }
    let target = state.pane_anywhere(pane).expect("terminal pane");
    let history = target.terminal.history_size();
    assert!(history > 0);
    let inner = pane_inner(&geo, pane);
    let bar = ui::main_content::scrollbar(inner, target).expect("scrollbar");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.track.x,
            bar.thumb.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.terminal_scroll_drag.is_some());
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 0);

    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            bar.track.x,
            bar.track.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), history);

    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            bar.track.x,
            bar.track.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.terminal_scroll_drag.is_none());
}

#[test]
fn mouse_drag_on_mouse_report_pane_skips_local_selection() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        target
            .terminal
            .feed(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
    }
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_terminal_event(
            mouse(kind, inner.x + 1, inner.y + 1),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut dirty,
        );
    }

    assert!(
        state.terminal_selection.is_none(),
        "mouse-report app owns selection"
    );
    assert!(state.selection.is_none());
}

#[test]
fn mouse_down_on_plain_pane_starts_local_selection() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            inner.x + 1,
            inner.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    assert_eq!(state.terminal_selection, Some(pane));
    assert!(dirty);
}

#[test]
fn mouse_wheel_on_alternate_screen_consumes_without_local_scroll() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let target = state.pane_mut_anywhere(pane).expect("terminal pane");
    target.terminal.feed(b"\x1b[?1049h\x1b[?1007h");
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(target.terminal.display_offset(), 0);
    assert!(dirty);
}

#[test]
fn key_and_paste_snap_scrolled_pane_back_to_bottom() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    scrolled_pane(&mut state, pane);
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(
        target.terminal.display_offset(),
        0,
        "typing returns to bottom"
    );

    scrolled_pane(&mut state, pane);
    handle_terminal_event(
        TerminalEvent::Paste("x".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("terminal pane");
    assert_eq!(
        target.terminal.display_offset(),
        0,
        "paste returns to bottom"
    );
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
        &mut dirty,
    );
    assert_eq!(state.tab_scroll, 0);
}

/// 目标窗格顶边框右端的视图切换按钮矩形。
fn toggle_button_of(geo: &Geometry, pane: PaneId) -> Rect {
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .expect("pane rect");
    ui::main_content::toggle_button(*rect).expect("toggle button visible")
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
        &mut dirty,
    );
    assert!(dirty);
    assert!(state.prompt_focused, "按钮是控件，不抢焦点");
    assert_eq!(view_of(&state, pane), PaneView::Terminal);

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), button.x, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(view_of(&state, pane), PaneView::Lx);
}

#[test]
fn lx_view_swallows_keys_and_paste() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    scrolled_pane(&mut state, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!dirty, "lx 视图按键被吞掉");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .terminal
            .display_offset()
            > 0,
        "按键不得吸回隐藏终端的滚动位置"
    );

    handle_terminal_event(
        TerminalEvent::Paste("x".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!dirty, "lx 视图粘贴被吞掉");
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .terminal
            .display_offset()
            > 0,
        "粘贴不得写入隐藏终端"
    );

    // Ctrl+Q 仍优先退出。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.should_quit);
}

#[test]
fn lx_view_content_click_only_focuses() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    crate::app::update::focus_prompt(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!state.prompt_focused);
    assert_eq!(state.active_tab().layout.focus(), pane);
    assert!(state.terminal_selection.is_none(), "lx 视图不启动终端选区");
    assert!(state.selection.is_none());
}

#[test]
fn lx_view_ignores_wheel_and_mouse_report() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        target
            .terminal
            .feed(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
    }
    scrolled_pane(&mut state, pane);
    let inner = pane_inner(&geo, pane);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for kind in [
        MouseEventKind::ScrollUp,
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_terminal_event(
            mouse(kind, inner.x, inner.y),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut dirty,
        );
    }
    let offset = state
        .pane_anywhere(pane)
        .expect("pane")
        .terminal
        .display_offset();
    assert!(offset > 0, "隐藏终端不接收滚轮/鼠标上报，视口保持不动");
    assert!(state.terminal_selection.is_none());
}

/// 窗格视图（测试断言用）。
fn view_of(state: &AppState, pane: PaneId) -> PaneView {
    state.pane_anywhere(pane).expect("pane exists").view
}

/// 按状态重新计算窗格矩形（拖拽后断言用）。
fn pane_rects_of(state: &AppState, geo: &Geometry, config: &Config) -> Vec<(PaneId, Rect)> {
    layout::pane_rects(
        &state.active_tab().layout,
        geo.view.panes,
        config.min_pane_width,
        config.min_pane_height,
    )
}

fn rect_of(rects: &[(PaneId, Rect)], pane: PaneId) -> Rect {
    rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| *rect)
        .expect("pane rect")
}

#[test]
fn mouse_drag_resizes_pane_boundary() {
    let config = Config::default();
    let mut state = demo_terminal();
    let left = state.active_tab().layout.focus();
    let right = state
        .active_tab_mut()
        .split_pane(left, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let before = pane_rects_of(&state, &geo, &config);
    let left_before = rect_of(&before, left);
    let boundary = rect_of(&before, right).x;
    let row = left_before.y + 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(
        state.resizing_pane,
        Some(layout::BoundaryHit {
            first: left,
            second: right,
            direction: ratatui::layout::Direction::Horizontal,
        })
    );

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary + 4, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let after = pane_rects_of(&state, &geo, &config);
    assert_eq!(rect_of(&after, left).width, left_before.width + 4);

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), boundary + 4, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.resizing_pane.is_none());
}

#[test]
fn mouse_drag_resizes_vertical_pane_boundary() {
    let config = Config::default();
    let mut state = demo_terminal();
    let top = state.active_tab().layout.focus();
    let bottom = state
        .active_tab_mut()
        .split_pane(top, ratatui::layout::Direction::Vertical)
        .expect("split succeeds");
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let before = pane_rects_of(&state, &geo, &config);
    let top_before = rect_of(&before, top);
    let boundary = rect_of(&before, bottom).y;
    let column = top_before.x + 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, boundary),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(
        state.resizing_pane,
        Some(layout::BoundaryHit {
            first: top,
            second: bottom,
            direction: ratatui::layout::Direction::Vertical,
        })
    );

    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            column,
            boundary + 4,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let after = pane_rects_of(&state, &geo, &config);
    assert_eq!(rect_of(&after, top).height, top_before.height + 4);

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), column, boundary + 4),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.resizing_pane.is_none());
}

#[test]
fn mouse_down_on_collapsed_pane_boundary_does_not_start_resize() {
    let config = Config::default();
    let mut state = demo_terminal();
    let left = state.active_tab().layout.focus();
    let right = state
        .active_tab_mut()
        .split_pane(left, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    assert!(state.active_tab_mut().layout.set_collapsed(right, true));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let rects = pane_rects_of(&state, &geo, &config);
    let boundary = rect_of(&rects, right).x;
    let row = rect_of(&rects, left).y + 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.resizing_pane.is_none());
}

#[test]
fn mouse_reporting_pane_does_not_steal_boundary_drag() {
    let config = Config::default();
    let mut state = demo_terminal();
    let left = state.active_tab().layout.focus();
    let right = state
        .active_tab_mut()
        .split_pane(left, ratatui::layout::Direction::Horizontal)
        .expect("split succeeds");
    {
        let target = state.pane_mut_anywhere(left).expect("terminal pane");
        target
            .terminal
            .feed(b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h");
    }
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let rects = pane_rects_of(&state, &geo, &config);
    let boundary = rect_of(&rects, right).x;
    let row = rect_of(&rects, left).y + 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.resizing_pane.is_some());
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
        &mut dirty,
    );
    assert_eq!(state.workspace_scroll, 1);
    assert_eq!(state.workspace_hover, Some(1), "滚动后按新偏移重算悬停");
}

#[test]
fn mouse_interacts_with_block_command_panel() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('#'),
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let panel = crate::ui::prompt::panel_layout(&state.prompt, area).expect("block panel visible");
    let rect = panel.rect;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 滚轮悬停在面板上：只滚动视口（记录显式视口起点），不移动高亮、不滚动 prompt 视口。
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, rect.x + 2, rect.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(
        state.prompt.panel().map(|panel| panel.active()),
        Some(0),
        "滚轮不动高亮"
    );
    assert_eq!(
        state.prompt.panel().map(|panel| panel.viewport()),
        Some(Some(1))
    );

    // 悬停条目：只改高亮。
    handle_terminal_event(
        mouse(MouseEventKind::Moved, rect.x + 2, rect.y + 4),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.panel().map(|panel| panel.active()), Some(3));

    // 左键点击第 2 项：确认插入 H2 并关闭面板。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            rect.x + 2,
            rect.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "## ");
    assert!(state.prompt.panel().is_none());
}

#[test]
fn mouse_wheel_over_mention_panel_scrolls_viewport_only() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('@'),
    );
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    let entries: Vec<crate::app::markdown::MentionEntry> = (0..8)
        .map(|index| crate::app::markdown::MentionEntry {
            path: format!("src/f{index}.rs"),
            is_directory: false,
        })
        .collect();
    crate::app::update::apply_mention_entries(&mut state, generation, entries);

    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let before = crate::ui::prompt::mention_layout(&state.prompt, area).expect("mention panel");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            before.rect.x + 2,
            before.rect.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    let after = crate::ui::prompt::mention_layout(&state.prompt, area).expect("mention panel");
    assert_eq!(after.start, before.start + 1, "滚轮下移视口一项");
    assert_eq!(
        state.prompt.mention().map(|panel| panel.active()),
        Some(0),
        "滚轮不动高亮"
    );
    assert!(dirty);
}

/// prompt 工具栏按钮矩形（按几何取，避免硬编码列）。
fn toolbar_button(geo: &Geometry, button: layout::PromptToolbarButton) -> Rect {
    layout::prompt_toolbar_button_rect(geo.view.prompt, button).expect("toolbar button visible")
}

/// 构造 prompt 工具栏测试场景：宽右栏、已同步几何。
fn toolbar_geometry(state: &mut AppState) -> (Config, Geometry) {
    let config = Config::default();
    state.prompt_width = 40;
    let geo = geometry(state, &config, Rect::new(0, 0, 120, 30));
    crate::app::update::resize_panes(state, &geo.rects);
    (config, geo)
}

#[test]
fn mouse_click_prompt_toolbar_undo_redo_and_save() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    state.prompt.insert_str("hello");
    assert!(!state.prompt.is_saved());
    assert!(state.prompt.can_undo());

    let undo = toolbar_button(&geo, layout::PromptToolbarButton::Undo);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), undo.x, undo.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "");
    assert!(state.prompt.can_redo());
    assert!(!state.prompt_focused, "工具栏按钮不抢焦点");
    assert!(dirty);

    let redo = toolbar_button(&geo, layout::PromptToolbarButton::Redo);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), redo.x, redo.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "hello");

    let save = toolbar_button(&geo, layout::PromptToolbarButton::Save);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), save.x, save.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt.is_saved(), "点击保存点复位为已保存");
}

#[test]
fn mouse_click_prompt_toolbar_select_all_then_typing_replaces_text() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    state.prompt.insert_str("first\nsecond");
    let select_all = toolbar_button(&geo, layout::PromptToolbarButton::SelectAll);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            select_all.x,
            select_all.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        state
            .selection
            .is_some_and(|selection| selection.pane() == state.prompt.id())
    );
    assert_eq!(
        update::prompt_selection_text(&state).as_deref(),
        Some("first\nsecond")
    );

    update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText("new".into()),
    );
    assert_eq!(state.prompt.text(), "new", "全选后输入替换全文");
}

#[test]
fn mouse_click_prompt_header_swallows_without_focus_or_cursor() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.prompt.insert_str("ab\ncd");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let bar = layout::prompt_toolbar_rect(geo.view.prompt).expect("toolbar visible");
    let divider = layout::prompt_divider_rect(geo.view.prompt).expect("divider visible");
    let select_all = toolbar_button(&geo, layout::PromptToolbarButton::SelectAll);
    let blank_x = select_all.x - 1;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), blank_x, bar.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), blank_x, divider.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    assert!(!state.prompt_focused, "表头空白不聚焦文本视口");
    assert!(state.selection.is_none());
    assert_eq!(state.prompt.cursor_cell(), Some((1, 2)), "光标保持在文末");
    assert!(!dirty, "表头空白点击无副作用");
}

#[test]
fn mouse_click_prompt_text_row_places_cursor_below_header() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.prompt.insert_str("ab\ncd");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let text = layout::prompt_text_rect(geo.view.prompt);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            text.x + 1,
            text.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt_focused);
    assert_eq!(state.prompt.cursor_cell(), Some((1, 1)));
    assert!(dirty);
}

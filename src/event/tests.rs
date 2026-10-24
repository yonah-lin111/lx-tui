use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::overlay::{ConfirmSwitchCwd, Overlay};
use crate::app::prompt::{PasteCandidateKind, PromptRowKind};
use crate::app::state::PaneView;
use crate::app::toast::{Toast, ToastKind};
use crate::layout;

use super::*;

mod lx_toast;

fn geometry(state: &AppState, config: &Config, screen: Rect) -> Geometry {
    let view = ui::layout::compute(
        screen,
        config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    );
    let mut rects = if state.prompt_left && state.panes_collapsed {
        Vec::new()
    } else {
        layout::pane_rects(
            &state.active_tab().layout,
            view.panes,
            config.min_pane_width,
            config.min_pane_height,
        )
    };
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

/// 侧栏折叠按钮矩形：tooltip 悬停测试的固定目标。
fn collapse_sidebar_area(geo: &Geometry) -> Rect {
    ui::collapse_buttons(&geo.view, false)
        .into_iter()
        .find(|button| button.target == ui::CollapseTarget::Sidebar)
        .expect("sidebar collapse button visible")
        .area
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
fn swapped_divider_drag_resizes_prompt_from_left() {
    let config = Config::default();
    let mut state = demo_terminal();
    update::swap_prompt_side(&mut state, 96, 38);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    assert!(geo.view.prompt_left);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 交换后分隔线两列为 prompt 右边框与窗格左边框；取 prompt 侧一列。
    let boundary = geo.view.prompt.right() - 1;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), boundary, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.resizing_prompt, "press on left divider starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary + 5, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt_width, geo.view.prompt.width + 5);
}

#[test]
fn mouse_move_over_swap_button_is_not_divider_hover() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let button = ui::swap_button(&geo.view).expect("swap button visible");

    handle_terminal_event(
        mouse(MouseEventKind::Moved, button.x, button.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        !state.prompt_hover,
        "swap button is a control, not a divider"
    );

    handle_terminal_event(
        mouse(MouseEventKind::Moved, geo.view.prompt.x, button.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt_hover, "divider rows still highlight on hover");
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
fn mouse_wheel_over_short_prompt_does_not_scroll_without_scrollbar() {
    // 回归：正文不溢出、滚动条不出现时，滚轮不得移动视口，也不产生多余重绘。
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let inner = crate::layout::prompt_text_rect(geo.view.prompt);
    state.prompt.resize(inner.width, inner.height);
    state.prompt.insert_str("1\n2");
    assert!(
        ui::prompt_scrollbar(&geo.view, &state).is_none(),
        "前提：短正文没有滚动条"
    );
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, inner.x + 1, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.scroll(), 0, "没有滚动条时滚轮不移动视口");
    assert!(!dirty, "没有位移就不要触发重绘");
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
fn mouse_track_click_prompt_tree_scrollbar_scrolls_list() {
    use crate::app::prompt::PromptItem;
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    update::show_prompt_tree(&mut state);
    let panel = &mut state.workspaces[0].prompt_panel;
    panel.tree.replace(
        vec![],
        (0..40)
            .map(|index| PromptItem::single(index, None, format!("item-{index:02}"), String::new()))
            .collect(),
    );
    let bar = ui::prompt_scrollbar(&geo.view, &state).expect("scrollbar is visible");
    let gutter = crate::layout::prompt_tree_scrollbar_rect(geo.view.prompt).expect("gutter");
    assert_eq!(bar.track, gutter, "树视图滚动条槽在搜索框与分割线之下");
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
    assert!(state.workspaces[0].prompt_panel.scroll > 0, "轨道点击跳转");
    assert!(!state.prompt_focused, "滚动条按下不抢焦点");
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
fn mouse_right_click_workspace_item_opens_workspace_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let (column, row) = (sections.workspaces.x, sections.workspaces.y);
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
        panic!("workspace menu overlay expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::Workspace(0)
    );
    assert_eq!(menu.anchor, (column, row));
    assert!(dirty);
}

#[test]
fn mouse_right_click_workspace_list_blank_opens_new_workspace_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let (column, row) = (sections.workspaces.x, sections.workspaces.y + 1);
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
        panic!("workspace list menu overlay expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::WorkspaceList
    );
    assert_eq!(
        menu.commands,
        vec![crate::app::overlay::MenuCommand::NewWorkspace]
    );
    assert_eq!(menu.anchor, (column, row));
    assert!(dirty);
}

#[test]
fn mouse_right_click_workspace_blank_retargets_open_menu() {
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
    assert!(state.overlay.is_some(), "窗格菜单先打开");

    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let (column, row) = (sections.workspaces.x, sections.workspaces.y + 1);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("workspace list menu overlay expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::WorkspaceList,
        "菜单改挂到工作区列表空白目标"
    );
    assert_eq!(menu.anchor, (column, row));
}

#[test]
fn mouse_right_click_agents_blank_does_not_open_menu() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let sections =
        ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed).expect("sections");
    let (column, row) = (sections.agents.x, sections.agents.y + 1);
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

    assert!(state.overlay.is_none());
    assert!(!dirty);
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
fn lx_view_edits_input_and_pastes_text() {
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
    assert!(dirty, "lx 输入产生重绘");
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "a",
        "按键进入 lx 输入框"
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .terminal
            .display_offset()
            > 0,
        "按键不得吸回隐藏终端的滚动位置"
    );

    // 普通文本粘贴进入 lx 输入框，不写入隐藏终端。
    dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("bc".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(dirty, "lx 视图粘贴产生重绘");
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "abc",
        "粘贴进入 lx 输入框"
    );
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
fn lx_view_path_paste_opens_reference_panel() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    let dir = env!("CARGO_MANIFEST_DIR").to_string();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        TerminalEvent::Paste(dir.clone()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(dirty, "路径粘贴产生重绘");
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(lx.paste_panel().is_some(), "路径粘贴打开参照面板");
    assert!(lx.input.contains(&dir), "占位路径写入 lx 输入框");

    // Enter 采纳默认高亮项（引用），面板关闭且占位被改写。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert!(lx.paste_panel().is_none(), "确认后关闭面板");
    assert!(lx.input.contains("@[refer-folder]"), "改写为引用格式");
}

#[test]
fn lx_view_content_click_focuses_and_starts_selection() {
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
    assert!(state.selection.is_none(), "lx 视图不启动 prompt 选区");
    assert!(
        state.lx_selection.is_some(),
        "lx 视图内容区按下开始整页选区"
    );
}

#[test]
fn lx_input_click_places_cursor_and_starts_text_selection() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let lx = &mut state.pane_mut_anywhere(pane).expect("pane").lx;
        for ch in "hello".chars() {
            lx.insert_char(ch);
        }
    }
    let inner = pane_inner(&geo, pane);
    let (x, y) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::input_geom(inner, &target.lx))
        .map(|geom| geom.text_origin())
        .expect("input geom");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), x + 2, y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.cursor, 2, "点击把光标定位到单元格");
    assert!(lx.has_text_selection(), "输入框内开始文本拖拽");
    assert!(state.lx_input_drag.is_some());
    assert!(state.lx_selection.is_none(), "输入框内不再启动整页选区");
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
fn mouse_motion_tracks_prompt_tree_hover() {
    let config = Config::default();
    let mut state = demo_terminal();
    crate::app::update::show_prompt_tree(&mut state);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let scroll = state.prompt_owner().prompt_panel.scroll;
    let hovered = state.prompt_owner().prompt_panel.selected;
    let rows_area_y = area.y + 2;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 悬停树行列表区（选中行可见）。
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            area.x + 2,
            rows_area_y + (hovered - scroll) as u16,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt_tree_hover, Some(hovered));
    assert!(dirty, "悬停变化置脏重绘");

    // 移到搜索行（区域内非列表行）：清除悬停。
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, area.x + 2, area.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt_tree_hover, None);
    assert!(dirty, "离开列表行清空悬停");

    // 离开提示词面板：保持清空。
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
    assert_eq!(state.prompt_tree_hover, None);

    // 编辑视图（非树视图）不产生列表悬停。
    crate::app::update::toggle_prompt_panel_mode(&mut state);
    assert!(
        !crate::app::update::prompt_tree_mode(&state),
        "已切到编辑视图"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Moved, area.x + 2, rows_area_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt_tree_hover, None);
}

#[test]
fn mouse_leaving_overlay_menu_clears_hover_feedback() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    crate::app::update::open_tab_menu(&mut state, 0, (10, 4));
    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("menu open");
    };
    let layout = ui::overlay::menu_layout(&state, geo.screen, menu);
    let item = layout.item_rects[1];

    // 悬停某行：高亮跟随。
    handle_terminal_event(
        mouse(MouseEventKind::Moved, item.x + 1, item.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.menu_hover);
    assert_eq!(
        state.overlay.as_ref().and_then(|overlay| match overlay {
            Overlay::Menu(menu) => Some(menu.selected),
            _ => None,
        }),
        Some(1)
    );

    // 移出容器：清除悬停高亮反馈（选中项保持不变）。
    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.screen.x,
            geo.screen.bottom().saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!state.menu_hover, "离开容器移除悬停效果");
    assert!(dirty);
}

#[test]
fn mouse_leaving_page_menu_clears_hover_feedback() {
    let config = Config::default();
    let mut state = demo_terminal();
    assert!(crate::app::update::open_temp_prompt(&mut state));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    assert!(crate::app::update::open_prompt_page_menu(
        &mut state,
        (10, 4)
    ));
    assert!(state.menu_hover, "页码菜单打开时指针位于容器内");

    // 移出容器：清除悬停高亮反馈。
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            geo.screen.x,
            geo.screen.bottom().saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!state.menu_hover, "离开容器移除悬停效果");

    // 回到边框按钮：恢复悬停并命中对应行。
    let Some(Overlay::PageMenu(menu)) = state.overlay.as_ref() else {
        panic!("page menu open");
    };
    let layout2 = ui::overlay::page_menu_layout(&state, geo.screen, menu);
    handle_terminal_event(
        mouse(
            MouseEventKind::Moved,
            layout2.new_rect.x,
            layout2.new_rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.menu_hover);
    assert_eq!(
        state.overlay.as_ref().and_then(|overlay| match overlay {
            Overlay::PageMenu(menu) => Some(menu.selected),
            _ => None,
        }),
        Some(0),
        "悬停 [add] 按钮"
    );
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
    let prompt = state.prompt.id();
    crate::app::update::apply_mention_entries(&mut state, prompt, generation, entries);

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
    layout::prompt_toolbar_button_rect(geo.view.prompt, button, false)
        .expect("toolbar button visible")
}

#[test]
fn mouse_click_folders_button_opens_referenced_menu() {
    let mut state = demo_terminal();
    state.active_workspace_mut().cwd = Some(std::path::PathBuf::from("/tmp/lx-event-ref"));
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let folders = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Folders,
        false,
    )
    .expect("folders button visible");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            folders.x,
            folders.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(matches!(
        state.overlay,
        Some(Overlay::ReferencedFolderMenu(_))
    ));
    assert!(dirty);
}

#[test]
fn folders_button_no_op_without_project_key() {
    let mut state = demo_terminal();
    state.active_workspace_mut().cwd = None;
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let folders = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Folders,
        false,
    )
    .expect("folders button visible");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            folders.x,
            folders.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.overlay.is_none(), "无项目键不打开菜单");
}

/// 构造已进入子目录的引用文件夹搜索弹窗（Ctrl/Cmd+Z 回退的前置）。
fn referenced_search_entered_state() -> AppState {
    let mut state = demo_terminal();
    let root = std::path::PathBuf::from("/tmp/lx-ref-z/root");
    let sub = root.join("docs");
    state.overlay = Some(Overlay::ReferencedFolderSearch(
        crate::app::overlay::ReferencedFolderSearch {
            anchor: (0, 0),
            folder: 1,
            key: Some(std::path::PathBuf::from("/tmp/lx-ref-z")),
            root: sub,
            initial_root: root.clone(),
            stack: vec![root],
            query: crate::app::overlay::TextInput::new(""),
            entries: Vec::new(),
            items: Vec::new(),
            scan_truncated: false,
            truncated: false,
            selected: 0,
            viewport: None,
            loading: false,
            failed: false,
            request: Some(1),
        },
    ));
    state
}

#[test]
fn referenced_search_ctrl_or_cmd_z_returns_to_parent_scope() {
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
        let mut state = referenced_search_entered_state();
        let (config, geo) = toolbar_geometry(&mut state);
        let mut sessions = HashMap::new();
        let mut dirty = false;
        handle_terminal_event(
            TerminalEvent::Key(KeyEvent::new(KeyCode::Char('z'), modifiers)),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut dirty,
        );
        let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
            panic!("search stays open: {modifiers:?}");
        };
        assert_eq!(
            search.root,
            std::path::PathBuf::from("/tmp/lx-ref-z/root"),
            "回退到父搜索范围: {modifiers:?}"
        );
        assert!(search.stack.is_empty(), "{modifiers:?}");
        assert!(search.request.is_some(), "回退后登记新扫描: {modifiers:?}");
        assert!(dirty, "{modifiers:?}");
    }
}

#[test]
fn referenced_search_ctrl_z_at_root_is_noop() {
    let mut state = referenced_search_entered_state();
    if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_mut() {
        search.stack.clear();
    }
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("search stays open");
    };
    assert!(
        search.root.ends_with("docs"),
        "初始根不再回退: {:?}",
        search.root
    );
}

/// 打开引用文件夹目录弹窗并放入两条记录。
fn referenced_menu_state() -> AppState {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let mut state = demo_terminal();
    state.overlay = Some(Overlay::ReferencedFolderMenu(
        crate::app::overlay::ReferencedFolderMenu {
            anchor: (0, 0),
            key: Some(std::path::PathBuf::from("/tmp/lx-ref-menu")),
            rows: vec![
                ReferencedFolderMenuRow::Folder {
                    id: 1,
                    path: std::path::PathBuf::from("/tmp/lx-ref-menu/alpha"),
                },
                ReferencedFolderMenuRow::Folder {
                    id: 2,
                    path: std::path::PathBuf::from("/tmp/lx-ref-menu/beta"),
                },
                ReferencedFolderMenuRow::Add,
            ],
            selected: 0,
            hover: None,
            viewport: None,
        },
    ));
    state
}

/// 弹窗几何（事件层与渲染层共用同一 layout）；`screen` 必须与事件几何一致。
fn referenced_menu_layout(
    state: &AppState,
    geo: &Geometry,
) -> crate::ui::overlay::ReferencedMenuLayout {
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu open");
    };
    crate::ui::overlay::referenced_menu_layout(geo.screen, menu)
}

#[test]
fn mouse_click_referenced_menu_row_selects_then_double_click_opens_search() {
    let mut state = referenced_menu_state();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let row = referenced_menu_layout(&state, &geo).rows[1];
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), row.x, row.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    // 单击只选中（与 prompt 列表一致）。
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderMenu(_))),
        "单击不打开内容弹窗"
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu open");
    };
    assert_eq!(menu.selected, 1, "单击选中该行");
    // 双击同一行才打开内容弹窗。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), row.x, row.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref() else {
        panic!("双击打开搜索");
    };
    assert_eq!(search.folder, 2, "双击行对应的记录被打开");
    assert!(dirty);
}

#[test]
fn mouse_click_referenced_menu_add_row_opens_input() {
    let mut state = referenced_menu_state();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let rows = referenced_menu_layout(&state, &geo).rows;
    let add = rows.last().copied().expect("add row");
    // 动作项单击即打开路径输入（与草稿保存选择器一致）。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), add.x, add.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderInput(_))),
        "新建行打开路径输入"
    );
    assert!(dirty);
}

#[test]
fn mouse_click_referenced_menu_blank_keeps_open_and_outside_closes() {
    let mut state = referenced_menu_state();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let layout = referenced_menu_layout(&state, &geo);
    let rows_bottom = layout.rows.last().expect("rows").y;
    let blank = Rect::new(layout.area.x + 2, rows_bottom + 2, 1, 1);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), blank.x, blank.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderMenu(_))),
        "弹窗内空白保持打开"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.overlay.is_none(), "弹窗之外关闭");
}

#[test]
fn referenced_menu_ctrl_c_copies_and_ctrl_d_or_del_removes() {
    for code in [KeyCode::Char('d'), KeyCode::Delete] {
        let mut state = referenced_menu_state();
        let (config, geo) = toolbar_geometry(&mut state);
        let mut sessions = HashMap::new();
        let mut dirty = false;
        handle_terminal_event(
            TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::CONTROL)),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut dirty,
        );
        assert!(
            matches!(state.overlay, Some(Overlay::ReferencedFolderDelete(_))),
            "移除键打开删除确认: {code:?}"
        );
    }
    let mut state = referenced_menu_state();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::ReferencedFolderMenu(_))),
        "复制不关弹窗"
    );
    assert!(matches!(
        state.effects.as_slice(),
        [crate::app::effects::Effect::CopyToClipboard { text, .. }]
            if text.contains("lx-ref-menu/alpha")
    ));
}

/// 打开引用文件夹目录弹窗并放入 `count` 条记录（末尾恒为新建行），用于溢出滚动测试。
fn referenced_menu_overflow_state(count: usize) -> AppState {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let mut state = demo_terminal();
    let mut rows: Vec<ReferencedFolderMenuRow> = (0..count)
        .map(|id| ReferencedFolderMenuRow::Folder {
            id: id as i64,
            path: std::path::PathBuf::from(format!("/tmp/lx-ref-menu/f{id:02}")),
        })
        .collect();
    rows.push(ReferencedFolderMenuRow::Add);
    state.overlay = Some(Overlay::ReferencedFolderMenu(
        crate::app::overlay::ReferencedFolderMenu {
            anchor: (0, 0),
            key: Some(std::path::PathBuf::from("/tmp/lx-ref-menu")),
            rows,
            selected: 0,
            hover: None,
            viewport: None,
        },
    ));
    state
}

#[test]
fn mouse_hover_over_referenced_menu_backgrounds_row_without_moving_highlight() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 悬停第 4 个可见行：只记录悬停行（铺悬停底色），不动 `❯` 选中标记，不动显式视口。
    let target = layout.rows[3];
    handle_terminal_event(
        mouse(MouseEventKind::Moved, target.x + 1, target.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.hover, Some(3), "悬停记录悬停行");
    assert_eq!(menu.selected, 0, "悬停不移动 `❯` 选中标记");
    assert_eq!(menu.viewport, None, "悬停不动显式视口");
    assert!(dirty);
}

#[test]
fn mouse_hover_over_referenced_menu_after_wheel_keeps_viewport() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 先滚轮滚开视口。
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            layout.rows[0].x + 1,
            layout.rows[0].y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    // 再悬停窗口内某行：只记录悬停行，显式视口保持不变（窗口不跳）。
    let layout = referenced_menu_layout(&state, &geo);
    let target = layout.rows[2];
    let row_index = layout.start + 2;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, target.x + 1, target.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.hover, Some(row_index), "悬停按可见行命中");
    assert_eq!(menu.selected, 0, "悬停不移动 `❯` 选中标记");
    assert_eq!(menu.viewport, Some(1), "悬停不改变已滚开的视口");
}

#[test]
fn mouse_hover_over_referenced_menu_paints_background_without_marker() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let target = layout.rows[3];
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, target.x + 1, target.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        geo.screen.width,
        geo.screen.height,
    ))
    .expect("backend");
    terminal
        .draw(|frame| {
            crate::ui::render(frame, &state, &config);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    // 悬停行：左侧无 `❯` 标记、整行铺悬停底色。
    assert_eq!(
        buffer[(target.x, target.y)].symbol(),
        " ",
        "悬停行左侧无 ❯ 标记"
    );
    assert_eq!(
        buffer[(target.x, target.y)].bg,
        ratatui::style::Color::Indexed(240),
        "悬停行铺悬停底色"
    );
    // 选中行（未悬停）只有 `❯` 标记、不铺底色。
    let selected = layout.rows[0];
    assert_eq!(
        buffer[(selected.x, selected.y)].symbol(),
        "❯",
        "选中行保留 ❯ 标记"
    );
    assert_eq!(
        buffer[(selected.x, selected.y)].bg,
        ratatui::style::Color::Indexed(236),
        "选中行不铺底色"
    );
}

#[test]
fn mouse_hover_after_opening_referenced_menu_backgrounds_row_without_marker() {
    use crate::app::overlay::ReferencedFolderMenuRow;
    let mut state = demo_terminal();
    state.active_workspace_mut().cwd = Some(std::path::PathBuf::from("/tmp/lx-event-hover"));
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 通过工具栏 [dir] 按钮真实打开弹窗。
    let folders = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Folders,
        false,
    )
    .expect("folders button visible");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            folders.x,
            folders.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    if let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_mut() {
        menu.rows = (0..8)
            .map(|id| ReferencedFolderMenuRow::Folder {
                id,
                path: std::path::PathBuf::from(format!("/tmp/lx-event-hover/f{id}")),
            })
            .chain(std::iter::once(ReferencedFolderMenuRow::Add))
            .collect();
        menu.selected = 0;
        menu.hover = None;
        menu.viewport = None;
    }
    let layout = referenced_menu_layout(&state, &geo);
    let target = layout.rows[3];
    handle_terminal_event(
        mouse(MouseEventKind::Moved, target.x + 1, target.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(
        geo.screen.width,
        geo.screen.height,
    ))
    .expect("backend");
    terminal
        .draw(|frame| {
            crate::ui::render(frame, &state, &config);
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    // 悬停后：悬停行只铺底色、无 `❯`；选中行（第 0 行）保留 `❯` 且无底色。
    assert_eq!(
        buffer[(target.x, target.y)].symbol(),
        " ",
        "悬停行左侧无 ❯ 标记"
    );
    assert_eq!(
        buffer[(target.x, target.y)].bg,
        ratatui::style::Color::Indexed(240),
        "悬停行铺悬停底色"
    );
    assert_eq!(
        buffer[(layout.rows[0].x, layout.rows[0].y)].symbol(),
        "❯",
        "选中行保留 ❯ 标记"
    );
    assert_eq!(
        buffer[(layout.rows[0].x, layout.rows[0].y)].bg,
        ratatui::style::Color::Indexed(236),
        "选中行不铺底色"
    );
}

#[test]
fn mouse_wheel_over_referenced_menu_scrolls_viewport_not_highlight() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    assert!(layout.scrollbar.is_some(), "记录溢出时出现滚动条");
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let prompt_scroll = state.prompt.scroll();

    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            layout.rows[0].x + 1,
            layout.rows[0].y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.viewport, Some(1), "滚轮下移视口一项");
    assert_eq!(menu.selected, 0, "滚轮不动高亮");
    assert_eq!(
        state.prompt.scroll(),
        prompt_scroll,
        "弹窗内滚轮不滚动 prompt"
    );
    assert!(dirty);
}

#[test]
fn mouse_click_referenced_menu_scrollbar_track_jumps_viewport() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let bar = layout.scrollbar.expect("list overflows");
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
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert!(
        menu.viewport.is_some_and(|offset| offset > 0),
        "轨道点击跳转视口"
    );
    assert_eq!(menu.selected, 0, "滚动条不动高亮");
    assert!(
        state.referenced_menu_scroll_drag.is_none(),
        "非 thumb 不进入拖拽"
    );
    assert!(dirty);
}

#[test]
fn mouse_drag_referenced_menu_scrollbar_thumb_scrolls_viewport() {
    let mut state = referenced_menu_overflow_state(30);
    let (config, geo) = toolbar_geometry(&mut state);
    let layout = referenced_menu_layout(&state, &geo);
    let bar = layout.scrollbar.expect("list overflows");
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
    assert_eq!(
        state.referenced_menu_scroll_drag,
        Some(0),
        "按住 thumb 进入拖拽"
    );

    let max = ui::widgets::scrollbar::offset_from_drag_row(&bar, bar.track.bottom() - 1, 0);
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
    let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() else {
        panic!("menu");
    };
    assert_eq!(menu.viewport, Some(max), "拖拽视口到末窗口");
    assert_eq!(menu.selected, 0, "拖拽不动高亮");

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
    assert!(state.referenced_menu_scroll_drag.is_none(), "松开清除拖拽");
}

#[test]
fn mouse_click_prompt_pin_toggles_pin() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let pin = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Pin,
        false,
    )
    .expect("pin visible");
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), pin.x, pin.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt_pinned, Some(state.active_workspace));
    assert!(dirty);

    let unpin = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Pin,
        false,
    )
    .expect("pin visible");
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), unpin.x, unpin.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt_pinned, None);
    assert!(dirty);
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

    // 默认绑定临时提示词：点击保存点提交缓冲并打开保存选择器。
    let save = toolbar_button(&geo, layout::PromptToolbarButton::Save);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), save.x, save.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        matches!(
            state.overlay,
            Some(crate::app::overlay::Overlay::SaveDraft(_))
        ),
        "临时提示词保存点打开保存选择器"
    );
    let Some(crate::app::prompt::PromptEditing::Temp(temp_id)) =
        state.workspaces[0].prompt_panel.editing
    else {
        panic!("temp bound");
    };
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .temp(temp_id)
            .map(|temp| temp.active_content()),
        Some("hello"),
        "提交缓冲到 temp"
    );
}

#[test]
fn ctrl_s_saves_prompt_from_editor() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    state.prompt_focused = true;

    // 默认绑定临时提示词：Ctrl+S 提交缓冲并打开保存选择器。
    state.prompt.insert_str("draft body");
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(dirty);
    assert!(
        matches!(
            state.overlay,
            Some(crate::app::overlay::Overlay::SaveDraft(_))
        ),
        "Ctrl+S 在临时提示词上打开保存选择器"
    );
    let Some(crate::app::prompt::PromptEditing::Temp(temp_id)) =
        state.workspaces[0].prompt_panel.editing
    else {
        panic!("temp bound");
    };
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .temp(temp_id)
            .map(|temp| temp.active_content()),
        Some("draft body")
    );
}

#[test]
fn ctrl_s_saves_open_item_immediately() {
    use crate::app::prompt::PromptItem;
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![PromptItem::single(7, None, "item", "body")]);
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, 7));
    state.prompt_focused = true;
    state.prompt.insert_str("+");
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt.is_saved(), "Ctrl+S 立即复位保存点");
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SavePromptContent { id: 7, .. }
        )),
        "Ctrl+S 登记条目落库效应"
    );
}

#[test]
fn ctrl_s_ignores_empty_prompt_content() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    state.prompt_focused = true;
    let press =
        |state: &mut AppState, sessions: &mut HashMap<PaneId, PtySession>, dirty: &mut bool| {
            handle_terminal_event(
                TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
                state,
                sessions,
                &geo,
                &config,
                dirty,
            );
        };

    // 空草稿：不响应（不开保存选择器、无落库效应）。
    press(&mut state, &mut sessions, &mut dirty);
    assert!(state.overlay.is_none(), "空草稿 Ctrl+S 不打开选择器");
    assert!(state.effects.is_empty(), "空草稿无落库效应");

    // 仅空白同样不响应。
    state.prompt.insert_str("   ");
    press(&mut state, &mut sessions, &mut dirty);
    assert!(state.overlay.is_none(), "空白草稿 Ctrl+S 不响应");

    // 非空内容回归：照常提交缓冲并打开选择器。
    state.prompt.insert_str("body");
    press(&mut state, &mut sessions, &mut dirty);
    assert!(
        matches!(
            state.overlay,
            Some(crate::app::overlay::Overlay::SaveDraft(_))
        ),
        "非空草稿照常打开选择器"
    );
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
        mouse(MouseEventKind::Down(MouseButton::Left), bar.x + 15, bar.y),
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

/// prompt 树测试场景：进入树视图并重建树，返回匹配行的点击坐标；
/// 树行自搜索框与分割线之下开始（行高 1）。
fn prompt_tree_click(
    state: &mut AppState,
    geo: &Geometry,
    folders: Vec<crate::app::prompt::PromptFolder>,
    items: Vec<crate::app::prompt::PromptItem>,
    kind: PromptRowKind,
) -> (u16, u16) {
    update::show_prompt_tree(state);
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(folders, items);
    let area = layout::prompt_text_rect(geo.view.prompt);
    let index = state.workspaces[0]
        .prompt_panel
        .rows()
        .iter()
        .position(|row| row.kind == kind)
        .expect("row visible");
    (area.x + 1, area.y + 2 + index as u16)
}

/// 左键双击同一坐标：事件循环连续注入两次按下。
fn prompt_tree_double_click(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    point: (u16, u16),
) {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), point.0, point.1),
            state,
            &mut sessions,
            geo,
            config,
            &mut dirty,
        );
    }
}

#[test]
fn mouse_double_click_prompt_item_row_opens_editor() {
    use crate::app::prompt::{PromptEditing, PromptItem, PromptMode};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let point = prompt_tree_click(
        &mut state,
        &geo,
        vec![],
        vec![PromptItem::single(7, None, "item", "item body")],
        PromptRowKind::Item,
    );

    prompt_tree_double_click(&mut state, &geo, &config, point);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert_eq!(
        state.workspaces[0].prompt_panel.editing,
        Some(PromptEditing::Item(7))
    );
    assert_eq!(
        state.prompt.text(),
        "item body",
        "双击进入编辑并载入条目内容"
    );
    assert!(state.prompt_focused);
}

#[test]
fn mouse_right_click_prompt_item_keeps_selection() {
    use crate::app::prompt::PromptItem;
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let item_row = prompt_tree_click(
        &mut state,
        &geo,
        vec![],
        vec![
            PromptItem::single(7, None, "one", "first body"),
            PromptItem::single(8, None, "two", "second body"),
        ],
        PromptRowKind::Item,
    );
    // 选中停在第二个条目上，右键落在第一个条目行。
    assert!(crate::app::update::select_prompt_item(&mut state, 0, 8));
    let selected_before = state.workspaces[0].prompt_panel.selected;
    let other = state.workspaces[0]
        .prompt_panel
        .rows()
        .into_iter()
        .position(|row| {
            state.workspaces[0]
                .prompt_panel
                .tree
                .item_of_row(row)
                .is_some_and(|item| item.id == 7)
        })
        .expect("item 7 row");
    assert_ne!(selected_before, other, "右键目标行与选中行不同");

    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Right),
            item_row.0,
            item_row.1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );

    let Some(Overlay::Menu(menu)) = state.overlay.as_ref() else {
        panic!("prompt item menu expected");
    };
    assert_eq!(
        menu.target,
        crate::app::overlay::OverlayTarget::PromptItem {
            workspace: 0,
            item_id: 7
        }
    );
    assert_eq!(
        state.workspaces[0].prompt_panel.selected, selected_before,
        "右键只挂菜单：不改动列表选中行"
    );
}

#[test]
fn mouse_double_click_prompt_temp_row_opens_editor() {
    use crate::app::prompt::{PromptEditing, PromptMode};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let point = prompt_tree_click(&mut state, &geo, vec![], vec![], PromptRowKind::Temp);

    prompt_tree_double_click(&mut state, &geo, &config, point);
    assert_eq!(state.workspaces[0].prompt_panel.mode, PromptMode::Editor);
    assert!(
        matches!(
            state.workspaces[0].prompt_panel.editing,
            Some(PromptEditing::Temp(_))
        ),
        "双击临时提示词行进入其编辑视图"
    );
}

#[test]
fn mouse_double_click_prompt_folder_toggles_once() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let point = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        vec![PromptItem::single(7, Some(1), "item", "item body")],
        PromptRowKind::Folder,
    );

    prompt_tree_double_click(&mut state, &geo, &config, point);
    assert!(
        state.workspaces[0]
            .prompt_panel
            .tree
            .folder(1)
            .is_some_and(|folder| folder.collapsed),
        "双击文件夹只折叠一次，不来回切换"
    );
}

#[test]
fn mouse_interacts_with_slash_panel() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt.resize(40, 8);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('/'),
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let panel = crate::ui::prompt::slash_layout(&state.prompt, area).expect("slash panel visible");
    let rect = panel.rect;
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 悬停第 2 项：只改高亮。
    handle_terminal_event(
        mouse(MouseEventKind::Moved, rect.x + 2, rect.y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(
        state.prompt.slash_panel().map(|panel| panel.active()),
        Some(1)
    );

    // 左键点击第 1 项：插入 add 模板并关闭面板。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            rect.x + 2,
            rect.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt.text().starts_with("&&& addTemplate --start"));
    assert!(state.prompt.slash_panel().is_none());
}

/// 构造带 add 模板块的宽屏场景，返回几何与按钮行文本区。
fn template_button_state() -> (AppState, Config, Geometry, Rect) {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = 80;
    // 模板块含标题空行与底部空行共 14 行，prompt 需足够高以完整显示起始行按钮。
    state.prompt.resize(78, 16);
    crate::app::update::focus_prompt(&mut state);
    let (content, _) =
        crate::app::markdown::slash_template_content(crate::app::markdown::SlashCommandId::Add);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText(content.into()),
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 220, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    (state, config, geo, area)
}

#[test]
fn mouse_template_buttons_clean_status_and_delete() {
    let (mut state, config, geo, area) = template_button_state();
    let buttons = crate::ui::prompt::template_buttons(&state.prompt, area);
    assert_eq!(buttons.len(), 4, "宽屏下按钮可见");
    let hit = |button| {
        buttons
            .iter()
            .find(|hit| hit.button == button)
            .copied()
            .expect("button")
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 状态按钮：todo → in_progress。
    let status = hit(crate::ui::prompt::TemplateBlockButton::Status);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            status.rect.x + 1,
            status.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        state
            .prompt
            .text()
            .contains("&&& addTemplate --end in_progress")
    );

    // 清理按钮：移除未填写空项。
    let clean = hit(crate::ui::prompt::TemplateBlockButton::Clean);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            clean.rect.x + 1,
            clean.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!state.prompt.text().contains("- Reference: "));
    assert!(
        state
            .prompt
            .text()
            .contains("&&& addTemplate --end in_progress")
    );

    // 删除按钮：整块消失。
    let buttons = crate::ui::prompt::template_buttons(&state.prompt, area);
    let delete = buttons
        .iter()
        .find(|hit| hit.button == crate::ui::prompt::TemplateBlockButton::Delete)
        .copied()
        .expect("delete button");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            delete.rect.x + 1,
            delete.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "");
}

#[test]
fn mouse_supple_copy_button_writes_clipboard_toast() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = 80;
    state.prompt.resize(78, 16);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText(
            "&&& x --start\n+++ supplement --start\n{title: Review}\n\n- keep me\n+++ supplement --end\n&&& x --end"
                .into(),
        ),
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 220, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let copy = crate::ui::prompt::block_buttons(&state.prompt, area)
        .into_iter()
        .find(|hit| hit.button == crate::ui::prompt::BlockButton::Copy)
        .expect("supple copy button visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            copy.rect.x,
            copy.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(dirty);
    assert!(state.toast.is_some(), "复制按钮弹出剪贴板反馈");
    assert_eq!(
        state.toast.as_ref().map(|toast| toast.title.as_deref()),
        Some(Some(crate::ui::text::TOAST_CLIPBOARD_TITLE)),
        "复制反馈沿用剪贴板 toast"
    );
}

#[test]
fn mouse_supple_fold_button_toggles_without_editing_text() {
    let config = Config::default();
    let mut state = AppState::demo();
    state.prompt_width = 80;
    state.prompt.resize(78, 16);
    crate::app::update::focus_prompt(&mut state);
    crate::app::update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertText(
            "&&& x --start\n+++ supplement --start\n{title: Review}\n\n- keep me\n+++ supplement --end\n&&& x --end"
                .into(),
        ),
    );
    state.prompt.mark_saved();
    let geo = geometry(&state, &config, Rect::new(0, 0, 220, 30));
    let area = crate::layout::prompt_text_rect(geo.view.prompt);
    let text = state.prompt.text().to_string();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    for folded in [false, true] {
        let fold = crate::ui::prompt::block_buttons(&state.prompt, area)
            .into_iter()
            .find(|hit| hit.button == crate::ui::prompt::BlockButton::Fold)
            .expect("supple fold button visible");
        assert_eq!(fold.folded, !folded, "按钮字形跟随折叠态");
        handle_terminal_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                fold.rect.x,
                fold.rect.y,
            ),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut dirty,
        );
        assert_eq!(state.prompt.supple_folded_at(1), folded);
        assert_eq!(state.prompt.text(), text, "折叠不修改正文");
        assert!(state.prompt.is_saved(), "折叠不置脏");
    }
}

/// 给当前焦点窗格挂 Agent 快照；返回窗格标识。
fn mount_agent(
    state: &mut AppState,
    kind: crate::detect::AgentKind,
    agent_state: crate::detect::AgentState,
) -> PaneId {
    let pane = state.active_tab().layout.focus();
    update::update_pane_agent(
        state,
        pane,
        Some(crate::detect::PaneAgentSnapshot {
            kind,
            state: agent_state,
            title: None,
        }),
    );
    pane
}

/// 追加 `count` 个各带一个 Agent 的标签。
fn create_agent_tabs(state: &mut AppState, count: usize) {
    use crate::detect::{AgentKind, AgentState};
    for _ in 0..count {
        update::create_tab(state);
        mount_agent(state, AgentKind::Codex, AgentState::Working);
    }
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
        &mut dirty,
    );

    assert!(dirty);
    assert_eq!(state.active_workspace().active_tab, 0);
    assert_eq!(state.active_tab().layout.focus(), first);
    assert_eq!(
        state.active_tab().pane(first).map(|pane| pane.view),
        Some(PaneView::Terminal)
    );
    assert!(!state.prompt_focused);
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
        &mut dirty,
    );
    assert_eq!(state.agent_scroll, 1);
    assert_eq!(state.agent_hover, Some(1), "hover follows the scrolled row");
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
fn rename_overlay_inside_click_keeps_open_close_button_and_outside_dismiss() {
    let screen = Rect::new(0, 0, 80, 24);
    let mut state = AppState::demo();
    let rename = || {
        Some(Overlay::Rename(crate::app::overlay::Rename {
            target: crate::app::overlay::RenameTarget::Workspace(0),
            input: crate::app::overlay::TextInput::new("name"),
        }))
    };
    state.overlay = rename();
    let shell = ui::overlay::rename_shell(screen).expect("shell");
    // 点击内部内容（输入行空白）：保持打开。
    handle_overlay_click(&mut state, screen, shell.inner.x + 1, shell.inner.y);
    assert!(state.overlay.is_some(), "点击内部内容不关闭");
    // 右上角 [✕]：关闭。
    let close = ui::widgets::modal::close_button_rect(shell.area).expect("close button");
    handle_overlay_click(&mut state, screen, close.x, close.y);
    assert!(state.overlay.is_none(), "[✕] 关闭");
    // 点击浮层之外：关闭。
    state.overlay = rename();
    handle_overlay_click(&mut state, screen, 0, 0);
    assert!(state.overlay.is_none(), "点击浮层之外关闭");
}

#[test]
fn trust_overlay_inside_click_keeps_open_and_close_button_refuses() {
    let screen = Rect::new(0, 0, 80, 24);
    let mut state = AppState::demo();
    let untrusted = PathBuf::from("/tmp/untrusted-close-test");
    state.overlay = Some(Overlay::TrustWorkspace(crate::app::trust::TrustWorkspace {
        path: untrusted.clone(),
        trigger: crate::app::trust::TrustTrigger::CreateWorkspace { cwd: untrusted },
    }));
    let shell = ui::overlay::trust_shell(screen).expect("shell");
    // 点击问题文本（内部内容）：保持打开且不授予信任。
    handle_overlay_click(&mut state, screen, shell.inner.x + 1, shell.inner.y);
    assert!(
        matches!(state.overlay, Some(Overlay::TrustWorkspace(_))),
        "内部点击不关闭"
    );
    assert!(state.effects.is_empty(), "未登记信任效应");
    // 右上角 [✕]：拒绝信任并关闭。
    let close = ui::widgets::modal::close_button_rect(shell.area).expect("close button");
    handle_overlay_click(&mut state, screen, close.x, close.y);
    assert!(state.overlay.is_none(), "[✕] 关闭");
    assert!(state.effects.is_empty(), "拒绝不登记信任效应");
}

#[test]
fn toast_inside_click_keeps_close_button_and_outside_dismiss() {
    let mut state = demo_terminal();
    let show = |state: &mut AppState| {
        update::show_toast(
            state,
            Toast::new(ToastKind::Default, "toast body", None, Instant::now()),
        );
    };
    show(&mut state);
    let (config, geo) = toolbar_geometry(&mut state);
    let area =
        ui::toast::rect(&state, &geo.view, &geo.rects, geo.screen, &config).expect("toast visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点击 toast 内部内容：保持显示。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            area.x + 1,
            area.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.toast.is_some(), "点击 toast 内容不关闭");

    // 点击右上角 [✕]：关闭。
    let close = ui::widgets::modal::close_button_rect(area).expect("close button");
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), close.x, close.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.toast.is_none(), "[✕] 关闭 toast");

    // 点击 toast 之外：关闭 toast 并继续处理点击。
    show(&mut state);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 0, 0),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.toast.is_none(), "点击 toast 之外关闭");
}

// ===== 粘贴入口与结果路由 =====

/// 独立临时目录（粘贴测试文件隔离）。
fn paste_temp_dir(label: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("lx-tui-event-paste-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

#[test]
fn terminal_paste_valid_paths_open_proposal_and_plain_text_inserts() {
    let dir = paste_temp_dir("terminal");
    let file = dir.join("note.txt");
    std::fs::write(&file, b"note").expect("file");
    let mut state = AppState::demo();
    state.prompt_focused = true;
    paste::handle_terminal_paste(&mut state, file.display().to_string());
    let panel = state.prompt.paste_panel().expect("proposal open");
    assert_eq!(panel.candidates.len(), 1);
    assert_eq!(panel.candidates[0].kind, PasteCandidateKind::File);
    // 新粘贴先取消旧提案；普通文本仍同步插入。
    paste::handle_terminal_paste(&mut state, "/api/users".to_string());
    assert!(state.prompt.paste_panel().is_none());
    assert_eq!(state.prompt.text(), "/api/users");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn terminal_empty_paste_is_ignored() {
    let mut state = AppState::demo();
    state.prompt_focused = true;
    state.prompt.insert_str("keep ");
    state.prompt.paste_cancel();
    // 终端自身粘贴只承载位图时送来空内容：没有正文可插入，静默忽略。
    paste::handle_terminal_paste(&mut state, String::new());
    assert_eq!(state.prompt.text(), "keep ");
    assert!(state.prompt.paste_panel().is_none());
}

#[test]
fn terminal_paste_missing_path_falls_back_to_text() {
    let mut state = AppState::demo();
    state.prompt_focused = true;
    let missing = std::env::temp_dir().join("lx-tui-definitely-missing-paste.txt");
    let _ = std::fs::remove_file(&missing);
    paste::handle_terminal_paste(&mut state, missing.display().to_string());
    assert!(state.prompt.paste_panel().is_none());
    assert_eq!(state.prompt.text(), missing.display().to_string());
}

#[test]
fn terminal_paste_plain_text_does_not_arm_completion_panel() {
    let mut state = AppState::demo();
    state.prompt_focused = true;
    state
        .prompt
        .insert_str("see @[refer-file](/tmp/template.md) then ");
    // 不存在且带 shell 转义的路径：按普通文本插入，且尾部 `a.md` 看起来像触发词。
    let pasted = "/missing/ai_agent_api\\(1\\).md a.md".to_string();
    paste::handle_terminal_paste(&mut state, pasted.clone());
    assert!(
        state.prompt.text().ends_with(&pasted),
        "原文插入：{:?}",
        state.prompt.text()
    );
    assert!(state.prompt.refer_panel().is_none(), "粘贴不重启补全面板");
    let before = state.prompt.text().to_string();
    // 紧跟的回车不得把粘贴文本尾部替换成候选原文。
    crate::app::update::apply_editor(&mut state, crate::app::actions::EditorCommand::Newline);
    let text = state.prompt.text();
    assert!(text.starts_with(&before), "只追加换行：{text:?}");
    assert_eq!(
        text.matches("@[refer-file]").count(),
        1,
        "未插入额外引用：{text:?}"
    );
}

#[test]
fn terminal_paste_shell_escaped_path_builds_proposal_from_real_file() {
    let dir = paste_temp_dir("escaped");
    let file = dir.join("ai_agent_api(1).md");
    std::fs::write(&file, b"x").expect("file");
    let escaped = format!("{}/ai_agent_api\\(1\\).md", dir.display());
    let mut state = AppState::demo();
    state.prompt_focused = true;
    paste::handle_terminal_paste(&mut state, escaped.clone());
    let panel = state.prompt.paste_panel().expect("转义路径打开提案");
    assert_eq!(panel.candidates[0].path, file, "候选用去转义后的真实路径");
    assert!(
        state
            .prompt
            .paste_confirm(crate::app::prompt::PasteAction::Reference)
    );
    assert_eq!(
        state.prompt.text(),
        format!("@[refer-file]({}/ai_agent_api\\(1\\).md) ", dir.display()),
        "引用按语法转义括号"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ===== 分页鼠标命中 =====

/// 预置一个两页条目并进入编辑视图的 demo 状态。
fn paged_prompt_state() -> AppState {
    use crate::app::prompt::{PromptItem, PromptPage};
    let mut state = demo_terminal();
    let mut item = PromptItem::single(7, None, "paged", "alpha");
    item.pages.push(PromptPage::saved(70, "Page 2", "beta"));
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![item]);
    assert!(crate::app::update::edit_prompt_item(&mut state, 0, 7));
    state
}

#[test]
fn mouse_click_prompt_page_arrows_switch_and_counter_opens_menu() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let border = crate::layout::prompt_border_rect(geo.view.prompt).expect("border row");
    // 第一页左箭头置灰：点击不切页、不移动正文光标。
    state.prompt.set_cursor_offset_for_test(0);
    let controls = ui::prompt::border_page_controls(border, "1/2").expect("controls");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            controls.prev.x,
            controls.prev.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "alpha", "边界箭头不动作");
    assert_eq!(state.prompt.cursor(), 0, "控件点击不移动正文光标");
    // 右箭头：切到第二页。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            controls.next.x,
            controls.next.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "beta");
    assert!(state.prompt_focused);
    // 计数：打开页码菜单。
    let controls = ui::prompt::border_page_controls(border, "2/2").expect("controls");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            controls.counter.x,
            controls.counter.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(matches!(state.overlay, Some(Overlay::PageMenu(_))));
    // Esc 关闭菜单且不穿透编辑器。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.overlay.is_none());
    assert_eq!(state.prompt.text(), "beta");
}

#[test]
fn mouse_click_prompt_page_name_starts_rename_and_keys_commit() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let border = crate::layout::prompt_border_rect(geo.view.prompt).expect("border row");
    // 顶边框 `╭─ paged · Page 1 `：点击页名起始后两列。
    let name_col = border.x + 2 + " paged ".len() as u16 + 2;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), name_col, border.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        crate::app::update::page_rename_active(&state),
        "点击页名进入重命名"
    );
    assert_eq!(state.prompt.text(), "alpha", "重命名不改正文");
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(
        state.workspaces[0]
            .prompt_panel
            .page_rename()
            .expect("rename active")
            .input
            .text(),
        "Page 1X",
        "字符输入进入页名"
    );
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!crate::app::update::page_rename_active(&state));
    let item = state.workspaces[0].prompt_panel.tree.item(7).expect("item");
    assert_eq!(item.pages[0].name, "Page 1X");
}

#[test]
fn mouse_click_prompt_padding_only_focuses_without_moving_cursor() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    state.prompt.set_text("short");
    state.prompt.set_cursor_offset_for_test(0);
    state.prompt_focused = false;
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let text_area = crate::layout::prompt_text_rect(geo.view.prompt);
    assert!(text_area.height >= 2, "测试视口需能容纳留白");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            text_area.x + 2,
            text_area.bottom() - 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(state.prompt_focused, "点击留白聚焦编辑器");
    assert_eq!(
        state.prompt.cursor(),
        0,
        "点击留白不移动光标到不存在的文本行"
    );
}

#[test]
fn lx_entry_click_switches_selection() {
    use crate::app::state::LxEntry;
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 160, 40));
    let pane = state.active_tab().layout.focus();
    let area = pane_inner(&geo, pane);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let mut target = None;
    for row in area.y..area.bottom() {
        for col in area.x..area.right() {
            if crate::ui::main_content::lx::entry_hit(area, lx, col, row)
                == Some(LxEntry::Automation)
            {
                target = Some((col, row));
            }
        }
    }
    let (col, row) = target.expect("assistant entry button");
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), col, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.entry,
        LxEntry::Automation,
        "点击入口切换选中"
    );
    assert!(state.lx_selection.is_none(), "入口点击不启动选区");
}

#[test]
fn event_input_paste_overlay_and_prompt_routing() {
    let mut state = demo_terminal();
    let config = Config::default();
    let geo = geometry(&state, &config, Rect::new(0, 0, 80, 24));
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 1. 浮层打开时：粘贴被吞掉，dirty 保持 false。
    state.overlay = Some(Overlay::ConfirmClose(crate::app::overlay::ConfirmClose {
        target: crate::app::overlay::OverlayTarget::Workspace(0),
    }));

    handle_terminal_event(
        TerminalEvent::Paste("echo test".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(!dirty, "浮层激活时粘贴被吞掉");

    // 2. prompt 聚焦时：粘贴进入 prompt 编辑器，dirty 置为 true。
    state.overlay = None;
    state.prompt_focused = true;
    dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("hello prompt".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(dirty, "prompt 聚焦时粘贴使画面变脏");
    assert!(state.prompt.text().contains("hello prompt"));
}

// ===== lx `@` 提及面板鼠标路由 =====

/// 为焦点 lx 窗格准备提及面板：设根、键入 @、写入缓存。
fn seed_lx_mention(
    state: &mut AppState,
    query: &str,
    entries: Vec<crate::app::markdown::MentionEntry>,
) {
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.cwd = Some(std::path::PathBuf::from("/work/demo"));
    }
    update::sync_lx_mention_root(state, pane);
    for ch in query.chars() {
        update::apply_lx_input(state, crate::app::actions::LxInput::Char(ch));
    }
    let (generation, root) = state
        .pane_mut_anywhere(pane)
        .and_then(|p| p.lx.take_mention_scan_request())
        .expect("scan request");
    update::apply_lx_mention_entries(state, pane, generation, &root, entries);
}

#[test]
fn lx_mention_panel_click_accepts_candidate_before_page_selection() {
    let config = Config::default();
    let mut state = AppState::demo();
    seed_lx_mention(
        &mut state,
        "@ma",
        vec![crate::app::markdown::MentionEntry {
            path: "main.rs".into(),
            is_directory: false,
        }],
    );
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let pane = state.active_tab().layout.focus();
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .copied()
        .expect("pane rect");
    let inner = crate::layout::pane_inner_rect(rect);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let panel = ui::main_content::lx::mention_layout(inner, lx).expect("mention panel");
    let (x, y) = (panel.content.x, panel.content.y);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), x, y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "@main.rs ",
        "点击候选按 Enter 规则采纳"
    );
    assert!(state.lx_selection.is_none(), "命中提及面板不启动整页选区");
}

#[test]
fn lx_mention_wheel_scrolls_panel_not_input() {
    let config = Config::default();
    let mut state = AppState::demo();
    let entries: Vec<crate::app::markdown::MentionEntry> = (0..10)
        .map(|i| crate::app::markdown::MentionEntry {
            path: format!("f{i}.rs"),
            is_directory: false,
        })
        .collect();
    seed_lx_mention(&mut state, "@", entries);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let pane = state.active_tab().layout.focus();
    let (_, rect) = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .copied()
        .expect("pane rect");
    let inner = crate::layout::pane_inner_rect(rect);
    let input_scroll_before = state.pane_anywhere(pane).expect("pane").lx.scroll();
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let panel = ui::main_content::lx::mention_layout(inner, lx).expect("mention panel");
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, panel.content.x, panel.content.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    let target = state.pane_anywhere(pane).expect("pane").lx.mention();
    assert_eq!(
        target.panel().expect("panel").viewport(),
        Some(1),
        "面板内滚轮滚动候选视口"
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.scroll(),
        input_scroll_before,
        "候选面板内滚轮不滚动输入框"
    );
}

/// 回归：lx 卡片「按下 → 松开（未拖拽）」必须真的切换折叠。
///
/// 真实 bug：`begin_lx_selection` 内部先 `clear_selection`（会把 `lx_card_press` 清空），
/// 早期代码先记 press 再 begin，等于记了个寂寞——每次松开都被当成拖拽，卡片永远展不开。
#[test]
fn lx_card_click_toggles_expansion_on_release() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        let mut chat = crate::app::state::LxChat::new();
        chat.messages.push(crate::app::state::ChatMessageView {
            sequence: 0,
            id: "m-param".into(),
            role: "assistant".into(),
            status: "complete".into(),
            turn_index: 0,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Tool {
                position: 0,
                call: crate::app::state::ToolCallView {
                    call_id: "call-param".into(),
                    name: "read".into(),
                    position: 0,
                    arguments_json: String::new(),
                    status: crate::app::state::ToolStatus::Completed,
                    summary: Some(Box::new(crate::app::state::ToolSummary {
                        // 足够长：折叠 2 行装不下，展开才多行。
                        path: Some(
                            "src/some/really/deeply/nested/path/that/keeps/going/mod.rs".repeat(3),
                        ),
                        range: Some((1, 9)),
                        ..Default::default()
                    })),
                    error: None,
                    result_json: None,
                    detail: None,
                },
            }],
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
        });
        chat.follow_bottom = true;
        target.lx.chat = chat;
    }
    let inner = pane_inner(&geo, pane);
    let card = {
        let target = state.pane_anywhere(pane).expect("pane");
        crate::ui::main_content::lx::history_view(inner, &target.lx)
            .and_then(|view| {
                crate::ui::main_content::lx::history::layout(view.body, &target.lx)
                    .cards
                    .into_iter()
                    .next()
            })
            .expect("可展开卡片")
    };
    let (column, row) = (inner.x + 3, card.card.y);
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("m-param", "call-param"),
        "初始折叠"
    );
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        state.lx_card_press.is_some(),
        "按下必须记录「可能是一次点击」（不能被 begin_lx_selection 清掉）"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut dirty,
    );
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .chat
            .detail_is_expanded("m-param", "call-param"),
        "未拖拽的点击必须展开卡片"
    );
}

/// 粘贴也是编辑：必须让双 Esc 的待命失效，避免带草稿选中后回填覆盖。
#[test]
fn lx_paste_cancels_rewind_arming() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.chat.session_id = Some("sess".into());
        target.lx.rewind.armed_at = Some(std::time::Instant::now());
    }
    assert!(paste::handle_lx_paste(&mut state, "pasted".into()));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.rewind.armed_at.is_none(), "粘贴必须清除撤销待命");
    assert_eq!(target.lx.input, "pasted");
}

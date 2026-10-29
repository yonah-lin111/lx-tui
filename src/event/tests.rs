use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::overlay::{ConfirmSwitchCwd, Overlay};
use crate::app::prompt::{PasteCandidateKind, PromptMode, PromptRowKind};
use crate::app::state::{PaneView, RightTab};
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_sidebar, "press on boundary starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary + 8, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.sidebar_width, boundary + 9);

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), boundary + 8, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_prompt, "press on boundary starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary - 5, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.resizing_prompt, "press on left divider starts resize");

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), boundary + 5, 5),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_drag, Some(0));

    handle_terminal_event(
        mouse(MouseEventKind::Drag(MouseButton::Left), 2, list_y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.workspace_drag, Some(2));

    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), 2, list_y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.scroll() > 0, "track click jumps down");
    assert!(
        !state.prompt_focused(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.workspaces[0].prompt_panel.scroll > 0, "轨道点击跳转");
    assert!(!state.prompt_focused(), "滚动条按下不抢焦点");
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
            &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.terminal_selection, Some(pane));

    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x + 2, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
            &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target
            .lx
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                sequence: 0,
                id: "m0".into(),
                role: "user".into(),
                status: "complete".into(),
                turn_index: 0,
                kind: "chat".into(),
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Text,
                        content: "hello history".into(),
                        provider_metadata_json: None,
                        started_at_ms: None,
                        ended_at_ms: None,
                        completed_at_ms: None,
                    },
                }],
                model_label: None,
                error_code: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                duration_ms: None,
                reports: Vec::new(),
                attachments: Vec::new(),
            });
    }
    crate::app::update::focus_prompt(&mut state);
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!state.prompt_focused());
    assert_eq!(state.active_tab().layout.focus(), pane);
    assert!(state.terminal_selection.is_none(), "lx 视图不启动终端选区");
    assert!(state.selection.is_none(), "lx 视图不启动 prompt 选区");
    assert!(
        state.lx_selection.is_some(),
        "lx 视图主消息列表按下开始内容选区"
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
        &mut ui::RenderCache::default(),
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
            &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
            &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
            &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt_pinned, None);
    assert!(dirty);
}

#[test]
fn mouse_click_prompt_block_focus_roundtrips_without_text_change() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let text = "intro\n&&& a --start\n{title: A}\nbody-a\n&&& a --end\noutro";
    state.prompt.set_text(text);
    let cursor = state.prompt.text().find("body-a").expect("body") + 1;
    state.prompt.set_cursor_offset_for_test(cursor);

    let block_focus = toolbar_button(&geo, layout::PromptToolbarButton::BlockFocus);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            block_focus.x,
            block_focus.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.block_focus_active(), "点击 [⊞] 进入块模式");
    assert_eq!(state.prompt.text(), text, "进入模式不改正文");
    assert!(dirty);

    let exit = toolbar_button(&geo, layout::PromptToolbarButton::BlockFocus);
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), exit.x, exit.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!state.prompt.block_focus_active(), "再点 [⊡] 退出块模式");
    assert_eq!(state.prompt.text(), text, "退出模式不改正文");
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "");
    assert!(state.prompt.can_redo());
    assert!(!state.prompt_focused(), "工具栏按钮不抢焦点");
    assert!(dirty);

    let redo = toolbar_button(&geo, layout::PromptToolbarButton::Redo);
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), redo.x, redo.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
    state.set_focus(crate::app::state::FocusedContainer::Prompt);

    // 默认绑定临时提示词：Ctrl+S 提交缓冲并打开保存选择器。
    state.prompt.insert_str("draft body");
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    state.prompt.insert_str("+");
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    let press =
        |state: &mut AppState, sessions: &mut HashMap<PaneId, PtySession>, dirty: &mut bool| {
            handle_terminal_event(
                TerminalEvent::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
                state,
                sessions,
                &geo,
                &config,
                &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            select_all.right() + 2,
            bar.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(!state.prompt_focused(), "表头空白不聚焦文本视口");
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_focused());
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

/// 左键单击同一坐标：一次按下紧接一次松开（点击语义在松开时生效）。
fn prompt_tree_click_once(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    point: (u16, u16),
) {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_terminal_event(
            mouse(kind, point.0, point.1),
            state,
            &mut sessions,
            geo,
            config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
}

/// 左键双击同一坐标：连续两次「按下 + 松开」。
fn prompt_tree_double_click(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    point: (u16, u16),
) {
    for _ in 0..2 {
        prompt_tree_click_once(state, geo, config, point);
    }
}

/// 一次按下 → 跨行拖动 → 松开，返回松开后的 `dirty`。
fn prompt_tree_drag(
    state: &mut AppState,
    geo: &Geometry,
    config: &Config,
    from: (u16, u16),
    to: (u16, u16),
) {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    for (kind, point) in [
        (MouseEventKind::Down(MouseButton::Left), from),
        (MouseEventKind::Drag(MouseButton::Left), to),
        (MouseEventKind::Up(MouseButton::Left), to),
    ] {
        handle_terminal_event(
            mouse(kind, point.0, point.1),
            state,
            &mut sessions,
            geo,
            config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
}

#[test]
fn mouse_drag_prompt_item_into_folder_queues_move_effect() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let folder_row = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        vec![PromptItem::single(7, None, "item", "item body")],
        PromptRowKind::Item,
    );
    let area = layout::prompt_text_rect(geo.view.prompt);
    let folder_index = state.workspaces[0]
        .prompt_panel
        .rows()
        .iter()
        .position(|row| row.kind == PromptRowKind::Folder)
        .expect("folder row");
    let folder_point = (area.x + 1, area.y + 2 + folder_index as u16);
    prompt_tree_drag(&mut state, &geo, &config, folder_row, folder_point);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::MovePrompt {
                id: 7,
                folder_id: Some(1),
                ..
            }
        )),
        "拖条目进文件夹登记移动效应: {:?}",
        state.effects
    );
}

/// 拖动中滚轮不得改视口：落点与跨行阈值都按绝对行号换算，滚动会让指针原地"跨行"，
/// 松手时把条目静默移进此时指针所在行的容器。
#[test]
fn mouse_wheel_during_prompt_drag_keeps_viewport_and_state() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    // 文件夹内一个条目 + 一批根层条目，使可见行超过视口（panel.scroll 可动）。
    let mut items = vec![PromptItem::single(7, Some(1), "inside", "inside body")];
    items.extend((0..40).map(|n| PromptItem::single(100 + n, None, format!("item {n}"), "body")));
    let item_point = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        items,
        PromptRowKind::Item,
    );
    let visible = update::prompt_tree_visible_rows(&state);
    assert!(
        state.prompt_owner().prompt_panel.rows().len() > visible,
        "夹具：列表必须长于视口才能滚动"
    );
    assert_eq!(state.prompt_owner().prompt_panel.scroll, 0);

    let mut sessions = HashMap::new();
    let mut dirty = false;
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::ScrollDown,
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_terminal_event(
            mouse(kind, item_point.0, item_point.1),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }

    assert_eq!(
        state.prompt_owner().prompt_panel.scroll,
        0,
        "拖动中滚轮不改视口"
    );
    assert!(state.prompt_drag.is_none(), "松开清理拖拽现场");
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::MovePrompt { .. }
                | crate::app::effects::Effect::MoveFolder { .. }
        )),
        "指针未动，不得登记移动效应: {:?}",
        state.effects
    );
}

#[test]
fn mouse_drag_prompt_row_does_not_open_editor() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let item_point = prompt_tree_click(
        &mut state,
        &geo,
        vec![PromptFolder {
            id: 1,
            parent_id: None,
            name: "folder".into(),
            collapsed: false,
        }],
        vec![
            PromptItem::single(7, None, "item", "item body"),
            PromptItem::single(8, None, "other", "other body"),
        ],
        PromptRowKind::Item,
    );
    let area = layout::prompt_text_rect(geo.view.prompt);
    let rows = state.workspaces[0].prompt_panel.rows().len();
    let blank = (area.x + 1, area.y + 2 + rows as u16);
    prompt_tree_drag(&mut state, &geo, &config, item_point, blank);
    assert_eq!(
        state.workspaces[0].prompt_panel.mode,
        PromptMode::Tree,
        "拖动结束不打开编辑器"
    );
}

/// 拖动中指针移出窗口会丢掉 Up：下一次无关按下必须先清场，否则它的 Up 会按旧行号
/// 消费残留现场（例如点工具栏却折叠了树里的文件夹）。
#[test]
fn mouse_down_discards_stale_prompt_drag() {
    use crate::app::prompt::{PromptFolder, PromptItem};
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let folder_point = prompt_tree_click(
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
    let mut sessions = HashMap::new();
    let mut dirty = false;
    // 只按下：现场登记、文件夹尚未折叠。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            folder_point.0,
            folder_point.1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_drag.is_some(), "按下登记现场");
    // 模拟 Up 丢失后的一次全新按下（别处）：旧现场必须被丢弃。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), 1, 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_drag.is_none(), "新按下丢弃残留现场");
    assert!(
        !state.workspaces[0]
            .prompt_panel
            .tree
            .folder(1)
            .is_some_and(|folder| folder.collapsed),
        "残留现场不得在无关松开时折叠文件夹"
    );
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
    assert!(state.prompt_focused());
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty);
    assert!(state.toast.is_some(), "复制按钮弹出剪贴板反馈");
    assert_eq!(
        state.toast.as_ref().map(|toast| toast.title.as_deref()),
        Some(Some(crate::ui::text::TOAST_CLIPBOARD_TITLE)),
        "复制反馈沿用剪贴板 toast"
    );
    assert_eq!(
        state.toast.as_ref().map(|toast| toast.message.as_str()),
        Some("Copied 9 chars"),
        "复制反馈显示复制的字符数（`- keep me` 共 9 字符）"
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
            &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
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
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
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
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    let missing = std::env::temp_dir().join("lx-tui-definitely-missing-paste.txt");
    let _ = std::fs::remove_file(&missing);
    paste::handle_terminal_paste(&mut state, missing.display().to_string());
    assert!(state.prompt.paste_panel().is_none());
    assert_eq!(state.prompt.text(), missing.display().to_string());
}

#[test]
fn terminal_paste_plain_text_does_not_arm_completion_panel() {
    let mut state = AppState::demo();
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
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
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
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
    let title = ui::prompt_frame::title_row(geo.view.prompt).expect("title row");
    assert_eq!(
        title.y,
        crate::layout::pane_inner_rect(geo.view.prompt).y + 1,
        "页控件已下移到标题行（工具栏下一行），不再是容器顶边框"
    );
    // 第一页左箭头置灰：点击不切页、不移动正文光标。
    state.prompt.set_cursor_offset_for_test(0);
    let controls = ui::prompt::page_controls(title, "1/2").expect("controls");
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(state.prompt.text(), "beta");
    assert!(state.prompt_focused());
    // 计数：打开页码菜单。
    let controls = ui::prompt::page_controls(title, "2/2").expect("controls");
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
    let title = ui::prompt_frame::title_row(geo.view.prompt).expect("title row");
    assert_eq!(
        title.y,
        crate::layout::pane_inner_rect(geo.view.prompt).y + 1,
        "页名已下移到标题行（工具栏下一行），不再是容器顶边框"
    );
    // 标题行 `├─ paged · Page 1 `：点击页名起始后两列。
    let name_col = title.x + 2 + " paged ".len() as u16 + 2;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), name_col, title.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
    state.set_focus(crate::app::state::FocusedContainer::Panes);
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt_focused(), "点击留白聚焦编辑器");
    assert_eq!(
        state.prompt.cursor(),
        0,
        "点击留白不移动光标到不存在的文本行"
    );
}

// ===== 右侧统一容器 tab 事件层 =====

#[test]
fn worktree_candidates_do_not_resurrect_mention_panel_on_todo() {
    // `apply_worktree_entries` 结尾无条件 `mention.refresh`，而 `clear()` 只 bump
    // generation、不清 cache。若 Todo 期间的回调仍被处理，刚收起的提及面板会被
    // 重新置 Some —— 一个看不见、却能吞滚轮的面板。
    use crate::app::markdown::{MentionEntry, WorktreeCandidate};
    use crate::db::Database;
    use crate::db::service::Persistence;
    use crate::event::chat::ChatStreams;
    use std::collections::HashSet;

    fn candidates() -> Vec<WorktreeCandidate> {
        vec![WorktreeCandidate {
            branch: "main".into(),
            path: PathBuf::from("/tmp/lx-tui-wt"),
            is_main: true,
        }]
    }

    let mut state = demo_terminal();
    state.prompt.resize(40, 8);
    update::focus_prompt(&mut state);
    // 制造提及面板：`@` 触发 + 扫描结果入缓存。
    update::apply_editor(
        &mut state,
        crate::app::actions::EditorCommand::InsertChar('@'),
    );
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested");
    let prompt = state.prompt.id();
    update::apply_mention_entries(
        &mut state,
        prompt,
        generation,
        vec![MentionEntry {
            path: "app.rs".into(),
            is_directory: false,
        }],
    );
    // 回调只在工作区 git 元数据的仓库根匹配时才被处理（`targeted`）：
    // 不造出来，下面的断言就只是"回调被丢弃"，测不到闸门。
    let prompt_workspace = state.prompt_workspace();
    state.workspaces[prompt_workspace].git = Some(crate::app::state::WorkspaceGit {
        repo_root: PathBuf::from("/tmp/lx-tui-wt"),
        checkout_path: PathBuf::from("/tmp/lx-tui-wt"),
        is_linked: false,
        branch: Some("main".into()),
        main_branch: Some("main".into()),
    });
    // 先建立 worktree 候选，再补一次扫描缓存（候选取值变化会清 cache）。
    state.prompt.apply_worktree_entries(candidates());
    let (generation, _) = state
        .prompt
        .take_mention_scan_request()
        .expect("scan requested again");
    update::apply_mention_entries(
        &mut state,
        prompt,
        generation,
        vec![MentionEntry {
            path: "app.rs".into(),
            is_directory: false,
        }],
    );
    assert!(state.prompt.mention().is_some(), "前提：提及面板可见");

    assert!(update::select_right_tab(&mut state, RightTab::Todo));
    assert!(state.prompt.mention().is_none(), "切走即收起");

    // 相同候选到达（cache 不失效）——这正是复活的那条路径。
    // 库路径必须**每次唯一**：固定路径会被并发测试与其他 worktree 反复打开，
    // 一旦某个分支把 `SCHEMA_VERSION` 推进到更高值，这个文件就被写成高版本库，
    // 低版本二进制再打开会直接 `UnsupportedSchema`（实测：v15 分支跑过之后，
    // 另一侧再跑这条用例必挂）。用完即删。
    let db_path = std::env::temp_dir().join(format!(
        "lx-tui-wt-gate-{}-{:?}.db",
        std::process::id(),
        std::thread::current().id()
    ));
    let untrusted = Database::open(&db_path).expect("open side db");
    let persistence = Persistence::with_database(untrusted);
    let mut streams = ChatStreams::default();
    let mut inflight = HashSet::new();
    handle_app_event(
        AppEvent::PromptWorktreeListed {
            repo_root: PathBuf::from("/tmp/lx-tui-wt"),
            candidates: candidates(),
            failed: false,
        },
        &mut state,
        &mut HashMap::new(),
        &mut inflight,
        &mut streams,
        &persistence,
    );

    assert!(
        state.prompt.mention().is_none(),
        "Todo 期间到达的 worktree 回调不得复活隐藏的提及面板"
    );
    drop(persistence);
    let _ = std::fs::remove_file(&db_path);
}

/// 容器顶边框上指定 tab 的矩形（按几何取，避免硬编码列）。
fn right_tab_rect(geo: &Geometry, tab: RightTab) -> Rect {
    ui::right_panel::tab_rects(geo.view.prompt)
        .into_iter()
        .find_map(|(candidate, rect)| (candidate == tab).then_some(rect))
        .expect("right tab visible")
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
fn mouse_click_current_tab_commits_page_rename() {
    // 设计：tab 与工具栏按钮同属控件，点一下即离开重命名输入态。
    // 点**当前** tab 时 `select_right_tab` 会因同 tab 早退，提交必须由 press 自己完成。
    let config = Config::default();
    let mut state = paged_prompt_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    assert!(crate::app::update::begin_page_rename(&mut state));
    crate::app::update::apply_page_rename_key(&mut state, crate::app::actions::OverlayKey::Clear);
    for ch in "Intro".chars() {
        crate::app::update::apply_page_rename_key(
            &mut state,
            crate::app::actions::OverlayKey::Char(ch),
        );
    }
    assert!(
        crate::app::update::page_rename_active(&state),
        "前提：重命名中"
    );
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let prompt_tab = right_tab_rect(&geo, RightTab::Prompt);
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            prompt_tab.x,
            prompt_tab.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.right_tab, RightTab::Prompt, "点的就是当前 tab");
    assert!(
        !crate::app::update::page_rename_active(&state),
        "提交并退出输入态"
    );
    let item = state.workspaces[0].prompt_panel.tree.item(7).expect("item");
    assert_eq!(item.pages[0].name, "Intro", "非空页名写回");
    assert!(dirty);
}

#[test]
fn mouse_click_current_tab_focuses_panel_and_dirties() {
    // 点 tab 就是「点了右侧容器」：即使 tab 没变、也没有重命名，
    // 焦点从主区挪到右栏也是一次真实变化，必须置脏（否则边框高亮不刷新）。
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(state.panes_focused(), "前提：主区持焦点");
    let mut sessions = HashMap::new();
    let mut dirty = false;
    let prompt_tab = right_tab_rect(&geo, RightTab::Prompt);

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            prompt_tab.x,
            prompt_tab.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.right_tab, RightTab::Prompt);
    assert!(state.prompt_focused(), "点当前 tab 也要让右栏进入选中态");
    assert!(dirty, "焦点变化必须置脏");

    // 已经聚焦右栏后再点一次：这次真的没有任何变化，不该白白重绘。
    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            prompt_tab.x,
            prompt_tab.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!dirty, "同 tab 且已聚焦时不该白白重绘");
}

#[test]
fn mouse_click_todo_blank_still_focuses_container() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.prompt.insert_str("ab\ncd");
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    // 点分割线（既不是按钮也不是行）：容器进入选中态，但不动数据、不穿透。
    let divider = layout::prompt_divider_rect(geo.view.prompt).expect("divider visible");
    let cursor = state.prompt.cursor();
    let focused_pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            divider.x + 1,
            divider.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(!state.prompt_focused(), "Todo 空白处点击不聚焦编辑器");
    assert!(state.todo_focused(), "点容器任意位置即让右侧容器进入选中态");
    assert_eq!(state.prompt.cursor(), cursor, "Todo 空白处点击不动光标");
    assert_eq!(
        state.active_tab().layout.focus(),
        focused_pane,
        "不穿透到窗格、不改焦点"
    );
    assert!(!dirty, "已聚焦时点空白不产生变化");
    assert!(sessions.is_empty(), "点击不写终端");
}

#[test]
fn mouse_click_prompt_toolbar_ignored_on_todo_tab() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let mode_before: PromptMode = state.prompt_owner().prompt_panel.mode;
    assert_eq!(mode_before, PromptMode::Editor, "前提：当前是编辑视图");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 编辑视图 `[≡]` 按钮：Todo 下必须整行吞掉，绝不执行动作。
    let mode = layout::prompt_toolbar_button_rect(
        geo.view.prompt,
        layout::PromptToolbarButton::Mode,
        false,
    )
    .expect("mode button visible");
    assert_eq!(
        layout::prompt_toolbar_button_at(geo.view.prompt, mode.x, mode.y, false),
        Some(layout::PromptToolbarButton::Mode),
        "前提：该坐标确实命中工具栏按钮"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), mode.x, mode.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(
        state.prompt_owner().prompt_panel.mode,
        mode_before,
        "Todo 下提示词工具栏按钮不切换视图"
    );
    assert!(state.overlay.is_none(), "Todo 下提示词工具栏按钮不打开浮层");
    // 同一坐标现在属于 Todo 自己的 `[new]` 按钮（两个面板共用工具栏行），
    // 因此这里**会**产生副作用：命中 Todo 按钮并进入编辑。旧断言「不置脏」
    // 随 Todo 成为真实面板而作废。
    assert_eq!(
        state.todo.editing().map(|edit| edit.target.is_none()),
        Some(true),
        "点击落到 Todo 的 [new] 按钮上，进入新建编辑"
    );
    assert!(dirty, "Todo 按钮命中必须置脏");
}

#[test]
fn mouse_wheel_over_todo_scrolls_list_without_moving_selection() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    let text = layout::prompt_text_rect(geo.view.prompt);
    state.prompt.resize(text.width, text.height);
    for _ in 0..=usize::from(text.height) {
        state.prompt.insert_str("line\n");
    }
    crate::app::update::set_prompt_scroll(&mut state, 2);
    assert!(state.prompt.scroll() > 0, "前提：正文可滚动且已有偏移");
    let prompt_scroll = state.prompt.scroll();
    let tree_scroll = state.prompt_owner().prompt_panel.scroll;

    // 塞满超过一屏的 todo，保证视口可以滚动。
    let visible = crate::ui::right_panel::todo::visible_rows(geo.view.prompt);
    assert!(visible > 0, "前提：列表区可见");
    state.todo.replace(
        (0..visible + 5)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("item {index}"),
                    crate::db::todo::TodoStatus::Todo,
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect(),
    );
    state.todo.set_viewport_rows(visible);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let selected = state.todo.selected();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, text.x + 1, text.y + 1),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.todo.scroll() > 0, "Todo 下滚轮滚动列表视口");
    assert_eq!(state.todo.selected(), selected, "滚轮不动高亮");
    assert!(dirty, "视口移动必须置脏");
    assert_eq!(
        state.prompt.scroll(),
        prompt_scroll,
        "Todo 下滚轮不动 prompt 正文视口"
    );
    assert_eq!(
        state.prompt_owner().prompt_panel.scroll,
        tree_scroll,
        "Todo 下滚轮不动树视图视口"
    );
}

#[test]
fn mouse_click_todo_row_selects_and_focuses() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.todo.replace(
        (0..3)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("item {index}"),
                    crate::db::todo::TodoStatus::Todo,
                    index,
                )
            })
            .collect(),
    );
    state
        .todo
        .set_viewport_rows(crate::ui::right_panel::todo::visible_rows(geo.view.prompt));
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点第二行。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            list.x + 1,
            list.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.todo.selected(), Some(1), "点击选中该行");
    assert!(state.todo_focused(), "点击行聚焦 Todo 面板");
    assert!(!state.prompt_focused(), "与 prompt 焦点互斥");
    assert!(dirty, "选中变化置脏");
}

#[test]
fn mouse_click_todo_toolbar_button_runs_action() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let new = crate::ui::right_panel::todo::button_rect(
        geo.view.prompt,
        crate::app::tooltip::TodoButton::New,
    )
    .expect("[new] visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), new.x, new.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(
        state
            .todo
            .editing()
            .is_some_and(|edit| edit.target.is_none()),
        "[new] 进入新建编辑"
    );
    assert!(state.todo_focused(), "按钮隐式聚焦");
    assert!(dirty, "按钮动作置脏");
}

#[test]
fn mouse_double_click_todo_row_enters_edit() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.todo.replace(
        (0..3)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("item {index}"),
                    crate::db::todo::TodoStatus::Todo,
                    index,
                )
            })
            .collect(),
    );
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;

    // 第一击：选中 + 聚焦，不进编辑。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            list.x + 1,
            list.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(state.todo.selected(), Some(2));
    assert!(!state.todo.is_editing(), "单击不进入编辑");

    // 第二击（仍在判定窗口内）：进入该行的行内编辑，输入框带原文。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            list.x + 1,
            list.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let edit = state.todo.editing().expect("双击进入编辑");
    assert_eq!(
        edit.target.as_deref(),
        Some(state.todo.items()[2].id.as_str())
    );
    assert_eq!(edit.input.text(), "item 2", "编辑现场带原文");
    assert!(state.todo_focused());
}

/// 点容器顶边框的 tab 旁空白：同样让右侧容器进入选中态（不应"只有点进输入框"才算）。
#[test]
fn mouse_click_todo_border_blank_focuses_container() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "前提：切 tab 后右栏持焦点");
    // 顶边框最右端（tab 之外、折叠按钮之外的空白）。
    let border = crate::layout::prompt_border_rect(geo.view.prompt).expect("border visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            border.right() - 3,
            border.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(
        state.todo_focused(),
        "切 tab 后右栏即选中；点边框空白不改焦点"
    );
    assert!(!state.prompt_focused());
    assert!(!dirty, "已聚焦时点边框空白不产生变化");
}

#[test]
fn mouse_click_todo_stats_row_focuses_without_data_change() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    // 条目必须多到「统计行所在行」在列表下标范围内：若列表区误吞了统计块，
    // 这一击就会选中一行——用单条列表测不出这个问题。
    let rows = usize::from(crate::layout::pane_inner_rect(geo.view.prompt).height);
    state.todo.replace(
        (0..rows.saturating_add(8))
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("item {index}"),
                    crate::db::todo::TodoStatus::Todo,
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect(),
    );
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let stats = crate::ui::right_panel::todo::stats_rect(geo.view.prompt).expect("统计行可见");
    let cursor = state.prompt.cursor();
    let focused_pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            stats.x + 2,
            stats.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.todo_focused(), "点统计块时右栏保持选中");
    assert!(!state.todo.is_editing(), "统计块不是行，不进入编辑");
    assert_eq!(state.todo.selected(), Some(0), "选中行不变");
    assert_eq!(state.prompt.cursor(), cursor, "光标不动");
    assert_eq!(
        state.active_tab().layout.focus(),
        focused_pane,
        "窗格焦点不动"
    );
    assert!(!dirty, "已聚焦时点统计块不产生变化");
}

#[test]
fn todo_keys_never_reach_pty() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "切到 Todo tab 后右栏持焦点");
    let mut sessions = HashMap::new();
    let keys = [
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        // 下面三种都**不在** `todo_key` 映射表里：它们只能靠 `route` 的
        // `todo_focused` 分支吞掉。若该分支写成「未映射键回落去编码写窗格」，
        // 这三条就会变成 `Routed::Pane`，测试随即失败。
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE),
    ];

    // 路由层是唯一可能产出终端字节的地方。这里断言的是**强不变量**：
    // Todo 聚焦时任何键只能路由成 `None`（吞掉）或 `Routed::Todo`（进面板）；
    // `Routed::Pane`/`Lx`/`Editor` 全都意味着字节会流向别处。
    // （不断言「`sessions` 表为空」——那张表只会被 `write_to_pane` 查不会插，
    // 空表上的断言恒真，是空转测试。）
    for key in keys {
        let routed = crate::input::route(
            key,
            alacritty_terminal::term::TermMode::empty(),
            crate::app::state::FocusedContainer::Todo,
            None,
            false,
        );
        assert!(
            matches!(routed, None | Some(crate::input::Routed::Todo(_))),
            "{key:?} 只允许进 Todo 或被吞掉，实际 {routed:?}"
        );
    }

    // 端到端：`a` 开新建、`j`/`x` 成为文本、`Enter` 提交、`Tab` 被吞——
    // 全都留在 Todo 面板里。
    for key in keys {
        let mut dirty = false;
        handle_terminal_event(
            TerminalEvent::Key(key),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut ui::RenderCache::default(),
            &mut dirty,
        );
    }
    assert_eq!(state.todo.items()[0].text, "jx", "按键落在 Todo 编辑框");
    assert!(!state.todo.is_editing(), "Enter 已提交");
}

/// 点终端窗格要提交编辑、并释放 Todo 焦点（`focus_pane` 的两件事）。
#[test]
fn focus_pane_releases_todo_focus() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    state.todo.replace(vec![crate::db::todo::TodoRecord::new(
        "keep",
        crate::db::todo::TodoStatus::Todo,
        0,
    )]);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(crate::app::update::begin_todo_edit(
        &mut state,
        crate::app::state::TodoEditTarget::New
    ));
    assert!(crate::app::update::apply_todo_paste(&mut state, "draft"));
    let pane = state.active_tab().layout.focus();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let inner = layout::pane_inner_rect(geo.view.panes);
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(
        state.active_tab().layout.focus(),
        pane,
        "前提：点的是终端窗格"
    );
    assert!(!state.todo_focused(), "点终端窗格释放 Todo 焦点");
    assert!(!state.todo.is_editing(), "编辑中的内容已提交");
    assert_eq!(state.todo.items().len(), 2, "提交后新增一条");
    assert_eq!(state.todo.items()[1].text, "draft");
}

/// 没有可撤销内容时 `Ctrl+Z` 必须被吞掉，不能漏进终端。
#[test]
fn ctrl_z_without_undo_is_consumed_not_forwarded() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "切到 Todo tab 后右栏持焦点");
    assert!(!state.todo.can_undo(), "前提：撤销槽为空");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    let key = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert!(
        !matches!(
            crate::input::route(
                key,
                alacritty_terminal::term::TermMode::empty(),
                crate::app::state::FocusedContainer::Todo,
                None,
                false
            ),
            Some(crate::input::Routed::Pane(_))
        ),
        "Ctrl+Z 在 Todo 聚焦时绝不路由到终端"
    );
    handle_terminal_event(
        TerminalEvent::Key(key),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(!state.todo.can_undo(), "无内容可撤销");
    assert!(!dirty, "撤销槽为空时不产生状态变化");
}

#[test]
fn todo_key_escape_blurs() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "切到 Todo tab 后右栏持焦点");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(!state.todo_focused(), "Esc 释放 Todo 焦点");
    assert!(dirty);
}

#[test]
fn ctrl_q_wins_over_todo_focus() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    assert!(state.todo_focused(), "切到 Todo tab 后右栏持焦点");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.should_quit, "Ctrl+Q 始终优先");
    assert!(dirty, "退出动作置脏");
    assert!(state.todo_focused(), "Ctrl+Q 不碰 todo 焦点");
    assert!(state.todo.items().is_empty(), "Ctrl+Q 不改 todo 列表");
    assert!(!state.todo.is_editing(), "Ctrl+Q 不开编辑");
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

#[test]
fn mouse_click_prompt_collapse_button_works_on_todo_tab() {
    // 折叠按钮在容器底边框、位于"Todo 整块吞掉"分支之前；这条用例锁住顺序：
    // 一旦把吞掉分支提到折叠按钮之前，Todo 下面板就再也折不起来。
    let config = Config::default();
    let mut state = demo_terminal();
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let target = ui::controls::collapse_buttons(&geo.view, state.agents_collapsed)
        .into_iter()
        .find(|button| button.target == ui::CollapseTarget::Prompt)
        .expect("prompt collapse button visible");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            target.area.x,
            target.area.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(
        state.prompt_collapsed,
        "Todo 下点折叠按钮仍要能折叠容器（顺序不能被吞掉分支遮蔽）"
    );
    assert_eq!(state.right_tab, RightTab::Todo, "折叠不改当前面板");
    assert!(dirty);
}

#[test]
fn pane_content_inner_is_zero_on_todo_tab() {
    // 兜底口径：Todo 下 prompt 没有可交互内容区，按内容区坐标换算必须拿到零矩形，
    // 否则残留路径会把鼠标位置写进一个看不见的编辑器光标。
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let prompt = state.prompt.id();
    let inner = layout::pane_inner_rect(geo.view.prompt);
    let active = super::mouse::hit::pane_content_inner(&state, &geo.view, prompt, inner);
    assert!(
        active.width > 0 && active.height > 0,
        "前提：Prompt 态内容区非空: {active:?}"
    );

    state.right_tab = RightTab::Todo;
    assert_eq!(
        super::mouse::hit::pane_content_inner(&state, &geo.view, prompt, inner),
        Rect::ZERO,
        "Todo 下 prompt 内容区为零矩形"
    );
}

#[test]
fn mouse_right_click_prompt_tree_on_todo_opens_no_menu() {
    // 提示词树右键菜单的闸门：Todo 下内容区不接收右键。
    // 正向对照在前提里：同一坐标在 Prompt 态确实能打开菜单。
    use crate::app::prompt::PromptItem;
    let config = Config::default();
    let mut state = demo_terminal();
    update::show_prompt_tree(&mut state);
    state.workspaces[0]
        .prompt_panel
        .tree
        .replace(vec![], vec![PromptItem::single(7, None, "item", "body")]);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let area = paste::prompt_text_area(&geo.view);
    let target = ui::prompt::tree::row_at(
        area,
        &state.prompt_owner().prompt_panel,
        area.x + 2,
        area.y + 2,
    )
    .map(|_| (area.x + 2, area.y + 2));
    let Some((col, row)) = target else {
        panic!("前提：该坐标落在树行上");
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), col, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(
        matches!(state.overlay, Some(Overlay::Menu(_))),
        "前提：Prompt 态同坐标能开菜单"
    );

    update::close_overlay(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Right), col, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.overlay.is_none(), "Todo 下右键不打开提示词树菜单");
    assert!(!dirty);
}

#[test]
fn mouse_drag_on_todo_does_not_move_hidden_cursor() {
    // 拖拽光标跟随的闸门：Todo 下拖动不得改写隐藏编辑器的光标。
    let config = Config::default();
    let mut state = demo_terminal();
    state.prompt.set_text("alpha beta gamma\nsecond line");
    state.prompt.resize(40, 8);
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let text = layout::prompt_text_rect(geo.view.prompt);
    let prompt = state.prompt.id();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 正向对照：Prompt 态拖拽会移动光标。
    update::focus_prompt(&mut state);
    update::begin_selection(&mut state, prompt, 0, 0);
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            text.x + 3,
            text.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.prompt.cursor() > 0, "前提：Prompt 态拖拽移动光标");

    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let cursor = state.prompt.cursor();
    // `select_right_tab` 已清选区；这里直接构造拖拽事件，验证闸门而非选区状态。
    state.selection = Some(crate::app::selection::Selection::begin(prompt, 0, 0));
    dirty = false;
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            text.x + 1,
            text.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert_eq!(
        state.prompt.cursor(),
        cursor,
        "Todo 下拖拽不改隐藏编辑器的光标"
    );
    // 这一条由两道防线共同保证：`drag.rs` 的 `prompt_panel_active` 闸门与
    // `pane_content_inner` 的零矩形兜底。删掉闸门后零矩形提前返回，行为不变；
    // 删掉兜底后闸门兜住，行为也不变。两道都删才会漏，因此单独移除任一处的
    // 变异都不会让本用例变红——这是**冗余防线**，不是测试失效。
    state.selection = None;
}

#[test]
fn mouse_click_title_row_focuses_without_moving_cursor() {
    let config = Config::default();
    let mut state = paged_prompt_state();
    state.set_focus(crate::app::state::FocusedContainer::Panes);
    state.prompt.set_cursor_offset_for_test(0);
    let geo = geometry(&state, &config, Rect::new(0, 0, 140, 30));
    let title = ui::prompt_frame::title_row(geo.view.prompt).expect("title row");
    assert_eq!(
        title.y,
        crate::layout::pane_inner_rect(geo.view.prompt).y + 1,
        "标题行 = 工具栏下一行的分割线"
    );
    // 标题行内找一个既非页控件、也非页名的空白列（避开两侧边框列的分割线拖拽区）。
    let blank_x = (title.x + 2..title.right().saturating_sub(2))
        .find(|x| {
            ui::prompt_frame::prompt_page_control_at(geo.view.prompt, &state, *x, title.y).is_none()
        })
        .expect("title row has blank column");
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), blank_x, title.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.prompt_focused(), "点标题行空白只把焦点交给编辑器");
    assert_eq!(state.prompt.cursor(), 0, "标题行点击不移动正文光标");
    assert!(dirty);
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!dirty, "浮层激活时粘贴被吞掉");

    // 2. prompt 聚焦时：粘贴进入 prompt 编辑器，dirty 置为 true。
    state.overlay = None;
    state.set_focus(crate::app::state::FocusedContainer::Prompt);
    dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("hello prompt".to_string()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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
            reports: Vec::new(),
            attachments: Vec::new(),
        });
        chat.follow_bottom = true;
        target.lx.chat = chat;
    }
    let inner = pane_inner(&geo, pane);
    let card = {
        let target = state.pane_anywhere(pane).expect("pane");
        let mut cache = crate::ui::RenderCache::default();
        crate::ui::main_content::lx::history_view(inner, &target.lx, cache.pane(pane))
            .and_then(|view| {
                crate::ui::main_content::lx::history::layout(
                    view.body,
                    &target.lx,
                    cache.pane(pane),
                )
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
        &mut ui::RenderCache::default(),
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
        &mut ui::RenderCache::default(),
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

#[test]
fn lx_history_selection_waits_for_copy_shortcut_and_follows_wheel_scroll() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 18));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.insert_str("keep my draft");
        for index in 0..20 {
            target
                .lx
                .chat
                .messages
                .push(crate::app::state::ChatMessageView {
                    sequence: index,
                    id: format!("m-{index}"),
                    role: "user".into(),
                    status: "complete".into(),
                    turn_index: index,
                    kind: "chat".into(),
                    shell: None,
                    steps: vec![crate::app::state::AssistantStepView::Part {
                        position: 0,
                        part: crate::app::state::ChatPartView {
                            kind: crate::chat::events::PartKind::Text,
                            content: format!("history-line-{index:02}"),
                            provider_metadata_json: None,
                            started_at_ms: None,
                            ended_at_ms: None,
                            completed_at_ms: None,
                        },
                    }],
                    model_label: None,
                    error_code: None,
                    input_tokens: None,
                    output_tokens: None,
                    cache_read_tokens: None,
                    duration_ms: None,
                    reports: Vec::new(),
                    attachments: Vec::new(),
                });
        }
        target.lx.chat.follow_bottom = false;
        target.lx.chat.history_scroll = 0;
    }
    let mut cache = ui::RenderCache::default();
    let geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("geom")
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 1. 按下首行（内容第 0 行），按住左键向下滚轮 2 次，再拖拽到可视区第 2 行后松开。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            geom.rect.x,
            geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, geom.rect.x + 4, geom.rect.y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            geom.rect.x + 15,
            geom.rect.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            geom.rect.x + 15,
            geom.rect.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );

    let selection = state.lx_selection.expect("松开鼠标后保留选区");
    assert!(!selection.is_dragging(), "松开后结束拖拽态");
    assert_eq!(
        selection.range(),
        Some(((0, 0), (4, 15))),
        "锚点钉在内容第 0 行，终点随两次滚轮延伸到内容第 4 行"
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.toast.is_none(),
        "松开鼠标不直接触发复制 toast"
    );

    // 2. 松开鼠标后继续滚轮滚动：选区不被清除，仍钉在原内容行范围上。
    handle_terminal_event(
        mouse(MouseEventKind::ScrollDown, geom.rect.x + 4, geom.rect.y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        state.lx_selection.and_then(|sel| sel.range()),
        Some(((0, 0), (4, 15))),
        "松开后滚轮滚动不丢失选区"
    );

    // 3. 按下 Ctrl+C 复制：弹出全局 toast 并显示复制字符数、保留选区高亮、不清空底部输入框草稿。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(
        lx.input, "keep my draft",
        "有选区时按 Ctrl+C 不清空输入框草稿"
    );
    assert!(lx.toast.is_none(), "复制反馈不再写入 lx 页内 toast");
    let toast = state
        .toast
        .as_ref()
        .expect("按 Ctrl+C 后弹出全局复制 toast");
    assert_eq!(
        toast.anchor, None,
        "全局复制 toast 固定在屏幕顶部居中（anchor = None）"
    );
    assert_eq!(
        toast.title.as_deref(),
        Some(crate::ui::text::TOAST_CLIPBOARD_TITLE)
    );
    assert!(
        toast.message.starts_with("Copied ") && toast.message.ends_with(" chars"),
        "显示复制字符数：{}",
        toast.message
    );
    assert!(state.lx_selection.is_some(), "复制后选区高亮继续保留");
}

#[test]
fn lx_task_output_and_agents_detail_selection_scroll_and_copy() {
    use crate::app::state::{ProcessEntry, TaskPage, TaskTab};

    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 22));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        let mut page = TaskPage::default();
        page.set_processes(
            vec![ProcessEntry {
                session_id: 1,
                command: "cargo test".into(),
                cwd: "/w".into(),
                tty: false,
                started_at_ms: 0,
            }],
            0,
        );
        page.output = (0..30).map(|idx| format!("proc-out-{idx:02}")).collect();
        page.output_scroll = 0;
        target.lx.task = Some(page);
    }
    let mut cache = ui::RenderCache::default();
    let geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("geom")
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // Processes 输出区：按下第 0 行 → 滚轮下滚（每次 3 行）→ 松开保留选区 → Ctrl+C 复制。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            geom.rect.x,
            geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            geom.rect.x + 10,
            geom.rect.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            geom.rect.x + 10,
            geom.rect.y + 1,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let selection = state.lx_selection.expect("进程输出选区保留");
    assert_eq!(
        selection.range(),
        Some(((0, 0), (4, 10))),
        "滚轮滚动 3 行后，视口第 1 行对应内容第 4 行"
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.toast.is_none(),
        "松开鼠标不自动复制"
    );
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let toast = state
        .toast
        .as_ref()
        .expect("后台管理页按 Ctrl+C 触发全局复制 toast");
    assert_eq!(toast.anchor, None, "全局复制 toast 固定在屏幕顶部居中");
    assert!(
        toast.message.starts_with("Copied ") && toast.message.ends_with(" chars"),
        "显示复制字符数：{}",
        toast.message
    );

    // 切换到 Agents 页签：旧选区自动清除；在可折叠卡片上拖拽只选字不折叠，单击才折叠。
    assert!(update::select_task_tab(&mut state, pane, 1));
    assert!(state.lx_selection.is_none(), "切换后台 Tab 清除旧选区");
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        if let Some(page) = target.lx.task.as_mut() {
            page.tab = TaskTab::Agents;
        }
        let mut run =
            crate::app::state::SubagentRun::from_record(&crate::db::chat::SubagentRecord {
                agent_id: "sub-1".into(),
                child_session_id: "sess-1".into(),
                parent_session_id: "parent".into(),
                spawn_message_id: None,
                role: crate::chat::agent::AgentRole::Explorer,
                status: "completed".into(),
                title: "explore".into(),
                created_at_ms: 0,
                completed_at_ms: Some(100),
                report_json: None,
                report_delivered: false,
            });
        run.loaded = true;
        run.chat.messages.push(crate::app::state::ChatMessageView {
            sequence: 0,
            id: "am-0".into(),
            role: "assistant".into(),
            status: "complete".into(),
            turn_index: 0,
            kind: "chat".into(),
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: crate::app::state::ChatPartView {
                    kind: crate::chat::events::PartKind::Reasoning,
                    content: "thinking body".into(),
                    provider_metadata_json: None,
                    started_at_ms: Some(0),
                    ended_at_ms: Some(1000),
                    completed_at_ms: Some(1000),
                },
            }],
            model_label: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            duration_ms: None,
            reports: Vec::new(),
            attachments: Vec::new(),
        });
        target.lx.agents.runs.push(run);
    }
    let agent_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("agent geom")
    };
    // 在 Agents 详情的 thinking 卡片行上拖拽：只选中文字，不展开思考块。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            agent_geom.rect.x + 2,
            agent_geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            agent_geom.rect.x + 8,
            agent_geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            agent_geom.rect.x + 8,
            agent_geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        !state.pane_anywhere(pane).expect("pane").lx.agents.runs[0]
            .chat
            .reasoning_expanded(0, 0),
        "在 Agents 卡片上拖拽选字不得误触展开"
    );
    assert!(state.lx_selection.is_some(), "拖拽后保留 Agents 详情选区");

    // 原地单击（无拖拽）：切换折叠状态并清除选区。
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            agent_geom.rect.x + 2,
            agent_geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            agent_geom.rect.x + 2,
            agent_geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.pane_anywhere(pane).expect("pane").lx.agents.runs[0]
            .chat
            .reasoning_expanded(0, 0),
        "原地单击 Agents 卡片切换展开"
    );
    assert!(state.lx_selection.is_none(), "单击后不保留空选区");

    // Agents 详情区跨屏滚轮拖选：给子 agent 塞入多条消息使其可滚动，验证边拖边滚延伸内容行坐标。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        let run = &mut target.lx.agents.runs[0];
        for idx in 1..20 {
            run.chat.messages.push(crate::app::state::ChatMessageView {
                sequence: idx,
                id: format!("am-{idx}"),
                role: "assistant".into(),
                status: "complete".into(),
                turn_index: idx,
                kind: "chat".into(),
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Text,
                        content: format!("agent-line-{idx:02}"),
                        provider_metadata_json: None,
                        started_at_ms: None,
                        ended_at_ms: None,
                        completed_at_ms: None,
                    },
                }],
                model_label: None,
                error_code: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                duration_ms: None,
                reports: Vec::new(),
                attachments: Vec::new(),
            });
        }
        run.follow = false;
        run.scroll = 0;
        run.rows_dirty = true;
    }
    let agent_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("agent geom")
    };
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            agent_geom.rect.x,
            agent_geom.rect.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::ScrollDown,
            agent_geom.rect.x + 6,
            agent_geom.rect.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
            agent_geom.rect.x + 6,
            agent_geom.rect.y + 2,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        state.lx_selection.and_then(|sel| sel.range()),
        Some(((0, 0), (3, 6))),
        "Agents 详情边拖边滚后，选区锚点钉在第 0 行、终点延伸到第 3 行"
    );
}

#[test]
fn terminal_selection_retains_on_mouse_up_scrolls_and_copies_on_ctrl_c() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 20));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("terminal pane");
        for idx in 0..35 {
            target
                .terminal
                .feed(format!("term-line-{idx:02}\r\n").as_bytes());
        }
    }
    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;

    // 1. 终端窗格拖拽选择后松开左键：不自动复制，保留仿真器选区高亮。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), inner.x, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(
            MouseEventKind::Drag(MouseButton::Left),
            inner.x + 11,
            inner.y,
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), inner.x + 11, inner.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        state.terminal_selection.is_none(),
        "松开鼠标后结束终端选区拖拽态"
    );
    assert!(state.toast.is_none(), "松开鼠标不自动触发复制 toast");
    assert_eq!(
        update::terminal_selection_text(&state, pane)
            .as_deref()
            .map(str::len),
        Some(12),
        "松开鼠标后终端选区保留在仿真器中"
    );

    // 2. 松开鼠标后滚轮向上回滚：选区不延伸、不清除，随内容行一起保留。
    let selected_before = update::terminal_selection_text(&state, pane).expect("selected");
    handle_terminal_event(
        mouse(MouseEventKind::ScrollUp, inner.x + 5, inner.y + 3),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        update::terminal_selection_text(&state, pane).as_deref(),
        Some(selected_before.as_str()),
        "松开后滚轮滚动不改变已选中的终端内容文本"
    );

    // 3. 按下 Ctrl+C 复制：弹出全局顶部居中 toast（anchor = None）并显示复制字符数，选区继续保留。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let toast = state
        .toast
        .as_ref()
        .expect("终端选区按 Ctrl+C 弹出全局复制 toast");
    assert_eq!(toast.anchor, None, "全局复制 toast 固定在屏幕顶部居中");
    assert_eq!(toast.message, "Copied 12 chars");
    assert!(
        update::terminal_selection_text(&state, pane).is_some(),
        "复制后终端选区高亮继续保留"
    );

    // 4. 在终端输入普通按键：自动清除终端选区。
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert!(
        update::terminal_selection_text(&state, pane).is_none(),
        "终端输入其它按键后自动清除选区"
    );
}

#[test]
fn double_and_triple_click_select_word_and_line_across_all_views() {
    let config = Config::default();
    let mut state = AppState::demo();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 24));
    let pane = state.active_tab().layout.focus();
    let inner = pane_inner(&geo, pane);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Lx;
        target.lx.insert_str("alpha beta_gamma delta");
        target
            .lx
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                sequence: 0,
                id: "m0".into(),
                role: "assistant".into(),
                status: "complete".into(),
                turn_index: 0,
                kind: "chat".into(),
                shell: None,
                steps: vec![crate::app::state::AssistantStepView::Part {
                    position: 0,
                    part: crate::app::state::ChatPartView {
                        kind: crate::chat::events::PartKind::Text,
                        content: "firstWord secondWord thirdWord".into(),
                        provider_metadata_json: None,
                        started_at_ms: None,
                        ended_at_ms: None,
                        completed_at_ms: None,
                    },
                }],
                model_label: None,
                error_code: None,
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                duration_ms: None,
                reports: Vec::new(),
                attachments: Vec::new(),
            });
    }
    update::resize_panes(&mut state, &geo.rects);
    state.prompt.set_text("prompt_one prompt_two\nsecond_line");

    let mut sessions = HashMap::new();
    let mut cache = ui::RenderCache::default();
    let mut dirty = false;

    // 1. lx 主内容区：双击选中 "second_word"，三击选中整行。
    let hist_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane)).expect("hist geom")
    };
    let word_x = hist_geom.rect.x + 15;
    let word_y = hist_geom.rect.y;
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), word_x, word_y),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), word_x, word_y),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    let range = state
        .lx_selection
        .and_then(|sel| sel.range())
        .expect("双击历史消息选中词");
    let copied_word = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::copy_text(inner, &target.lx, range, cache.pane(pane))
    };
    assert_eq!(copied_word, "secondWord");

    // 第 3 击选中整行。
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), word_x, word_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), word_x, word_y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    let line_range = state
        .lx_selection
        .and_then(|sel| sel.range())
        .expect("三击历史消息选中整行");
    let copied_line = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::copy_text(inner, &target.lx, line_range, cache.pane(pane))
    };
    assert!(
        copied_line.contains("firstWord secondWord thirdWord"),
        "三击复制整行：{copied_line}"
    );

    // 2. lx 底部输入框：双击选中 "beta_gamma"，松开后保留高亮，Ctrl+C 复制且不清空草稿。
    let input_geom = {
        let target = state.pane_anywhere(pane).expect("pane");
        ui::main_content::lx::input_geom(inner, &target.lx).expect("input geom")
    };
    let (ix, iy) = input_geom.text_origin();
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), ix + 8, iy),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), ix + 8, iy),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    assert_eq!(
        state
            .pane_anywhere(pane)
            .and_then(|t| t.lx.selected_text())
            .as_deref(),
        Some("beta_gamma"),
        "双击 lx 输入框选中词并保留高亮"
    );
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        state.toast.as_ref().map(|t| t.message.as_str()),
        Some("Copied 10 chars")
    );
    assert_eq!(
        state.pane_anywhere(pane).expect("pane").lx.input,
        "alpha beta_gamma delta",
        "输入框有选区时按 Ctrl+C 不清空草稿"
    );

    // 3. 右侧 Prompt 编辑器：双击选词、三击选整逻辑行。
    let prompt_area = super::paste::prompt_text_area(&geo.view);
    let px = prompt_area.x + 12;
    let py = prompt_area.y;
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), px, py),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), px, py),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    assert_eq!(
        update::prompt_selection_text(&state).as_deref(),
        Some("prompt_two"),
        "双击 Prompt 选中单词"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), px, py),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), px, py),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        update::prompt_selection_text(&state).as_deref(),
        Some("prompt_one prompt_two"),
        "三击 Prompt 选中整条逻辑行"
    );

    // 4. 终端窗格：双击选词、三击选整行。
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = PaneView::Terminal;
        target.terminal.feed(b"term_alpha term_beta\r\n");
    }
    let tx = inner.x + 12;
    let ty = inner.y;
    for _ in 0..2 {
        handle_terminal_event(
            mouse(MouseEventKind::Down(MouseButton::Left), tx, ty),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
        handle_terminal_event(
            mouse(MouseEventKind::Up(MouseButton::Left), tx, ty),
            &mut state,
            &mut sessions,
            &geo,
            &config,
            &mut cache,
            &mut dirty,
        );
    }
    assert_eq!(
        update::terminal_selection_text(&state, pane).as_deref(),
        Some("term_beta"),
        "双击终端窗格选中词"
    );
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), tx, ty),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    handle_terminal_event(
        mouse(MouseEventKind::Up(MouseButton::Left), tx, ty),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut cache,
        &mut dirty,
    );
    assert_eq!(
        update::terminal_selection_text(&state, pane).as_deref(),
        Some("term_alpha term_beta\n"),
        "三击终端窗格选中整行"
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

#[test]
fn lx_paste_image_path_opens_panel_and_confirms_refer_image() {
    let dir = std::env::temp_dir().join(format!("lx-paste-img-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let img_path = dir.join("PixPin_2026-10-29_09-52-07.png");
    let img: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]));
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("png");
    std::fs::write(&img_path, &png).expect("write");

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state.pane_mut_anywhere(pane).expect("pane").view = PaneView::Lx;

    let path_str = img_path.to_string_lossy().into_owned();
    assert!(paste::handle_lx_paste(&mut state, path_str.clone()));
    let target = state.pane_mut_anywhere(pane).expect("pane");
    assert!(
        target.lx.paste_panel().is_some(),
        "粘贴有路径的图片必须弹出二级面板"
    );
    assert!(
        target
            .lx
            .paste_confirm(crate::app::prompt::PasteAction::Reference)
    );
    assert_eq!(target.lx.input, format!("@[refer-image]({path_str}) "));

    let _ = std::fs::remove_dir_all(&dir);
}

// ===== ask 面板接线：点击吞掉、粘贴只进手写缓冲 =====

/// 流中窗格 + 一道两选项的挂起提问（真实 request 路径会跑串行锁，这里直接构造，
/// 但 `request` 必须是 busy，否则面板会被 sweep 当成孤儿清掉）。
fn busy_ask_state() -> (AppState, PaneId) {
    use crate::app::state::{AskState, RequestKey, RequestState};
    use crate::chat::tools::ask::{AskOption, AskQuestion};

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let key = RequestKey {
        pane,
        session: "sess-ask".into(),
        generation: 1,
    };
    let question = AskQuestion {
        id: "layout".into(),
        header: "布局".into(),
        question: "用哪种布局？".into(),
        multi_select: false,
        options: vec![
            AskOption {
                label: "two columns".into(),
                description: "left list".into(),
                preview: Some("┌──┐\n└──┘".into()),
            },
            AskOption {
                label: "stacked".into(),
                description: "top down".into(),
                preview: None,
            },
        ],
    };
    let target = state.pane_mut_anywhere(pane).expect("focused pane");
    target.lx.input = "draft".into();
    target.lx.chat.session_id = Some(key.session.clone());
    target.lx.chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id: "a1".into(),
    };
    target.lx.ask = Some(AskState::new(key, "call".into(), vec![question]));
    (state, pane)
}

/// 面板内第一处命中 `AskHit::Option(0)` 的坐标（几何由渲染层给出，事件层不重算）。
fn ask_option_point(state: &AppState, pane: PaneId, geo: &Geometry) -> (u16, u16) {
    let rect = geo
        .rects
        .iter()
        .find(|(id, _)| *id == pane)
        .map(|(_, rect)| *rect)
        .expect("pane rect");
    let inner = layout::pane_inner_rect(rect);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    let area = ui::main_content::lx::ask_rect(inner, lx).expect("ask rect");
    for row in area.y..area.bottom() {
        for column in area.x..area.right() {
            if ui::main_content::lx::ask_hit(inner, lx, column, row)
                == Some(crate::app::state::AskHit::Option(0))
            {
                return (column, row);
            }
        }
    }
    panic!("找不到选项行");
}

#[test]
fn ask_click_selects_option_and_does_not_reach_history() {
    let config = Config::default();
    let (mut state, pane) = busy_ask_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let (column, row) = ask_option_point(&state, pane, &geo);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let ask = state
        .pane_anywhere(pane)
        .and_then(|pane| pane.lx.ask.as_ref())
        .expect("面板仍在（busy 不会被 sweep）");
    assert_eq!(ask.selected[0], vec![0], "单击选项即选中（单选独占）");
    assert!(!ask.editing, "点选项不提交、不进入编辑");
    assert!(dirty, "点击必须重绘");
    assert!(
        state.lx_card_press.is_none(),
        "面板内点击不得登记历史卡片点击"
    );
}

#[test]
fn task_page_swallows_ask_hits_so_the_list_stays_clickable() {
    // 后台任务页与 ask 面板共用输入框矩形：页打开时面板不可见，点击必须落在
    // 任务列表上，绝不能命中隐藏面板改一个看不见的答案。
    let config = Config::default();
    let (mut state, pane) = busy_ask_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let (column, row) = ask_option_point(&state, pane, &geo);
    let inner = layout::pane_inner_rect(
        geo.rects
            .iter()
            .find(|(id, _)| *id == pane)
            .map(|(_, rect)| *rect)
            .expect("pane rect"),
    );
    state.pane_mut_anywhere(pane).expect("pane").lx.task = Some(crate::app::state::TaskPage {
        tab: crate::app::state::TaskTab::Processes,
        ..Default::default()
    });

    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    // 任务页接管了输入框矩形，ask 几何已经不在这里：点击绝不能落到隐藏面板。
    assert!(
        ui::main_content::lx::ask_hit(inner, lx, column, row).is_none(),
        "任务页打开时 ask 几何必须失效"
    );

    let before = state
        .pane_anywhere(pane)
        .and_then(|pane| pane.lx.ask.as_ref())
        .expect("ask")
        .selected
        .clone();
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    let after = state
        .pane_anywhere(pane)
        .and_then(|pane| pane.lx.ask.as_ref())
        .expect("ask")
        .selected
        .clone();
    assert_eq!(before, after, "隐藏的面板不得被点击改状态");
}

#[test]
fn ask_paste_only_enters_the_custom_buffer() {
    let config = Config::default();
    let (mut state, pane) = busy_ask_state();
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    let mut sessions = HashMap::new();

    // 列表态：整段吞掉，草稿一个字节都不动。
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("ignored".into()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(!dirty);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.input, "draft");
    let ask = lx.ask.as_ref().expect("ask");
    assert_eq!(ask.custom[0].text(), "");

    // 编辑态：写入手写缓冲，草稿仍不变。
    state
        .pane_mut_anywhere(pane)
        .and_then(|pane| pane.lx.ask.as_mut())
        .expect("ask")
        .begin_edit();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Paste("own words".into()),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(dirty);
    let lx = &state.pane_anywhere(pane).expect("pane").lx;
    assert_eq!(lx.input, "draft", "草稿不变量");
    assert_eq!(
        lx.ask.as_ref().expect("ask").custom[0].text(),
        "own words",
        "编辑态粘贴进入手写缓冲"
    );
}

// ===== 单一焦点源：同一时刻只有一个容器高亮/持有光标/收键 =====

/// 点右侧容器后主区让出焦点高亮（真机缺陷：两者同时描边强调色）。
#[test]
fn focusing_right_panel_releases_pane_highlight() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), list.x + 1, list.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.todo_focused());
    assert!(!state.panes_focused(), "主区必须让出焦点");
    assert!(!state.sidebar_focused());
    assert!(state.todo_focused() ^ state.prompt_focused() ^ state.panes_focused());
}

/// 点主区窗格后右侧容器让出焦点：不会出现"两个容器同时高亮"。
#[test]
fn focusing_pane_releases_right_panel_highlight() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), list.x + 1, list.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.todo_focused());

    let inner = layout::pane_inner_rect(geo.rects[0].1);
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.panes_focused(), "点主区后主区持焦点");
    assert!(!state.todo_focused(), "右侧容器必须让出焦点");
}

/// 点侧栏后：侧栏高亮，主区与右侧容器都不再高亮。
#[test]
fn focusing_sidebar_releases_other_containers() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    // 先让右侧容器持焦点。
    state.set_focus(crate::app::state::FocusedContainer::Todo);
    let mut sessions = HashMap::new();
    let mut dirty = false;

    // 点 Agents 分区空白（侧栏内、非条目）。
    let sections = ui::layout::sidebar_sections(geo.view.sidebar, state.agents_collapsed)
        .expect("sidebar sections visible");
    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            sections.agents.x,
            sections.agents.y + sections.agents.height.saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert!(state.sidebar_focused(), "点侧栏空白让侧栏持焦点");
    assert!(!state.todo_focused(), "右侧容器让出焦点");
    assert!(!state.panes_focused(), "主区让出焦点");
}

/// 侧栏持焦点时按键全部吞掉，绝不写进 PTY 或其它输入框。
#[test]
fn sidebar_focus_swallows_all_keys() {
    use alacritty_terminal::term::TermMode;
    for key in [
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    ] {
        let routed = crate::input::route(
            key,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Sidebar,
            None,
            true,
        );
        assert!(routed.is_none(), "{key:?} 必须被吞掉，实际 {routed:?}");
    }
}

/// 焦点容器互斥由类型保证：设置任一容器后，其余三个的查询都返回 false。
#[test]
fn focused_container_is_mutually_exclusive() {
    use crate::app::state::FocusedContainer;
    let mut state = AppState::demo();
    for target in [
        FocusedContainer::Sidebar,
        FocusedContainer::Panes,
        FocusedContainer::Prompt,
        FocusedContainer::Todo,
    ] {
        state.set_focus(target);
        let flags = [
            state.sidebar_focused(),
            state.panes_focused(),
            state.prompt_focused(),
            state.todo_focused(),
        ];
        assert_eq!(flags.iter().filter(|flag| **flag).count(), 1, "{target:?}");
        assert_eq!(state.focus(), target);
    }
}

/// 释放 Todo 滚动条拖拽后必须清空抓取状态：否则之后所有左键拖拽都被 Todo 分支吞掉。
#[test]
fn todo_scrollbar_drag_releases_on_mouse_up() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    let visible = crate::ui::right_panel::todo::visible_rows(geo.view.prompt);
    state.todo.replace(
        (0..visible + 8)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("t{index}"),
                    crate::db::todo::TodoStatus::Todo,
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect(),
    );
    state.todo.set_viewport_rows(visible);
    let bar =
        crate::ui::right_panel::todo::scrollbar(geo.view.prompt, &state).expect("前提：列表溢出");
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
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.todo_scroll_drag.is_some(), "前提：抓住了 thumb");

    handle_terminal_event(
        mouse(
            MouseEventKind::Up(MouseButton::Left),
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

    assert!(
        state.todo_scroll_drag.is_none(),
        "松开必须释放抓取状态，否则后续所有拖拽被劫持"
    );
}

/// Prompt tab 下点正文右缘不得滚动隐藏的 Todo 列表（两者共用最右一列）。
#[test]
fn prompt_gutter_click_does_not_scroll_hidden_todo() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert_eq!(state.right_tab, RightTab::Prompt, "前提：Prompt tab");
    let visible = crate::ui::right_panel::todo::visible_rows(geo.view.prompt);
    state.todo.replace(
        (0..visible + 8)
            .map(|index| {
                crate::db::todo::TodoRecord::new(
                    format!("t{index}"),
                    crate::db::todo::TodoStatus::Todo,
                    i64::try_from(index).unwrap_or(i64::MAX),
                )
            })
            .collect(),
    );
    state.todo.set_viewport_rows(visible);
    // 点 Todo 列表区最右列的最后一行：两面板的最右列重合（都是 inner.right()-1），
    // 这一行只在 Todo 列表轨道内，能真正触发「隐藏滚动条被点中」的路径。
    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let bar = Rect {
        x: list.right(),
        y: list.y,
        width: 1,
        height: list.height,
    };
    let mut sessions = HashMap::new();
    let mut dirty = false;

    handle_terminal_event(
        mouse(
            MouseEventKind::Down(MouseButton::Left),
            bar.x,
            bar.bottom().saturating_sub(1),
        ),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(state.todo.scroll(), 0, "隐藏的 Todo 列表不得被滚动");
    assert!(state.todo_scroll_drag.is_none(), "也不得开始 Todo 拖拽");
}

/// 侧栏持焦点时粘贴必须吞掉（与键盘路由同口径：绝不穿透到别的容器）。
#[test]
fn paste_while_sidebar_focused_is_swallowed() {
    let mut state = demo_terminal();
    assert!(crate::app::update::focus_sidebar(&mut state));
    assert!(state.sidebar_focused(), "前提：侧栏持焦点");
    let before = state.prompt.text().to_string();
    let mut sessions = HashMap::new();
    let mut dirty = false;

    super::input::handle_paste_event("secret".into(), &mut state, &mut sessions, &mut dirty);

    assert_eq!(state.prompt.text(), before, "不得写进 prompt");
    assert!(!dirty, "无变化不置脏");
}

/// 悬停换算走真实入口：幽灵行打开时把鼠标移到 b 那一行，`todo_hover` 必须是 b 的**列表**下标。
#[test]
fn hover_below_ghost_row_stores_list_index() {
    let mut state = demo_terminal();
    let (config, geo) = toolbar_geometry(&mut state);
    assert!(crate::app::update::select_right_tab(
        &mut state,
        RightTab::Todo
    ));
    state.todo.replace(vec![
        crate::db::todo::TodoRecord::new("a", crate::db::todo::TodoStatus::Todo, 0),
        crate::db::todo::TodoRecord::new("b", crate::db::todo::TodoStatus::Todo, 1),
        crate::db::todo::TodoRecord::new("c", crate::db::todo::TodoStatus::Todo, 2),
    ]);
    // 幽灵行插在 1：显示序列 a(0) / 幽灵(1) / b(2) / c(3)。
    crate::app::update::begin_todo_edit(&mut state, crate::app::state::TodoEditTarget::After(0));
    assert!(state.todo.is_editing(), "前提：幽灵行打开");

    let list = crate::ui::right_panel::todo::list_rect(geo.view.prompt);
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, list.x + 3, list.y + 2),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );

    assert_eq!(
        state.todo_hover,
        Some(1),
        "显示第 2 行是列表下标 1（b），不是显示下标 2（那会被渲染层当成 c）"
    );
}

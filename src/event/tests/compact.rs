//! 紧凑布局的端到端事件/几何测试：顶部栏、左右全屏栏与隐藏控件。

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::test_support::*;
use super::*;
use crate::app::state::{CompactRestore, FocusedContainer, RightTab};
use crate::event::session::Geometry;

fn compact_geometry(state: &mut AppState, config: &Config, width: u16, height: u16) -> Geometry {
    let screen = Rect::new(0, 0, width, height);
    update::sync_compact_layout(state, width, config);
    let view = ui::layout::compute_with_panel(
        screen,
        config,
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
        state.compact_panel(),
    );
    let mut rects = if view.main_content.width == 0
        || view.main_content.height == 0
        || (!view.compact && state.prompt_left && state.panes_collapsed)
    {
        Vec::new()
    } else {
        tiling::pane_rects(
            &state.active_tab().layout,
            view.main_content,
            config.min_pane_width,
            config.min_pane_height,
        )
    };
    if !state.prompt_collapsed && view.right_panel.width > 0 {
        rects.push((state.prompt.id(), view.right_panel));
    }
    Geometry {
        screen,
        rects,
        view,
    }
}

fn click(state: &mut AppState, config: &Config, geo: &Geometry, column: u16, row: u16) -> bool {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Down(MouseButton::Left), column, row),
        state,
        &mut sessions,
        geo,
        config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    dirty
}

fn key(state: &mut AppState, config: &Config, geo: &Geometry, code: KeyCode) -> bool {
    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)),
        state,
        &mut sessions,
        geo,
        config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    dirty
}

#[test]
fn threshold_160_enters_compact_and_161_stays_desktop() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 160, 24);
    assert!(geo.view.compact);
    assert_eq!(geo.view.compact_header.height, 1);

    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 161, 24);
    assert!(!geo.view.compact);
    assert_eq!(geo.view.tab_bar.height, 1);
    assert!(geo.view.left_panel.width > 0);
}

#[test]
fn compact_chat_gives_full_width_to_panes() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert!(geo.view.compact);
    assert_eq!(geo.view.main_content, Rect::new(0, 1, 100, 23));
    assert_eq!(geo.rects.len(), 1, "only the pane is interactive");
    assert_eq!(geo.rects[0].1.width, 100);
    assert_eq!(
        state.compact_restore,
        Some(CompactRestore {
            sidebar_collapsed: false,
            prompt_collapsed: false,
            panes_collapsed: false,
        })
    );
}

#[test]
fn short_terminal_uses_one_nav_row() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 40, 10);
    assert_eq!(geo.view.compact_header, Rect::new(0, 0, 40, 1));
    assert_eq!(geo.view.main_content, Rect::new(0, 1, 40, 9));
}

#[test]
fn repeated_sync_keeps_original_snapshot() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.sidebar_collapsed = true;
    state.prompt_collapsed = true;
    state.panes_collapsed = true;
    let _ = compact_geometry(&mut state, &config, 100, 24);
    let _ = compact_geometry(&mut state, &config, 100, 24);
    assert_eq!(
        state.compact_restore,
        Some(CompactRestore {
            sidebar_collapsed: true,
            prompt_collapsed: true,
            panes_collapsed: true,
        })
    );
    assert!(!state.sidebar_collapsed);
}

#[test]
fn left_button_opens_fullscreen_sidebar_and_toggles_back() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    let left = geo.view.compact_left_button;
    assert!(click(&mut state, &config, &geo, left.x, left.y));
    assert_eq!(state.focus(), FocusedContainer::Sidebar);
    assert_eq!(
        state.compact_panel(),
        Some(crate::app::state::CompactPanel::Left)
    );

    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert_eq!(geo.view.left_panel, Rect::new(0, 1, 100, 23));
    assert_eq!(geo.view.main_content.width, 0);
    assert!(
        geo.rects.is_empty(),
        "fullscreen sidebar keeps panes out of geometry"
    );
    assert!(geo.view.left_panel.contains((30, 10).into()));

    assert!(click(&mut state, &config, &geo, left.x, left.y));
    assert_eq!(state.focus(), FocusedContainer::Panes);
    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert_eq!(geo.view.main_content, Rect::new(0, 1, 100, 23));
}

#[test]
fn right_button_opens_fullscreen_right_panel() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    let right = geo.view.compact_right_button;
    assert!(click(&mut state, &config, &geo, right.x, right.y));
    assert_eq!(state.focus(), FocusedContainer::Prompt);

    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert_eq!(geo.view.right_panel, Rect::new(0, 1, 100, 23));
    assert_eq!(geo.view.main_content.width, 0);
    assert!(geo.rects.iter().all(|(id, _)| *id == state.prompt.id()));
    assert!(geo.view.right_panel.contains((30, 10).into()));

    assert!(click(&mut state, &config, &geo, right.x, right.y));
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn clicking_tab_returns_from_panel_and_switches_tab() {
    let config = Config::default();
    let mut state = demo_terminal();
    update::create_tab(&mut state);
    update::switch_tab(&mut state, 0);
    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert!(click(
        &mut state,
        &config,
        &geo,
        geo.view.compact_right_button.x,
        geo.view.compact_right_button.y
    ));
    assert_eq!(state.focus(), FocusedContainer::Prompt);

    let geo = compact_geometry(&mut state, &config, 100, 24);
    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let (index, rect) = bar.tabs[1];
    assert!(click(&mut state, &config, &geo, rect.x, rect.y));
    assert_eq!(state.active_workspace().active_tab, 1);
    assert_eq!(state.focus(), FocusedContainer::Panes);
    assert_eq!(index, 1);
}

#[test]
fn compact_tab_bar_supports_add_and_scroll() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 40, 24);
    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let add = bar.add.expect("add button");
    assert!(click(&mut state, &config, &geo, add.x, add.y));
    assert_eq!(state.active_workspace().tabs.len(), 2);

    for _ in 0..12 {
        update::create_tab(&mut state);
    }
    let geo = compact_geometry(&mut state, &config, 40, 24);
    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    assert!(bar.overflow);
    let right = bar.scroll_right.expect("right scroll button");
    assert!(click(&mut state, &config, &geo, right.x, right.y));
    assert!(state.tab_scroll > 0);
}

#[test]
fn escape_closes_fullscreen_sidebar_only() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert!(click(
        &mut state,
        &config,
        &geo,
        geo.view.compact_left_button.x,
        geo.view.compact_left_button.y
    ));
    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert!(key(&mut state, &config, &geo, KeyCode::Esc));
    assert_eq!(state.focus(), FocusedContainer::Panes);

    state.set_focus(FocusedContainer::Prompt);
    state.right_tab = RightTab::Prompt;
    let geo = compact_geometry(&mut state, &config, 100, 24);
    key(&mut state, &config, &geo, KeyCode::Esc);
    assert_eq!(
        state.focus(),
        FocusedContainer::Prompt,
        "右栏 Esc 不得关闭整栏"
    );
}

#[test]
fn header_gap_click_is_swallowed() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert!(!click(
        &mut state,
        &config,
        &geo,
        geo.view.tab_bar.right() - 1,
        0
    ));
    assert_eq!(state.focus(), FocusedContainer::Panes);
    assert!(state.overlay.is_none());
}

#[test]
fn hidden_controls_have_no_hit_area() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert!(ui::collapse_buttons(&geo.view, false).is_empty());
    assert!(ui::exit_button(&geo.view).is_none());
    assert!(!ui::prompt_pin_at(&geo.view, &state, 96, 20));
    assert!(!ui::swap_button_at(&geo.view, 50, 0));
}

#[test]
fn fullscreen_sidebar_click_hits_real_rows() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    click(
        &mut state,
        &config,
        &geo,
        geo.view.compact_left_button.x,
        geo.view.compact_left_button.y,
    );
    let geo = compact_geometry(&mut state, &config, 100, 24);
    let sections = ui::layout::sidebar_sections(geo.view.left_panel, false).expect("sections");
    let target = ui::workspace_item_at(
        &geo.view,
        &state,
        sections.workspaces.x + 1,
        sections.workspaces.y,
    );
    assert!(target.is_some(), "fullscreen sidebar rows stay clickable");
}

#[test]
fn fullscreen_left_panel_keeps_agents_toggle_clickable() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    click(
        &mut state,
        &config,
        &geo,
        geo.view.compact_left_button.x,
        geo.view.compact_left_button.y,
    );
    let geo = compact_geometry(&mut state, &config, 100, 24);
    let agents_btn = ui::collapse_buttons(&geo.view, state.agents_collapsed)
        .into_iter()
        .find(|btn| btn.target == ui::CollapseTarget::Agents)
        .expect("agents collapse button visible in fullscreen left panel");
    assert!(click(
        &mut state,
        &config,
        &geo,
        agents_btn.area.x,
        agents_btn.area.y
    ));
    assert!(state.agents_collapsed);
}

#[test]
fn history_mode_in_fullscreen_sidebar_takes_double_escape_to_return_chat() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.set_focus(FocusedContainer::Sidebar);
    update::toggle_workspace_mode(&mut state);
    assert!(update::workspace_history_mode(&state));
    let geo = compact_geometry(&mut state, &config, 100, 24);

    assert!(key(&mut state, &config, &geo, KeyCode::Esc));
    assert!(!update::workspace_history_mode(&state));
    assert_eq!(state.focus(), FocusedContainer::Sidebar);

    let geo = compact_geometry(&mut state, &config, 100, 24);
    assert!(key(&mut state, &config, &geo, KeyCode::Esc));
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn fullscreen_right_panel_switches_prompt_and_todo_tabs() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 100, 24);
    click(
        &mut state,
        &config,
        &geo,
        geo.view.compact_right_button.x,
        geo.view.compact_right_button.y,
    );
    let geo = compact_geometry(&mut state, &config, 100, 24);
    let tabs = ui::right_panel::tab_rects(geo.view.right_panel);
    let todo_rect = tabs
        .iter()
        .find_map(|(tab, rect)| (*tab == RightTab::Todo).then_some(*rect))
        .expect("todo tab visible");
    assert!(click(&mut state, &config, &geo, todo_rect.x, todo_rect.y));
    assert_eq!(state.right_tab, RightTab::Todo);
    assert_eq!(state.focus(), FocusedContainer::Todo);
}

#[test]
fn hidden_chat_selection_does_not_swallow_escape_in_fullscreen_left_panel() {
    let config = Config::default();
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.view = crate::app::state::PaneView::Lx;
        target.lx.insert_str("draft");
        target.lx.begin_text_selection(0, 0);
        target.lx.drag_text_selection(0, 3);
    }
    let _ = compact_geometry(&mut state, &config, 100, 24);
    state.set_focus(FocusedContainer::Sidebar);
    let geo = compact_geometry(&mut state, &config, 100, 24);

    assert!(key(&mut state, &config, &geo, KeyCode::Esc));
    assert_eq!(
        state.focus(),
        FocusedContainer::Panes,
        "隐藏 Chat 选区不得吞掉全屏左栏的 Esc"
    );
}

#[test]
fn compact_hover_never_arms_tooltip_or_divider_hover() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.settings_available = true;
    let geo = compact_geometry(&mut state, &config, 100, 24);
    let bar = ui::tab_bar::layout(&geo.view, state.active_workspace(), state.tab_scroll);
    let add = bar.add.expect("add tab button");

    let mut sessions = HashMap::new();
    let mut dirty = false;
    handle_terminal_event(
        mouse(MouseEventKind::Moved, add.x, add.y),
        &mut state,
        &mut sessions,
        &geo,
        &config,
        &mut ui::RenderCache::default(),
        &mut dirty,
    );
    assert!(state.tooltip.is_none());
    assert!(!state.sidebar_hover);
    assert!(!state.prompt_hover);
}

#[test]
fn desktop_collapsed_panes_exclude_panes_from_geometry_rects() {
    let config = desktop_config();
    let mut state = demo_terminal();
    state.prompt_left = true;
    state.panes_collapsed = true;
    let geo = geometry(&state, &config, Rect::new(0, 0, 120, 30));
    assert!(!geo.view.compact);
    assert_eq!(geo.view.main_content.width, crate::tiling::COLLAPSED_STRIP);
    assert!(
        geo.rects.iter().all(|(id, _)| *id == state.prompt.id()),
        "折叠主区不得把 4 列窄条放入 PTY/命中几何: {:?}",
        geo.rects
    );
}

#[test]
fn sync_layout_geometry_preserves_lx_cache_and_reveals_active_tab_on_narrow_resize() {
    let config = Config::default();
    let mut state = demo_terminal();
    let pane = state.active_tab().layout.focus();
    for _ in 0..8 {
        update::create_tab(&mut state);
    }
    let last_tab = state.active_workspace().tabs.len() - 1;
    assert_eq!(state.active_workspace().active_tab, last_tab);
    let mut sessions = HashMap::new();
    let mut ui_cache = ui::RenderCache::default();
    ui_cache.pane(pane);
    let mut last_rects = Vec::new();
    let mut last_screen = Rect::ZERO;
    let mut last_compact = false;
    let mut dirty = false;

    // 从宽屏（180 列，tab_scroll = 0）缩窄至 60 列紧凑布局：
    let wide = compact_geometry(&mut state, &config, 180, 24);
    session::sync_layout_geometry(
        &mut state,
        &mut sessions,
        &mut ui_cache,
        &wide,
        &mut last_rects,
        &mut last_screen,
        &mut last_compact,
        &mut dirty,
    );
    assert_eq!(state.tab_scroll, 0);

    let narrow = compact_geometry(&mut state, &config, 60, 24);
    session::sync_layout_geometry(
        &mut state,
        &mut sessions,
        &mut ui_cache,
        &narrow,
        &mut last_rects,
        &mut last_screen,
        &mut last_compact,
        &mut dirty,
    );
    assert!(
        state.tab_scroll > 0,
        "缩窄进入紧凑后应自动把右侧激活标签滚入可见区"
    );

    // 打开全屏左栏（rects 为空）后，存活 lx 窗格的缓存仍保留：
    state.set_focus(FocusedContainer::Sidebar);
    let left_geo = compact_geometry(&mut state, &config, 60, 24);
    session::sync_layout_geometry(
        &mut state,
        &mut sessions,
        &mut ui_cache,
        &left_geo,
        &mut last_rects,
        &mut last_screen,
        &mut last_compact,
        &mut dirty,
    );
    assert!(
        ui_cache.has_pane(pane),
        "全屏左栏期间不得误删隐藏窗格的渲染缓存"
    );
}

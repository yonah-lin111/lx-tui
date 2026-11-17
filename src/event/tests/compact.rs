//! 紧凑布局的端到端事件/几何测试：状态条入口、全屏面板与隐藏控件。

use std::collections::HashMap;

use super::test_support::*;
use super::*;
use crate::app::state::FocusedContainer;
use crate::event::session::Geometry;

fn compact_geometry(state: &mut AppState, config: &Config, width: u16, height: u16) -> Geometry {
    let screen = Rect::new(0, 0, width, height);
    update::sync_compact_layout(state, width, height, config);
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
    let mut rects = if view.main_content.width == 0 || view.main_content.height == 0 {
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

#[test]
fn compact_chat_geometry_gives_full_width_to_panes() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 60, 20);
    assert!(geo.view.compact);
    assert_eq!(geo.view.main_content, Rect::new(0, 2, 60, 18));
    assert_eq!(geo.rects.len(), 1, "only the pane is interactive");
    assert_eq!(geo.rects[0].1.width, 60);
    assert_eq!(
        state.compact_restore,
        Some(crate::app::state::CompactRestore {
            prompt_collapsed: false,
            panes_collapsed: false,
        })
    );
}

#[test]
fn repeated_sync_keeps_original_snapshot() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.prompt_collapsed = true;
    state.panes_collapsed = true;
    let _ = compact_geometry(&mut state, &config, 60, 20);
    let _ = compact_geometry(&mut state, &config, 60, 20);
    assert_eq!(
        state.compact_restore,
        Some(crate::app::state::CompactRestore {
            prompt_collapsed: true,
            panes_collapsed: true,
        })
    );
}

#[test]
fn clicking_switch_opens_fullscreen_switcher() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 60, 20);
    let switch = ui::widgets::compact_header::layout(&geo.view, &state)
        .switch
        .expect("switch");
    assert!(click(&mut state, &config, &geo, switch.x, switch.y));
    assert!(matches!(
        state.overlay,
        Some(crate::app::overlay::Overlay::CompactSwitcher(_))
    ));
}

#[test]
fn clicking_back_returns_to_chat() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.set_focus(FocusedContainer::Prompt);
    let geo = compact_geometry(&mut state, &config, 60, 20);
    assert_eq!(geo.view.main_content.width, 0);
    assert!(geo.rects.iter().all(|(id, _)| *id == state.prompt.id()));
    let back = ui::widgets::compact_header::layout(&geo.view, &state)
        .back
        .expect("back");
    assert!(click(&mut state, &config, &geo, back.x, back.y));
    assert_eq!(state.focus(), FocusedContainer::Panes);
}

#[test]
fn fullscreen_panel_keeps_panes_out_of_geometry() {
    let config = Config::default();
    let mut state = demo_terminal();
    state.set_focus(FocusedContainer::Todo);
    state.right_tab = crate::app::state::RightTab::Todo;
    let geo = compact_geometry(&mut state, &config, 60, 20);
    assert_eq!(geo.view.right_panel, Rect::new(0, 1, 60, 19));
    assert!(geo.rects.iter().all(|(id, _)| *id == state.prompt.id()));
}

#[test]
fn hidden_collapse_button_has_no_hit_area() {
    let config = Config::default();
    let mut state = demo_terminal();
    let _ = compact_geometry(&mut state, &config, 60, 20);
    assert!(
        ui::collapse_buttons(&compact_geometry(&mut state, &config, 60, 20).view, false).is_empty()
    );
    assert!(!ui::prompt_pin_at(
        &compact_geometry(&mut state, &config, 60, 20).view,
        &state,
        56,
        18
    ));
}

#[test]
fn compact_switcher_click_close_dismisses() {
    let config = Config::default();
    let mut state = demo_terminal();
    let geo = compact_geometry(&mut state, &config, 60, 20);
    let switch = ui::widgets::compact_header::layout(&geo.view, &state)
        .switch
        .expect("switch");
    click(&mut state, &config, &geo, switch.x, switch.y);
    let Some(crate::app::overlay::Overlay::CompactSwitcher(switcher)) = state.overlay.as_ref()
    else {
        panic!("switcher open");
    };
    let close = ui::overlay::compact_switcher_layout(geo.screen, switcher)
        .close
        .expect("close");
    click(&mut state, &config, &geo, close.x, close.y);
    assert!(state.overlay.is_none());
}

#[test]
fn compact_toast_click_dismisses_bottom_banner() {
    let config = Config::default();
    let mut state = demo_terminal();
    update::show_toast(
        &mut state,
        Toast::new(
            ToastKind::Default,
            "copied",
            None,
            std::time::Instant::now(),
        ),
    );
    let geo = compact_geometry(&mut state, &config, 60, 20);
    let toast =
        ui::toast::rect(&state, &geo.view, &geo.rects, geo.screen, &config).expect("toast banner");
    assert_eq!(toast.y, 19);
    assert!(click(&mut state, &config, &geo, toast.x, toast.y));
    assert!(state.toast.is_none());
}

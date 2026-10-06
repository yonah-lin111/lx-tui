//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::state::AppState;
use crate::app::update;
use crate::config::Config;

const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: 100,
    height: 24,
};

fn view_for(state: &AppState) -> ViewLayout {
    crate::ui::layout::compute(
        SCREEN,
        &Config::default(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
    )
}

fn state_with_tabs(count: usize) -> AppState {
    let mut state = AppState::demo();
    for _ in 1..count {
        update::create_tab(&mut state);
    }
    state
}

#[test]
fn single_tab_places_add_button_after_tab_without_arrows() {
    let state = AppState::demo();
    let view = view_for(&state);
    let bar = layout(&view, state.active_workspace(), 0);
    assert!(!bar.overflow);
    assert_eq!(bar.scroll_left, None);
    assert_eq!(bar.scroll_right, None);
    let (_, rect) = bar.tabs[0];
    let add = bar.add.expect("add button is visible");
    assert_eq!(add.x, rect.right());
    assert_eq!(add.y, view.tab_bar.y);
}

#[test]
fn overflow_shows_scroll_buttons_and_clips_add_before_exit() {
    let state = state_with_tabs(20);
    let view = view_for(&state);
    let bar = layout(&view, state.active_workspace(), 0);
    assert!(bar.overflow);
    let exit = crate::ui::exit_button(&view).expect("exit button is visible");
    let left = bar.scroll_left.expect("left button");
    let right = bar.scroll_right.expect("right button");
    let add = bar.add.expect("add button");
    assert_eq!(left.x, view.tab_bar.x);
    assert_eq!(right.right(), exit.x);
    assert_eq!(add.right(), right.x);
    let last_visible = bar.tabs.last().expect("visible tabs");
    assert!(last_visible.1.right() <= add.x);
}

#[test]
fn max_scroll_reveals_last_tab_and_clamps_overflow() {
    let state = state_with_tabs(20);
    let view = view_for(&state);
    let bar = layout(&view, state.active_workspace(), 0);
    assert!(bar.max_scroll > 0);
    let scrolled = layout(&view, state.active_workspace(), bar.max_scroll);
    assert_eq!(scrolled.scroll, bar.max_scroll);
    let last_index = state.active_workspace().tabs.len() - 1;
    assert_eq!(
        scrolled.tabs.last().map(|(index, _)| *index),
        Some(last_index)
    );
    let beyond = layout(&view, state.active_workspace(), bar.max_scroll + 99);
    assert_eq!(beyond.scroll, bar.max_scroll);
}

#[test]
fn tab_at_hits_visible_tabs_and_separator() {
    let state = AppState::demo();
    let view = view_for(&state);
    let bar = layout(&view, state.active_workspace(), 0);
    let (index, rect) = bar.tabs[0];
    assert_eq!(tab_at(&bar, rect.x, rect.y), Some(index));
    assert_eq!(tab_at(&bar, rect.right() - 1, rect.y), Some(index));
    assert_eq!(tab_at(&bar, rect.right(), rect.y), None);
    assert_eq!(tab_at(&bar, rect.x, rect.y + 1), None);
}

#[test]
fn reveal_scroll_jumps_to_clipped_active_tab() {
    let state = state_with_tabs(20);
    let view = view_for(&state);
    let bar = layout(&view, state.active_workspace(), 0);
    let target = reveal_scroll(&bar, state.active_workspace());
    assert!(target > 0);
    let after = layout(&view, state.active_workspace(), target);
    let active = state.active_workspace().active_tab;
    assert!(
        after
            .tabs
            .iter()
            .any(|(index, rect)| { *index == active && rect.width == after.tab_widths[active] })
    );
}

#[test]
fn reveal_scroll_keeps_visible_active_untouched() {
    let state = AppState::demo();
    let view = view_for(&state);
    let bar = layout(&view, state.active_workspace(), 0);
    assert_eq!(reveal_scroll(&bar, state.active_workspace()), 0);
}

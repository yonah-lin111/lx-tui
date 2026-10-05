//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::actions::Action;
use crate::app::update;
use std::time::Instant;

fn view_for(state: &AppState) -> ViewLayout {
    crate::ui::layout::compute(
        Rect::new(0, 0, 100, 24),
        &Config::default(),
        state.sidebar_collapsed,
        state.prompt_collapsed,
        state.prompt_width,
    )
}

fn pane_rects(state: &AppState, view: &ViewLayout) -> Vec<(PaneId, Rect)> {
    crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        Config::default().min_pane_width,
    )
}

fn show(state: &mut AppState, anchor: Option<PaneId>) {
    update::show_toast(
        state,
        Toast::new(ToastKind::Info, text::TOAST_COPIED, anchor, Instant::now()),
    );
}

#[test]
fn anchored_toast_centers_below_container_top_border() {
    let mut state = AppState::demo();
    let prompt = state.prompt.id();
    show(&mut state, Some(prompt));
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let rect = rect(
        &state,
        &view,
        &rects,
        Rect::new(0, 0, 100, 24),
        &Config::default(),
    )
    .expect("toast area is resolvable");
    assert_eq!(rect.y, view.prompt.y + 1);
    assert_eq!(rect.x, view.prompt.x + (view.prompt.width - rect.width) / 2);
    assert_eq!(rect.height, 3);
    assert_eq!(
        usize::from(rect.width),
        text::TOAST_COPIED.chars().count() + 4
    );
}

#[test]
fn pane_anchored_toast_targets_that_pane() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    show(&mut state, Some(pane));
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let Some((_, container)) = rects.iter().find(|(id, _)| *id == pane) else {
        panic!("active pane has a rect");
    };
    let rect = rect(
        &state,
        &view,
        &rects,
        Rect::new(0, 0, 100, 24),
        &Config::default(),
    )
    .expect("toast area is resolvable");
    assert_eq!(rect.y, container.y + 1);
    assert_eq!(rect.x, container.x + (container.width - rect.width) / 2);
}

#[test]
fn narrow_container_falls_back_to_screen_top_center() {
    let mut state = AppState::demo();
    let prompt = state.prompt.id();
    update::apply(Action::TogglePrompt, &mut state);
    show(&mut state, Some(prompt));
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let rect = rect(
        &state,
        &view,
        &rects,
        Rect::new(0, 0, 100, 24),
        &Config::default(),
    )
    .expect("toast area is resolvable");
    assert_eq!(rect.y, 1);
    assert_eq!(rect.x, (100 - rect.width) / 2);
}

#[test]
fn missing_anchor_falls_back_to_screen_top_center() {
    let mut state = AppState::demo();
    show(&mut state, None);
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let rect = rect(
        &state,
        &view,
        &rects,
        Rect::new(0, 0, 100, 24),
        &Config::default(),
    )
    .expect("toast area is resolvable");
    assert_eq!(rect.y, 1);
    assert_eq!(rect.x, (100 - rect.width) / 2);
}

#[test]
fn below_minimum_screen_has_no_toast() {
    let mut state = AppState::demo();
    show(&mut state, None);
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    assert!(
        rect(
            &state,
            &view,
            &rects,
            Rect::new(0, 0, 30, 8),
            &Config::default()
        )
        .is_none()
    );
}

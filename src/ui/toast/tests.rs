//! 单元测试；仅测试构建编译。

use super::*;
use crate::app::actions::Action;
use crate::app::update;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use std::time::Instant;

fn view_for(state: &AppState) -> ViewLayout {
    crate::ui::layout::compute(
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

fn pane_rects(state: &AppState, view: &ViewLayout) -> Vec<(PaneId, Rect)> {
    crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        Config::default().min_pane_width,
        Config::default().min_pane_height,
    )
}

fn show(state: &mut AppState, anchor: Option<PaneId>) {
    update::show_toast(
        state,
        Toast::new(
            ToastKind::Default,
            text::TOAST_COPIED,
            anchor,
            Instant::now(),
        ),
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
fn titled_toast_keeps_three_rows_and_fits_title() {
    let mut state = AppState::demo();
    update::show_toast(
        &mut state,
        Toast::new(ToastKind::Default, text::TOAST_COPIED, None, Instant::now())
            .with_title(text::TOAST_CLIPBOARD_TITLE),
    );
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
    assert_eq!(rect.height, 3);
    assert_eq!(
        usize::from(rect.width),
        text::TOAST_COPIED.chars().count() + 4
    );
}

#[test]
fn toast_paints_overlay_panel_background() {
    let mut state = AppState::demo();
    show(&mut state, None);
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let config = Config::default();
    let area = rect(&state, &view, &rects, Rect::new(0, 0, 100, 24), &config)
        .expect("toast area is resolvable");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24))
        .expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| crate::ui::render(frame, &state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    assert_eq!(
        buffer[(area.x, area.y)].bg,
        ratatui::style::Color::Indexed(236)
    );
    assert_eq!(
        buffer[(area.x + 1, area.y + 1)].bg,
        ratatui::style::Color::Indexed(236)
    );
    // 整块面板不透明：边框、正文与关闭按钮的所有单元格都铺浮层底色。
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            assert_eq!(
                buffer[(x, y)].bg,
                ratatui::style::Color::Indexed(236),
                "toast 单元格 ({x},{y}) 透出底层"
            );
        }
    }
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

#[test]
fn toast_renders_close_button_and_hit_test() {
    let mut state = AppState::demo();
    show(&mut state, None);
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let screen = Rect::new(0, 0, 100, 24);
    let area = rect(&state, &view, &rects, screen, &Config::default()).expect("toast area");
    let close = crate::ui::widgets::modal::close_button_rect(area).expect("close button fits");
    assert!(close_button_at(area, close.x, close.y));
    assert!(
        !close_button_at(area, area.x + 1, area.y),
        "内容区不命中关闭按钮"
    );

    let mut terminal =
        Terminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    terminal
        .draw(|frame| {
            render(frame, screen, &state, &view, &rects, &Config::default());
        })
        .expect("draw succeeds");
    let buffer = terminal.backend().buffer();
    let label: String = (close.x..close.right())
        .map(|x| buffer[(x, close.y)].symbol())
        .collect();
    assert_eq!(label, text::MODAL_CLOSE);
}

/// 三档语义色：default 强调青、warning 黄、danger 红；**边框与标题同色**。
#[test]
fn kinds_drive_border_and_title_colors() {
    assert_eq!(border_style(ToastKind::Default), style::accent());
    assert_eq!(border_style(ToastKind::Warning).fg, Some(Color::Yellow));
    assert_eq!(border_style(ToastKind::Danger), style::error());

    // 标题：默认档沿用边框标题样式（青灰），warning/danger 与边框同色。
    assert_eq!(title_style(ToastKind::Default), style::border_title());
    assert_eq!(title_style(ToastKind::Warning).fg, Some(Color::Yellow));
    assert_eq!(title_style(ToastKind::Danger).fg, Some(Color::Red));
    assert_eq!(
        title_style(ToastKind::Warning).fg,
        border_style(ToastKind::Warning).fg,
        "warning 的边框与标题同色"
    );
    assert_eq!(
        title_style(ToastKind::Danger).fg,
        border_style(ToastKind::Danger).fg,
        "danger 的边框与标题同色"
    );
}

/// 渲染：warning toast 的边框与标题都落黄色单元格。
#[test]
fn warning_toast_paints_yellow_border_and_title() {
    let mut state = AppState::demo();
    let pane = state.prompt.id();
    update::show_toast(
        &mut state,
        Toast::new(ToastKind::Warning, "watch out", Some(pane), Instant::now())
            .with_title("Heads up"),
    );
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let area = rect(
        &state,
        &view,
        &rects,
        Rect::new(0, 0, 100, 24),
        &Config::default(),
    )
    .expect("toast area");
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("terminal");
    terminal
        .draw(|frame| {
            render(
                frame,
                Rect::new(0, 0, 100, 24),
                &state,
                &view,
                &rects,
                &Config::default(),
            );
        })
        .expect("draw");
    let buffer = terminal.backend().buffer();
    // 顶边框与标题里的字符都应是黄色。
    let yellow = (area.x..area.right())
        .filter(|x| buffer[(*x, area.y)].fg == Color::Yellow)
        .count();
    assert!(yellow > 0, "warning 的边框/标题变黄");
}

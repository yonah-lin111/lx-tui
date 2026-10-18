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
fn titled_toast_keeps_three_rows_and_fits_title() {
    let mut state = AppState::demo();
    update::show_toast(
        &mut state,
        Toast::new(ToastKind::Info, text::TOAST_COPIED, None, Instant::now())
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

/// 警告档 toast 的边框必须是**黄色**（`agent_warn`），与 Info 的青、Error 的红分开。
///
/// 用户要求：输入并发送了不是命令面板里的 `/xxx` 时，用警告性提示（黄色）。
#[test]
fn warning_toast_border_is_yellow() {
    assert_eq!(
        border_style(ToastKind::Warning),
        crate::ui::style::agent_warn(),
        "警告档边框 = 聊天历史里「中断」同一档黄"
    );
    // 三档互不相同，避免再次退化成「警告也红/也青」。
    let info = border_style(ToastKind::Info);
    let warning = border_style(ToastKind::Warning);
    let error = border_style(ToastKind::Error);
    assert_ne!(warning, info);
    assert_ne!(warning, error);
    assert_ne!(info, error);
}

/// 未知命令 toast 端到端：整块画出来，边框每个角都是黄色。
#[test]
fn unknown_command_toast_paints_yellow_border_and_echoes_command() {
    let mut state = AppState::demo();
    update::show_toast(
        &mut state,
        Toast::new(
            ToastKind::Warning,
            text::chat::unknown_command("/foo 参数"),
            None,
            Instant::now(),
        ),
    );
    let mut terminal = Terminal::new(TestBackend::new(120, 24)).expect("backend");
    terminal
        .draw(|frame| {
            let screen = frame.area();
            crate::ui::render(frame, &state, &Config::default());
            let _ = screen;
        })
        .expect("draw");

    let buffer = terminal.backend().buffer();
    let view = view_for(&state);
    let rects = pane_rects(&state, &view);
    let area = rect(
        &state,
        &view,
        &rects,
        Rect::new(0, 0, 120, 24),
        &Config::default(),
    )
    .expect("toast area");

    // 四角与上下边都取警告黄。
    for (x, y) in [
        (area.x, area.y),
        (area.right() - 1, area.y),
        (area.x, area.bottom() - 1),
        (area.right() - 1, area.bottom() - 1),
    ] {
        assert_eq!(
            buffer[(x, y)].fg,
            crate::ui::style::agent_warn()
                .fg
                .unwrap_or(ratatui::style::Color::Reset),
            "边框格 ({x},{y}) 应为警告黄"
        );
    }
    // 正文回显用户敲的命令，并给出可用命令提示。
    let inner: String = (area.x..area.right())
        .map(|x| buffer[(x, area.y + 1)].symbol())
        .collect();
    assert!(inner.contains("/foo"), "回显命令：{inner}");
    assert!(inner.contains("/model"), "提示可用命令：{inner}");
}

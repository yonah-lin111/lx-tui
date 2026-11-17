//! `ui::controls` 单元测试：侧栏底边框 `[WEB]` 按钮几何、宽度门槛与悬浮提示。

use ratatui::Terminal as RatatuiTerminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::AppState;
use crate::app::tooltip::TooltipTarget;
use crate::ui::{layout, render, text};

fn render_lines(state: &AppState) -> Vec<String> {
    let config = crate::ui::test_support::desktop_config();
    let mut terminal =
        RatatuiTerminal::new(TestBackend::new(100, 24)).expect("test backend is infallible");
    if let Err(error) = terminal.draw(|frame| render(frame, state, &config)) {
        panic!("draw failed: {error}");
    }
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

fn view_for(state: &AppState) -> layout::ViewLayout {
    layout::compute(
        Rect::new(0, 0, 100, 24),
        &crate::ui::test_support::desktop_config(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    )
}

#[test]
fn web_button_renders_and_hits() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let view = view_for(&state);
    let web = web_button(&view, &state).expect("web button fits in the sidebar");
    assert_eq!(web.x, view.left_panel.x + 1);
    assert_eq!(web.y, view.left_panel.bottom() - 1);
    assert_eq!(web.width, 5);
    assert_eq!(web.height, 1);
    for column in web.x..web.right() {
        assert!(web_button_at(&view, &state, column, web.y));
    }
    assert!(!web_button_at(&view, &state, web.x - 1, web.y));
    assert!(!web_button_at(&view, &state, web.right(), web.y));
    assert!(!web_button_at(
        &view,
        &state,
        web.x,
        web.y.saturating_sub(1)
    ));

    let lines = render_lines(&state);
    let bottom: Vec<char> = lines[web.y as usize]
        .chars()
        .skip(usize::from(view.left_panel.x))
        .take(usize::from(view.left_panel.width))
        .collect();
    assert_eq!(bottom[1..6].iter().collect::<String>(), text::WEB_LABEL);

    // 折叠态或服务不可用时隐藏。
    state.sidebar_collapsed = true;
    let collapsed_view = view_for(&state);
    assert!(web_button(&collapsed_view, &state).is_none());
    assert!(!web_button_at(&collapsed_view, &state, web.x, web.y));
    state.sidebar_collapsed = false;
    state.settings_available = false;
    let unavailable = view_for(&state);
    assert!(web_button(&unavailable, &state).is_none());
    assert!(!web_button_at(&unavailable, &state, web.x, web.y));
}

#[test]
fn web_button_width_threshold_is_eleven() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let mut view = view_for(&state);

    view.left_panel = Rect::new(0, 0, 10, 24);
    assert!(
        web_button(&view, &state).is_none(),
        "宽度 10 时 [WEB] 必须隐藏"
    );
    assert!(!web_button_at(&view, &state, 1, 23));

    view.left_panel = Rect::new(0, 0, 11, 24);
    assert!(
        web_button(&view, &state).is_some(),
        "宽度 11 时 [WEB] 恰好显示"
    );
    assert!(web_button_at(&view, &state, 1, 23));
}

/// 侧栏底边框左端渲染 `[WEB]`，`[WEB]` 右侧至折叠按钮之间只剩底边线。
#[test]
fn sidebar_bottom_border_has_web_button() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let view = view_for(&state);
    let web = web_button(&view, &state).expect("web button fits");
    let collapse = collapse_buttons(&view, state.agents_collapsed)
        .into_iter()
        .find(|button| button.target == CollapseTarget::Sidebar)
        .expect("sidebar collapse button visible");
    let lines = render_lines(&state);
    let bottom: Vec<char> = lines[web.y as usize]
        .chars()
        .skip(usize::from(view.left_panel.x))
        .take(usize::from(view.left_panel.width))
        .collect();
    let bottom_str: String = bottom.iter().collect();

    assert_eq!(
        bottom[1..6].iter().collect::<String>(),
        text::WEB_LABEL,
        "{bottom_str}"
    );
    let after_web = usize::from(web.right() - view.left_panel.x);
    let collapse_start = usize::from(collapse.area.x - view.left_panel.x);
    let tail: String = bottom[after_web..collapse_start].iter().collect();
    assert!(
        !tail.is_empty(),
        "`[WEB]` 与折叠按钮之间应有底边线: {bottom_str}"
    );
    assert!(
        tail.chars().all(|c| c == '─'),
        "`[WEB]` 右侧至折叠按钮之间应全为 `─`: {bottom_str}"
    );
    assert!(
        !bottom_str.contains("[SET]") && !bottom_str.contains("[USE]"),
        "不应再出现旧按钮: {bottom_str}"
    );
}

/// 侧栏底边框 tooltip 命中 `[WEB]`，且按钮右侧至折叠按钮之间无多余目标。
#[test]
fn tooltip_target_points_at_web_button() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let view = view_for(&state);
    let rects = crate::tiling::pane_rects(
        &state.active_tab().layout,
        view.main_content,
        crate::ui::test_support::desktop_config().min_pane_width,
        crate::ui::test_support::desktop_config().min_pane_height,
    );
    let screen = Rect::new(0, 0, 100, 24);
    let web = web_button(&view, &state).expect("web button fits");
    let collapse = collapse_buttons(&view, state.agents_collapsed)
        .into_iter()
        .find(|button| button.target == CollapseTarget::Sidebar)
        .expect("sidebar collapse button visible");

    assert_eq!(
        text::tooltip_hint(TooltipTarget::SidebarWeb, &state),
        "open web"
    );
    for column in web.x..web.right() {
        assert_eq!(
            crate::ui::tooltip::target_at(&view, &state, &rects, screen, column, web.y),
            Some(TooltipTarget::SidebarWeb)
        );
    }
    let mut column = web.right();
    while column < collapse.area.x {
        assert_eq!(
            crate::ui::tooltip::target_at(&view, &state, &rects, screen, column, web.y),
            None,
            "侧栏底边框第 {column} 列不应有 tooltip 目标（[WEB] 右侧无按钮）"
        );
        column += 1;
    }
}

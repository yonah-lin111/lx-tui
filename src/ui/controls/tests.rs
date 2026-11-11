//! `ui::controls` 单元测试：侧栏底边框 `[SET]`、`[USE]` 按钮几何、宽度门槛与悬浮提示。

use ratatui::Terminal as RatatuiTerminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;

use super::*;
use crate::app::state::AppState;
use crate::app::tooltip::TooltipTarget;
use crate::config::Config;
use crate::ui::{layout, render, text};

fn render_lines(state: &AppState) -> Vec<String> {
    let config = Config::default();
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
        &Config::default(),
        state.sidebar_collapsed,
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
        state.prompt_left,
        state.panes_collapsed,
    )
}

#[test]
fn usage_button_renders_right_of_settings_and_hits() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let view = view_for(&state);
    let settings = settings_button(&view, &state).expect("settings fits in the sidebar");
    let usage = usage_button(&view, &state).expect("usage fits in the sidebar");
    assert_eq!(usage.y, view.left_panel.bottom() - 1);
    assert_eq!(usage.x, view.left_panel.x + 7);
    assert_eq!(usage.width, 5);
    assert_eq!(usage.right(), view.left_panel.x + 12);
    assert_eq!(usage.x, settings.right() + 1);
    for x in usage.x..usage.right() {
        assert!(usage_button_at(&view, &state, x, usage.y));
    }
    assert!(!usage_button_at(&view, &state, usage.x - 1, usage.y));
    assert!(!usage_button_at(&view, &state, usage.right(), usage.y));
    assert!(!usage_button_at(
        &view,
        &state,
        usage.x,
        usage.y.saturating_sub(1)
    ));

    let lines = render_lines(&state);
    let bottom: Vec<char> = lines[usage.y as usize]
        .chars()
        .skip(usize::from(view.left_panel.x))
        .take(usize::from(view.left_panel.width))
        .collect();
    assert_eq!(bottom[6], '─', "[SET] 与 [USE] 之间保留 1 列底边线");
    let rendered_usage: String = bottom[7..12].iter().collect();
    assert_eq!(rendered_usage, text::USAGE_LABEL);

    // 折叠态、服务不可用或宽度 < 17 时隐藏。
    state.sidebar_collapsed = true;
    let collapsed_view = view_for(&state);
    assert!(usage_button(&collapsed_view, &state).is_none());
    assert!(!usage_button_at(&collapsed_view, &state, usage.x, usage.y));
    state.sidebar_collapsed = false;
    state.settings_available = false;
    let unavailable = view_for(&state);
    assert!(usage_button(&unavailable, &state).is_none());
    assert!(!usage_button_at(&unavailable, &state, usage.x, usage.y));
}

#[test]
fn usage_button_width_threshold_is_seventeen() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let mut view = view_for(&state);

    view.left_panel = Rect::new(0, 0, 16, 24);
    assert!(
        settings_button(&view, &state).is_some(),
        "宽度 16 时 [SET] 仍显示"
    );
    assert!(
        usage_button(&view, &state).is_none(),
        "宽度 16 时 [USE] 必须隐藏"
    );
    assert!(!usage_button_at(&view, &state, 7, 23));

    view.left_panel = Rect::new(0, 0, 17, 24);
    assert!(
        settings_button(&view, &state).is_some(),
        "宽度 17 时 [SET] 显示"
    );
    assert!(
        usage_button(&view, &state).is_some(),
        "宽度 17 时 [USE] 恰好显示"
    );
    assert!(usage_button_at(&view, &state, 7, 23));
}

/// 侧栏底边框 `[SET]` 右侧隔 1 列渲染 `[USE]`，`[USE]` 右侧至折叠按钮之间只剩底边线。
#[test]
fn sidebar_bottom_border_has_no_button_after_settings() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let view = view_for(&state);
    let settings = settings_button(&view, &state).expect("settings fits");
    let usage = usage_button(&view, &state).expect("usage fits");
    let collapse = collapse_buttons(&view, state.agents_collapsed)
        .into_iter()
        .find(|button| button.target == CollapseTarget::Sidebar)
        .expect("sidebar collapse button visible");
    let lines = render_lines(&state);
    let bottom: Vec<char> = lines[settings.y as usize]
        .chars()
        .skip(usize::from(view.left_panel.x))
        .take(usize::from(view.left_panel.width))
        .collect();
    let bottom_str: String = bottom.iter().collect();

    assert_eq!(
        bottom[1..6].iter().collect::<String>(),
        text::SETTINGS_LABEL,
        "{bottom_str}"
    );
    assert_eq!(
        bottom[6], '─',
        "[SET] 与 [USE] 之间应为 1 列 `─`: {bottom_str}"
    );
    assert_eq!(
        bottom[7..12].iter().collect::<String>(),
        text::USAGE_LABEL,
        "{bottom_str}"
    );
    let after_usage_start = usize::from(usage.right() - view.left_panel.x);
    let collapse_start = usize::from(collapse.area.x - view.left_panel.x);
    let tail: String = bottom[after_usage_start..collapse_start].iter().collect();
    assert!(
        !tail.is_empty(),
        "`[USE]` 与折叠按钮之间应有底边线: {bottom_str}"
    );
    assert!(
        tail.chars().all(|c| c == '─'),
        "`[USE]` 右侧至折叠按钮之间应全为 `─`: {bottom_str}"
    );
    assert!(
        !bottom_str.contains("[UI]"),
        "不应再出现 [UI]: {bottom_str}"
    );
}

/// 侧栏底边框 tooltip 命中 `[SET]` 与 `[USE]`，且 `[USE]` 右侧至折叠按钮之间无多余目标。
#[test]
fn no_tooltip_target_points_at_the_removed_ui_button() {
    let mut state = AppState::demo();
    state.sidebar_collapsed = false;
    state.settings_available = true;
    let view = view_for(&state);
    let rects = crate::tiling::pane_rects(
        &state.active_tab().layout,
        view.main_content,
        Config::default().min_pane_width,
        Config::default().min_pane_height,
    );
    let screen = Rect::new(0, 0, 100, 24);
    let settings = settings_button(&view, &state).expect("settings fits");
    let usage = usage_button(&view, &state).expect("usage fits");
    let collapse = collapse_buttons(&view, state.agents_collapsed)
        .into_iter()
        .find(|button| button.target == CollapseTarget::Sidebar)
        .expect("sidebar collapse button visible");

    assert_eq!(
        text::tooltip_hint(TooltipTarget::SidebarSettings, &state),
        "open settings"
    );
    assert_eq!(
        text::tooltip_hint(TooltipTarget::SidebarUsage, &state),
        "open usage"
    );
    for column in settings.x..settings.right() {
        assert_eq!(
            crate::ui::tooltip::target_at(&view, &state, &rects, screen, column, settings.y),
            Some(TooltipTarget::SidebarSettings)
        );
    }
    assert_eq!(
        crate::ui::tooltip::target_at(&view, &state, &rects, screen, settings.right(), settings.y),
        None,
        "[SET] 与 [USE] 之间的间隔列不应有 tooltip 目标"
    );
    for column in usage.x..usage.right() {
        assert_eq!(
            crate::ui::tooltip::target_at(&view, &state, &rects, screen, column, usage.y),
            Some(TooltipTarget::SidebarUsage)
        );
    }
    let mut column = usage.right();
    while column < collapse.area.x {
        assert_eq!(
            crate::ui::tooltip::target_at(&view, &state, &rects, screen, column, settings.y),
            None,
            "侧栏底边框第 {column} 列不应有 tooltip 目标（[USE] 右侧无按钮）"
        );
        column += 1;
    }
}

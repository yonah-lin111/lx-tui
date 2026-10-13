//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn normal_width_shows_all_regions() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(view.sidebar.width, config.sidebar_width);
    assert_eq!(view.sidebar.height, 30);
    assert_eq!(view.tab_bar.height, 1);
    assert_eq!(view.tab_bar, Rect::new(24, 0, 76, 1));
    assert_eq!(view.prompt, Rect::new(70, 1, 30, 29));
    assert_eq!(view.panes.width, 100 - config.sidebar_width - 30);
    assert_eq!(view.panes.height, 29);
    assert_eq!(view.panes.bottom(), 30);
}

#[test]
fn tab_bar_spans_main_area_and_prompt() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(view.tab_bar.right(), view.prompt.right());
    assert_eq!(view.prompt.y, view.panes.y);
    assert_eq!(view.prompt.height, view.panes.height);
    assert_eq!(view.prompt.bottom(), view.panes.bottom());
}

#[test]
fn narrow_width_hides_sidebar_but_keeps_prompt() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 60, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(view.sidebar.width, 0);
    assert_eq!(view.prompt.width, 30);
    assert_eq!(view.panes.width, 30);
}

#[test]
fn collapsed_sidebar_keeps_strip() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 120, 30),
        &config,
        true,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(view.sidebar.width, COLLAPSED_STRIP);
    assert_eq!(view.prompt.width, 30);
    assert_eq!(view.panes.width, 120 - COLLAPSED_STRIP - 30);
}

#[test]
fn collapsed_narrow_sidebar_is_hidden() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 60, 30),
        &config,
        true,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(view.sidebar.width, 0);
    assert_eq!(view.prompt.width, 30);
}

#[test]
fn collapsed_prompt_becomes_right_strip() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        true,
        30,
        false,
        false,
    );
    assert_eq!(view.prompt, Rect::new(96, 1, COLLAPSED_STRIP, 29));
    assert_eq!(
        view.panes.width,
        100 - config.sidebar_width - COLLAPSED_STRIP
    );
}

#[test]
fn collapsing_prompt_keeps_stored_width() {
    let config = Config::default();
    let collapsed = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        true,
        42,
        false,
        false,
    );
    let expanded = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        42,
        false,
        false,
    );
    assert_eq!(collapsed.prompt.width, COLLAPSED_STRIP);
    assert_eq!(expanded.prompt.width, 42);
}

#[test]
fn prompt_width_is_clamped_both_sides() {
    let config = Config::default();
    let too_narrow = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        5,
        false,
        false,
    );
    assert_eq!(too_narrow.prompt.width, config.min_pane_width);
    let too_wide = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        90,
        false,
        false,
    );
    assert_eq!(
        too_wide.prompt.width,
        100 - config.sidebar_width - config.min_pane_width
    );
}

#[test]
fn prompt_splits_evenly_when_area_is_tiny() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 10, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(view.prompt.width, 5);
    assert_eq!(view.panes.width, 5);
}

#[test]
fn tiny_area_does_not_overflow() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 10, 2),
        &config,
        false,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(view.panes.height, 1);
    assert_eq!(view.panes.bottom(), 2);
    assert_eq!(view.prompt.bottom(), 2);
}

#[test]
fn sidebar_sections_split_at_vertical_middle_when_expanded() {
    let sidebar = Rect::new(0, 0, 24, 24);
    let sections = sidebar_sections(sidebar, false).expect("sections are visible");
    assert_eq!(sections.workspaces, Rect::new(1, 1, 22, 11));
    assert_eq!(sections.divider, Rect::new(1, 12, 22, 1));
    assert_eq!(sections.agents, Rect::new(1, 13, 22, 10));
}

#[test]
fn sidebar_sections_dock_divider_to_bottom_when_agents_collapsed() {
    let sidebar = Rect::new(0, 0, 24, 24);
    let sections = sidebar_sections(sidebar, true).expect("sections are visible");
    assert_eq!(sections.workspaces.height, 21);
    assert_eq!(sections.divider.y, 22);
    assert_eq!(sections.agents.height, 0);
    assert_eq!(sections.divider.bottom(), sidebar.bottom() - 1);
}

#[test]
fn sidebar_sections_need_room_for_content() {
    assert_eq!(sidebar_sections(Rect::new(0, 0, 2, 24), false), None);
    assert_eq!(sidebar_sections(Rect::new(0, 0, 24, 3), false), None);
}

#[test]
fn default_side_width_is_two_fifths_of_main_area() {
    let config = Config::default();
    let area = Rect::new(0, 0, 100, 30);
    assert_eq!(default_side_width(area, &config, false), 30);
    assert_eq!(
        default_side_width(area, &config, true),
        (100 - COLLAPSED_STRIP) * 2 / 5
    );
    assert_eq!(
        default_side_width(Rect::new(0, 0, 60, 30), &config, false),
        24
    );
}

#[test]
fn side_width_is_two_fifths_and_splits_when_tiny() {
    assert_eq!(side_width(76, 10), 30);
    assert_eq!(side_width(9, 10), 4);
}

#[test]
fn prompt_width_at_follows_boundary_and_clamps() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(prompt_width_at(&view, 70, config.min_pane_width), 30);
    assert_eq!(prompt_width_at(&view, 90, config.min_pane_width), 10);
    assert_eq!(
        prompt_width_at(&view, 0, config.min_pane_width),
        100 - config.sidebar_width - config.min_pane_width
    );
}

#[test]
fn sidebar_width_is_clamped_to_config_bounds() {
    let config = Config::default();
    let too_narrow = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        5,
        false,
        30,
        false,
        false,
    );
    assert_eq!(too_narrow.sidebar.width, config.sidebar_min_width);
    let too_wide = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        99,
        false,
        30,
        false,
        false,
    );
    assert_eq!(too_wide.sidebar.width, config.sidebar_max_width);
    let collapsed = compute(
        Rect::new(0, 0, 120, 30),
        &config,
        true,
        99,
        false,
        30,
        false,
        false,
    );
    assert_eq!(collapsed.sidebar.width, COLLAPSED_STRIP);
}

#[test]
fn sidebar_width_at_follows_boundary_and_keeps_pane_minimum() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        30,
        false,
        false,
    );
    assert_eq!(sidebar_width_at(&view, &config, 23), config.sidebar_width);
    assert_eq!(sidebar_width_at(&view, &config, 30), 31);
    assert_eq!(
        sidebar_width_at(&view, &config, 60),
        config.sidebar_max_width
    );
    assert_eq!(
        sidebar_width_at(&view, &config, 0),
        config.sidebar_min_width
    );

    // 主区只剩 10 列时上限收缩到总宽 - prompt - min_pane_width。
    let tight = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        24,
        false,
        60,
        false,
        false,
    );
    assert_eq!(sidebar_width_at(&tight, &config, 99), 30);
}

#[test]
fn prompt_left_places_prompt_next_to_sidebar() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        46,
        true,
        false,
    );
    assert!(view.prompt_left);
    assert_eq!(view.tab_bar, Rect::new(24, 0, 76, 1));
    assert_eq!(view.prompt, Rect::new(24, 1, 46, 29));
    assert_eq!(view.panes, Rect::new(70, 1, 30, 29));
}

#[test]
fn collapsed_panes_dock_to_right_edge() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        46,
        true,
        true,
    );
    assert_eq!(view.panes, Rect::new(96, 1, COLLAPSED_STRIP, 29));
    assert_eq!(view.prompt, Rect::new(24, 1, 72, 29));
}

#[test]
fn collapsed_prompt_ignored_when_prompt_left() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        true,
        46,
        true,
        false,
    );
    // 位置模型：prompt 在左不可折叠，折叠标记失效。
    assert_eq!(view.prompt, Rect::new(24, 1, 46, 29));
}

#[test]
fn prompt_width_at_mirrors_for_left_prompt() {
    let config = Config::default();
    let view = compute(
        Rect::new(0, 0, 100, 30),
        &config,
        false,
        config.sidebar_width,
        false,
        46,
        true,
        false,
    );
    // 分隔线两列：69（prompt 右边框，映射为当前宽度）与 70（窗格左边框）。
    assert_eq!(prompt_width_at(&view, 69, config.min_pane_width), 46);
    assert_eq!(prompt_width_at(&view, 40, config.min_pane_width), 17);
    assert_eq!(
        prompt_width_at(&view, 99, config.min_pane_width),
        100 - config.sidebar_width - config.min_pane_width
    );
}

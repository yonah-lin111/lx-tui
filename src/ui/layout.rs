//! 屏幕区域划分：纯函数，输入终端矩形与状态。

use ratatui::layout::Rect;

use crate::config::Config;
use crate::layout::COLLAPSED_STRIP;

/// 主界面四个区域的矩形。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewLayout {
    pub sidebar: Rect,
    pub tab_bar: Rect,
    pub panes: Rect,
    pub prompt: Rect,
}

/// 划分主界面；侧栏折叠且宽度充足时保留窄条，右栏 prompt 始终可见。
pub fn compute(
    area: Rect,
    config: &Config,
    sidebar_collapsed: bool,
    prompt_collapsed: bool,
    prompt_width: u16,
) -> ViewLayout {
    let sidebar_width = sidebar_width(area, config, sidebar_collapsed);
    let sidebar = Rect {
        width: sidebar_width,
        ..area
    };
    let available = area.width.saturating_sub(sidebar_width);
    let prompt_width = if prompt_collapsed {
        COLLAPSED_STRIP.min(available)
    } else {
        clamp_prompt_width(available, prompt_width, config.min_pane_width)
    };
    let prompt = Rect {
        x: area.x + area.width - prompt_width,
        width: prompt_width,
        ..area
    };
    let main = Rect {
        x: area.x + sidebar_width,
        width: available.saturating_sub(prompt_width),
        ..area
    };
    let tab_bar = Rect {
        height: 1.min(main.height),
        ..main
    };
    let panes = Rect {
        x: main.x,
        y: main.y + tab_bar.height,
        width: main.width,
        height: main.height.saturating_sub(tab_bar.height),
    };

    ViewLayout {
        sidebar,
        tab_bar,
        panes,
        prompt,
    }
}

/// 启动默认右栏宽度：主区可用宽度的一半，复刻旧 50% 分割的版面。
pub fn default_prompt_width(area: Rect, config: &Config, sidebar_collapsed: bool) -> u16 {
    let available = area
        .width
        .saturating_sub(sidebar_width(area, config, sidebar_collapsed));
    clamp_prompt_width(available, available / 2, config.min_pane_width)
}

/// 拖拽分割线到屏幕列 `boundary_x` 时右栏应有的宽度（屏幕坐标）。
pub fn prompt_width_at(view: &ViewLayout, boundary_x: u16, min_width: u16) -> u16 {
    let screen_right = view.prompt.right();
    let available = screen_right.saturating_sub(view.sidebar.width);
    clamp_prompt_width(
        available,
        screen_right.saturating_sub(boundary_x),
        min_width,
    )
}

/// 左栏宽度：展开取配置值，折叠且宽度充足取窄条，否则隐藏。
fn sidebar_width(area: Rect, config: &Config, sidebar_collapsed: bool) -> u16 {
    let wide_enough = area.width >= config.narrow_width;
    if !sidebar_collapsed && wide_enough {
        config.sidebar_width.min(area.width)
    } else if sidebar_collapsed && wide_enough {
        COLLAPSED_STRIP
    } else {
        0
    }
}

/// 右栏宽度约束：两侧各保持至少 `min_width`；空间不足时均分。
fn clamp_prompt_width(available: u16, width: u16, min_width: u16) -> u16 {
    if available >= min_width.saturating_mul(2) {
        width.clamp(min_width, available - min_width)
    } else {
        available / 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_width_shows_all_regions() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 100, 30), &config, false, false, 30);
        assert_eq!(view.sidebar.width, config.sidebar_width);
        assert_eq!(view.sidebar.height, 30);
        assert_eq!(view.tab_bar.height, 1);
        assert_eq!(view.prompt, Rect::new(70, 0, 30, 30));
        assert_eq!(view.panes.width, 100 - config.sidebar_width - 30);
        assert_eq!(view.panes.height, 29);
        assert_eq!(view.panes.bottom(), 30);
    }

    #[test]
    fn narrow_width_hides_sidebar_but_keeps_prompt() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 60, 30), &config, false, false, 30);
        assert_eq!(view.sidebar.width, 0);
        assert_eq!(view.prompt.width, 30);
        assert_eq!(view.panes.width, 30);
    }

    #[test]
    fn collapsed_sidebar_keeps_strip() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 120, 30), &config, true, false, 30);
        assert_eq!(view.sidebar.width, COLLAPSED_STRIP);
        assert_eq!(view.prompt.width, 30);
        assert_eq!(view.panes.width, 120 - COLLAPSED_STRIP - 30);
    }

    #[test]
    fn collapsed_narrow_sidebar_is_hidden() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 60, 30), &config, true, false, 30);
        assert_eq!(view.sidebar.width, 0);
        assert_eq!(view.prompt.width, 30);
    }

    #[test]
    fn collapsed_prompt_becomes_right_strip() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 100, 30), &config, false, true, 30);
        assert_eq!(view.prompt, Rect::new(97, 0, COLLAPSED_STRIP, 30));
        assert_eq!(
            view.panes.width,
            100 - config.sidebar_width - COLLAPSED_STRIP
        );
    }

    #[test]
    fn collapsing_prompt_keeps_stored_width() {
        let config = Config::default();
        let collapsed = compute(Rect::new(0, 0, 100, 30), &config, false, true, 42);
        let expanded = compute(Rect::new(0, 0, 100, 30), &config, false, false, 42);
        assert_eq!(collapsed.prompt.width, COLLAPSED_STRIP);
        assert_eq!(expanded.prompt.width, 42);
    }

    #[test]
    fn prompt_width_is_clamped_both_sides() {
        let config = Config::default();
        let too_narrow = compute(Rect::new(0, 0, 100, 30), &config, false, false, 5);
        assert_eq!(too_narrow.prompt.width, config.min_pane_width);
        let too_wide = compute(Rect::new(0, 0, 100, 30), &config, false, false, 90);
        assert_eq!(
            too_wide.prompt.width,
            100 - config.sidebar_width - config.min_pane_width
        );
    }

    #[test]
    fn prompt_splits_evenly_when_area_is_tiny() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 10, 30), &config, false, false, 30);
        assert_eq!(view.prompt.width, 5);
        assert_eq!(view.panes.width, 5);
    }

    #[test]
    fn tiny_area_does_not_overflow() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 10, 2), &config, false, false, 30);
        assert_eq!(view.panes.height, 1);
        assert_eq!(view.panes.bottom(), 2);
    }

    #[test]
    fn default_prompt_width_is_half_of_main_area() {
        let config = Config::default();
        let area = Rect::new(0, 0, 100, 30);
        assert_eq!(default_prompt_width(area, &config, false), 38);
        assert_eq!(
            default_prompt_width(area, &config, true),
            (100 - COLLAPSED_STRIP) / 2
        );
        assert_eq!(
            default_prompt_width(Rect::new(0, 0, 60, 30), &config, false),
            30
        );
    }

    #[test]
    fn prompt_width_at_follows_boundary_and_clamps() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 100, 30), &config, false, false, 30);
        assert_eq!(prompt_width_at(&view, 70, config.min_pane_width), 30);
        assert_eq!(prompt_width_at(&view, 90, config.min_pane_width), 10);
        assert_eq!(
            prompt_width_at(&view, 0, config.min_pane_width),
            100 - config.sidebar_width - config.min_pane_width
        );
    }
}

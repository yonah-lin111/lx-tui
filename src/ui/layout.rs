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
    pub status: Rect,
}

/// 划分主界面；侧栏折叠且宽度充足时保留窄条，否则为零宽。
pub fn compute(area: Rect, config: &Config, sidebar_collapsed: bool) -> ViewLayout {
    let wide_enough = area.width >= config.narrow_width;
    let show_sidebar = !sidebar_collapsed && wide_enough;
    let sidebar_width = if show_sidebar {
        config.sidebar_width.min(area.width)
    } else if sidebar_collapsed && wide_enough {
        COLLAPSED_STRIP
    } else {
        0
    };

    // 侧栏占满全高；主区依次为标签栏、窗格、状态栏。
    let sidebar = Rect {
        width: sidebar_width,
        ..area
    };
    let main = Rect {
        x: area.x + sidebar_width,
        width: area.width - sidebar_width,
        ..area
    };
    let tab_bar = Rect {
        height: 1.min(main.height),
        ..main
    };
    let status = Rect {
        y: main.y + main.height.saturating_sub(1),
        height: 1.min(main.height),
        ..main
    };
    let panes = Rect {
        x: main.x,
        y: main.y + tab_bar.height,
        width: main.width,
        height: main.height.saturating_sub(tab_bar.height + status.height),
    };

    ViewLayout {
        sidebar,
        tab_bar,
        panes,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_width_shows_sidebar() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 100, 30), &config, false);
        assert_eq!(view.sidebar.width, config.sidebar_width);
        assert_eq!(view.sidebar.height, 30);
        assert_eq!(view.tab_bar.height, 1);
        assert_eq!(view.status.height, 1);
        assert_eq!(view.panes.width, 100 - config.sidebar_width);
        assert_eq!(view.panes.height, 28);
    }

    #[test]
    fn narrow_width_hides_sidebar() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 60, 30), &config, false);
        assert_eq!(view.sidebar.width, 0);
        assert_eq!(view.panes.width, 60);
    }

    #[test]
    fn collapsed_sidebar_keeps_strip() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 120, 30), &config, true);
        assert_eq!(view.sidebar.width, COLLAPSED_STRIP);
        assert_eq!(view.panes.width, 120 - COLLAPSED_STRIP);
    }

    #[test]
    fn collapsed_narrow_sidebar_is_hidden() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 60, 30), &config, true);
        assert_eq!(view.sidebar.width, 0);
        assert_eq!(view.panes.width, 60);
    }

    #[test]
    fn tiny_area_does_not_overflow() {
        let config = Config::default();
        let view = compute(Rect::new(0, 0, 10, 2), &config, false);
        assert_eq!(view.panes.height, 0);
    }
}

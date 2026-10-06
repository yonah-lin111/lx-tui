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

/// 划分主界面；标签栏横跨主区与右栏，右栏与窗格同顶同底；
/// 侧栏折叠且宽度充足时保留窄条，右栏 prompt 始终可见。
pub fn compute(
    area: Rect,
    config: &Config,
    sidebar_collapsed: bool,
    sidebar_width: u16,
    prompt_collapsed: bool,
    prompt_width: u16,
) -> ViewLayout {
    let sidebar_width = resolved_sidebar_width(area, config, sidebar_collapsed, sidebar_width);
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
    let main = Rect {
        x: area.x + sidebar_width,
        width: available.saturating_sub(prompt_width),
        ..area
    };
    let tab_bar = Rect {
        x: main.x,
        y: area.y,
        width: available,
        height: 1.min(area.height),
    };
    let body_y = area.y + tab_bar.height;
    let body_height = area.height.saturating_sub(tab_bar.height);
    let prompt = Rect {
        x: area.x + area.width - prompt_width,
        y: body_y,
        width: prompt_width,
        height: body_height,
    };
    let panes = Rect {
        x: main.x,
        y: body_y,
        width: main.width,
        height: body_height,
    };

    ViewLayout {
        sidebar,
        tab_bar,
        panes,
        prompt,
    }
}

/// 侧栏分区矩形：上半工作区列表、表头行（分割线 + Agents 标题与折叠按钮）、下半 agents。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SidebarSections {
    pub workspaces: Rect,
    pub divider: Rect,
    pub agents: Rect,
}

/// 侧栏分区几何：展开态表头在竖直中线；agents 折叠时内容收为 0 行、表头贴底。
/// 内容区不足两行时返回 None，由调用方退回单列表渲染。
pub fn sidebar_sections(sidebar: Rect, agents_collapsed: bool) -> Option<SidebarSections> {
    if sidebar.width <= 2 || sidebar.height <= 2 {
        return None;
    }
    let inner = Rect::new(
        sidebar.x + 1,
        sidebar.y + 1,
        sidebar.width - 2,
        sidebar.height - 2,
    );
    if inner.height < 2 {
        return None;
    }
    let divider_row = if agents_collapsed {
        inner.height - 1
    } else {
        inner.height / 2
    };
    let workspaces = Rect {
        height: divider_row,
        ..inner
    };
    let divider = Rect {
        y: inner.y + divider_row,
        height: 1,
        ..inner
    };
    let agents = Rect {
        y: inner.y + divider_row + 1,
        height: inner.height - divider_row - 1,
        ..inner
    };
    Some(SidebarSections {
        workspaces,
        divider,
        agents,
    })
}

/// 启动默认右栏宽度：主区可用宽度的一半，复刻旧 50% 分割的版面。
pub fn default_prompt_width(area: Rect, config: &Config, sidebar_collapsed: bool) -> u16 {
    let available = area.width.saturating_sub(resolved_sidebar_width(
        area,
        config,
        sidebar_collapsed,
        config.sidebar_width,
    ));
    clamp_prompt_width(available, available / 2, config.min_pane_width)
}

/// 拖拽侧栏分割线到屏幕列 `boundary_x` 时侧栏应有的宽度（屏幕坐标）。
///
/// 上限同时受配置与"主区至少 `min_pane_width`"约束；空间不足时回退最小宽度。
pub fn sidebar_width_at(view: &ViewLayout, config: &Config, boundary_x: u16) -> u16 {
    let total = view
        .panes
        .right()
        .saturating_sub(view.sidebar.x)
        .max(view.sidebar.width);
    let max = config
        .sidebar_max_width
        .min(total.saturating_sub(config.min_pane_width))
        .max(config.sidebar_min_width);
    let desired = boundary_x.saturating_sub(view.sidebar.x).saturating_add(1);
    desired.clamp(config.sidebar_min_width, max)
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

/// 左栏宽度：展开取运行时值并钳制配置边界，折叠且宽度充足取窄条，否则隐藏。
fn resolved_sidebar_width(area: Rect, config: &Config, sidebar_collapsed: bool, width: u16) -> u16 {
    let wide_enough = area.width >= config.narrow_width;
    if !sidebar_collapsed && wide_enough {
        width
            .clamp(config.sidebar_min_width, config.sidebar_max_width)
            .min(area.width)
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
mod tests;

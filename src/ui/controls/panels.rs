//! 面板折叠按钮、Agents 折叠按钮、Prompt 钉住按钮与折叠窄条。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::tiling::COLLAPSED_STRIP;
use crate::ui::{layout, left_panel, style, text};

/// 展开态折叠按钮：标签宽度与距面板右缘的留白。
pub(crate) const PANEL_BUTTON_WIDTH: u16 = 3;
pub(crate) const PANEL_BUTTON_MARGIN: u16 = 1;

/// 折叠面板的按钮目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseTarget {
    Sidebar,
    Prompt,
    /// 右列主区（prompt 在左时的可折叠面板）。
    Panes,
    Agents,
}

/// 折叠按钮：目标、命中矩形与当前折叠态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollapseButton {
    pub target: CollapseTarget,
    pub area: Rect,
    pub collapsed: bool,
}

/// 左栏、右列与 agents 表头内的折叠按钮；渲染与鼠标命中共用同一几何。
/// 位置模型下只有右列面板可折叠：prompt 在右折叠 prompt，prompt 在左折叠主区。
pub fn collapse_buttons(view: &layout::ViewLayout, agents_collapsed: bool) -> Vec<CollapseButton> {
    let mut buttons = Vec::new();
    if let Some(button) = panel_button(CollapseTarget::Sidebar, view.left_panel) {
        buttons.push(button);
    }
    let (target, panel) = if view.right_panel_left {
        (CollapseTarget::Panes, view.main_content)
    } else {
        (CollapseTarget::Prompt, view.right_panel)
    };
    if let Some(button) = panel_button(target, panel) {
        buttons.push(button);
    }
    if let Some(button) = agents_button(view, agents_collapsed) {
        buttons.push(button);
    }
    buttons
}

/// 命中测试：返回坐标所在折叠按钮的目标。
pub fn collapse_button_at(
    view: &layout::ViewLayout,
    agents_collapsed: bool,
    column: u16,
    row: u16,
) -> Option<CollapseTarget> {
    collapse_buttons(view, agents_collapsed)
        .into_iter()
        .find(|button| button.area.contains((column, row).into()))
        .map(|button| button.target)
}

/// 单个面板的折叠按钮：折叠态取整条窄条，展开态在底边右端。
fn panel_button(target: CollapseTarget, panel: Rect) -> Option<CollapseButton> {
    if panel.width == 0 || panel.height == 0 {
        return None;
    }
    let row = panel.bottom().saturating_sub(1);
    if panel.width == COLLAPSED_STRIP {
        Some(CollapseButton {
            target,
            area: Rect::new(panel.x, row, COLLAPSED_STRIP, 1),
            collapsed: true,
        })
    } else if panel.width > COLLAPSED_STRIP {
        Some(CollapseButton {
            target,
            area: Rect::new(
                panel.right() - PANEL_BUTTON_WIDTH - PANEL_BUTTON_MARGIN,
                row,
                PANEL_BUTTON_WIDTH,
                1,
            ),
            collapsed: false,
        })
    } else {
        None
    }
}

/// agents 分区折叠按钮：位于分区表头行，右端与面板顶部折叠按钮同列。
fn agents_button(view: &layout::ViewLayout, agents_collapsed: bool) -> Option<CollapseButton> {
    if view.left_panel.width <= COLLAPSED_STRIP {
        return None;
    }
    let sections = layout::sidebar_sections(view.left_panel, agents_collapsed)?;
    Some(CollapseButton {
        target: CollapseTarget::Agents,
        area: left_panel::agents_button_area(view.left_panel, sections.divider),
        collapsed: agents_collapsed,
    })
}

/// prompt 底边钉住按钮：展开态位于底边折叠按钮左侧（间隔 1 列 `─`）；折叠态或空间不足返回 None。
///
/// 标签宽度随钉住态变化（未钉 `[pin]` 5 列 / 已钉 `[unpin]` 7 列），右缘固定贴折叠按钮左侧。
pub fn prompt_pin_button(panel: Rect, pinned: bool) -> Option<Rect> {
    if panel.width <= COLLAPSED_STRIP || panel.height == 0 {
        return None;
    }
    let label = if pinned {
        text::PROMPT_PINNED_LABEL
    } else {
        text::PROMPT_UNPINNED_LABEL
    };
    let width = u16::try_from(label.chars().count()).unwrap_or(u16::MAX);
    let collapse_x = panel
        .right()
        .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN);
    let x = collapse_x.checked_sub(width.saturating_add(1))?;
    if x <= panel.x {
        return None;
    }
    Some(Rect::new(x, panel.bottom().saturating_sub(1), width, 1))
}

/// 钉住按钮命中：坐标是否落在右侧容器底边钉住按钮内。
pub fn prompt_pin_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> bool {
    !state.prompt_collapsed
        && prompt_pin_button(view.right_panel, state.prompt_pinned.is_some())
            .is_some_and(|area| area.contains((column, row).into()))
}

/// 在面板底边与 agents 表头绘制折叠按钮与 prompt 钉住按钮，必须晚于面板内容渲染。
///
/// 折叠态面板标签恰好占满 3 个内容列（水平居中），其余按钮贴所在行右端。
pub(crate) fn render_panel_buttons(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
    for button in collapse_buttons(view, state.agents_collapsed) {
        let label = match (button.target, button.collapsed) {
            (CollapseTarget::Sidebar, false) => text::SIDEBAR_COLLAPSE_LABEL,
            (CollapseTarget::Sidebar, true) => text::SIDEBAR_EXPAND_LABEL,
            (CollapseTarget::Prompt | CollapseTarget::Panes, false) => text::RIGHT_COLLAPSE_LABEL,
            (CollapseTarget::Prompt | CollapseTarget::Panes, true) => text::RIGHT_EXPAND_LABEL,
            (CollapseTarget::Agents, false) => text::AGENTS_COLLAPSE_LABEL,
            (CollapseTarget::Agents, true) => text::AGENTS_EXPAND_LABEL,
        };
        let start = match button.target {
            CollapseTarget::Sidebar if button.collapsed => collapsed_content_x(button.area, true),
            CollapseTarget::Prompt | CollapseTarget::Panes if button.collapsed => {
                collapsed_content_x(button.area, false)
            }
            _ => button.area.x,
        };
        for (offset, symbol) in label.chars().enumerate() {
            let x = start + offset as u16;
            if let Some(cell) = frame.buffer_mut().cell_mut((x, button.area.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(style::accent());
            }
        }
    }
    render_prompt_pin(frame, view, state);
}

/// 绘制 prompt 底边钉住按钮：未钉 `[pin]`、已钉 `[unpin]`，逐格强调色。
fn render_prompt_pin(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
    if state.prompt_collapsed {
        return;
    }
    let pinned = state.prompt_pinned.is_some();
    let Some(area) = prompt_pin_button(view.right_panel, pinned) else {
        return;
    };
    let label = if pinned {
        text::PROMPT_PINNED_LABEL
    } else {
        text::PROMPT_UNPINNED_LABEL
    };
    for (offset, symbol) in label.chars().enumerate() {
        let x = area.x + offset as u16;
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}

/// 折叠窄条：分隔线贴主区一侧，内容区为空，仅由折叠按钮绘制居中图标。
pub(crate) fn render_collapsed_strip(frame: &mut Frame<'_>, area: Rect, separator_right: bool) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let x = if separator_right {
        area.right() - 1
    } else {
        area.x
    };
    let buf = frame.buffer_mut();
    for y in area.y..area.bottom() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(text::STRIP_LINE);
            cell.set_style(style::muted());
        }
    }
}

/// 折叠条内容区起始列：扣除贴主区的分隔线，标签恰好占满内容区（水平居中）。
pub(crate) fn collapsed_content_x(area: Rect, separator_right: bool) -> u16 {
    if separator_right {
        area.x
    } else {
        area.x.saturating_add(1)
    }
}

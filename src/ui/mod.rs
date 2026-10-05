//! 渲染层：只读取状态，禁止修改状态或执行 IO。

pub mod layout;
pub mod style;
pub mod terminal;
pub mod text;
pub mod toast;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, Paragraph, Wrap};

use crate::app::state::{AppState, Pane, PaneKind};
use crate::config::Config;
use crate::layout::{COLLAPSED_STRIP, PaneId};

/// 展开态折叠按钮：标签宽度与距面板右缘的留白。
const PANEL_BUTTON_WIDTH: u16 = 3;
const PANEL_BUTTON_MARGIN: u16 = 1;

/// 渲染整个界面。
pub fn render(frame: &mut Frame<'_>, state: &AppState, config: &Config) {
    let area = frame.area();
    if area.width < config.min_width || area.height < config.min_height {
        render_min_size_notice(frame, area, config);
        return;
    }

    let view = layout::compute(
        area,
        config,
        state.sidebar_collapsed,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let pane_rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
    );

    if view.sidebar.width > 0 {
        if state.sidebar_collapsed {
            render_collapsed_strip(frame, view.sidebar, true);
        } else {
            render_sidebar(frame, view.sidebar, state);
        }
    }
    render_tab_bar(frame, view.tab_bar, state);
    render_exit_button(frame, &view);
    render_panes(frame, &pane_rects, state);
    render_prompt(frame, view.prompt, state);
    render_collapse_buttons(frame, &view, state);
    render_resize_hint(frame, &view, state);
    toast::render(frame, area, state, &view, &pane_rects, config);
}

/// 折叠面板的按钮目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseTarget {
    Sidebar,
    Prompt,
    Agents,
}

/// 折叠按钮：目标、命中矩形与当前折叠态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollapseButton {
    pub target: CollapseTarget,
    pub area: Rect,
    pub collapsed: bool,
}

/// 左栏、右栏顶边与 agents 表头内的折叠按钮；渲染与鼠标命中共用同一几何。
pub fn collapse_buttons(view: &layout::ViewLayout, agents_collapsed: bool) -> Vec<CollapseButton> {
    let mut buttons = Vec::new();
    if let Some(button) = panel_button(CollapseTarget::Sidebar, view.sidebar) {
        buttons.push(button);
    }
    if let Some(button) = panel_button(CollapseTarget::Prompt, view.prompt) {
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

/// 退出按钮矩形：标签栏最右端贴右缘，与右栏折叠态按钮同列；空间不足时不显示。
pub fn exit_button(view: &layout::ViewLayout) -> Option<Rect> {
    let width = text::EXIT_LABEL.chars().count() as u16;
    let bar = view.tab_bar;
    if bar.height == 0 || bar.width < width {
        return None;
    }
    Some(Rect::new(bar.right() - width, bar.y, width, 1))
}

/// 命中测试：坐标是否落在退出按钮内。
pub fn exit_button_at(view: &layout::ViewLayout, column: u16, row: u16) -> bool {
    exit_button(view).is_some_and(|area| area.contains((column, row).into()))
}

/// 单个面板的折叠按钮：折叠态取整条窄条，展开态在顶边右端。
fn panel_button(target: CollapseTarget, panel: Rect) -> Option<CollapseButton> {
    if panel.width == 0 || panel.height == 0 {
        return None;
    }
    if panel.width == COLLAPSED_STRIP {
        Some(CollapseButton {
            target,
            area: Rect::new(panel.x, panel.y, COLLAPSED_STRIP, 1),
            collapsed: true,
        })
    } else if panel.width > COLLAPSED_STRIP {
        Some(CollapseButton {
            target,
            area: Rect::new(
                panel.right() - PANEL_BUTTON_WIDTH - PANEL_BUTTON_MARGIN,
                panel.y,
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
    if view.sidebar.width <= COLLAPSED_STRIP {
        return None;
    }
    let sections = layout::sidebar_sections(view.sidebar, agents_collapsed)?;
    Some(CollapseButton {
        target: CollapseTarget::Agents,
        area: agents_button_area(view.sidebar, sections.divider),
        collapsed: agents_collapsed,
    })
}

/// agents 折叠按钮矩形：右端与面板顶部折叠按钮对齐，纵向落在表头行。
fn agents_button_area(sidebar: Rect, divider: Rect) -> Rect {
    Rect::new(
        sidebar
            .right()
            .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN),
        divider.y,
        PANEL_BUTTON_WIDTH,
        1,
    )
}

/// 在面板顶边与 agents 表头绘制折叠按钮，必须晚于面板内容渲染。
///
/// 折叠态面板标签恰好占满 3 个内容列（水平居中），其余按钮贴所在行右端。
fn render_collapse_buttons(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
    for button in collapse_buttons(view, state.agents_collapsed) {
        let label = match (button.target, button.collapsed) {
            (CollapseTarget::Sidebar, false) => text::SIDEBAR_COLLAPSE_LABEL,
            (CollapseTarget::Sidebar, true) => text::SIDEBAR_EXPAND_LABEL,
            (CollapseTarget::Prompt, false) => text::PROMPT_COLLAPSE_LABEL,
            (CollapseTarget::Prompt, true) => text::PROMPT_EXPAND_LABEL,
            (CollapseTarget::Agents, false) => text::AGENTS_COLLAPSE_LABEL,
            (CollapseTarget::Agents, true) => text::AGENTS_EXPAND_LABEL,
        };
        let start = match button.target {
            CollapseTarget::Sidebar if button.collapsed => collapsed_content_x(button.area, true),
            CollapseTarget::Prompt if button.collapsed => collapsed_content_x(button.area, false),
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
}

/// 折叠窄条：分隔线贴主区一侧，内容区为空，仅由折叠按钮绘制居中图标。
fn render_collapsed_strip(frame: &mut Frame<'_>, area: Rect, separator_right: bool) {
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
fn collapsed_content_x(area: Rect, separator_right: bool) -> u16 {
    if separator_right {
        area.x
    } else {
        area.x.saturating_add(1)
    }
}

/// 右栏分割线提示：悬停或拖拽时把两列边框改为强调色。
fn render_resize_hint(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
    if state.prompt_collapsed
        || !(state.resizing_prompt || state.prompt_hover)
        || view.prompt.x == 0
    {
        return;
    }
    let buf = frame.buffer_mut();
    for column in [view.prompt.x.saturating_sub(1), view.prompt.x] {
        for row in view.panes.y..view.panes.bottom() {
            if let Some(cell) = buf.cell_mut((column, row))
                && cell.symbol() == text::STRIP_LINE
            {
                cell.set_style(style::accent());
            }
        }
    }
}

/// 侧栏：上半工作区列表，竖直中线的分割线标题为 agents，下半为 agents 占位区。
fn render_sidebar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let items: Vec<ListItem<'_>> = state
        .workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| {
            let style = if index == state.active_workspace {
                style::accent()
            } else {
                style::text()
            };
            ListItem::new(Line::from(Span::styled(workspace.name.as_str(), style)))
        })
        .collect();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(false))
        .title(Span::styled(
            format!(" {} ", text::SIDEBAR_TITLE),
            style::muted(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let Some(sections) = layout::sidebar_sections(area, state.agents_collapsed) else {
        frame.render_widget(List::new(items), inner);
        return;
    };
    frame.render_widget(List::new(items), sections.workspaces);
    render_agents_header(
        frame,
        sections.divider,
        agents_button_area(area, sections.divider),
    );
    // agents 分区暂无内容，保持空占位。
}

/// agents 表头：贯穿的横线与左对齐的 ` Agents ` 标题；与折叠按钮重叠时省略标题。
fn render_agents_header(frame: &mut Frame<'_>, row: Rect, button: Rect) {
    if row.width == 0 {
        return;
    }
    let buf = frame.buffer_mut();
    for x in row.x..row.right() {
        if let Some(cell) = buf.cell_mut((x, row.y)) {
            cell.reset();
            cell.set_symbol(text::DIVIDER_MID);
            cell.set_style(style::border(false));
        }
    }
    let label = format!(" {} ", text::SIDEBAR_AGENTS_TITLE);
    let label_width = label.chars().count() as u16;
    if label_width >= row.width {
        return;
    }
    // 与顶部 Workspaces 标题同列左对齐。
    let label_area = Rect::new(row.x, row.y, label_width, 1);
    if label_area.intersects(button) {
        return;
    }
    for (offset, symbol) in label.chars().enumerate() {
        if let Some(cell) = buf.cell_mut((label_area.x + offset as u16, row.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::muted());
        }
    }
}

/// 退出按钮：标签栏右端强调色标签，点击退出应用。
fn render_exit_button(frame: &mut Frame<'_>, view: &layout::ViewLayout) {
    let Some(area) = exit_button(view) else {
        return;
    };
    for (offset, symbol) in text::EXIT_LABEL.chars().enumerate() {
        if let Some(cell) = frame
            .buffer_mut()
            .cell_mut((area.x + offset as u16, area.y))
        {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}

/// 标签栏：当前工作区的标签切换。
fn render_tab_bar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let workspace = state.active_workspace();
    let mut spans: Vec<Span<'_>> = Vec::with_capacity(workspace.tabs.len() * 2);
    for (index, tab) in workspace.tabs.iter().enumerate() {
        let style = if index == workspace.active_tab {
            style::accent()
        } else {
            style::muted()
        };
        spans.push(Span::styled(format!(" {} ", tab.title), style));
        spans.push(Span::styled("│", style::muted()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// 主区域：BSP 平铺窗格；矩形由调用方按当前几何计算，与命中测试共用。
fn render_panes(frame: &mut Frame<'_>, pane_rects: &[(PaneId, Rect)], state: &AppState) {
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    for (id, rect) in pane_rects {
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let Some(pane) = tab.pane(*id) else {
            continue;
        };
        let focused = *id == focus;
        let title_style = if focused {
            style::accent()
        } else {
            style::muted()
        };
        let mut title = pane_display_title(*id, pane);
        if pane.exited {
            title.push_str(" (exited)");
        }
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style::border(focused))
            .title(Span::styled(format!(" {title} "), title_style));
        let inner = block.inner(*rect);
        frame.render_widget(block, *rect);
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        match pane.kind {
            PaneKind::Terminal | PaneKind::Prompt => {
                terminal::render(
                    inner,
                    frame.buffer_mut(),
                    &pane.terminal,
                    focused,
                    state.selection_for(*id),
                );
            }
            PaneKind::Placeholder => {}
        }
    }
}

/// 右栏 prompt 面板：全局固定区域，内容可选择复制；折叠时渲染为窄条。
fn render_prompt(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if state.prompt_collapsed {
        render_collapsed_strip(frame, area, false);
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(false))
        .title(Span::styled(
            format!(" {} ", text::PROMPT_TITLE),
            style::muted(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    terminal::render(
        inner,
        frame.buffer_mut(),
        &state.prompt.pane().terminal,
        false,
        state.selection_for(state.prompt.id()),
    );
}

/// 窗格标题：prompt 固定标题，空占位与终端走通用标题规则。
fn pane_display_title(id: PaneId, pane: &Pane) -> String {
    match pane.kind {
        PaneKind::Prompt => text::PROMPT_TITLE.to_string(),
        PaneKind::Terminal | PaneKind::Placeholder => text::pane_title(id, pane.terminal.title()),
    }
}

/// 终端尺寸不足时的提示。
fn render_min_size_notice(frame: &mut Frame<'_>, area: Rect, config: &Config) {
    let message = format!(
        "{} ({}x{})",
        text::MIN_SIZE_HINT,
        config.min_width,
        config.min_height
    );
    let line = Rect {
        y: area.y + area.height / 2,
        height: 1.min(area.height),
        ..area
    };
    frame.render_widget(
        Paragraph::new(message)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        line,
    );
}

#[cfg(test)]
mod tests;

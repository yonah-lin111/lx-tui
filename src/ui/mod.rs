//! 渲染层：只读取状态，禁止修改状态或执行 IO。

pub mod layout;
pub mod main_content;
pub mod markdown;
pub mod overlay;
pub mod prompt;
pub mod sidebar;
pub mod style;
pub mod tab_bar;
pub mod text;
pub mod toast;
pub mod widgets;

pub use sidebar::{
    add_workspace_button, agent_item_at, agent_item_index_at, agent_list_rows, agent_scrollbar,
    agents_section_at, sidebar_boundary_at, workspace_drop_index, workspace_group_toggle_at,
    workspace_item_at, workspace_list_rows, workspace_scrollbar, workspace_section_at,
};

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AppState, Pane, PaneKind, PaneView};
use crate::config::Config;
use crate::layout::{COLLAPSED_STRIP, PaneId};

/// 展开态折叠按钮：标签宽度与距面板右缘的留白。
pub(crate) const PANEL_BUTTON_WIDTH: u16 = 3;
pub(crate) const PANEL_BUTTON_MARGIN: u16 = 1;

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
        state.sidebar_width,
        state.prompt_collapsed,
        state.prompt_width,
    );
    let pane_rects = crate::layout::pane_rects(
        &state.active_tab().layout,
        view.panes,
        config.min_pane_width,
        config.min_pane_height,
    );

    if view.sidebar.width > 0 {
        if state.sidebar_collapsed {
            render_collapsed_strip(frame, view.sidebar, true);
        } else {
            sidebar::render(frame, view.sidebar, state);
        }
    }
    tab_bar::render(frame, &view, state);
    render_exit_button(frame, &view);
    let pane_cursor = render_panes(frame, &pane_rects, state);
    let prompt_cursor = render_prompt(frame, view.prompt, state).or(pane_cursor);
    render_panel_buttons(frame, &view, state);
    render_resize_hint(frame, &view, state);
    toast::render(frame, area, state, &view, &pane_rects, config);
    let overlay_cursor = overlay::render(frame, area, state);
    // 浮层是模态：重命名浮层接管硬件光标，其余浮层不显示光标。
    let cursor = match state.overlay {
        Some(_) => overlay_cursor,
        None => prompt_cursor,
    };
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
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

/// 左栏、右栏底边与 agents 表头内的折叠按钮；渲染与鼠标命中共用同一几何。
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

/// 退出按钮矩形：标签栏最右端贴右缘，右缘与右栏折叠态按钮对齐；空间不足时不显示。
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

/// prompt 钉住按钮：展开态位于底边折叠按钮左侧（间隔 1 列）；折叠态或空间不足返回 None。
///
/// 标签宽度随钉住态变化（`[pin]` / `[unpin]`），右缘固定贴折叠按钮左侧。
pub fn prompt_pin_button(panel: Rect, pinned: bool) -> Option<Rect> {
    if panel.width <= COLLAPSED_STRIP || panel.height == 0 {
        return None;
    }
    let label = if pinned {
        text::PROMPT_UNPIN_LABEL
    } else {
        text::PROMPT_PIN_LABEL
    };
    let width = label.chars().count() as u16;
    let collapse_x = panel
        .right()
        .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN);
    let x = collapse_x.checked_sub(width + 1)?;
    if x <= panel.x {
        return None;
    }
    Some(Rect::new(x, panel.bottom().saturating_sub(1), width, 1))
}

/// 钉住按钮命中：坐标是否落在 prompt 底边钉住按钮内。
pub fn prompt_pin_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> bool {
    prompt_pin_button(view.prompt, state.prompt_pinned.is_some())
        .is_some_and(|area| area.contains((column, row).into()))
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
    if view.sidebar.width <= COLLAPSED_STRIP {
        return None;
    }
    let sections = layout::sidebar_sections(view.sidebar, agents_collapsed)?;
    Some(CollapseButton {
        target: CollapseTarget::Agents,
        area: sidebar::agents_button_area(view.sidebar, sections.divider),
        collapsed: agents_collapsed,
    })
}

/// 在面板底边与 agents 表头绘制折叠按钮与 prompt 钉住按钮，必须晚于面板内容渲染。
///
/// 折叠态面板标签恰好占满 3 个内容列（水平居中），其余按钮贴所在行右端。
fn render_panel_buttons(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
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
    render_prompt_pin(frame, view, state);
}

/// prompt 底边钉住按钮：标签显示点击后的动作（未钉 `[pin]`、已钉 `[unpin]`）。
fn render_prompt_pin(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
    let pinned = state.prompt_pinned.is_some();
    let Some(area) = prompt_pin_button(view.prompt, pinned) else {
        return;
    };
    let label = if pinned {
        text::PROMPT_UNPIN_LABEL
    } else {
        text::PROMPT_PIN_LABEL
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

/// 左右栏分割线提示：悬停或拖拽时把相邻两列边框改为强调色。
fn render_resize_hint(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
    if !state.sidebar_collapsed
        && view.sidebar.width > 0
        && (state.resizing_sidebar || state.sidebar_hover)
    {
        highlight_divider(
            frame,
            view,
            [view.sidebar.right().saturating_sub(1), view.sidebar.right()],
        );
    }
    if !state.prompt_collapsed && view.prompt.x > 0 && (state.resizing_prompt || state.prompt_hover)
    {
        highlight_divider(
            frame,
            view,
            [view.prompt.x.saturating_sub(1), view.prompt.x],
        );
    }
}

/// 把分割线相邻两列中的边框符号改为强调色。
fn highlight_divider(frame: &mut Frame<'_>, view: &layout::ViewLayout, columns: [u16; 2]) {
    let buf = frame.buffer_mut();
    for column in columns {
        for row in view.panes.y..view.panes.bottom() {
            if let Some(cell) = buf.cell_mut((column, row))
                && cell.symbol() == text::STRIP_LINE
            {
                cell.set_style(style::accent());
            }
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

/// 主区域：BSP 平铺窗格；矩形由调用方按当前几何计算，与命中测试共用。
///
/// 返回聚焦终端窗格的光标绝对坐标：终端把 IME 预输入绘制在硬件光标处，需与仿真光标同步。
fn render_panes(
    frame: &mut Frame<'_>,
    pane_rects: &[(PaneId, Rect)],
    state: &AppState,
) -> Option<(u16, u16)> {
    let tab = state.active_tab();
    let focus = tab.layout.focus();
    let mut cursor = None;
    for (id, rect) in pane_rects {
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let Some(pane) = tab.pane(*id) else {
            continue;
        };
        // prompt 持有键盘焦点时窗格让出焦点表现，避免双焦点指示。
        let focused = *id == focus && !state.prompt_focused;
        let mut title = match pane.view {
            PaneView::Lx => text::LX_TITLE.to_string(),
            PaneView::Terminal => pane_display_title(*id, pane),
        };
        if pane.exited {
            title.push_str(" (exited)");
        }
        // 边框标题统一淡蓝色（Cyan + dim）；焦点只由边框颜色区分，按钮除外。
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(style::border(focused))
            .title(Span::styled(format!(" {title} "), style::border_title()));
        let inner = block.inner(*rect);
        frame.render_widget(block, *rect);
        if inner.width == 0 || inner.height == 0 {
            continue;
        }
        match pane.kind {
            PaneKind::Terminal => {
                if let Some((row, col)) =
                    main_content::render(inner, frame.buffer_mut(), pane, focused)
                {
                    cursor = Some((inner.x + col, inner.y + row));
                }
                if let Some(scrollbar) = main_content::scrollbar(inner, pane) {
                    widgets::scrollbar::render(frame, &scrollbar);
                }
                main_content::draw_toggle_button(frame.buffer_mut(), *rect, pane.view);
                render_pane_agent_label(frame, *rect, &title, pane, focused);
            }
            PaneKind::Placeholder => {}
        }
    }
    cursor
}

/// 窗格顶边框右侧 Agent 标记 ` agent:xxx`：前缀强调色、值 muted（与 prompt 顶边框 `ws:` 对齐），
/// 窗格聚焦时值改用强调色高亮；右端与切换按钮间隔 1 列；按钮缺失、空间不足或会覆盖左侧标题时不绘制。
fn render_pane_agent_label(
    frame: &mut Frame<'_>,
    rect: Rect,
    title: &str,
    pane: &Pane,
    focused: bool,
) {
    let Some(agent) = pane.agent.as_ref() else {
        return;
    };
    let Some(button) = main_content::toggle_button(rect) else {
        return;
    };
    let value_style = style::border_value(focused);
    let spans = [
        (" ".to_string(), style::muted()),
        (text::PANE_AGENT_PREFIX.to_string(), style::accent()),
        (text::agent_label(agent.kind).to_string(), value_style),
    ];
    let label_width: u16 = spans
        .iter()
        .map(|(content, _)| content.chars().count() as u16)
        .sum();
    let Some(anchor) = button.x.checked_sub(2) else {
        return;
    };
    let Some(start) = anchor
        .checked_add(1)
        .and_then(|end| end.checked_sub(label_width))
    else {
        return;
    };
    // " {title} " 与标记之间至少留 1 列，避免覆盖左侧标题。
    let title_width = u16::try_from(title.width())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    if start < rect.x.saturating_add(title_width).saturating_add(1) {
        return;
    }
    let buf = frame.buffer_mut();
    let mut x = start;
    for (content, span_style) in spans {
        for symbol in content.chars() {
            if let Some(cell) = buf.cell_mut((x, rect.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(span_style);
            }
            x = x.saturating_add(1);
        }
    }
}

/// 右栏 prompt 编辑器：全局固定区域，聚焦时可输入，内容可选择复制；折叠时渲染为窄条。
///
/// 文本区固定预留最右 1 列作滚动条槽，文本溢出时该列显示滚动条。
/// 返回聚焦时的硬件光标位置：终端把 IME 预输入绘制在硬件光标处，需与编辑器光标同步。
fn render_prompt(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    if state.prompt_collapsed {
        render_collapsed_strip(frame, area, false);
        return None;
    }
    let focused = state.prompt_focused;
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(focused))
        .title(Span::styled(
            format!(" {} ", text::PROMPT_TITLE),
            style::border_title(),
        ));
    if let Some(name) = prompt_path_name(state) {
        // 前缀与值分色，与底边框 `b:分支` 一致：前缀强调色、值 muted。
        let label = Line::from(vec![
            Span::styled(" ", style::muted()),
            Span::styled(text::PROMPT_WORKSPACE_PREFIX, style::accent()),
            Span::styled(name.to_string(), style::muted()),
            Span::styled(" ", style::muted()),
        ])
        .alignment(Alignment::Right);
        block = block.title_top(label);
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    prompt::render_header(area, frame.buffer_mut(), &state.prompt, focused);
    render_prompt_branch_status(frame, area, state);
    let text_area = crate::layout::prompt_text_rect(area);
    prompt::render(
        text_area,
        frame.buffer_mut(),
        &state.prompt,
        state.selection_for(state.prompt.id()),
    );
    if let Some(scrollbar) = prompt_scrollbar_for(area, state) {
        widgets::scrollbar::render(frame, &scrollbar);
    }
    let (row, col) = focused.then(|| state.prompt.cursor_cell()).flatten()?;
    Some((
        text_area.x + col.min(text_area.width.saturating_sub(1)),
        text_area.y + row,
    ))
}

/// Prompt 顶栏路径名：优先显式绑定根路径末段，回退显示中工作区路径末段。
fn prompt_path_name(state: &AppState) -> Option<&str> {
    state
        .prompt_root
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .or_else(|| workspace_path_name(state))
}

/// 显示中 prompt 所属工作区的路径末段名：优先 checkout 路径，回退工作区 cwd；
/// 无可用路径或路径无末段时为 None。
fn workspace_path_name(state: &AppState) -> Option<&str> {
    state
        .workspaces
        .get(state.prompt_workspace())
        .and_then(|workspace| {
            workspace
                .git
                .as_ref()
                .map(|git| git.checkout_path.as_path())
                .or(workspace.cwd.as_deref())
        })
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
}

/// prompt 滚动条几何：文本溢出内容区时可见；渲染与鼠标命中共用。
pub fn prompt_scrollbar(
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    prompt_scrollbar_for(view.prompt, state)
}

fn prompt_scrollbar_for(
    panel: Rect,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    let gutter = crate::layout::prompt_scrollbar_rect(panel)?;
    widgets::scrollbar::layout(
        gutter,
        state.prompt.visual_rows().len(),
        usize::from(gutter.height),
        state.prompt.scroll(),
    )
}

/// prompt 底边框左侧 git 状态：`b:分支`（linked worktree 显示仓库主 checkout 分支），
/// linked worktree 追加 ` wt:工作区名`；取显示中 prompt 所属工作区，非 git 不显示，
/// 右端避让钉住按钮与折叠按钮。
fn render_prompt_branch_status(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(workspace) = state.workspaces.get(state.prompt_workspace()) else {
        return;
    };
    let branch = workspace.git.as_ref().and_then(|git| git.status_branch());
    let linked = workspace.git.as_ref().is_some_and(|git| git.is_linked);
    if branch.is_none() && !linked {
        return;
    }
    let start = area.x.saturating_add(2);
    let end = prompt_pin_button(area, state.prompt_pinned.is_some()).map_or(
        area.right()
            .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN),
        |pin| pin.x.saturating_sub(1),
    );
    if area.height == 0 || end <= start {
        return;
    }
    let spans = branch_status_spans(
        branch,
        linked.then_some(workspace.name.as_str()),
        usize::from(end - start),
    );
    if spans.is_empty() {
        return;
    }
    let line: Vec<Span<'_>> = spans
        .iter()
        .map(|(text, span_style)| Span::styled(text.as_str(), *span_style))
        .collect();
    Paragraph::new(Line::from(line)).render(
        Rect::new(start, area.bottom().saturating_sub(1), end - start, 1),
        frame.buffer_mut(),
    );
}

/// 底边框分支状态片段：`b:分支` 与 linked worktree 的 ` wt:工作区名`；
/// 工作区名按剩余宽度截断，放不下时省略 `wt:` 片段。
fn branch_status_spans(
    branch: Option<&str>,
    worktree: Option<&str>,
    available: usize,
) -> Vec<(String, Style)> {
    let mut spans = vec![(" ".to_string(), style::muted())];
    if let Some(branch) = branch {
        spans.push((text::PROMPT_BRANCH_PREFIX.to_string(), style::accent()));
        spans.push((branch.to_string(), style::muted()));
    }
    if let Some(worktree) = worktree {
        let separator = if branch.is_some() { " " } else { "" };
        let prefix = format!("{separator}{}", text::PROMPT_WORKTREE_PREFIX);
        let used = spans_width(&spans).saturating_add(prefix.len());
        if used < available {
            spans.push((prefix, style::accent()));
            spans.push((text::ellipsize(worktree, available - used), style::muted()));
        }
    }
    spans
}

/// 片段列宽合计。
fn spans_width(spans: &[(String, Style)]) -> usize {
    spans.iter().map(|(text, _)| text.width()).sum()
}

/// 窗格标题：空占位与终端走通用标题规则。
fn pane_display_title(id: PaneId, pane: &Pane) -> String {
    text::pane_title(id, pane.terminal.title(), pane.cwd_label.as_deref())
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

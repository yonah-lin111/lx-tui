//! 渲染层：只读取状态，禁止修改状态或执行 IO。

pub mod layout;
pub mod main_content;
pub mod markdown;
pub mod overlay;
pub mod prompt;
pub mod style;
pub mod tab_bar;
pub mod text;
pub mod toast;
pub mod widgets;

use std::collections::BTreeMap;
use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::state::{AppState, Pane, PaneKind, PaneView, Workspace};
use crate::config::Config;
use crate::layout::{COLLAPSED_STRIP, PaneId};

/// 展开态折叠按钮：标签宽度与距面板右缘的留白。
const PANEL_BUTTON_WIDTH: u16 = 3;
const PANEL_BUTTON_MARGIN: u16 = 1;

/// 分组 linked 子项的缩进列数。
const WORKSPACE_CHILD_INDENT: usize = 2;

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
    );

    if view.sidebar.width > 0 {
        if state.sidebar_collapsed {
            render_collapsed_strip(frame, view.sidebar, true);
        } else {
            render_sidebar(frame, view.sidebar, state);
        }
    }
    tab_bar::render(frame, &view, state);
    render_exit_button(frame, &view);
    let pane_cursor = render_panes(frame, &pane_rects, state);
    let prompt_cursor = render_prompt(frame, view.prompt, state).or(pane_cursor);
    render_collapse_buttons(frame, &view, state);
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

/// 在面板底边与 agents 表头绘制折叠按钮，必须晚于面板内容渲染。
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

/// 侧栏：上半工作区列表，竖直中线的分割线标题为 agents，下半为 agents 占位区。
fn render_sidebar(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
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
        let items = workspace_items(state, usize::from(inner.width));
        frame.render_widget(List::new(items), inner);
        return;
    };
    let list = sections.workspaces;
    let scrollbar = workspace_scrollbar_for(list, state);
    let list_area = match &scrollbar {
        Some(_) => Rect {
            width: list.width.saturating_sub(1),
            ..list
        },
        None => list,
    };
    let items = workspace_items(state, usize::from(list_area.width));
    let mut list_state = ListState::default().with_offset(state.workspace_scroll);
    frame.render_stateful_widget(List::new(items), list_area, &mut list_state);
    if let Some(scrollbar) = &scrollbar {
        widgets::scrollbar::render(frame, scrollbar);
    }
    if let Some(button) = add_button_area(area) {
        render_add_workspace_button(frame, button);
    }
    render_agents_header(
        frame,
        sections.divider,
        agents_button_area(area, sections.divider),
    );
    // agents 分区暂无内容，保持空占位。
}

/// 工作区列表项：激活项强调色；拖动排序中的项反显；启动工作区在名字后追加不可移除的 `*` 标记。
///
/// 同一 git 仓库有 ≥2 个已打开工作区且含主 checkout 时，主项之后的 linked 项缩进为子项
/// 并显示分支短名（自动命名时）；列表保持工作区原顺序，命中映射仍为一行一项。
/// 标记项为标记预留 2 列，名字超宽先截断，保证 `*` 不被裁剪。
fn workspace_items(state: &AppState, width: usize) -> Vec<ListItem<'_>> {
    let marker_width = text::INITIAL_WORKSPACE_MARKER.chars().count();
    let grouped = grouped_child_flags(state);
    state
        .workspaces
        .iter()
        .enumerate()
        .map(|(index, workspace)| {
            let dragging = state.workspace_drag == Some(index);
            let mut item_style = if index == state.active_workspace {
                style::accent()
            } else {
                style::text()
            };
            let mut marker_style = style::marker();
            if dragging {
                item_style = item_style.add_modifier(Modifier::REVERSED);
                marker_style = marker_style.add_modifier(Modifier::REVERSED);
            }
            let child = grouped.get(index).copied().unwrap_or(false);
            let indent = if child { WORKSPACE_CHILD_INDENT } else { 0 };
            let label = workspace_item_label(workspace, child);
            let name_width = width
                .saturating_sub(indent)
                .saturating_sub(if workspace.is_initial {
                    marker_width
                } else {
                    0
                });
            let mut spans = Vec::new();
            if indent > 0 {
                spans.push(Span::styled(" ".repeat(indent), item_style));
            }
            spans.push(Span::styled(
                text::ellipsize(&label, name_width),
                item_style,
            ));
            if workspace.is_initial {
                spans.push(Span::styled(text::INITIAL_WORKSPACE_MARKER, marker_style));
            }
            ListItem::new(Line::from(spans))
        })
        .collect()
}

/// 分组子项标记：同仓库 ≥2 个已打开工作区、含非 linked 主项，且主项在子项之前。
///
/// 主项之前的 linked 项不缩进（列表不做父项前置重排）。
fn grouped_child_flags(state: &AppState) -> Vec<bool> {
    let mut groups: BTreeMap<&Path, Vec<usize>> = BTreeMap::new();
    for (index, workspace) in state.workspaces.iter().enumerate() {
        if let Some(git) = workspace.git.as_ref() {
            groups
                .entry(git.repo_root.as_path())
                .or_default()
                .push(index);
        }
    }
    let mut flags = vec![false; state.workspaces.len()];
    for members in groups.values() {
        if members.len() < 2 {
            continue;
        }
        let Some(parent) = members.iter().copied().find(|index| {
            state
                .workspaces
                .get(*index)
                .and_then(|workspace| workspace.git.as_ref())
                .is_some_and(|git| !git.is_linked)
        }) else {
            continue;
        };
        for member in members {
            let linked = state
                .workspaces
                .get(*member)
                .and_then(|workspace| workspace.git.as_ref())
                .is_some_and(|git| git.is_linked);
            if linked && *member > parent {
                flags[*member] = true;
            }
        }
    }
    flags
}

/// 列表项标签：分组子项自动命名时显示分支短名（去 `worktree/` 前缀），其余显示工作区名。
fn workspace_item_label(workspace: &Workspace, grouped_child: bool) -> String {
    if grouped_child
        && !workspace.name_is_manual
        && let Some(branch) = workspace.git.as_ref().and_then(|git| git.branch.as_deref())
    {
        return branch
            .strip_prefix("worktree/")
            .unwrap_or(branch)
            .to_string();
    }
    workspace.name.clone()
}

/// 工作区列表滚动条几何；不需要滚动或分区缺失时 None。
pub fn workspace_scrollbar(
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    let sections = layout::sidebar_sections(view.sidebar, state.agents_collapsed)?;
    workspace_scrollbar_for(sections.workspaces, state)
}

fn workspace_scrollbar_for(
    list: Rect,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    widgets::scrollbar::layout(
        list,
        state.workspaces.len(),
        usize::from(list.height),
        state.workspace_scroll,
    )
}

/// 工作区列表可见行数；无分区时 None。
pub fn workspace_list_rows(view: &layout::ViewLayout, state: &AppState) -> Option<usize> {
    let sections = layout::sidebar_sections(view.sidebar, state.agents_collapsed)?;
    Some(usize::from(sections.workspaces.height))
}

/// 坐标是否落在侧栏 workspaces 区（含 footer）。
pub fn workspace_section_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> bool {
    layout::sidebar_sections(view.sidebar, state.agents_collapsed)
        .is_some_and(|sections| sections.workspaces.contains((column, row).into()))
}

/// 侧栏分割线命中：展开态相邻两列边框（侧栏右边框与主区左列），限主区行范围。
pub fn sidebar_boundary_at(view: &layout::ViewLayout, column: u16, row: u16) -> bool {
    if view.sidebar.width <= COLLAPSED_STRIP || view.panes.width == 0 {
        return false;
    }
    if row < view.panes.y || row >= view.panes.bottom() {
        return false;
    }
    column == view.sidebar.right().saturating_sub(1) || column == view.sidebar.right()
}

/// 新建工作区按钮矩形：侧栏顶边框右端（原折叠按钮位置）；折叠或空间不足时不显示。
pub fn add_workspace_button(view: &layout::ViewLayout) -> Option<Rect> {
    add_button_area(view.sidebar)
}

fn add_button_area(sidebar: Rect) -> Option<Rect> {
    if sidebar.width <= COLLAPSED_STRIP || sidebar.height == 0 {
        return None;
    }
    let width = text::ADD_WORKSPACE_LABEL.chars().count() as u16;
    let x = sidebar
        .right()
        .saturating_sub(width.saturating_add(PANEL_BUTTON_MARGIN));
    (sidebar.width >= width.saturating_add(PANEL_BUTTON_MARGIN).saturating_add(1))
        .then_some(Rect::new(x, sidebar.y, width, 1))
}

/// 新建工作区按钮：强调色标签，点击立即创建并激活。
fn render_add_workspace_button(frame: &mut Frame<'_>, area: Rect) {
    for (offset, symbol) in text::ADD_WORKSPACE_LABEL.chars().enumerate() {
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

/// 工作区列表内容区（不含滚动条列）；分区缺失时退回侧栏内容区。
fn workspace_list_rect(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let list = match layout::sidebar_sections(view.sidebar, state.agents_collapsed) {
        Some(sections) => sections.workspaces,
        None => {
            let sidebar = view.sidebar;
            Rect::new(
                sidebar.x.saturating_add(1),
                sidebar.y.saturating_add(1),
                sidebar.width.saturating_sub(2),
                sidebar.height.saturating_sub(2),
            )
        }
    };
    (list.width > 0 && list.height > 0).then_some(list)
}

/// 工作区项行命中：返回被点工作区索引；滚动条列与空行不命中。
///
/// 第 N 项渲染在内容区第 `N - workspace_scroll` 行；分区缺失时退回整块内容区。
pub fn workspace_item_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let list = workspace_list_rect(view, state)?;
    if !list.contains((column, row).into()) {
        return None;
    }
    let content_width = match workspace_scrollbar_for(list, state) {
        Some(_) => list.width.saturating_sub(1),
        None => list.width,
    };
    if column >= list.x.saturating_add(content_width) {
        return None;
    }
    let index = state
        .workspace_scroll
        .saturating_add(usize::from(row - list.y));
    (index < state.workspaces.len()).then_some(index)
}

/// 拖动排序目标索引：指针行映射到列表行（纵向越界钳到首/末项，横向不限）。
pub fn workspace_drop_index(
    view: &layout::ViewLayout,
    state: &AppState,
    row: u16,
) -> Option<usize> {
    let list = workspace_list_rect(view, state)?;
    if state.workspaces.is_empty() {
        return None;
    }
    let clamped = row.clamp(list.y, list.bottom().saturating_sub(1));
    let slot = state
        .workspace_scroll
        .saturating_add(usize::from(clamped - list.y));
    Some(slot.min(state.workspaces.len().saturating_sub(1)))
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
        let title_style = if focused {
            style::accent()
        } else {
            style::muted()
        };
        let mut title = match pane.view {
            PaneView::Lx => text::LX_TITLE.to_string(),
            PaneView::Terminal => pane_display_title(*id, pane),
        };
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
            PaneKind::Terminal => {
                if let Some((row, col)) =
                    main_content::render(inner, frame.buffer_mut(), pane, focused, state.lx_phase)
                {
                    cursor = Some((inner.x + col, inner.y + row));
                }
                if let Some(scrollbar) = main_content::scrollbar(inner, pane) {
                    widgets::scrollbar::render(frame, &scrollbar);
                }
                main_content::draw_toggle_button(frame.buffer_mut(), *rect, pane.view);
            }
            PaneKind::Placeholder => {}
        }
    }
    cursor
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
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(focused))
        .title(Span::styled(
            format!(" {} ", text::PROMPT_TITLE),
            style::muted(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
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

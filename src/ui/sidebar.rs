//! 侧栏渲染：Workspaces 列表与 Agents 分区；渲染、命中与滚动几何共用。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Paragraph};

use crate::app::state::{AppState, Workspace};
use crate::detect::PaneAgentSnapshot;
use crate::layout::{COLLAPSED_STRIP, PaneId};

use super::{PANEL_BUTTON_MARGIN, PANEL_BUTTON_WIDTH, layout, style, text, widgets};

/// 工作区列表顶层项的统一缩进列数；子项连接符与父项/普通项名字在此列对齐。
pub(crate) const WORKSPACE_ITEM_INDENT: usize = 2;

/// 侧栏：上半工作区列表，竖直中线的分割线标题为 agents，下半为 agents 分区。
pub fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(false))
        .title(Span::styled(
            format!(" {} ", text::SIDEBAR_TITLE),
            style::border_title(),
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
    render_agents(frame, sections.agents, state);
}

/// 工作区列表项：激活项强调色填充（青底黑字）、鼠标悬停项终端反显（同右键菜单选中项）；
/// 拖动排序中的项反显；启动工作区在名字后追加不可移除的 `*` 标记。悬停与选中互斥，拖动反显优先。
///
/// 顶层项统一缩进 2 列：组父项行在左缘显示折叠箭头（accent 色，占 2 列），其名字与
/// 分组子项的连接符同列；子项连接符（`├─ `/`└─ `，末位按可见子项判定）之后是名字。
/// 非 git / 未分组项与父项名字同列。折叠组只渲染父项与当前激活子项。
/// 行由 `AppState::workspace_rows` 给出，渲染与命中一一对应。
/// 标记项为标记预留 2 列，名字超宽先截断，保证 `*` 不被裁剪。
fn workspace_items(state: &AppState, width: usize) -> Vec<ListItem<'_>> {
    let marker_width = text::INITIAL_WORKSPACE_MARKER.chars().count();
    let rows = state.workspace_rows();
    let drag_block = state
        .workspace_drag
        .map(|from| state.workspace_block(from))
        .unwrap_or_default();
    let mut items = Vec::with_capacity(rows.len());
    for (row_index, row) in rows.iter().enumerate() {
        let Some(workspace) = state.workspaces.get(row.index) else {
            items.push(ListItem::new(Line::default()));
            continue;
        };
        // 拖动反显只在指针真正移动后生效；按下未移动直接显示选中态，避免文字先亮、空白后补的两段式高亮。
        let dragging = state.workspace_dragging && drag_block.contains(&row.index);
        // 选中背景整行铺满；拖动块整行反显；hover 复用终端原生反显（同右键菜单）。
        let highlight = if dragging {
            Some(style::selection())
        } else if row.index == state.active_workspace {
            Some(style::selected_item())
        } else if state.workspace_hover == Some(row.index) {
            Some(style::selection())
        } else {
            None
        };
        let item_style = if row.index == state.active_workspace {
            style::accent()
        } else {
            style::text()
        }
        .patch(highlight.unwrap_or_default());
        let prefix_style = if row.parent && highlight.is_none() {
            style::accent()
        } else {
            item_style
        };
        let highlight = highlight.unwrap_or_default();
        let prefix = if row.parent {
            let arrow = if row.collapsed {
                text::WORKSPACE_GROUP_COLLAPSED
            } else {
                text::WORKSPACE_GROUP_EXPANDED
            };
            format!("{arrow} ")
        } else if row.child {
            let group = workspace_repo_root(state, row.index);
            let last = !rows[row_index + 1..]
                .iter()
                .any(|later| later.child && workspace_repo_root(state, later.index) == group);
            let tree = if last {
                text::WORKSPACE_TREE_LAST
            } else {
                text::WORKSPACE_TREE_MIDDLE
            };
            format!("{:width$}{tree} ", "", width = WORKSPACE_ITEM_INDENT)
        } else {
            " ".repeat(WORKSPACE_ITEM_INDENT)
        };
        let icon = if workspace.git.is_some() {
            text::WORKSPACE_GIT_ICON
        } else {
            text::WORKSPACE_NON_GIT_ICON
        };
        let icon_prefix = format!("{icon} ");
        let icon_style = if row.index == state.active_workspace {
            item_style
        } else {
            style::muted().patch(highlight)
        };
        let label = workspace_item_label(workspace, row.child);
        let index_text = row
            .child_index
            .map(|index| format!(" {index}"))
            .unwrap_or_default();
        let name_width = width
            .saturating_sub(prefix.chars().count())
            .saturating_sub(icon_prefix.chars().count())
            .saturating_sub(index_text.chars().count())
            .saturating_sub(if workspace.is_initial {
                marker_width
            } else {
                0
            });
        let mut spans = Vec::new();
        if !prefix.is_empty() {
            spans.push(Span::styled(prefix, prefix_style));
        }
        spans.push(Span::styled(icon_prefix, icon_style));
        spans.push(Span::styled(
            text::ellipsize(&label, name_width),
            item_style,
        ));
        if !index_text.is_empty() {
            spans.push(Span::styled(index_text, item_style));
        }
        if workspace.is_initial {
            spans.push(Span::styled(text::INITIAL_WORKSPACE_MARKER, item_style));
        }
        items.push(ListItem::new(Line::from(spans)).style(highlight));
    }
    items
}

/// 工作区所属仓库根（分组键）；无 git 元数据时 None。
fn workspace_repo_root(state: &AppState, index: usize) -> Option<&std::path::Path> {
    state
        .workspaces
        .get(index)
        .and_then(|workspace| workspace.git.as_ref())
        .map(|git| git.repo_root.as_path())
}

/// 列表项标签：分组子项自动命名时显示分支短名（去 `worktree/` 前缀），其余显示工作区名。
fn workspace_item_label(workspace: &Workspace, grouped_child: bool) -> String {
    if grouped_child
        && !workspace.name_is_manual
        && let Some(branch) = workspace.git.as_ref().and_then(|git| git.short_branch())
    {
        return branch.to_string();
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
        state.workspace_rows().len(),
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

/// 工作区项行命中：返回被点工作区索引；滚动条列、空行与被折叠隐藏的行不命中。
///
/// 可见行由 `AppState::workspace_rows` 给出；分区缺失时退回整块内容区。
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
    let slot = state
        .workspace_scroll
        .saturating_add(usize::from(row - list.y));
    state.workspace_rows().get(slot).map(|entry| entry.index)
}

/// 分组折叠箭头命中：仅父项行的箭头格（内容区第 0 列）；返回父项工作区索引。
pub fn workspace_group_toggle_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let list = workspace_list_rect(view, state)?;
    if column != list.x || !list.contains((column, row).into()) {
        return None;
    }
    let slot = state
        .workspace_scroll
        .saturating_add(usize::from(row - list.y));
    state
        .workspace_rows()
        .get(slot)
        .filter(|entry| entry.parent)
        .map(|entry| entry.index)
}

/// 拖动排序目标索引：指针行映射到可见行的工作区索引（纵向越界钳到首/末行，横向不限）。
pub fn workspace_drop_index(
    view: &layout::ViewLayout,
    state: &AppState,
    row: u16,
) -> Option<usize> {
    let list = workspace_list_rect(view, state)?;
    let rows = state.workspace_rows();
    if rows.is_empty() {
        return None;
    }
    let clamped = row.clamp(list.y, list.bottom().saturating_sub(1));
    let slot = state
        .workspace_scroll
        .saturating_add(usize::from(clamped - list.y));
    rows.get(slot.min(rows.len().saturating_sub(1)))
        .map(|entry| entry.index)
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
            cell.set_style(style::border_title());
        }
    }
}

/// agents 折叠按钮矩形：右端与面板顶部折叠按钮对齐，纵向落在表头行。
pub(super) fn agents_button_area(sidebar: Rect, divider: Rect) -> Rect {
    Rect::new(
        sidebar
            .right()
            .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN),
        divider.y,
        PANEL_BUTTON_WIDTH,
        1,
    )
}

/// Agents 分区条目：标签索引、窗格标识与 Agent 快照；渲染与命中一一对应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentListItem {
    pub tab_index: usize,
    pub pane_id: PaneId,
    pub snapshot: PaneAgentSnapshot,
}

/// 当前激活工作区的 Agents 条目（按标签与树序，与 `AppState::agent_panes` 一致）。
pub fn agent_items(state: &AppState) -> Vec<AgentListItem> {
    let workspace = state.active_workspace();
    state
        .agent_panes()
        .into_iter()
        .filter_map(|(tab_index, pane_id)| {
            let snapshot = workspace
                .tabs
                .get(tab_index)?
                .pane(pane_id)?
                .agent
                .clone()?;
            Some(AgentListItem {
                tab_index,
                pane_id,
                snapshot,
            })
        })
        .collect()
}

/// Agents 分区内容区；分区缺失或折叠收为 0 行时 None。
fn agents_list_rect(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let sections = layout::sidebar_sections(view.sidebar, state.agents_collapsed)?;
    (sections.agents.width > 0 && sections.agents.height > 0).then_some(sections.agents)
}

/// Agents 列表滚动条几何；不需要滚动或分区缺失时 None。
pub fn agent_scrollbar(
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    let list = agents_list_rect(view, state)?;
    agent_scrollbar_for(list, state)
}

fn agent_scrollbar_for(
    list: Rect,
    state: &AppState,
) -> Option<widgets::scrollbar::ScrollbarLayout> {
    widgets::scrollbar::layout(
        list,
        state.agent_panes().len(),
        usize::from(list.height),
        state.agent_scroll,
    )
}

/// Agents 列表可见行数；分区缺失或折叠时为 None。
pub fn agent_list_rows(view: &layout::ViewLayout, state: &AppState) -> Option<usize> {
    agents_list_rect(view, state).map(|list| usize::from(list.height))
}

/// 坐标是否落在 Agents 分区（含空白与滚动条列）。
pub fn agents_section_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> bool {
    agents_list_rect(view, state).is_some_and(|list| list.contains((column, row).into()))
}

/// Agents 条目槽位命中（含滚动偏移）；滚动条列、空行不命中。
pub fn agent_item_index_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let list = agents_list_rect(view, state)?;
    if !list.contains((column, row).into()) {
        return None;
    }
    let content_width = match agent_scrollbar_for(list, state) {
        Some(_) => list.width.saturating_sub(1),
        None => list.width,
    };
    if column >= list.x.saturating_add(content_width) {
        return None;
    }
    let slot = state.agent_scroll.saturating_add(usize::from(row - list.y));
    (slot < state.agent_panes().len()).then_some(slot)
}

/// Agents 条目命中：返回被点窗格标识；未命中返回 None。
pub fn agent_item_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<PaneId> {
    let index = agent_item_index_at(view, state, column, row)?;
    state.agent_panes().get(index).map(|(_, pane)| *pane)
}

/// Agents 分区：条目列表（状态圆点 + Agent 名称 + 弱化归属）、空状态与滚动条。
pub fn render_agents(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let items = agent_items(state);
    if items.is_empty() {
        let line = Line::from(Span::styled(
            format!(" {}", text::AGENTS_EMPTY),
            style::muted(),
        ));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let scrollbar = agent_scrollbar_for(area, state);
    let list_area = match &scrollbar {
        Some(_) => Rect {
            width: area.width.saturating_sub(1),
            ..area
        },
        None => area,
    };
    let items = agent_list_items(state, &items, usize::from(list_area.width));
    let mut list_state = ListState::default().with_offset(state.agent_scroll);
    frame.render_stateful_widget(List::new(items), list_area, &mut list_state);
    if let Some(scrollbar) = &scrollbar {
        widgets::scrollbar::render(frame, scrollbar);
    }
}

/// Agents 条目行：悬停使用高亮底色；聚焦窗格对应条目用行首符号标记（不独立着色）；
/// 工作/阻塞实心圆点着色、空闲空心圆点弱化；归属标签固定右对齐，宽度不足先截断名称。
fn agent_list_items<'a>(
    state: &'a AppState,
    items: &[AgentListItem],
    width: usize,
) -> Vec<ListItem<'a>> {
    let focused = (!state.prompt_focused).then(|| state.active_tab().layout.focus());
    let mut rows = Vec::with_capacity(items.len());
    for (slot, item) in items.iter().enumerate() {
        let snapshot = &item.snapshot;
        let highlight = if state.agent_hover == Some(slot) {
            style::selection()
        } else {
            Style::default()
        };
        let marker = if focused == Some(item.pane_id) {
            format!("{} ", text::AGENT_SELECTED_MARKER)
        } else {
            "  ".to_string()
        };
        let dot = format!("{} ", text::agent_status_dot(snapshot.state));
        let location = text::agent_location(item.tab_index);
        let fixed = marker.chars().count() + dot.chars().count() + location.chars().count();
        let name = text::ellipsize(
            text::agent_label(snapshot.kind),
            width.saturating_sub(fixed + 1),
        );
        let padding = width.saturating_sub(fixed + name.chars().count());
        let marker_style = style::strong().patch(highlight);
        let dot_style = style::agent_status(snapshot.state).patch(highlight);
        let spans = vec![
            Span::styled(marker, marker_style),
            Span::styled(dot, dot_style),
            Span::styled(name, style::strong().patch(highlight)),
            Span::styled(" ".repeat(padding), Style::default().patch(highlight)),
            Span::styled(location, style::muted().patch(highlight)),
        ];
        rows.push(ListItem::new(Line::from(spans)).style(highlight));
    }
    rows
}

#[cfg(test)]
mod tests;

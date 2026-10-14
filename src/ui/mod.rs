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
pub mod tooltip;
pub mod trust;
pub mod widgets;

pub use sidebar::{
    add_workspace_button, agent_item_at, agent_item_index_at, agent_list_rows, agent_scrollbar,
    agents_section_at, sidebar_boundary_at, workspace_drop_index, workspace_group_toggle_at,
    workspace_item_at, workspace_list_blank_at, workspace_list_rows, workspace_scrollbar,
    workspace_section_at,
};

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::prompt::{PromptEditing, PromptMode};
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
        state.prompt_left,
        state.panes_collapsed,
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
            render_settings_button(frame, &view, state);
        }
    }
    tab_bar::render(frame, &view, state);
    render_exit_button(frame, &view);
    // 主区折叠：只画右缘窄条，不渲染窗格内容（PTY 尺寸保持折叠前）。
    let pane_cursor = if state.prompt_left && state.panes_collapsed {
        render_collapsed_strip(frame, view.panes, false);
        None
    } else {
        render_panes(frame, &pane_rects, state)
    };
    let prompt_cursor = render_prompt(frame, &view, state).or(pane_cursor);
    render_swap_button(frame, &view);
    render_panel_buttons(frame, &view, state);
    render_resize_hint(frame, &view, state);
    // 页码菜单打开时其边框按钮 tooltip 需绘制在菜单之上（常规 tooltip 早于浮层绘制会被盖住）。
    let page_menu_open = matches!(
        state.overlay,
        Some(crate::app::overlay::Overlay::PageMenu(_))
    );
    let tooltip_area = (!page_menu_open)
        .then(|| tooltip::render(frame, area, state, &view, &pane_rects))
        .flatten();
    let toast_area = toast::render(frame, area, state, &view, &pane_rects, config);
    let overlay_cursor = overlay::render(frame, area, state);
    let menu_tooltip_area = page_menu_open
        .then(|| tooltip::render(frame, area, state, &view, &pane_rects))
        .flatten();
    // 浮层是模态：重命名浮层接管硬件光标，其余浮层不显示光标。
    // 非模态时若光标位置被 tooltip/toast 盒覆盖（后绘制），一并隐藏，避免覆盖浮层。
    let cursor = match state.overlay {
        Some(_) => overlay_cursor,
        None => prompt_cursor.filter(|position| {
            !tooltip_area
                .into_iter()
                .chain(menu_tooltip_area)
                .chain(toast_area)
                .any(|box_area| box_area.contains((*position).into()))
        }),
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
    if let Some(button) = panel_button(CollapseTarget::Sidebar, view.sidebar) {
        buttons.push(button);
    }
    let (target, panel) = if view.prompt_left {
        (CollapseTarget::Panes, view.panes)
    } else {
        (CollapseTarget::Prompt, view.prompt)
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

/// 交换按钮矩形：主区与 prompt 分隔线垂直正中 2 列（左格 `<`、右格 `>`）；
/// 两侧面板缺失或过矮（内容区 < 3 行）时返回 None。
pub fn swap_button(view: &layout::ViewLayout) -> Option<Rect> {
    let width = u16::try_from(text::SWAP_PROMPT_LABEL.chars().count()).unwrap_or(u16::MAX);
    let prompt = view.prompt;
    if view.panes.width == 0 || prompt.width == 0 || prompt.height < 3 {
        return None;
    }
    let x = if view.prompt_left {
        prompt.right().saturating_sub(1)
    } else {
        prompt.x.saturating_sub(1)
    };
    Some(Rect::new(x, prompt.y + prompt.height / 2, width, 1))
}

/// 命中测试：坐标是否落在交换按钮内。
pub fn swap_button_at(view: &layout::ViewLayout, column: u16, row: u16) -> bool {
    swap_button(view).is_some_and(|area| area.contains((column, row).into()))
}

/// 交换按钮：覆盖在分隔线正中绘制 `<` 与 `>`；必须晚于两侧面板渲染。
fn render_swap_button(frame: &mut Frame<'_>, view: &layout::ViewLayout) {
    let Some(area) = swap_button(view) else {
        return;
    };
    for (offset, symbol) in text::SWAP_PROMPT_LABEL.chars().enumerate() {
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
}

/// 编辑视图标题：绑定条目的名称；临时提示词显示固定标题；无绑定不显示。
fn prompt_edit_title(state: &AppState) -> Option<String> {
    let panel = &state.prompt_owner().prompt_panel;
    match panel.editing? {
        PromptEditing::Item(id) => panel.tree.item(id).map(|item| item.name.clone()),
        PromptEditing::Temp(_) => Some(text::PROMPT_DRAFT_TITLE.to_string()),
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
    // 右列面板折叠期间分隔线不可拖拽，也不高亮。
    let right_collapsed = if view.prompt_left {
        state.panes_collapsed
    } else {
        state.prompt_collapsed
    };
    if !right_collapsed && view.panes.width > 0 && (state.resizing_prompt || state.prompt_hover) {
        let prompt = view.prompt;
        let columns = if view.prompt_left {
            [prompt.right().saturating_sub(1), prompt.right()]
        } else {
            [prompt.x.saturating_sub(1), prompt.x]
        };
        highlight_divider(frame, view, columns);
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
            PaneView::Lx => text::lx_pane_title(pane.lx.entry),
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
                if let Some((row, col)) = main_content::render(
                    inner,
                    frame.buffer_mut(),
                    pane,
                    focused,
                    state
                        .lx_selection
                        .as_ref()
                        .filter(|selection| selection.pane() == *id),
                ) {
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

/// 显示中编辑目标的页信息：当前序号（1 起）、总页数、页名与重命名现场。
struct PromptPageHeader {
    current: usize,
    total: usize,
    name: String,
    rename: Option<(String, usize)>,
}

/// 读显示中编辑目标的页信息；非编辑视图或未绑定目标时为 None。
fn prompt_page_header(state: &AppState) -> Option<PromptPageHeader> {
    let panel = &state.workspaces.get(state.prompt_workspace())?.prompt_panel;
    if panel.mode != PromptMode::Editor {
        return None;
    }
    let (pages, active) = match panel.editing? {
        PromptEditing::Item(id) => {
            let item = panel.tree.item(id)?;
            (item.pages.as_slice(), item.active_page)
        }
        PromptEditing::Temp(id) => {
            let temp = panel.temp(id)?;
            (temp.pages.as_slice(), temp.active_page)
        }
    };
    let index = pages.iter().position(|page| page.id == active).unwrap_or(0);
    let name = pages
        .get(index)
        .map(|page| page.name.clone())
        .unwrap_or_default();
    let rename = panel
        .page_rename()
        .map(|rename| (rename.input.text().to_string(), rename.input.cursor()));
    Some(PromptPageHeader {
        current: index + 1,
        total: pages.len(),
        name,
        rename,
    })
}

/// 命中 prompt 标题行页控件（编辑视图）；
/// 页重命名输入期间名称区不命中（改由键盘输入消费）。
pub fn prompt_page_control_at(
    panel: Rect,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<prompt::PageControl> {
    if state.prompt_collapsed {
        return None;
    }
    let header = crate::layout::prompt_header_rect(panel)?;
    let page = prompt_page_header(state)?;
    let title = prompt_edit_title(state)?;
    let data = prompt::PageHeaderData {
        current: page.current,
        total: page.total,
        name: &page.name,
        rename: page
            .rename
            .as_ref()
            .map(|(input, cursor)| (input.as_str(), *cursor)),
    };
    if page.rename.is_some() {
        let controls = prompt::page_controls(header, &text::page_counter(page.current, page.total));
        if let Some(controls) = controls.as_ref() {
            for (rect, control) in [
                (controls.prev, prompt::PageControl::Prev),
                (controls.counter, prompt::PageControl::Counter),
                (controls.next, prompt::PageControl::Next),
            ] {
                if rect.contains((column, row).into()) {
                    return Some(control);
                }
            }
        }
        return None;
    }
    prompt::header_page_control_at(header, &title, &data, column, row)
}

/// 右栏 prompt：编辑视图渲染 markdown 编辑器（聚焦时可输入、可选择复制）、
/// 树视图渲染提示词管理器；折叠时渲染为窄条。
///
/// 文本区固定预留最右 1 列作滚动条槽，文本/树溢出时该列显示滚动条。
/// 返回编辑视图聚焦时的硬件光标位置：终端把 IME 预输入绘制在硬件光标处，需与编辑器光标同步。
fn render_prompt(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) -> Option<(u16, u16)> {
    let area = view.prompt;
    if area.width == 0 || area.height == 0 {
        return None;
    }
    if state.prompt_collapsed {
        render_collapsed_strip(frame, area, false);
        return None;
    }
    let focused = state.prompt_focused;
    let mode = state.prompt_owner().prompt_panel.mode;
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(style::border(focused))
        .title(Span::styled(
            format!(" {} ", text::PROMPT_TITLE),
            style::border_title(),
        ));
    // 右侧路径名与左侧标题重叠时不绘制，保证面板标题始终可见。
    if let Some(name) = prompt_path_name(state) {
        let left = text::PROMPT_TITLE.width() + 2;
        let right = name.width() + text::PROMPT_WORKSPACE_PREFIX.width() + 1;
        if left + right < usize::from(area.width.saturating_sub(2)) {
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
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    let title = prompt_edit_title(state);
    let draft = matches!(
        state.prompt_owner().prompt_panel.editing,
        Some(PromptEditing::Temp(_))
    );
    let page = prompt_page_header(state);
    let page_data = page.as_ref().map(|page| prompt::PageHeaderData {
        current: page.current,
        total: page.total,
        name: &page.name,
        rename: page
            .rename
            .as_ref()
            .map(|(input, cursor)| (input.as_str(), *cursor)),
    });
    prompt::render_header(
        area,
        frame.buffer_mut(),
        &state.prompt,
        focused,
        mode,
        title.as_deref(),
        draft,
        state.prompt_pinned.is_some(),
        page_data,
    );
    render_prompt_branch_status(frame, area, state);
    let text_area = crate::layout::prompt_text_rect(area);
    let tree_cursor = match mode {
        PromptMode::Tree => prompt::tree::render(
            text_area,
            frame.buffer_mut(),
            &state.prompt_owner().prompt_panel,
            focused,
            state.prompt_tree_hover,
        ),
        PromptMode::Editor => {
            prompt::render(
                text_area,
                frame.buffer_mut(),
                &state.prompt,
                state.selection_for(state.prompt.id()),
            );
            None
        }
    };
    if let Some(scrollbar) = prompt_scrollbar_for(area, state) {
        widgets::scrollbar::render(frame, &scrollbar);
    }
    if mode == PromptMode::Tree {
        // 树视图硬件光标落在搜索框输入位置（与模态输入框一致），未聚焦不显示。
        return focused.then_some(tree_cursor).flatten();
    }
    if let Some(header) = crate::layout::prompt_header_rect(area)
        && let Some(page) = page.as_ref()
        && page.rename.is_some()
    {
        let title = title.as_deref().unwrap_or("");
        let data = prompt::PageHeaderData {
            current: page.current,
            total: page.total,
            name: &page.name,
            rename: page
                .rename
                .as_ref()
                .map(|(input, cursor)| (input.as_str(), *cursor)),
        };
        return focused
            .then(|| prompt::header_rename_cursor(header, title, &data))
            .flatten();
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
    let owner = state.prompt_owner();
    let (gutter, total, offset) = match owner.prompt_panel.mode {
        PromptMode::Tree => (
            crate::layout::prompt_tree_scrollbar_rect(panel)?,
            owner.prompt_panel.rows().len(),
            owner.prompt_panel.scroll,
        ),
        PromptMode::Editor => (
            crate::layout::prompt_scrollbar_rect(panel)?,
            state.prompt.visual_rows().len() + state.prompt.bottom_padding(),
            state.prompt.scroll(),
        ),
    };
    widgets::scrollbar::layout(gutter, total, usize::from(gutter.height), offset)
}

/// 侧栏底边框左侧设置按钮几何：展开态且宽度足够时贴靠左边框（与右端折叠按钮同列）。
///
/// 折叠态、过窄或服务不可用时隐藏；渲染与鼠标命中共用。
pub fn settings_button(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let area = view.sidebar;
    if !state.settings_available
        || state.sidebar_collapsed
        || area.width < SETTINGS_BUTTON_MIN_WIDTH
        || area.height < 2
    {
        return None;
    }
    Some(Rect::new(
        area.x + SETTINGS_BUTTON_MARGIN,
        area.bottom().saturating_sub(1),
        SETTINGS_BUTTON_WIDTH,
        1,
    ))
}

/// 命中侧栏底边框设置按钮。
pub fn settings_button_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> bool {
    settings_button(view, state).is_some_and(|area| area.contains((column, row).into()))
}

/// 设置按钮几何常量：宽度取自文案、左边距 1 列（紧贴左边框内侧，与右端折叠按钮对称）。
const SETTINGS_BUTTON_WIDTH: u16 = text::SETTINGS_LABEL.len() as u16;
const SETTINGS_BUTTON_MARGIN: u16 = 1;
/// 显示设置按钮所需的最小侧栏宽度：按钮 + 右端折叠按钮与其边距的安全余量。
const SETTINGS_BUTTON_MIN_WIDTH: u16 = SETTINGS_BUTTON_WIDTH + 3 + 1 + 2;

/// 绘制侧栏底边框左侧设置按钮：逐格 reset + 强调色，与折叠按钮绘制方式一致。
fn render_settings_button(frame: &mut Frame<'_>, view: &layout::ViewLayout, state: &AppState) {
    let Some(area) = settings_button(view, state) else {
        return;
    };
    for (offset, symbol) in text::SETTINGS_LABEL.chars().enumerate() {
        let x = area.x + offset as u16;
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}

/// prompt 底边框左侧 git 状态：`b:分支`（linked worktree 显示仓库主 checkout 分支），
/// linked worktree 追加 ` wt:工作区名`；取显示中 prompt 所属工作区，非 git 不显示，
/// 右端避让折叠按钮。
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
    let end = area
        .right()
        .saturating_sub(PANEL_BUTTON_WIDTH + PANEL_BUTTON_MARGIN);
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

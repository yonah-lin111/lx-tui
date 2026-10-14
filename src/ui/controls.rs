//! 界面控制按钮与装饰：折叠、退出、交换、设置按钮以及分割线高亮提示。

use ratatui::Frame;
use ratatui::layout::Rect;

use super::{layout, sidebar, style, text};
use crate::app::state::AppState;
use crate::layout::COLLAPSED_STRIP;

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
pub(crate) fn render_swap_button(frame: &mut Frame<'_>, view: &layout::ViewLayout) {
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

/// 左右栏分割线提示：悬停或拖拽时把相邻两列边框改为强调色。
pub(crate) fn render_resize_hint(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
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
pub(crate) fn render_exit_button(frame: &mut Frame<'_>, view: &layout::ViewLayout) {
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

/// `[SET]` 右侧的 `[UI]` 按钮宽度（取自文案）。
const UI_GALLERY_BUTTON_WIDTH: u16 = text::UI_GALLERY_LABEL.len() as u16;
/// `[SET]` 与 `[UI]` 之间的空档（列）。
const UI_GALLERY_BUTTON_GAP: u16 = 1;
/// 显示 `[UI]` 所需的最小侧栏宽度：设置按钮本身、空档与 UI 按钮，外加右端折叠按钮余量；
/// 不足时只隐藏 `[UI]`、保留 `[SET]`。
const UI_GALLERY_BUTTON_MIN_WIDTH: u16 =
    SETTINGS_BUTTON_MIN_WIDTH + UI_GALLERY_BUTTON_GAP + UI_GALLERY_BUTTON_WIDTH;

/// 侧栏底边框左侧 `[SET]` 右侧的 `[UI]` 按钮几何：仅侧栏展开、宽度足够时显示。
///
/// 不依赖设置服务可用性；位置固定为 `[SET]` 槽位右侧隔 1 列，即使 `[SET]` 因服务
/// 不可用而未绘制也保持同一列，避免按钮随服务状态跳动。侧栏折叠或过窄时返回 None。
pub fn ui_gallery_button(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let area = view.sidebar;
    if state.sidebar_collapsed || area.width < UI_GALLERY_BUTTON_MIN_WIDTH || area.height < 2 {
        return None;
    }
    Some(Rect::new(
        area.x + SETTINGS_BUTTON_MARGIN + SETTINGS_BUTTON_WIDTH + UI_GALLERY_BUTTON_GAP,
        area.bottom().saturating_sub(1),
        UI_GALLERY_BUTTON_WIDTH,
        1,
    ))
}

/// 命中侧栏底边框 `[UI]` 按钮。
pub fn ui_gallery_button_at(
    view: &layout::ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> bool {
    ui_gallery_button(view, state).is_some_and(|area| area.contains((column, row).into()))
}

/// 绘制侧栏底边框左侧 `[UI]` 按钮：逐格 reset + 强调色，与 `[SET]` 绘制方式一致。
pub(crate) fn render_ui_gallery_button(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
    let Some(area) = ui_gallery_button(view, state) else {
        return;
    };
    for (offset, symbol) in text::UI_GALLERY_LABEL.chars().enumerate() {
        let x = area.x + offset as u16;
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}

/// 绘制侧栏底边框左侧设置按钮：逐格 reset + 强调色，与折叠按钮绘制方式一致。
pub(crate) fn render_settings_button(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
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

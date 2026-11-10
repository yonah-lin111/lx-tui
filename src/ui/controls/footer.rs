//! 侧栏底边框的设置 `[SET]` 与用量 `[USE]` 按钮几何、命中与绘制。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::ui::{layout, style, text};

/// 侧栏底边框左侧设置按钮几何：展开态且宽度足够时贴靠左边框（与右端折叠按钮同列）。
///
/// 折叠态、过窄或服务不可用时隐藏；渲染与鼠标命中共用。
pub fn settings_button(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let area = view.left_panel;
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

/// 用量按钮几何常量：位于 `[SET]` 右侧，中间留 1 列底边框线间距。
const USAGE_BUTTON_WIDTH: u16 = text::USAGE_LABEL.len() as u16;
const USAGE_BUTTON_GAP: u16 = 1;
/// 显示用量按钮所需的最小侧栏宽度：`[SET]` 最小宽度 (11) + 间距 (1) + `[USE]` 宽度 (5) = 17。
const USAGE_BUTTON_MIN_WIDTH: u16 =
    SETTINGS_BUTTON_MIN_WIDTH + USAGE_BUTTON_GAP + USAGE_BUTTON_WIDTH;

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

/// 侧栏底边框 `[USE]` 按钮几何：展开态、服务可用且宽度 >= 17 列时显示在 `[SET]` 右侧。
pub fn usage_button(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let area = view.left_panel;
    if !state.settings_available
        || state.sidebar_collapsed
        || area.width < USAGE_BUTTON_MIN_WIDTH
        || area.height < 2
    {
        return None;
    }
    Some(Rect::new(
        area.x + SETTINGS_BUTTON_MARGIN + SETTINGS_BUTTON_WIDTH + USAGE_BUTTON_GAP,
        area.bottom().saturating_sub(1),
        USAGE_BUTTON_WIDTH,
        1,
    ))
}

/// 命中侧栏底边框 `[USE]` 按钮。
pub fn usage_button_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> bool {
    usage_button(view, state).is_some_and(|area| area.contains((column, row).into()))
}

/// 绘制侧栏底边框 `[USE]` 按钮：逐格 reset + 强调色。
pub(crate) fn render_usage_button(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
    let Some(area) = usage_button(view, state) else {
        return;
    };
    for (offset, symbol) in text::USAGE_LABEL.chars().enumerate() {
        let x = area.x + offset as u16;
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}

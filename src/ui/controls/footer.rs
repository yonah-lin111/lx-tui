//! 侧栏底边框统一 Web 控制台 `[WEB]` 按钮几何、命中与绘制。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::state::AppState;
use crate::ui::{layout, style, text};

/// 侧栏底边框 `[WEB]` 按钮几何：展开态且宽度足够时贴靠左边框（与右端折叠按钮同列）。
///
/// 折叠态、过窄或服务不可用时隐藏；渲染与鼠标命中共用。
pub fn web_button(view: &layout::ViewLayout, state: &AppState) -> Option<Rect> {
    let area = view.left_panel;
    if !state.settings_available
        || state.sidebar_collapsed
        || area.width < WEB_BUTTON_MIN_WIDTH
        || area.height < 2
    {
        return None;
    }
    Some(Rect::new(
        area.x + WEB_BUTTON_MARGIN,
        area.bottom().saturating_sub(1),
        WEB_BUTTON_WIDTH,
        1,
    ))
}

/// 命中侧栏底边框 `[WEB]` 按钮。
pub fn web_button_at(view: &layout::ViewLayout, state: &AppState, column: u16, row: u16) -> bool {
    web_button(view, state).is_some_and(|area| area.contains((column, row).into()))
}

/// 按钮几何常量：宽度取自文案、左边距 1 列（紧贴左边框内侧，与右端折叠按钮对称）。
const WEB_BUTTON_WIDTH: u16 = text::WEB_LABEL.len() as u16;
const WEB_BUTTON_MARGIN: u16 = 1;
/// 显示按钮所需的最小侧栏宽度：按钮 + 右端折叠按钮与其边距的安全余量。
const WEB_BUTTON_MIN_WIDTH: u16 = WEB_BUTTON_WIDTH + 3 + 1 + 2;

/// 绘制侧栏底边框 `[WEB]` 按钮：逐格 reset + 强调色，与折叠按钮绘制方式一致。
pub(crate) fn render_web_button(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
    let Some(area) = web_button(view, state) else {
        return;
    };
    for (offset, symbol) in text::WEB_LABEL.chars().enumerate() {
        let x = area.x + offset as u16;
        if let Some(cell) = frame.buffer_mut().cell_mut((x, area.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::accent());
        }
    }
}

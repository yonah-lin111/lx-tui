//! 左右栏分割线提示：悬停或拖拽时把相邻两列边框改为强调色。

use ratatui::Frame;

use crate::app::state::AppState;
use crate::ui::{layout, style, text};

/// 左右栏分割线提示：悬停或拖拽时把相邻两列边框改为强调色。
pub(crate) fn render_resize_hint(
    frame: &mut Frame<'_>,
    view: &layout::ViewLayout,
    state: &AppState,
) {
    if !state.sidebar_collapsed
        && view.left_panel.width > 0
        && (state.resizing_sidebar || state.sidebar_hover)
    {
        highlight_divider(
            frame,
            view,
            [
                view.left_panel.right().saturating_sub(1),
                view.left_panel.right(),
            ],
        );
    }
    // 右列面板折叠期间分隔线不可拖拽，也不高亮。
    let right_collapsed = if view.right_panel_left {
        state.panes_collapsed
    } else {
        state.prompt_collapsed
    };
    if !right_collapsed
        && view.main_content.width > 0
        && (state.resizing_prompt || state.prompt_hover)
    {
        let prompt = view.right_panel;
        let columns = if view.right_panel_left {
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
        for row in view.main_content.y..view.main_content.bottom() {
            if let Some(cell) = buf.cell_mut((column, row))
                && cell.symbol() == text::STRIP_LINE
            {
                cell.set_style(style::accent());
            }
        }
    }
}

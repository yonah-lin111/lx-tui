//! 主区与右栏交换按钮的几何、命中与绘制。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::ui::{layout, style, text};

/// 交换按钮矩形：主区与 prompt 分隔线垂直正中 2 列（左格 `<`、右格 `>`）；
/// 两侧面板缺失或过矮（内容区 < 3 行）时返回 None。
pub fn swap_button(view: &layout::ViewLayout) -> Option<Rect> {
    let width = u16::try_from(text::SWAP_PROMPT_LABEL.chars().count()).unwrap_or(u16::MAX);
    let prompt = view.right_panel;
    if view.main_content.width == 0 || prompt.width == 0 || prompt.height < 3 {
        return None;
    }
    let x = if view.right_panel_left {
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

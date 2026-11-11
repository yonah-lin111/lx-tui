//! 标签栏右端 `[EXIT]` 按钮的几何、命中与绘制。

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::ui::{layout, style, text};

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

/// 退出按钮：标签栏右端强调色标签，点击请求退出（首次显示全局 toast）。
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

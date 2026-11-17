//! 紧凑布局顶部栏：左栏入口、tab 条占位与右栏入口。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::style::Style;

use crate::app::state::AppState;
use crate::ui::layout::ViewLayout;
use crate::ui::widgets::clear;
use crate::ui::{style, text};

/// 命中左栏入口。
pub fn left_at(view: &ViewLayout, column: u16, row: u16) -> bool {
    view.compact_left_button.contains((column, row).into())
}

/// 命中右栏入口。
pub fn right_at(view: &ViewLayout, column: u16, row: u16) -> bool {
    view.compact_right_button.contains((column, row).into())
}

/// 渲染单行顶部栏；先清底，tab 条随后由 `tab_bar::render` 画进中间区域。
pub fn render(frame: &mut Frame<'_>, view: &ViewLayout, state: &AppState) {
    let area = view.compact_header;
    if area.width == 0 || area.height == 0 {
        return;
    }
    clear::render(frame, area);
    render_buttons(frame.buffer_mut(), view, state);
}

/// 左侧 `[ws]`、右侧 `[panel]`；打开对应全屏栏时使用实心强调色。
fn render_buttons(buf: &mut Buffer, view: &ViewLayout, state: &AppState) {
    let left_active = state.sidebar_focused();
    let right_active = state.prompt_focused() || state.todo_focused();
    write(
        buf,
        view.compact_left_button.x,
        view.compact_left_button.y,
        text::COMPACT_LEFT_LABEL,
        view.compact_left_button.width,
        if left_active {
            style::selected_item()
        } else {
            style::accent()
        },
    );
    write(
        buf,
        view.compact_right_button.x,
        view.compact_right_button.y,
        text::COMPACT_RIGHT_LABEL,
        view.compact_right_button.width,
        if right_active {
            style::selected_item()
        } else {
            style::accent()
        },
    );
    if view.tab_bar.width > 0 && view.tab_bar.x > view.compact_left_button.right() {
        write(
            buf,
            view.compact_left_button.right(),
            view.compact_left_button.y,
            text::TAB_SEPARATOR,
            1,
            style::muted(),
        );
    }
    if view.tab_bar.width > 0 && view.tab_bar.right() < view.compact_right_button.x {
        write(
            buf,
            view.tab_bar.right(),
            view.compact_right_button.y,
            text::TAB_SEPARATOR,
            1,
            style::muted(),
        );
    }
}

fn write(buf: &mut Buffer, x: u16, y: u16, value: &str, max_width: u16, style: Style) {
    if max_width == 0 || value.is_empty() {
        return;
    }
    buf.set_stringn(x, y, value, usize::from(max_width), style);
}

#[cfg(test)]
mod tests;

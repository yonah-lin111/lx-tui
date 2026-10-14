//! 引用文件夹浮层：目录菜单、路径输入弹窗与内容搜索弹窗的几何、命中与渲染。
//!
//! 菜单几何与行渲染见 `menu`，路径输入弹窗见 `input`，内容搜索弹窗见 `search`；
//! 共性绘制（整行底色、行内文本）在本模块收口。

mod input;
mod menu;
mod search;

pub use input::{
    ReferencedInputButton, referenced_input_button_at, referenced_input_shell,
    render_referenced_input,
};
pub use menu::{
    ReferencedMenuAction, ReferencedMenuLayout, ReferencedMenuRowLayout, referenced_menu_hit_at,
    referenced_menu_layout, render_referenced_menu,
};
pub use search::{
    ReferencedSearchLayout, referenced_search_copy_at, referenced_search_item_at,
    referenced_search_layout, render_referenced_search, render_referenced_search_footer,
};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthChar;

/// 整行底色：从 `rect` 左端铺满整行，保证选中/悬停反馈跨满行宽。
pub(super) fn fill_row(frame: &mut Frame<'_>, rect: Rect, row_style: Style) {
    for y in rect.y..rect.y.saturating_add(rect.height) {
        for x in rect.x..rect.right() {
            if let Some(cell) = frame.buffer_mut().cell_mut((x, y)) {
                cell.reset();
                cell.set_char(' ');
                cell.set_style(row_style);
            }
        }
    }
}

/// 行内文本绘制：超出右界整体丢弃（不裁半个宽字符）。
pub(super) fn paint_text(frame: &mut Frame<'_>, rect: Rect, content: &str, cell_style: Style) {
    let mut x = rect.x;
    for ch in content.chars() {
        let width = UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        if x.saturating_add(width) > rect.right() {
            break;
        }
        if let Some(cell) = frame.buffer_mut().cell_mut((x, rect.y)) {
            cell.reset();
            cell.set_char(ch);
            cell.set_style(cell_style);
        }
        x = x.saturating_add(width);
    }
}

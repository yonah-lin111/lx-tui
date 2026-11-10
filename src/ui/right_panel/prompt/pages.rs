//! 页列表渲染与命中：搜索框、分割线、`❯` 选中标记、标题截断与状态圆点。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::markdown::TemplateStatus;
use crate::app::prompt::{PageRow, PromptPanel, page_rows};
use crate::ui::{style, text, widgets};

use super::tree;

/// 派生当前面板的可见页列表行。
pub(crate) fn panel_page_rows(panel: &PromptPanel) -> Vec<PageRow> {
    let Some((pages, active)) = panel.editing_pages() else {
        return Vec::new();
    };
    page_rows(pages, active, panel.pages_view.search.text())
}

/// 滚动偏移钳制。
fn effective_scroll(panel: &PromptPanel, rows_len: usize) -> usize {
    panel.pages_view.scroll.min(rows_len.saturating_sub(1))
}

/// 渲染页列表视图；返回搜索框的硬件光标坐标。
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    panel: &PromptPanel,
    focused: bool,
    hover: Option<usize>,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let cursor = paint_search_row(buf, area, panel);
    if area.height <= 1 {
        return cursor;
    }
    tree::paint_divider(buf, area, area.y + 1, focused);
    if area.height <= 2 {
        return cursor;
    }
    let rows_area = Rect {
        y: area.y + 2,
        height: area.height - 2,
        ..area
    };
    let rows = panel_page_rows(panel);
    if rows.is_empty() {
        if rows_area.height >= 1 {
            let text_w = u16::try_from(text::PROMPT_PAGES_NO_MATCHES.width()).unwrap_or(u16::MAX);
            let x = rows_area
                .x
                .saturating_add(rows_area.width.saturating_sub(text_w) / 2);
            tree::paint_text(
                buf,
                x,
                rows_area.y,
                text::PROMPT_PAGES_NO_MATCHES,
                style::muted(),
                rows_area.right(),
            );
        }
        return cursor;
    }
    let scroll = effective_scroll(panel, rows.len());
    let selected_index = panel.pages_view.selected.min(rows.len().saturating_sub(1));
    for (index, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(rows_area.height))
    {
        let y = rows_area.y + (index - scroll) as u16;
        paint_page_row(
            buf,
            rows_area,
            y,
            row,
            index == selected_index,
            hover == Some(index),
        );
    }
    cursor
}

/// 绘制搜索框行：`>` 前缀 + 单行输入视口（占位 `search pages`）。
fn paint_search_row(buf: &mut Buffer, area: Rect, panel: &PromptPanel) -> Option<(u16, u16)> {
    let mut x = tree::paint_text(
        buf,
        area.x,
        area.y,
        text::INPUT_PREFIX_ICON,
        style::accent(),
        area.right(),
    );
    x = tree::paint_text(buf, x, area.y, " ", style::text(), area.right());
    let width = usize::from(area.right().saturating_sub(x));
    let search = &panel.pages_view.search;
    let input = widgets::input::view(search.text(), search.cursor(), width);
    if search.text().is_empty() {
        tree::paint_text(
            buf,
            x,
            area.y,
            text::PROMPT_PAGES_SEARCH_PLACEHOLDER,
            style::muted(),
            area.right(),
        );
    } else {
        tree::paint_search_spans(
            buf,
            x,
            area.y,
            input.visible,
            input.start_char,
            search.selection(),
            area.right(),
        );
    }
    Some((x.saturating_add(input.cursor_col), area.y))
}

/// 页搜索框点击列对应的全文字符索引；搜索框行之外返回 None。
pub fn search_char_at(area: Rect, panel: &PromptPanel, column: u16, row: u16) -> Option<usize> {
    if !area.contains((column, row).into()) || row != area.y {
        return None;
    }
    let search = &panel.pages_view.search;
    tree::input_char_at(area, search.text(), search.cursor(), column)
}

/// 绘制单行页条目：`❯ ` + 标题（当前页加粗）+ 右对齐状态圆点。
fn paint_page_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    row: &PageRow,
    selected: bool,
    hovered: bool,
) {
    let highlight = if hovered {
        tree::fill_row(buf, area, y);
        style::selection()
    } else {
        Style::default()
    };
    let base = style::text().patch(highlight);
    let marker = if selected {
        format!("{} ", text::AGENT_SELECTED_MARKER)
    } else {
        "  ".to_string()
    };
    let x = tree::paint_text(buf, area.x, y, &marker, base, area.right());
    let title_style = if row.current {
        style::strong().patch(highlight)
    } else {
        base
    };
    let used_right = if let Some(status) = row.status {
        let (dot, dot_style) = match status {
            TemplateStatus::Todo => (text::PAGE_STATUS_OPEN, style::muted().patch(highlight)),
            TemplateStatus::InProgress => (
                text::PAGE_STATUS_DOT,
                base.patch(style::template_status(TemplateStatus::InProgress)),
            ),
            TemplateStatus::Done => (
                text::PAGE_STATUS_DOT,
                base.patch(style::template_status(TemplateStatus::Done)),
            ),
        };
        let dot_width = u16::try_from(dot.width()).unwrap_or(1);
        let dot_x = area.right().saturating_sub(dot_width);
        if dot_x >= x {
            tree::paint_text(buf, dot_x, y, dot, dot_style, area.right());
            dot_x.saturating_sub(1)
        } else {
            area.right()
        }
    } else {
        area.right()
    };
    let max_width = usize::from(used_right.saturating_sub(x));
    tree::paint_name(buf, x, y, &row.title, title_style, max_width, used_right);
}

/// 页列表可见行命中（文本区坐标）；未命中或命中搜索框/分割线返回 None。
pub fn page_row_at(area: Rect, panel: &PromptPanel, column: u16, row: u16) -> Option<usize> {
    if !area.contains((column, row).into()) {
        return None;
    }
    let rows_y = area.y.saturating_add(2);
    if row < rows_y {
        return None;
    }
    let rows_len = panel_page_rows(panel).len();
    if rows_len == 0 {
        return None;
    }
    let index = effective_scroll(panel, rows_len) + usize::from(row - rows_y);
    (index < rows_len).then_some(index)
}

#[cfg(test)]
mod tests;

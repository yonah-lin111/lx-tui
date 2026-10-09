//! 提示词树视图渲染与命中：只读状态，无 IO。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::prompt::{PromptPanel, PromptRow, PromptRowKind};
use crate::ui::{style, text};

/// 渲染树内容区：空树显示占位；选中行整行高亮，条目状态右对齐。
pub fn render(area: Rect, buf: &mut Buffer, panel: &PromptPanel) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = panel.rows();
    if rows.is_empty() {
        paint_text(
            buf,
            area.x,
            area.y,
            text::PROMPT_TREE_EMPTY,
            style::muted(),
            area.right(),
        );
        return;
    }
    let scroll = panel.scroll.min(rows.len().saturating_sub(1));
    for (offset, row) in rows
        .iter()
        .skip(scroll)
        .take(usize::from(area.height))
        .enumerate()
    {
        let selected = scroll + offset == panel.selected;
        paint_row(buf, area, area.y + offset as u16, panel, *row, selected);
    }
}

/// 命中可见行索引（`panel.rows` 的下标，非选中索引）；区域外或超出末行返回 None。
pub fn row_at(area: Rect, panel: &PromptPanel, column: u16, row: u16) -> Option<usize> {
    if !area.contains((column, row).into()) {
        return None;
    }
    let index = panel.scroll + usize::from(row.saturating_sub(area.y));
    (index < panel.rows().len()).then_some(index)
}

/// 绘制单行：连接线 + 图标 + 名称 +（条目）右对齐状态标签。
fn paint_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    panel: &PromptPanel,
    row: PromptRow,
    selected: bool,
) {
    let base = if selected {
        style::selection()
    } else {
        style::text()
    };
    let sub = if selected {
        style::selection()
    } else {
        style::muted()
    };
    if selected {
        fill_row(buf, area, y);
    }
    let mut x = area.x;
    if row.depth > 0 {
        x = paint_text(
            buf,
            x,
            y,
            &format!("{} ", text::PROMPT_TREE_CONNECTOR),
            sub,
            area.right(),
        );
    }
    match row.kind {
        PromptRowKind::Folder => {
            let Some(folder) = panel.tree.folder_of_row(row) else {
                return;
            };
            let triangle = if folder.collapsed {
                text::PROMPT_TREE_COLLAPSED
            } else {
                text::PROMPT_TREE_EXPANDED
            };
            let triangle_style = if selected { base } else { style::accent() };
            x = paint_text(buf, x, y, triangle, triangle_style, area.right());
            x = paint_text(buf, x, y, " ", base, area.right());
            let max = usize::from(area.right().saturating_sub(x));
            paint_name(buf, x, y, &folder.name, base, max, area.right());
        }
        PromptRowKind::Item => {
            let Some(item) = panel.tree.item_of_row(row) else {
                return;
            };
            let icon_style = if selected { base } else { style::muted() };
            x = paint_text(
                buf,
                x,
                y,
                text::PROMPT_TREE_ITEM_ICON,
                icon_style,
                area.right(),
            );
            x = paint_text(buf, x, y, " ", base, area.right());
            let status = text::prompt_status_label(item.status);
            let status_width = width_of(status) as u16;
            let gap = u16::from(status_width > 0);
            let max = usize::from(
                area.right()
                    .saturating_sub(x)
                    .saturating_sub(status_width + gap),
            );
            paint_name(buf, x, y, &item.name, base, max, area.right());
            if status_width > 0 && status_width <= area.width {
                let status_x = area.right() - status_width;
                let status_style = if selected {
                    base
                } else {
                    style::prompt_status(item.status)
                };
                paint_text(buf, status_x, y, status, status_style, area.right());
            }
        }
    }
}

/// 选中行整行铺选中底色，保证高亮跨满行宽。
fn fill_row(buf: &mut Buffer, area: Rect, y: u16) {
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(style::selection());
        }
    }
}

/// 绘制文本直到右界；超出右界的字符整体丢弃。
fn paint_text(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    content: &str,
    cell_style: ratatui::style::Style,
    right: u16,
) -> u16 {
    let mut x = x;
    for ch in content.chars() {
        let width = ch.width().unwrap_or(0) as u16;
        if x.saturating_add(width) > right {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(ch);
            cell.set_style(cell_style);
        }
        x = x.saturating_add(width);
    }
    x
}

/// 绘制名称：放不下时以 `…` 截断，并保证不越过右界。
fn paint_name(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    name: &str,
    cell_style: ratatui::style::Style,
    max_width: usize,
    right: u16,
) {
    if max_width == 0 {
        return;
    }
    if width_of(name) <= max_width {
        paint_text(buf, x, y, name, cell_style, right);
        return;
    }
    if max_width == 1 {
        paint_text(buf, x, y, "…", cell_style, right);
        return;
    }
    let truncated: String = name
        .chars()
        .scan(0usize, |width, ch| {
            let cell = ch.width().unwrap_or(0);
            if *width + cell > max_width - 1 {
                return None;
            }
            *width += cell;
            Some(ch)
        })
        .collect();
    let next = paint_text(buf, x, y, &truncated, cell_style, right);
    paint_text(buf, next, y, "…", cell_style, right);
}

/// 文本显示宽度（列）。
fn width_of(content: &str) -> usize {
    content.chars().map(|ch| ch.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests;

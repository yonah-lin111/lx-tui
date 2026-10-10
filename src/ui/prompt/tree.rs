//! 提示词树视图渲染与命中：只读状态，无 IO。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::prompt::{PromptPanel, PromptRow, PromptRowKind};
use crate::ui::{style, text};

/// 渲染树内容区：顶部搜索框行 + 置顶草稿行 + 过滤后的树行；
/// 空树/无命中显示占位；选中行整行高亮，条目状态右对齐。
pub fn render(area: Rect, buf: &mut Buffer, panel: &PromptPanel) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    paint_search_row(buf, area, panel);
    if area.height <= 1 {
        return;
    }
    let rows_area = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let rows = panel.rows();
    let scroll = panel.scroll.min(rows.len().saturating_sub(1));
    for (index, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(rows_area.height))
    {
        let selected = index == panel.selected;
        let connector = if row.depth > 0 && !last_sibling(&rows, index) {
            text::PROMPT_TREE_MIDDLE
        } else {
            text::PROMPT_TREE_LAST
        };
        // 过滤时折叠被忽略：过滤视图下所有可见文件夹按展开态绘制三角。
        let searching = !panel.search.trim().is_empty();
        let expanded = row.kind == PromptRowKind::Folder
            && (!panel
                .tree
                .folder_of_row(*row)
                .is_some_and(|folder| folder.collapsed)
                || searching);
        paint_row(
            buf,
            rows_area,
            rows_area.y + (index - scroll) as u16,
            panel,
            *row,
            selected,
            connector,
            expanded,
        );
    }
    // 无树行时在草稿入口下方显示空状态 / 无命中占位。
    if rows.len() <= 1 && scroll == 0 && rows_area.height >= 2 {
        let empty = if panel.search.trim().is_empty() {
            text::PROMPT_TREE_EMPTY
        } else {
            text::PROMPT_TREE_NO_MATCHES
        };
        paint_text(
            buf,
            rows_area.x,
            rows_area.y + 1,
            empty,
            style::muted(),
            rows_area.right(),
        );
    }
}

/// 绘制顶部搜索框行：图标 + 过滤词（为空时显示占位）。
fn paint_search_row(buf: &mut Buffer, area: Rect, panel: &PromptPanel) {
    let mut x = paint_text(
        buf,
        area.x,
        area.y,
        text::PROMPT_TREE_SEARCH_ICON,
        style::accent(),
        area.right(),
    );
    x = paint_text(buf, x, area.y, " ", style::text(), area.right());
    if panel.search.is_empty() {
        paint_text(
            buf,
            x,
            area.y,
            text::PROMPT_TREE_SEARCH_PLACEHOLDER,
            style::muted(),
            area.right(),
        );
    } else {
        paint_text(buf, x, area.y, &panel.search, style::text(), area.right());
    }
}

/// 可见行是否为同级末项（决定 `└─` / `├─`）：其后再无同深度行，或先遇到更浅的行。
fn last_sibling(rows: &[PromptRow], index: usize) -> bool {
    let depth = rows[index].depth;
    for row in &rows[index + 1..] {
        if row.depth < depth {
            return true;
        }
        if row.depth == depth {
            return false;
        }
    }
    true
}

/// 命中可见行索引（`panel.rows` 的下标，非选中索引）；搜索框行、区域外或超出末行返回 None。
pub fn row_at(area: Rect, panel: &PromptPanel, column: u16, row: u16) -> Option<usize> {
    if !area.contains((column, row).into()) {
        return None;
    }
    let rows_y = area.y.saturating_add(1);
    if row < rows_y {
        return None;
    }
    let index = panel.scroll + usize::from(row - rows_y);
    (index < panel.rows().len()).then_some(index)
}

/// 绘制单行：连接线 + 图标 + 名称 +（条目）右对齐状态标签。
#[allow(clippy::too_many_arguments)]
fn paint_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    panel: &PromptPanel,
    row: PromptRow,
    selected: bool,
    connector: &str,
    expanded: bool,
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
        x = paint_text(buf, x, y, &format!("{connector} "), sub, area.right());
    }
    match row.kind {
        PromptRowKind::Draft => {
            let icon_style = if selected { base } else { style::accent() };
            x = paint_text(
                buf,
                x,
                y,
                text::PROMPT_TREE_DRAFT_ICON,
                icon_style,
                area.right(),
            );
            x = paint_text(buf, x, y, " ", base, area.right());
            let max = usize::from(area.right().saturating_sub(x));
            paint_name(
                buf,
                x,
                y,
                text::PROMPT_TREE_DRAFT_LABEL,
                base,
                max,
                area.right(),
            );
        }
        PromptRowKind::Folder => {
            let Some(folder) = panel.tree.folder_of_row(row) else {
                return;
            };
            let triangle = if expanded {
                text::PROMPT_TREE_EXPANDED
            } else {
                text::PROMPT_TREE_COLLAPSED
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

//! 提示词树视图渲染与命中：只读状态，无 IO。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::prompt::{PromptPanel, PromptRow, PromptRowKind, PromptSection};
use crate::ui::widgets;
use crate::ui::{style, text};

/// 渲染树内容区：顶部搜索框行 + 分割线 + 过滤后的树行；返回搜索框硬件光标位置；
/// 空树/无命中显示占位；选中行以行首 `❯` 标记（同 Agents 条目），悬停行铺选中底色。
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
    paint_divider(buf, area, area.y + 1, focused);
    if area.height <= 2 {
        return cursor;
    }
    let rows_area = Rect {
        y: area.y + 2,
        height: area.height - 2,
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
        let hovered = hover == Some(index) && !matches!(row.kind, PromptRowKind::Section(_));
        let prefix = tree_prefix(&rows, index);
        // 过滤时折叠被忽略：过滤视图下所有可见文件夹按展开态绘制三角。
        let searching = !panel.search.text().trim().is_empty();
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
            hovered,
            &prefix,
            expanded,
        );
    }
    // 无树行时显示空状态 / 无命中占位。
    if rows.is_empty() && rows_area.height >= 1 {
        let empty = if panel.search.text().trim().is_empty() {
            text::PROMPT_TREE_EMPTY
        } else {
            text::PROMPT_TREE_NO_MATCHES
        };
        paint_text(
            buf,
            rows_area.x,
            rows_area.y,
            empty,
            style::muted(),
            rows_area.right(),
        );
    }
    cursor
}

/// 绘制搜索框下方分割线：整行 `─`，两端 `├` / `┤` 衔接面板边框；样式跟随边框焦点态。
fn paint_divider(buf: &mut Buffer, area: Rect, y: u16, focused: bool) {
    let divider_style = style::border(focused);
    let left_join = area.x.saturating_sub(1);
    for x in left_join..=area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            let symbol = if x == left_join && left_join < area.x {
                text::DIVIDER_LEFT_JOIN
            } else {
                text::DIVIDER_MID
            };
            cell.set_symbol(symbol);
            cell.set_style(divider_style);
        }
    }
    if let Some(cell) = buf.cell_mut((area.right().saturating_add(1), y)) {
        cell.reset();
        cell.set_symbol(text::DIVIDER_RIGHT_JOIN);
        cell.set_style(divider_style);
    }
}

/// 绘制顶部搜索框行：`>` 前缀 + 单行输入视口（与模态输入框一致：硬件光标 + 占位）；
/// 返回光标位置（图标与空档之后为输入起点）。
fn paint_search_row(buf: &mut Buffer, area: Rect, panel: &PromptPanel) -> Option<(u16, u16)> {
    let mut x = paint_text(
        buf,
        area.x,
        area.y,
        text::INPUT_PREFIX_ICON,
        style::accent(),
        area.right(),
    );
    x = paint_text(buf, x, area.y, " ", style::text(), area.right());
    let width = usize::from(area.right().saturating_sub(x));
    let input = widgets::input::view(panel.search.text(), panel.search.cursor(), width);
    let (visible, text_style) = if panel.search.text().is_empty() {
        (text::PROMPT_TREE_SEARCH_PLACEHOLDER, style::muted())
    } else {
        (input.visible, style::text())
    };
    paint_text(buf, x, area.y, visible, text_style, area.right());
    Some((x.saturating_add(input.cursor_col), area.y))
}

/// 行前缀：各祖先层的续接线（`│` 或空格）+ 本行连接符（`├─` / `└─`）；根层为空。
///
/// 第 s 段（列 3s）续接第 s+1 层行的连接符列：该层祖先之后仍有同层行时画 `│  `，
/// 否则留空；保证嵌套文件夹内的条目与后续兄弟分支的竖线相接。
fn tree_prefix(rows: &[PromptRow], index: usize) -> String {
    let depth = rows[index].depth;
    if depth == 0 {
        return String::new();
    }
    let mut prefix = String::new();
    for level in 1..depth {
        let ancestor = rows[..index].iter().rposition(|row| row.depth == level);
        let last = ancestor.is_none_or(|ancestor| last_sibling(rows, ancestor));
        prefix.push_str(if last { "   " } else { "│  " });
    }
    prefix.push_str(if last_sibling(rows, index) {
        text::PROMPT_TREE_LAST
    } else {
        text::PROMPT_TREE_MIDDLE
    });
    prefix.push(' ');
    prefix
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

/// 命中可见行索引（`panel.rows` 的下标，非选中索引）；搜索框/分割线行、区域外或超出末行返回 None。
pub fn row_at(area: Rect, panel: &PromptPanel, column: u16, row: u16) -> Option<usize> {
    if !area.contains((column, row).into()) {
        return None;
    }
    let rows_y = area.y.saturating_add(2);
    if row < rows_y {
        return None;
    }
    let index = panel.scroll + usize::from(row - rows_y);
    (index < panel.rows().len()).then_some(index)
}

/// 绘制单行：行首标记列（`❯ ` 或空）+ 前缀（续接线 + 连接符）+ 图标 + 名称
/// +（条目）右对齐的彩色计数组；分区标签行只画 muted 标签；
/// 悬停行整行铺选中底色（与工作区列表一致），文字统一切换为高亮前景。
#[allow(clippy::too_many_arguments)]
fn paint_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    panel: &PromptPanel,
    row: PromptRow,
    selected: bool,
    hovered: bool,
    prefix: &str,
    expanded: bool,
) {
    let highlight = if hovered {
        style::selection()
    } else {
        ratatui::style::Style::default()
    };
    if hovered {
        fill_row(buf, area, y);
    }
    let base = style::text().patch(highlight);
    let sub = style::muted().patch(highlight);
    let accent = style::accent().patch(highlight);
    let muted = style::muted().patch(highlight);
    let mut x = area.x;
    // 分区标签不可选中：不占标记列，标签贴左缘。
    if !matches!(row.kind, PromptRowKind::Section(_)) {
        let marker = if selected {
            format!("{} ", text::AGENT_SELECTED_MARKER)
        } else {
            "  ".to_string()
        };
        x = paint_text(buf, x, y, &marker, base, area.right());
    }
    if !prefix.is_empty() {
        x = paint_text(buf, x, y, prefix, sub, area.right());
    }
    match row.kind {
        PromptRowKind::Folder => {
            let Some(folder) = panel.tree.folder_of_row(row) else {
                return;
            };
            let triangle = if expanded {
                text::PROMPT_TREE_EXPANDED
            } else {
                text::PROMPT_TREE_COLLAPSED
            };
            x = paint_text(buf, x, y, triangle, accent, area.right());
            x = paint_text(buf, x, y, " ", base, area.right());
            let max = usize::from(area.right().saturating_sub(x));
            paint_name(buf, x, y, &folder.name, base, max, area.right());
        }
        PromptRowKind::Item => {
            let Some(item) = panel.tree.item_of_row(row) else {
                return;
            };
            x = paint_text(buf, x, y, text::PROMPT_TREE_ITEM_ICON, muted, area.right());
            x = paint_text(buf, x, y, " ", base, area.right());
            // 右侧段：各页模板块计数合计（零数量省略）；
            // 物理宽度不足时从尾部逐个省略完整计数标签，名称最后截断。
            let mut segments: Vec<(String, ratatui::style::Style)> = Vec::new();
            let counts = item.template_counts();
            for (status, count) in [
                (crate::app::markdown::TemplateStatus::Todo, counts.todo),
                (
                    crate::app::markdown::TemplateStatus::InProgress,
                    counts.in_progress,
                ),
                (crate::app::markdown::TemplateStatus::Done, counts.done),
            ] {
                if count == 0 {
                    continue;
                }
                segments.push((
                    text::template_count_label(status, count),
                    style::template_status(status).patch(highlight),
                ));
            }
            let used = paint_right_segments(buf, area, y, &mut segments, x);
            let max = usize::from(used.saturating_sub(x).saturating_sub(1));
            paint_name(buf, x, y, &item.name, base, max, area.right());
        }
        PromptRowKind::Section(section) => {
            let label = match section {
                PromptSection::Temp => text::PROMPT_SECTION_TMP,
                PromptSection::Saved => text::PROMPT_SECTION_SAVED,
            };
            paint_text(buf, x, y, label, sub, area.right());
        }
        PromptRowKind::Temp => {
            let Some(temp) = panel.temp_of_row(row) else {
                return;
            };
            x = paint_text(buf, x, y, text::PROMPT_TREE_ITEM_ICON, muted, area.right());
            x = paint_text(buf, x, y, " ", base, area.right());
            let max = usize::from(area.right().saturating_sub(x));
            paint_name(buf, x, y, temp.display_name(), base, max, area.right());
        }
    }
}

/// 悬停行整行铺选中底色，保证反馈跨满行宽。
fn fill_row(buf: &mut Buffer, area: Rect, y: u16) {
    for x in area.x..area.right() {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(style::selection());
        }
    }
}

/// 右对齐绘制状态/计数段（从右向左排布）；宽度不足时从尾部逐个省略完整标签；
/// 返回最左绘制段之后的 x（名称区右界，名称与前一段留 1 列空档）。
fn paint_right_segments(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    segments: &mut Vec<(String, ratatui::style::Style)>,
    left_limit: u16,
) -> u16 {
    loop {
        let total: usize = segments
            .iter()
            .map(|(label, _)| width_of(label))
            .sum::<usize>()
            + segments.len().saturating_sub(1);
        if segments.is_empty() || total <= usize::from(area.right().saturating_sub(left_limit)) {
            break;
        }
        segments.pop();
    }
    let mut x = area.right();
    for (index, (label, label_style)) in segments.iter().enumerate().rev() {
        let width = width_of(label) as u16;
        x = x.saturating_sub(width);
        paint_text(buf, x, y, label, *label_style, area.right());
        if index > 0 {
            x = x.saturating_sub(1);
        }
    }
    x
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

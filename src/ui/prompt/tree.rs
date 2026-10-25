//! 提示词树视图渲染与命中：只读状态，无 IO。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::prompt::{PromptDropHit, PromptPanel, PromptRow, PromptRowKind, PromptSection};
use crate::app::state::{PromptDrag, PromptDropMark};
use crate::ui::widgets;
use crate::ui::{style, text};

/// 渲染树内容区：顶部搜索框行 + 分割线 + 过滤后的树行；返回搜索框硬件光标位置；
/// 空树/无命中显示占位；选中行以行首 `❯` 标记（同 Agents 条目），悬停行铺选中底色。
///
/// `drag` 为仅在指针已跨行时给出的拖拽现场，渲染据此画两处反馈（与工作区列表同款
/// 整行底色）：**源行压暗**表示"这一项正在被拖走"，**目标容器的行铺 `selection`**
/// 表示"松手会落到这里"——高亮跟着指针走，就是拖动最直接的可视反馈。
/// 落点为根层时没有容器行可铺，改在末行以下空白画一行 `→ move to root` 提示。
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    panel: &PromptPanel,
    focused: bool,
    hover: Option<usize>,
    drag: Option<PromptDrag>,
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
    let scroll = effective_scroll(panel, rows.len());
    let drop_mark = drag
        .map(|drag| drag.drop_mark)
        .unwrap_or(PromptDropMark::None);
    for (index, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(rows_area.height))
    {
        let selected = index == panel.selected;
        let section = matches!(row.kind, PromptRowKind::Section(_));
        let hovered = hover == Some(index) && !section;
        let drag_row = if section {
            DragRow::None
        } else if drag.is_some_and(|drag| drag.dragging && drag.from_row == index) {
            DragRow::Source
        } else if drop_mark == PromptDropMark::Row(index) {
            DragRow::Target
        } else {
            DragRow::None
        };
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
            drag_row,
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
    // 根层没有容器行可铺底色：在末行下方的空白处铺一行落点提示，让"会落回根层"可见。
    // 空树时该行属于空状态占位（见上），根层提示与它互斥，否则会盖掉 `no prompts yet`。
    if drop_mark == PromptDropMark::Root && !rows.is_empty() {
        let blank = rows.len().saturating_sub(scroll);
        if blank < usize::from(rows_area.height) {
            let y = rows_area.y + blank as u16;
            fill_row(buf, rows_area, y);
            paint_text(
                buf,
                rows_area.x,
                y,
                text::PROMPT_TREE_DROP_ROOT,
                style::text().patch(style::selection()),
                rows_area.right(),
            );
        }
    }
    cursor
}

/// 单行的拖拽反馈：源行压暗表示"正在搬走"，目标行铺底色表示"松手落在这里"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragRow {
    None,
    Source,
    Target,
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

/// 滚动偏移的钳制：`panel.scroll` 超出末行时按末行显示。
/// 渲染与命中共用，避免行数骤减的瞬时窗口里出现"画的是末行、命中的是越界行"。
fn effective_scroll(panel: &PromptPanel, rows_len: usize) -> usize {
    panel.scroll.min(rows_len.saturating_sub(1))
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
    let rows_len = panel.rows().len();
    let index = effective_scroll(panel, rows_len) + usize::from(row - rows_y);
    (index < rows_len).then_some(index)
}

/// 拖拽落点：区域外或搜索框/分割线行 → `Outside`；末行以下空白 → `Root`；其余按可见行换算。
pub fn drop_hit_at(area: Rect, panel: &PromptPanel, column: u16, row: u16) -> PromptDropHit {
    if !area.contains((column, row).into()) {
        return PromptDropHit::Outside;
    }
    let rows_y = area.y.saturating_add(2);
    if row < rows_y {
        return PromptDropHit::Outside;
    }
    let rows_len = panel.rows().len();
    let index = effective_scroll(panel, rows_len) + usize::from(row - rows_y);
    if index < rows_len {
        PromptDropHit::Row(index)
    } else {
        PromptDropHit::Root
    }
}

/// 绘制单行：行首标记列（`❯ ` 或空）+ 前缀（续接线 + 连接符）+ 图标 + 名称
/// +（条目）右对齐的彩色计数组；分区标签行只画 muted 标签；
/// 悬停行与拖拽目标行整行铺选中底色（与工作区列表一致），文字统一切换为高亮前景；
/// 拖拽源行整行压暗，读作"这一项已经被拿起来了"。
#[allow(clippy::too_many_arguments)]
fn paint_row(
    buf: &mut Buffer,
    area: Rect,
    y: u16,
    panel: &PromptPanel,
    row: PromptRow,
    selected: bool,
    hovered: bool,
    drag_row: DragRow,
    prefix: &str,
    expanded: bool,
) {
    let highlighted = hovered || drag_row == DragRow::Target;
    let highlight = if highlighted {
        style::selection()
    } else if drag_row == DragRow::Source {
        // 源行不铺底色（底色会被读成"落点"），改为整体 dim，作为"拖走中"的弱化标记。
        style::muted()
    } else {
        ratatui::style::Style::default()
    };
    if highlighted {
        fill_row(buf, area, y);
    }
    let base = style::text().patch(highlight);
    let sub = style::muted().patch(highlight);
    // 折叠三角是特殊图标：保持 accent 色，hover 只提供背景，避免图标颜色随 hover 变化。
    let accent = highlight.patch(style::accent());
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
            // 右侧段：各页模板块计数合计 `n|m|x`（数字按状态分色、分隔符弱化、
            // 三态全零整组省略）；放不下整组时整体省略，名称最后截断。
            let segments = text::template_count_segments(&item.template_counts());
            let group_width = segments
                .iter()
                .map(|(label, _)| width_of(label))
                .sum::<usize>();
            let group_x = area
                .right()
                .saturating_sub(u16::try_from(group_width).unwrap_or(u16::MAX));
            let used = if !segments.is_empty() && group_x > x {
                let mut cursor = group_x;
                for (label, status) in &segments {
                    let label_style = match status {
                        Some(status) => base.patch(style::template_status(*status)),
                        None => base.patch(style::muted()),
                    };
                    cursor = paint_text(buf, cursor, y, label, label_style, area.right());
                }
                group_x
            } else {
                area.right()
            };
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

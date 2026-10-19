//! markdown 表格渲染：列宽测量、按宽度收缩、方框网格与单元格内换行。
//!
//! 单元格先做行内渲染（折叠 `**` 等标记）再按列宽换行；`<br>` 变体换行成多物理行。
//! 表格总宽绝不超过给定宽度（列宽按比例收缩、每列不低于其最宽字符宽）。

use ratatui::style::Style;

use super::{MdLine, char_width, display_width, inline_spans, style};

/// 表格：表头 + 对齐行 + 数据行，用方框网格。
///
/// `width` 为 Some 时按可用宽度收缩列宽（超出部分在单元格内软换行），
/// 保证方框网格不超宽、不被外层硬折断。
pub(super) fn table(
    source: &[&str],
    start: usize,
    lines: &mut Vec<MdLine>,
    width: Option<usize>,
) -> usize {
    let mut rows = Vec::new();
    let mut index = start;
    while index < source.len() && source[index].trim_start().starts_with('|') {
        rows.push(parse_row(source[index]));
        index += 1;
    }
    if rows.is_empty() {
        return index;
    }
    let header = rows.first().cloned().unwrap_or_default();
    // 第二行若为对齐行（全为 `:`/`-`），解析对齐后跳过。
    let align = rows
        .get(1)
        .map(|cells| cells.iter().map(|cell| column_align(cell)).collect())
        .unwrap_or_else(|| vec![Align::Left; header.len()]);
    let body: Vec<Vec<String>> = rows
        .iter()
        .skip(if rows.len() > 1 { 2 } else { 1 })
        .cloned()
        .collect();
    let columns = header.len();
    let mut widths = vec![1usize; columns];
    for row in std::iter::once(&header).chain(body.iter()) {
        for (column, cell) in row.iter().enumerate() {
            // 自然宽度按可见内容测（去掉 `**` 等标记、`<br>` 取最宽行）。
            let width = cell_measure(cell);
            if let Some(slot) = widths.get_mut(column) {
                *slot = (*slot).max(width);
            }
        }
    }
    // 行尾若有数据行但其列数不足，补齐到表头列数。
    let header = pad_row(header, columns);
    let body = body
        .into_iter()
        .map(|row| pad_row(row, columns))
        .collect::<Vec<_>>();
    if let Some(width) = width {
        let floors: Vec<usize> = (0..columns)
            .map(|column| {
                std::iter::once(&header)
                    .chain(body.iter())
                    .filter_map(|row| row.get(column))
                    .flat_map(|cell| cell.chars())
                    .map(|ch| char_width(ch) as usize)
                    .max()
                    .unwrap_or(1)
                    .max(1)
            })
            .collect();
        shrink_widths(&mut widths, &floors, width);
    }
    grid_table(&header, &body, &widths, &align, lines);
    index
}

/// 按可用宽度收缩列宽：总占宽（各列内容 + 每列装饰）超出 `width` 时按比例削减，
/// 每列不低于其「最宽字符」宽度（保证至少放得下一个字符）；削减后由 `grid_table`
/// 在单元格内换行。
fn shrink_widths(widths: &mut [usize], floors: &[usize], width: usize) {
    let columns = widths.len();
    if columns == 0 {
        return;
    }
    // 每列装饰：左右各 1 空格 + 1 个竖线；另有 `columns + 1` 条竖线（首尾 + 列间）。
    let decorations = columns + 1 + columns * 2;
    let available = width.saturating_sub(decorations);
    let floor_total: usize = floors.iter().copied().sum();
    if floor_total > available {
        // 即使按最小列宽也放不下：只保证各列不低于最小字符宽（外层仍可能轻微超宽，
        // 但不再出现「列宽小于单字」导致的错位）。
        for (slot, floor) in widths.iter_mut().zip(floors) {
            *slot = (*slot).max(*floor);
        }
        return;
    }
    let total: usize = widths.iter().sum();
    if total <= available {
        return;
    }
    // 按比例分配，再补齐/削减到 [floor, original] 区间。
    let mut scaled = vec![0usize; columns];
    for index in 0..columns {
        let share = widths[index].saturating_mul(available) / total;
        scaled[index] = share.clamp(floors[index], widths[index]);
    }
    let mut used: usize = scaled.iter().sum();
    while used > available {
        let Some((index, _)) = scaled
            .iter()
            .enumerate()
            .filter(|(index, value)| **value > floors[*index])
            .max_by_key(|(_, value)| **value)
        else {
            break;
        };
        scaled[index] -= 1;
        used -= 1;
    }
    while used < available {
        let Some((index, _)) = scaled
            .iter()
            .enumerate()
            .filter(|(index, value)| **value < widths[*index])
            .max_by_key(|(index, value)| widths[*index] - **value)
        else {
            break;
        };
        scaled[index] += 1;
        used += 1;
    }
    for (slot, value) in widths.iter_mut().zip(scaled) {
        *slot = value;
    }
}

/// 把一行单元格补齐到 `columns` 列（不足补空串）。
fn pad_row(mut cells: Vec<String>, columns: usize) -> Vec<String> {
    while cells.len() < columns {
        cells.push(String::new());
    }
    cells
}

/// 解析一行表格为单元格（去掉首尾 `|` 与空白）。
fn parse_row(line: &str) -> Vec<String> {
    let trimmed = line.trim().trim_start_matches('|').trim_end_matches('|');
    trimmed
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// 列对齐。
#[derive(Debug, Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

/// 由对齐行单元格解析列对齐（`:--` 左、`:-:` 中、`--:` 右）。
fn column_align(cell: &str) -> Align {
    let left = cell.starts_with(':');
    let right = cell.ends_with(':');
    match (left, right) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    }
}

/// 方框网格表格：`┌─┬─┐` / `│ x │` / `├─┼─┤` / `└─┴─┘`。
///
/// 单元格先做行内渲染（折叠 `**` 等标记，链接显示 `文字 (url)`），按列宽换行；
/// 单个逻辑行拆成多条物理行（同一行内各列的对应物理行并排）。表头与数据行同色，
/// 仅在单元格显式加粗处加粗（避免表头整行变黄）。
fn grid_table(
    header: &[String],
    body: &[Vec<String>],
    widths: &[usize],
    align: &[Align],
    lines: &mut Vec<MdLine>,
) {
    // 单元格行：两端与列间用 `│`，内容左右各留 1 空格；内容超出列宽时换行成多物理行。
    let border = |left: &str, fill: &str, mid: &str, right: &str| -> String {
        let segments: Vec<String> = widths.iter().map(|width| fill.repeat(width + 2)).collect();
        format!("{left}{}{right}", segments.join(mid))
    };
    let row_lines = |cells: &[String], fallback: Style| -> Vec<Vec<(String, Style)>> {
        let rule = style::md_read_rule();
        // 每列：行内渲染 → 拆 `<br>` → 按列宽换行（保留分段样式与对齐）。
        let wrapped: Vec<Vec<Vec<(String, Style)>>> = widths
            .iter()
            .enumerate()
            .map(|(column, width)| {
                let empty = String::new();
                let cell = cells.get(column).unwrap_or(&empty);
                let align = align.get(column).copied().unwrap_or(Align::Left);
                let rendered = render_cell_spans(cell, fallback);
                let logical = split_br(&rendered);
                let lines: Vec<Vec<(String, Style)>> = logical
                    .iter()
                    .flat_map(|line| wrap_cell_spans(line, *width, align))
                    .collect();
                if lines.is_empty() {
                    vec![vec![(" ".repeat(*width), fallback)]]
                } else {
                    lines
                }
            })
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let mut out = Vec::with_capacity(height);
        for row in 0..height {
            let mut spans = vec![("│".to_string(), rule)];
            for (column, cell_lines) in wrapped.iter().enumerate() {
                let blank = vec![(" ".repeat(widths[column]), fallback)];
                let line = cell_lines.get(row).unwrap_or(&blank);
                spans.push((" ".to_string(), rule));
                spans.extend(line.iter().cloned());
                spans.push((" ".to_string(), rule));
                spans.push(("│".to_string(), rule));
            }
            out.push(spans);
        }
        out
    };
    lines.push(MdLine::plain(
        0,
        border("┌", "─", "┬", "┐"),
        style::md_read_rule(),
    ));
    for spans in row_lines(header, style::text()) {
        lines.push(MdLine::spans(0, spans));
    }
    // 表头之后、以及每一条数据行之后都补一条水平分割线（末尾与下边框合并）。
    // 第一列为空的行视为上一条目的**续行**（对齐 markdown 里「同组多行、首列留空」的写法）：
    // 其上方不画分割线，使同组内容视觉合并为一块。
    lines.push(MdLine::plain(
        0,
        border("├", "─", "┼", "┤"),
        style::md_read_rule(),
    ));
    for index in 0..body.len() {
        for spans in row_lines(&body[index], style::text()) {
            lines.push(MdLine::spans(0, spans));
        }
        let next_is_continuation = body
            .get(index + 1)
            .is_none_or(|next| is_continuation_row(next));
        if index + 1 < body.len() && !next_is_continuation {
            lines.push(MdLine::plain(
                0,
                border("├", "─", "┼", "┤"),
                style::md_read_rule(),
            ));
        }
    }
    lines.push(MdLine::plain(
        0,
        border("└", "─", "┴", "┘"),
        style::md_read_rule(),
    ));
}

/// 续行判定：逻辑行第一列为空（首列留空即视为上一条目的续行，不单独画分割线）。
fn is_continuation_row(row: &[String]) -> bool {
    row.first().is_none_or(|cell| cell.trim().is_empty())
}

/// 单元格自然宽度：折叠行内标记后取最宽行（`<br>` 分行）。
fn cell_measure(cell: &str) -> usize {
    let rendered = inline_spans(cell);
    split_br(&rendered)
        .iter()
        .map(|line| display_width(&spans_text(line)))
        .max()
        .unwrap_or(0)
}

/// 单元格渲染为带样式分段：先折叠行内标记（`**`/`` ` ``/链接等），未显式指定样式处
/// 用 `fallback`（表头与数据行同为正文色）。
fn render_cell_spans(cell: &str, fallback: Style) -> Vec<(String, Style)> {
    if cell.is_empty() {
        return Vec::new();
    }
    inline_spans(cell)
        .into_iter()
        .map(|(value, style)| {
            let style = if style == style::text() {
                fallback
            } else {
                style
            };
            (value, style)
        })
        .collect()
}

/// 把分段按 `<br>` / `<br/>` / `<br />` 拆成多行（保留各段样式）。
fn split_br(spans: &[(String, Style)]) -> Vec<Vec<(String, Style)>> {
    let mut lines: Vec<Vec<(String, Style)>> = vec![Vec::new()];
    for (value, style) in spans {
        let mut rest = value.as_str();
        while let Some(index) = find_br(rest) {
            let head = &rest[..index];
            if !head.is_empty() {
                push_span(lines.last_mut().expect("line"), head, *style);
            }
            lines.push(Vec::new());
            rest = &rest[index + br_len(&rest[index..])..];
        }
        if !rest.is_empty() {
            push_span(lines.last_mut().expect("line"), rest, *style);
        }
    }
    lines
}

/// 找到下一个 `<br>` 变体的起始位置。
fn find_br(value: &str) -> Option<usize> {
    let mut search = 0usize;
    while let Some(offset) = value[search..].find('<') {
        let index = search + offset;
        if br_len(&value[index..]) > 0 {
            return Some(index);
        }
        search = index + 1;
    }
    None
}

/// `<br>` 变体在串首时的字节长度；非 `<br>` 返回 0。
fn br_len(value: &str) -> usize {
    for candidate in ["<br />", "<br/>", "<br>"] {
        if value.starts_with(candidate) {
            return candidate.len();
        }
    }
    0
}

/// 追加分段（与末段同色则合并）。
fn push_span(line: &mut Vec<(String, Style)>, value: &str, style: Style) {
    if value.is_empty() {
        return;
    }
    if let Some(last) = line.last_mut()
        && last.1 == style
    {
        last.0.push_str(value);
        return;
    }
    line.push((value.to_string(), style));
}

/// 分段纯文本。
fn spans_text(spans: &[(String, Style)]) -> String {
    spans.iter().map(|(value, _)| value.as_str()).collect()
}

/// 单元格按列宽换行（优先在空白处断行，超长不可断内容按显示列硬切），保留分段样式，
/// 每物理行按对齐补空格到列宽。
fn wrap_cell_spans(
    line: &[(String, Style)],
    width: usize,
    align: Align,
) -> Vec<Vec<(String, Style)>> {
    let width = width.max(1);
    let cells: Vec<(char, Style)> = line
        .iter()
        .flat_map(|(value, style)| value.chars().map(|ch| (ch, *style)))
        .collect();
    if cells.is_empty() {
        return vec![vec![(" ".repeat(width), style::text())]];
    }
    let mut out: Vec<Vec<(String, Style)>> = Vec::new();
    let mut current: Vec<(String, Style)> = Vec::new();
    let mut col = 0usize;
    // 以「词 + 尾随空白」为切分单位，中英文混排时中文逐字成词。
    let mut segment: Vec<(char, Style)> = Vec::new();
    let flush_segment = |segment: &mut Vec<(char, Style)>,
                         current: &mut Vec<(String, Style)>,
                         col: &mut usize,
                         out: &mut Vec<Vec<(String, Style)>>| {
        for (ch, style) in segment.drain(..) {
            let cell_w = char_width(ch) as usize;
            if *col + cell_w > width {
                out.push(pad_line(current, width, align));
                current.clear();
                *col = 0;
            }
            push_span(current, &ch.to_string(), style);
            *col += cell_w;
        }
    };
    for (ch, style) in &cells {
        segment.push((*ch, *style));
        if ch.is_whitespace() {
            flush_segment(&mut segment, &mut current, &mut col, &mut out);
        }
    }
    flush_segment(&mut segment, &mut current, &mut col, &mut out);
    if !current.is_empty() {
        out.push(pad_line(&current, width, align));
    }
    if out.is_empty() {
        out.push(vec![(" ".repeat(width), style::text())]);
    }
    out
}

/// 把一条物理行按对齐把剩余空列补成空格分段。
fn pad_line(line: &[(String, Style)], width: usize, align: Align) -> Vec<(String, Style)> {
    let len = display_width(&spans_text(line));
    let pad = width.saturating_sub(len);
    let (before, after) = match align {
        Align::Left => (0, pad),
        Align::Right => (pad, 0),
        Align::Center => (pad / 2, pad - pad / 2),
    };
    let mut out = Vec::new();
    let pad_style = line.first().map(|(_, style)| *style).unwrap_or_default();
    if before > 0 {
        out.push((" ".repeat(before), pad_style));
    }
    out.extend(line.iter().cloned());
    if after > 0 {
        out.push((" ".repeat(after), pad_style));
    }
    if out.is_empty() {
        out.push((" ".repeat(width), pad_style));
    }
    out
}

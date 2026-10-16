//! `markdown` item 的渲染器：把 agent 回答的 markdown 源码渲染成带样式的行。
//!
//! 这是**面向阅读的渲染**（吃掉 `#`、`**`、``` 等标记），与编辑器用的
//! `ui/markdown.rs` 高亮（保留标记上色）是两回事，故独立实现。
//!
//! 支持全部常见 markdown 格式：六级标题、段落、无序/有序/嵌套列表、任务列表、
//! 引用、围栏/缩进代码块、表格（对齐）、水平线、图片，以及行内
//! 粗体/斜体/粗斜体/删除线/行内代码/链接/自动链接/裸 URL。
//!
//! 采用 lx 直角风格：标题隐藏 `#`、列表用 `•`、引用用 `>`（不用左边框）、
//! 代码块做轻量语法高亮；表格用方框网格；任务列表复用 claude 一组复选框
//! （`[x]`/`[ ]`）。

use ratatui::style::Style;

use crate::ui::{style, text};

use super::SampleLine;

mod highlight;

/// 渲染整个样例文档为若干行（含末尾 token 页脚）。
pub(super) fn markdown_lines(tokens: super::Tokens) -> Vec<SampleLine> {
    let mut lines = Vec::new();
    let source_lines: Vec<&str> = text::agent::MD_SAMPLE.split('\n').collect();
    let mut index = 0;
    while index < source_lines.len() {
        let line = source_lines[index];
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            index += 1;
            continue;
        }
        if trimmed.trim_start().starts_with("```") {
            index = fenced_code(&source_lines, index, &mut lines);
            continue;
        }
        if is_indented_code(line) {
            index = indented_code(&source_lines, index, &mut lines);
            continue;
        }
        if let Some((level, body)) = heading(trimmed) {
            lines.push(heading_line(level, body));
            index += 1;
            continue;
        }
        if is_rule(trimmed) {
            lines.push(SampleLine::plain(
                0,
                RULE.repeat(40),
                style::markdown_rule(),
            ));
            index += 1;
            continue;
        }
        if trimmed.trim_start().starts_with('|') {
            index = table(&source_lines, index, &mut lines);
            continue;
        }
        if trimmed.trim_start().starts_with('>') {
            index = blockquote(&source_lines, index, &mut lines);
            continue;
        }
        if let Some(item) = list_item(line) {
            lines.push(item.render());
            index += 1;
            continue;
        }
        if let Some((alt, url)) = image(trimmed) {
            lines.push(SampleLine::spans(
                0,
                vec![
                    (alt.to_string(), style::text()),
                    (" ".to_string(), style::text()),
                    (format!("({url})"), style::markdown_image()),
                ],
            ));
            index += 1;
            continue;
        }
        // 普通段落行：行内渲染。
        lines.push(SampleLine::spans(0, inline_spans(trimmed)));
        index += 1;
    }
    // 每一步都带 token 页脚。
    lines.push(SampleLine::plain(
        2,
        super::format_tokens_line(tokens),
        style::agent_tokens(),
    ));
    lines
}

/// 水平线字符。
const RULE: &str = "─";
/// 引用 / 代码竖条（直角）。
const BAR_MARKER: &str = "│";

/// ATX 标题：`#{1,6}` 加后随空白；返回（级别、正文）。
fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    let spaces = rest.bytes().take_while(|b| *b == b' ').count();
    (spaces > 0).then(|| (hashes as u8, rest[spaces..].trim_end()))
}

/// 渲染一行标题：隐藏 `#` 标记，正文用分级样式。
fn heading_line(level: u8, body: &str) -> SampleLine {
    SampleLine::plain(0, body, style::markdown_heading_level(level))
}

/// 水平线：仅由同一种 `-`/`*`/`_` 组成且至少 3 个。
fn is_rule(line: &str) -> bool {
    let mut symbol = None;
    let mut count = 0;
    for ch in line.chars() {
        match ch {
            ' ' | '\t' => {}
            '-' | '*' | '_' => {
                if symbol.is_some_and(|current| current != ch) {
                    return false;
                }
                symbol = Some(ch);
                count += 1;
            }
            _ => return false,
        }
    }
    count >= 3
}

/// 缩进代码块行：4 个以上前导空格且非空。
fn is_indented_code(line: &str) -> bool {
    let indent = line.len() - line.trim_start_matches(' ').len();
    indent >= 4 && !line.trim().is_empty()
}

/// 图片 `![alt](url)`。
fn image(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("![")?;
    let alt_close = rest.find(']')?;
    let after = rest.get(alt_close + 1..)?;
    let after = after.strip_prefix('(')?;
    let url_close = after.find(')')?;
    Some((&rest[..alt_close], &after[..url_close]))
}

/// 围栏代码块：从起始 ``` 行读到闭合 ```；隐藏围栏，显示语言标记。
fn fenced_code(source: &[&str], start: usize, lines: &mut Vec<SampleLine>) -> usize {
    let fence = source[start].trim();
    let lang = fence.trim_start_matches('`').trim();
    let mut index = start + 1;
    let mut body = Vec::new();
    while index < source.len() {
        if source[index].trim_start().starts_with("```") {
            index += 1;
            break;
        }
        body.push(source[index]);
        index += 1;
    }
    render_code(&body, (!lang.is_empty()).then_some(lang), lines);
    index
}

/// 缩进代码块：收集连续 4 空格缩进行。
fn indented_code(source: &[&str], start: usize, lines: &mut Vec<SampleLine>) -> usize {
    let mut index = start;
    let mut body = Vec::new();
    while index < source.len()
        && (is_indented_code(source[index]) || source[index].trim().is_empty())
    {
        if source[index].trim().is_empty() {
            // 段内空行保留，但行尾空行不追加。
            if index + 1 < source.len() && is_indented_code(source[index + 1]) {
                body.push("");
                index += 1;
                continue;
            }
            break;
        }
        // 去掉 4 空格缩进。
        body.push(source[index].strip_prefix("    ").unwrap_or(source[index]));
        index += 1;
    }
    render_code(&body, None, lines);
    index
}

/// 渲染代码块：每行前置直角竖条，语言作为 muted 首行；代码正文按语言做轻量语法高亮。
fn render_code(body: &[&str], lang: Option<&str>, lines: &mut Vec<SampleLine>) {
    if let Some(lang) = lang {
        lines.push(SampleLine::spans(
            0,
            vec![
                (format!("{BAR_MARKER} "), style::markdown_rule()),
                (lang.to_string(), style::markdown_code_lang()),
            ],
        ));
    }
    for line in body {
        let mut spans = vec![(format!("{BAR_MARKER} "), style::markdown_rule())];
        spans.extend(highlight::highlight_line(lang, line));
        lines.push(SampleLine::spans(0, spans));
    }
}

/// 引用块：收集连续 `>` 行，用 `>` 作前缀（不用左边框）。
fn blockquote(source: &[&str], start: usize, lines: &mut Vec<SampleLine>) -> usize {
    let mut index = start;
    while index < source.len() {
        let trimmed = source[index].trim_start();
        let Some(rest) = trimmed.strip_prefix('>') else {
            break;
        };
        let body = rest.strip_prefix(' ').unwrap_or(rest);
        lines.push(SampleLine::spans(
            0,
            vec![
                (
                    format!("{} ", text::agent::MD_QUOTE_MARKER),
                    style::markdown_marker(),
                ),
                (body.to_string(), style::markdown_quote()),
            ],
        ));
        index += 1;
    }
    index
}

/// 列表项（无序 / 有序 / 任务），含两级嵌套缩进。
#[derive(Debug, Clone, Copy)]
enum ListItem<'a> {
    Bullet {
        depth: usize,
        text: &'a str,
    },
    Ordered {
        depth: usize,
        number: &'a str,
        text: &'a str,
    },
    Task {
        depth: usize,
        done: bool,
        text: &'a str,
    },
}

impl ListItem<'_> {
    /// 渲染一行列表项（lx：`•` 项目符号；任务用 claude 一组 `[x]`/`[ ]`）。
    fn render(&self) -> SampleLine {
        match *self {
            Self::Bullet { depth, text: body } => SampleLine::spans(
                indent(depth),
                vec![
                    ("• ".to_string(), style::markdown_bullet()),
                    (body.to_string(), style::text()),
                ],
            ),
            Self::Ordered {
                depth,
                number,
                text: body,
            } => SampleLine::spans(
                indent(depth),
                vec![
                    (format!("{number}. "), style::markdown_ordered()),
                    (body.to_string(), style::text()),
                ],
            ),
            Self::Task {
                depth,
                done,
                text: body,
            } => {
                let (marker, body_style) = if done {
                    ("[x]", style::markdown_task_done())
                } else {
                    ("[ ]", style::markdown_task_open())
                };
                SampleLine::spans(
                    indent(depth),
                    vec![
                        (format!("{marker} "), body_style),
                        (body.to_string(), body_style),
                    ],
                )
            }
        }
    }
}

/// 列表项缩进（每级 2 列）。
fn indent(depth: usize) -> u16 {
    (depth * 2) as u16
}

/// 解析列表项；非列表返回 None。
fn list_item(line: &str) -> Option<ListItem<'_>> {
    let spaces = line.len() - line.trim_start_matches(' ').len();
    let depth = spaces / 2;
    let rest = &line[spaces..];
    // 任务列表：`- [ ] ` / `- [x] `。
    if let Some(after) = rest.strip_prefix("- [") {
        let done = after.starts_with('x') || after.starts_with('X');
        let marker = after.get(1..)?.strip_prefix("] ")?;
        return Some(ListItem::Task {
            depth,
            done,
            text: marker.trim_end(),
        });
    }
    // 无序列表。
    if let Some(after) = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))
    {
        return Some(ListItem::Bullet {
            depth,
            text: after.trim_end(),
        });
    }
    // 有序列表 `N. `。
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && rest.as_bytes().get(digits) == Some(&b'.') {
        let after = rest.get(digits + 1..)?.strip_prefix(' ')?;
        return Some(ListItem::Ordered {
            depth,
            number: &rest[..digits],
            text: after.trim_end(),
        });
    }
    None
}

/// 表格：表头 + 对齐行 + 数据行，用方框网格。
fn table(source: &[&str], start: usize, lines: &mut Vec<SampleLine>) -> usize {
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
            let width = display_width(cell);
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
    grid_table(&header, &body, &widths, &align, lines);
    index
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

/// 单元格按显示宽度与对齐填充。
fn pad(cell: &str, width: usize, align: Align) -> String {
    let len = display_width(cell);
    let pad = width.saturating_sub(len);
    let (before, after) = match align {
        Align::Left => (0, pad),
        Align::Right => (pad, 0),
        Align::Center => (pad / 2, pad - pad / 2),
    };
    format!("{}{}{}", " ".repeat(before), cell, " ".repeat(after))
}

/// 方框网格表格：`┌─┬─┐` / `│ x │` / `├─┼─┤` / `└─┴─┘`。
fn grid_table(
    header: &[String],
    body: &[Vec<String>],
    widths: &[usize],
    align: &[Align],
    lines: &mut Vec<SampleLine>,
) {
    let border = |left: &str, fill: &str, mid: &str, right: &str| -> String {
        let segments: Vec<String> = widths.iter().map(|width| fill.repeat(width + 2)).collect();
        format!("{left}{}{right}", segments.join(mid))
    };
    // 单元格行：两端与列间用 `│`，内容左右各留 1 空格。
    let row_line = |cells: &[String], value_style: Style| -> Vec<(String, Style)> {
        let rule = style::markdown_rule();
        let mut spans = vec![("│".to_string(), rule)];
        for (column, width) in widths.iter().enumerate() {
            let empty = String::new();
            let cell = cells.get(column).unwrap_or(&empty);
            let aligned = pad(
                cell,
                *width,
                align.get(column).copied().unwrap_or(Align::Left),
            );
            spans.push((" ".to_string(), rule));
            spans.push((aligned, value_style));
            spans.push((" ".to_string(), rule));
            spans.push(("│".to_string(), rule));
        }
        spans
    };
    lines.push(SampleLine::plain(
        0,
        border("┌", "─", "┬", "┐"),
        style::markdown_rule(),
    ));
    lines.push(SampleLine::spans(
        0,
        row_line(header, style::markdown_strong()),
    ));
    lines.push(SampleLine::plain(
        0,
        border("├", "─", "┼", "┤"),
        style::markdown_rule(),
    ));
    for row in body {
        lines.push(SampleLine::spans(0, row_line(row, style::text())));
    }
    lines.push(SampleLine::plain(
        0,
        border("└", "─", "┴", "┘"),
        style::markdown_rule(),
    ));
}

/// 行内渲染：吃掉粗体/斜体/粗斜体/删除线/行内代码标记；链接显示 `文字 (url)`。
fn inline_spans(text: &str) -> Vec<(String, Style)> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let rest = &text[index..];
        // 行内代码：`code`（内部不再解析）。
        if let Some(inner) = delimited(rest, "`", "`") {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::markdown_inline_code()));
            index += inner.len() + 2;
            continue;
        }
        // 粗斜体 `***…***`。
        if let Some(inner) = delimited(rest, "***", "***") {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::markdown_strong_emphasis()));
            index += inner.len() + 6;
            continue;
        }
        // 粗体 `**…**` / `__…__`。
        if let Some(inner) = delimited(rest, "**", "**").or_else(|| delimited(rest, "__", "__")) {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::markdown_strong()));
            index += inner.len() + 4;
            continue;
        }
        // 删除线 `~~…~~`。
        if let Some(inner) = delimited(rest, "~~", "~~") {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::markdown_strikethrough()));
            index += inner.len() + 4;
            continue;
        }
        // 斜体 `*…*` / `_…_`（排除双写）。
        if let Some(inner) = emphasis(rest) {
            flush(&mut plain, &mut spans);
            spans.push((inner.to_string(), style::markdown_emphasis()));
            index += inner.len() + 2;
            continue;
        }
        // 链接 `[文字](url)`。
        if rest.starts_with('[')
            && let Some((label, url, consumed)) = link(rest)
        {
            flush(&mut plain, &mut spans);
            spans.push((label.to_string(), style::markdown_link_text()));
            spans.push((format!(" ({url})"), style::markdown_url()));
            index += consumed;
            continue;
        }
        // 自动链接 `<url>`。
        if rest.starts_with('<')
            && let Some(close) = rest.find('>')
        {
            let url = &rest[1..close];
            if url.starts_with("http://") || url.starts_with("https://") {
                flush(&mut plain, &mut spans);
                spans.push((url.to_string(), style::markdown_url()));
                index += close + 1;
                continue;
            }
        }
        // 裸 URL。
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            flush(&mut plain, &mut spans);
            spans.push((rest[..end].to_string(), style::markdown_url()));
            index += end;
            continue;
        }
        let ch = rest.chars().next().unwrap_or('\0');
        plain.push(ch);
        index += ch.len_utf8();
    }
    flush(&mut plain, &mut spans);
    spans
}

/// 把累积的普通文本刷入 spans（正文色）。
fn flush(plain: &mut String, spans: &mut Vec<(String, Style)>) {
    if !plain.is_empty() {
        spans.push((std::mem::take(plain), style::text()));
    }
}

/// 匹配 `open … close` 并返回内部文本（需非空）。
fn delimited<'a>(rest: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let after = rest.strip_prefix(open)?;
    let end = after.find(close)?;
    (end > 0).then(|| &after[..end])
}

/// 斜体 `*…*` / `_…_`；排除双写前缀。
fn emphasis(rest: &str) -> Option<&str> {
    let mut chars = rest.chars();
    let ch = chars.next()?;
    if ch != '*' && ch != '_' {
        return None;
    }
    if chars.next() == Some(ch) {
        return None;
    }
    delimited(rest, &ch.to_string(), &ch.to_string())
}

/// 链接 `[文字](url)`；返回（文字、URL、消耗字节数）。
fn link(rest: &str) -> Option<(&str, &str, usize)> {
    let after = rest.strip_prefix('[')?;
    let label_close = after.find(']')?;
    let url_start = after.get(label_close + 1..)?.strip_prefix('(')?;
    let url_close = url_start.find(')')?;
    let consumed = 1 + label_close + 1 + 1 + url_close + 1;
    Some((&after[..label_close], &url_start[..url_close], consumed))
}

/// 字符串的显示宽度（按字符宽度求和）。
fn display_width(value: &str) -> usize {
    value.chars().map(|ch| char_width(ch) as usize).sum()
}

/// 单个字符的显示宽度。
fn char_width(symbol: char) -> u16 {
    use unicode_width::UnicodeWidthChar;
    symbol.width().unwrap_or(1).min(2) as u16
}

#[cfg(test)]
mod tests;

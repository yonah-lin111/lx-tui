//! `markdown` item 的渲染器：把 agent 回答的 markdown 源码渲染成带样式的行。
//!
//! 这是**面向阅读的渲染**（吃掉 `#`、`**`、``` 等标记），与编辑器用的
//! `ui/markdown.rs` 高亮（保留标记上色）是两回事，故独立实现。
//!
//! 支持全部常见 markdown 格式：六级标题、段落、无序/有序/嵌套列表、任务列表、
//! 引用、围栏/缩进代码块、表格（对齐）、水平线、图片，以及行内
//! 粗体/斜体/粗斜体/删除线/行内代码/链接/自动链接/裸 URL。
//!
//! 四套候选（`a` codex / `b` claude / `c` opencode / `d` lx）覆盖同一份样例，
//! 差异体现在：标题是否保留 `#`、列表项目符号、引用前缀、代码块是否带直角竖条、
//! 表格是方框网格还是无框分隔线。

use ratatui::style::Style;

use crate::ui::{style, text};

use super::SampleLine;

/// 渲染整个样例文档为若干行（含末尾 token 页脚）。
///
/// `variant`：0 codex / 1 claude / 2 opencode / 3 lx；四套只是“摆盘”差异。
pub(super) fn markdown_lines(variant: usize, tokens: super::Tokens) -> Vec<SampleLine> {
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
            index = fenced_code(&source_lines, index, variant, &mut lines);
            continue;
        }
        if is_indented_code(line) {
            index = indented_code(&source_lines, index, variant, &mut lines);
            continue;
        }
        if let Some((level, body)) = heading(trimmed) {
            lines.push(heading_line(level, body, variant));
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
            index = table(&source_lines, index, variant, &mut lines);
            continue;
        }
        if trimmed.trim_start().starts_with('>') {
            index = blockquote(&source_lines, index, variant, &mut lines);
            continue;
        }
        if let Some(item) = list_item(line) {
            lines.push(item.render(variant));
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
/// 引用竖条（claude 套）。
const QUOTE_BAR_CLAUDE: &str = "▎";
/// 引用 / 代码竖条（lx 套，直角）。
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

/// 渲染一行标题：codex 保留 `#`，其余三级隐藏标记。
fn heading_line(level: u8, body: &str, variant: usize) -> SampleLine {
    let heading_style = style::markdown_heading_level(level);
    if variant == 0 {
        // codex：`#` 标记保留，正文用分级样式。
        SampleLine::spans(
            0,
            vec![
                (
                    format!("{} ", "#".repeat(usize::from(level))),
                    style::markdown_marker(),
                ),
                (body.to_string(), heading_style),
            ],
        )
    } else {
        SampleLine::plain(0, body, heading_style)
    }
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

/// 围栏代码块：从起始 ``` 行读到闭合 ```；codex/claude/opencode 隐藏围栏与语言，
/// lx 套在每行前置直角竖条并显示语言。
fn fenced_code(
    source: &[&str],
    start: usize,
    variant: usize,
    lines: &mut Vec<SampleLine>,
) -> usize {
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
    render_code(&body, (!lang.is_empty()).then_some(lang), variant, lines);
    index
}

/// 缩进代码块：收集连续 4 空格缩进行。
fn indented_code(
    source: &[&str],
    start: usize,
    variant: usize,
    lines: &mut Vec<SampleLine>,
) -> usize {
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
    render_code(&body, None, variant, lines);
    index
}

/// 渲染代码块：非 lx 套缩进 2 列；lx 套每行前置直角竖条，语言作为 muted 首行。
fn render_code(body: &[&str], lang: Option<&str>, variant: usize, lines: &mut Vec<SampleLine>) {
    if variant == 3 {
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
            lines.push(SampleLine::spans(
                0,
                vec![
                    (format!("{BAR_MARKER} "), style::markdown_rule()),
                    ((*line).to_string(), style::markdown_code_block()),
                ],
            ));
        }
    } else {
        for line in body {
            lines.push(SampleLine::plain(
                2,
                (*line).to_string(),
                style::markdown_code_block(),
            ));
        }
    }
}

/// 引用块：收集连续 `>` 行；codex/opencode 显示 `>`、claude 用 `▎`、lx 用 `│`。
fn blockquote(source: &[&str], start: usize, variant: usize, lines: &mut Vec<SampleLine>) -> usize {
    let mut index = start;
    while index < source.len() {
        let trimmed = source[index].trim_start();
        let Some(rest) = trimmed.strip_prefix('>') else {
            break;
        };
        let body = rest.strip_prefix(' ').unwrap_or(rest);
        let marker = match variant {
            1 => QUOTE_BAR_CLAUDE,
            3 => BAR_MARKER,
            _ => ">",
        };
        lines.push(SampleLine::spans(
            0,
            vec![
                (format!("{marker} "), style::markdown_marker()),
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
    /// 按候选风格渲染一行列表项。
    fn render(&self, variant: usize) -> SampleLine {
        match *self {
            Self::Bullet { depth, text: body } => {
                // codex/lx 用 `•`，claude/opencode 用 `-`。
                let marker = if matches!(variant, 0 | 3) { "•" } else { "-" };
                SampleLine::spans(
                    indent(depth),
                    vec![
                        (format!("{marker} "), style::markdown_bullet()),
                        (body.to_string(), style::text()),
                    ],
                )
            }
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
                // codex 用 `✔`/`□`，其余用 `[x]`/`[ ]`。
                let (marker, body_style) = if done {
                    (
                        if variant == 0 { "✔" } else { "[x]" },
                        style::markdown_task_done(),
                    )
                } else {
                    (
                        if variant == 0 { "□" } else { "[ ]" },
                        style::markdown_task_open(),
                    )
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

/// 表格：表头 + 对齐行 + 数据行；codex 无框分隔线，其余方框网格。
fn table(source: &[&str], start: usize, variant: usize, lines: &mut Vec<SampleLine>) -> usize {
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
    if variant == 0 {
        codex_table(&header, &body, &widths, &align, lines);
    } else {
        grid_table(&header, &body, &widths, &align, lines);
    }
    index
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

/// codex 无框表格：表头 + 粗分隔线 + 数据行（列间 2 空格）。
fn codex_table(
    header: &[String],
    body: &[Vec<String>],
    widths: &[usize],
    align: &[Align],
    lines: &mut Vec<SampleLine>,
) {
    let header_spans: Vec<(String, Style)> =
        join_cells(header, widths, align, style::markdown_strong());
    lines.push(SampleLine::spans(0, header_spans));
    let rule = widths
        .iter()
        .map(|width| "━".repeat(*width))
        .collect::<Vec<_>>()
        .join("  ");
    lines.push(SampleLine::plain(0, rule, style::markdown_rule()));
    for row in body {
        lines.push(SampleLine::spans(
            0,
            join_cells(row, widths, align, style::text()),
        ));
    }
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

/// 列间以 2 空格拼接单元格（无框表格）。
fn join_cells(
    cells: &[String],
    widths: &[usize],
    align: &[Align],
    value_style: Style,
) -> Vec<(String, Style)> {
    let mut spans = Vec::new();
    for (column, width) in widths.iter().enumerate() {
        if column > 0 {
            spans.push(("  ".to_string(), style::text()));
        }
        let empty = String::new();
        let cell = cells.get(column).unwrap_or(&empty);
        let aligned = pad(
            cell,
            *width,
            align.get(column).copied().unwrap_or(Align::Left),
        );
        spans.push((aligned, value_style));
    }
    spans
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

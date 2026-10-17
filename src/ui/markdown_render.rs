//! 面向阅读的 markdown 渲染器：把 markdown 源码渲染成带样式的行（吃掉标记）。
//!
//! 这是**面向阅读的渲染**（隐藏 `#`、`**`、``` 等标记），与编辑器用的
//! `ui/markdown.rs` 高亮（保留标记上色）是两回事，故独立实现。
//! UI 查看器的 agent 回答样例与 lx 聊天历史共用本模块，保证 markdown 观感一致。
//!
//! 支持全部常见 markdown 格式：六级标题、段落、无序/有序/嵌套列表、任务列表、
//! 引用、围栏/缩进代码块、表格（对齐）、水平线、图片，以及行内
//! 粗体/斜体/粗斜体/删除线/行内代码/链接/自动链接/裸 URL。
//!
//! 采用 lx 直角风格：标题隐藏 `#`、列表用 `•`、引用用 `>`（不用左边框）、
//! 代码块做轻量语法高亮；表格用方框网格；任务列表复用 claude 一组复选框
//! （`[x]`/`[ ]`）。

use ratatui::style::Style;

use crate::ui::style;

mod highlight;
mod table;

/// 渲染出的一行：左缩进列 + 多色分段（拼接即该行纯文本）。
#[derive(Debug, Clone, PartialEq)]
pub struct MdLine {
    pub indent: u16,
    /// 悬挂缩进：软换行时**续行**相对首行额外缩进的列数（列表/引用的标记宽度）。
    /// 首行含 `• ` / `1. ` / `> ` 等标记，续行要跳过标记、对齐正文起始列。
    pub hang: u16,
    pub spans: Vec<(String, Style)>,
}

impl MdLine {
    /// 单色单段行。
    pub(crate) fn plain(indent: u16, value: impl Into<String>, style: Style) -> Self {
        Self {
            indent,
            hang: 0,
            spans: vec![(value.into(), style)],
        }
    }

    /// 多段行。
    pub(crate) fn spans(indent: u16, spans: Vec<(String, Style)>) -> Self {
        Self {
            indent,
            hang: 0,
            spans,
        }
    }

    /// 多段行 + 悬挂缩进（列表项/引用：续行对齐正文起始列）。
    pub(crate) fn hanging(indent: u16, hang: u16, spans: Vec<(String, Style)>) -> Self {
        Self {
            indent,
            hang,
            spans,
        }
    }

    /// 该行的显示宽度（缩进 + 各分段宽度）。
    pub fn width(&self) -> usize {
        usize::from(self.indent)
            + self
                .spans
                .iter()
                .map(|(value, _)| display_width(value))
                .sum::<usize>()
    }

    /// 该行的纯文本（不含缩进）。
    pub fn text(&self) -> String {
        self.spans.iter().map(|(value, _)| value.as_str()).collect()
    }
}

/// 把 markdown 源码渲染成带样式的行（任意来源；不附加 token 页脚）。
///
/// 调用方负责换行与宽度裁剪；本函数只做块级结构还原与行内标记折叠。
pub fn render(source: &str) -> Vec<MdLine> {
    render_blocks(source, None)
}

/// 把 markdown 源码渲染成**恰好不超过 `width` 列**的行（块级：表格自适应宽度、其余软换行）。
///
/// 与 [`render`] 共用块级解析，但表格按可用宽度收缩列宽并在单元格内换行
/// （对齐 opencode 的 markdown 表格），避免窄宽度下把方框网格硬折断成「变形」；
/// 其余块（段落/列表/引用/代码）按词优先软换行，超长不可断的内容按字符切分。
pub fn render_wrapped(source: &str, width: u16) -> Vec<MdLine> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();
    for line in render_blocks(source, Some(width)) {
        wrap_md_line(&line, width, &mut out);
    }
    out
}

/// 块级解析：`width` 为 Some 时表格按可用宽度收缩（其余块仍按自然宽度产出，由调用方换行）。
fn render_blocks(source: &str, width: Option<usize>) -> Vec<MdLine> {
    let mut lines = Vec::new();
    let source_lines: Vec<&str> = source.split('\n').collect();
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
        // 缩进列表项优先于缩进代码块：模型常用 4 空格嵌套（`- xxx` 下接 `    - aaa`），
        // 若按 CommonMark 的 4 空格规则当成代码块，整段会被渲染成一坨 `│ - aaa` 贴最左，
        // 列表看起来就“没有缩进”。真代码块请用围栏（```）。
        if let Some(item) = list_item(line) {
            lines.push(item.render());
            index += 1;
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
            let rule_width = width.unwrap_or(40).max(1);
            lines.push(MdLine::plain(
                0,
                RULE.repeat(rule_width),
                style::markdown_rule(),
            ));
            index += 1;
            continue;
        }
        if trimmed.trim_start().starts_with('|') {
            index = table::table(&source_lines, index, &mut lines, width);
            continue;
        }
        if trimmed.trim_start().starts_with('>') {
            index = blockquote(&source_lines, index, &mut lines);
            continue;
        }
        if let Some((alt, url)) = image(trimmed) {
            lines.push(MdLine::spans(
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
        lines.push(MdLine::spans(0, inline_spans(trimmed)));
        index += 1;
    }
    lines
}

/// 水平线字符。
const RULE: &str = "─";
/// 引用 / 代码竖条（直角）。
const BAR_MARKER: &str = "│";
/// 引用块前缀标记。
const QUOTE_MARKER: &str = ">";

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
fn heading_line(level: u8, body: &str) -> MdLine {
    MdLine::plain(0, body, style::markdown_heading_level(level))
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

/// 缩进代码块行：4 个以上前导空格且非空（列表项已在调用前按列表处理，走不到这里）。
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
fn fenced_code(source: &[&str], start: usize, lines: &mut Vec<MdLine>) -> usize {
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
fn indented_code(source: &[&str], start: usize, lines: &mut Vec<MdLine>) -> usize {
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
fn render_code(body: &[&str], lang: Option<&str>, lines: &mut Vec<MdLine>) {
    if let Some(lang) = lang {
        lines.push(MdLine::spans(
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
        lines.push(MdLine::spans(0, spans));
    }
}

/// 引用块：收集连续 `>` 行，用 `>` 作前缀（不用左边框）。
fn blockquote(source: &[&str], start: usize, lines: &mut Vec<MdLine>) -> usize {
    let mut index = start;
    while index < source.len() {
        let trimmed = source[index].trim_start();
        let Some(rest) = trimmed.strip_prefix('>') else {
            break;
        };
        let body = rest.strip_prefix(' ').unwrap_or(rest);
        let marker = format!("{QUOTE_MARKER} ");
        lines.push(MdLine::hanging(
            0,
            marker_width(&marker),
            vec![
                (marker, style::markdown_marker()),
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
    fn render(&self) -> MdLine {
        match *self {
            Self::Bullet { depth, text: body } => {
                let marker = "• ";
                MdLine::hanging(
                    indent(depth),
                    marker_width(marker),
                    marker_spans(marker, style::markdown_bullet(), body),
                )
            }
            Self::Ordered {
                depth,
                number,
                text: body,
            } => {
                let marker = format!("{number}. ");
                MdLine::hanging(
                    indent(depth),
                    marker_width(&marker),
                    marker_spans(&marker, style::markdown_ordered(), body),
                )
            }
            Self::Task {
                depth,
                done,
                text: body,
            } => {
                let (marker, body_style) = if done {
                    ("[x] ", style::markdown_task_done())
                } else {
                    ("[ ] ", style::markdown_task_open())
                };
                let mut spans = vec![(marker.to_string(), body_style)];
                spans.extend(
                    inline_spans(body)
                        .into_iter()
                        .map(|(value, _)| (value, body_style)),
                );
                MdLine::hanging(indent(depth), marker_width(marker), spans)
            }
        }
    }
}

/// 列表项内容：标记分段 + 行内渲染的正文（粗体/斜体/代码等在列表内同样折叠）。
fn marker_spans(marker: &str, marker_style: Style, body: &str) -> Vec<(String, Style)> {
    let mut spans = vec![(marker.to_string(), marker_style)];
    spans.extend(inline_spans(body));
    spans
}

/// 列表项缩进（每级 2 列）。
fn indent(depth: usize) -> u16 {
    (depth * 2) as u16
}

/// 标记的显示宽度（悬挂缩进列数）。
fn marker_width(marker: &str) -> u16 {
    u16::try_from(display_width(marker)).unwrap_or(u16::MAX)
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
            text: marker.trim(),
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
            text: after.trim(),
        });
    }
    // 有序列表 `N. `。
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && rest.as_bytes().get(digits) == Some(&b'.') {
        let after = rest.get(digits + 1..)?.strip_prefix(' ')?;
        return Some(ListItem::Ordered {
            depth,
            number: &rest[..digits],
            text: after.trim(),
        });
    }
    None
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
pub fn display_width(value: &str) -> usize {
    value.chars().map(|ch| char_width(ch) as usize).sum()
}

/// 把一条渲染行按可用宽度软换行（保留缩进；续行缩进到内容起始列）。
///
/// 优先在空白/汉字边界断行（对齐 opencode/主流 agent 的 word wrap），
/// 单个超宽字符单独成行；供表格外的段落/列表/引用/代码使用。
fn wrap_md_line(line: &MdLine, width: usize, out: &mut Vec<MdLine>) {
    let first_indent = usize::from(line.indent);
    let hang = usize::from(line.hang);
    let cont_indent = first_indent.saturating_add(hang);
    let cells: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|(value, span_style)| value.chars().map(|ch| (ch, *span_style)))
        .collect();
    if cells.is_empty() {
        out.push(MdLine::spans(line.indent, Vec::new()));
        return;
    }
    let mut index = 0usize;
    let mut first = true;
    while index < cells.len() {
        // 首行含标记（`• ` / `1. ` / `> `），续行用悬挂缩进对齐正文起始列。
        let indent = if first { first_indent } else { cont_indent };
        let avail = width.saturating_sub(indent).max(1);
        // 适应 `avail` 列的最大结束下标。
        let mut col = 0usize;
        let mut end = index;
        while end < cells.len() {
            let cell = char_width(cells[end].0) as usize;
            if col + cell > avail {
                break;
            }
            col += cell;
            end += 1;
        }
        if end == index {
            end = index + 1; // 单字符超宽：单独成行。
        }
        // 未到行尾时，回退到最近一个空白之后断行（词优先）。
        let mut break_at = end;
        if end < cells.len() {
            for probe in (index..end).rev() {
                if cells[probe].0.is_whitespace() {
                    break_at = probe + 1;
                    break;
                }
            }
        }
        // 首行不能断在标记内部（否则出现 `•` 独占一行的孤儿标记）：
        // 断点没越过标记宽度时改为按列硬切。
        if first && break_at <= index.saturating_add(hang) {
            break_at = end;
        }
        let mut spans: Vec<(String, Style)> = Vec::new();
        for (ch, span_style) in &cells[index..break_at] {
            if let Some(last) = spans.last_mut()
                && last.1 == *span_style
            {
                last.0.push(*ch);
                continue;
            }
            spans.push((ch.to_string(), *span_style));
        }
        trim_trailing_whitespace(&mut spans);
        out.push(MdLine::spans(
            u16::try_from(indent).unwrap_or(u16::MAX),
            spans,
        ));
        index = break_at;
        first = false;
        // 跳过下一物理行的行首空白。
        while index < cells.len() && cells[index].0 == ' ' {
            index += 1;
        }
    }
}

/// 去掉行尾空白分段（软换行产生的悬挂空格）。
fn trim_trailing_whitespace(spans: &mut Vec<(String, Style)>) {
    while let Some((value, _)) = spans.last() {
        let trimmed = value.trim_end();
        if trimmed.len() == value.len() {
            break;
        }
        if trimmed.is_empty() {
            spans.pop();
        } else {
            let trimmed = trimmed.to_string();
            spans.last_mut().expect("last").0 = trimmed;
            break;
        }
    }
}

/// 单个字符的显示宽度。
fn char_width(symbol: char) -> u16 {
    use unicode_width::UnicodeWidthChar;
    symbol.width().unwrap_or(1).min(2) as u16
}

#[cfg(test)]
mod tests;

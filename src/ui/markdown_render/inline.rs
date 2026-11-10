//! 行内 Markdown 渲染：折叠粗体/斜体/粗斜体/删除线/行内代码/链接标记并生成着色分段。

use ratatui::style::Style;

use crate::ui::style;

use super::{char_width, display_width};

/// 单行内的超链接列区间（列偏移相对 `MdLine::indent` 之后的正文起点，左闭右开 `start_col..end_col`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdLinkSpan {
    pub start_col: u16,
    pub end_col: u16,
    pub url: String,
}

/// 行内渲染分段：携带显示文本、样式与可选的 `http(s)://` 超链接目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InlineSpan {
    pub(super) text: String,
    pub(super) style: Style,
    pub(super) link: Option<String>,
}

/// 判定是否为安全的 `http://` 或 `https://` 网页链接。
pub(super) fn is_web_url(url: &str) -> bool {
    let trimmed = url.trim();
    let rest = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"));
    rest.is_some_and(|tail| {
        !tail.is_empty() && !tail.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    })
}

/// 行内渲染：吃掉粗体/斜体/粗斜体/删除线/行内代码标记；网页链接仅显示带下划线的 `文字`。
#[cfg(test)]
pub(super) fn inline_spans(text: &str) -> Vec<(String, Style)> {
    inline_spans_with_links(text)
        .into_iter()
        .map(|span| (span.text, span.style))
        .collect()
}

/// 行内渲染并返回（着色分段, 超链接列区间）。
pub(super) fn spans_and_links(text: &str) -> (Vec<(String, Style)>, Vec<MdLinkSpan>) {
    build_spans_and_links(None, inline_spans_with_links(text), None)
}

/// 带行首标记（列表符号 `• ` / `1. ` / `[x] ` 或引用 `> `）的行内渲染与超链接列区间。
pub(super) fn marker_spans_and_links(
    marker: &str,
    marker_style: Style,
    body: &str,
    patch_base: Option<Style>,
) -> (Vec<(String, Style)>, Vec<MdLinkSpan>) {
    build_spans_and_links(
        Some((marker.to_string(), marker_style)),
        inline_spans_with_links(body),
        patch_base,
    )
}

/// 标题行内渲染与超链接列区间：非超链接文本统一采用 `heading_style`，超链接文本叠加链接蓝与下划线。
pub(super) fn heading_spans_and_links(
    body: &str,
    heading_style: Style,
) -> (Vec<(String, Style)>, Vec<MdLinkSpan>) {
    let inline: Vec<InlineSpan> = inline_spans_with_links(body)
        .into_iter()
        .map(|mut span| {
            span.style = if span.link.is_some() {
                heading_style.patch(style::md_read_link_text())
            } else {
                heading_style
            };
            span
        })
        .collect();
    if inline.is_empty() {
        return (vec![(body.to_string(), heading_style)], Vec::new());
    }
    build_spans_and_links(None, inline, None)
}

fn build_spans_and_links(
    lead: Option<(String, Style)>,
    inline: Vec<InlineSpan>,
    patch_base: Option<Style>,
) -> (Vec<(String, Style)>, Vec<MdLinkSpan>) {
    let mut spans: Vec<(String, Style)> =
        Vec::with_capacity(inline.len() + usize::from(lead.is_some()));
    let mut links: Vec<MdLinkSpan> = Vec::new();
    let mut cursor_col = 0u16;
    let mut prev_link: Option<String> = None;
    if let Some((marker, style)) = lead {
        cursor_col = u16::try_from(display_width(&marker)).unwrap_or(u16::MAX);
        spans.push((marker, style));
    }
    for span in inline {
        let cols = u16::try_from(display_width(&span.text)).unwrap_or(u16::MAX);
        let next_col = cursor_col.saturating_add(cols);
        let span_link = span.link;
        if let Some(ref url) = span_link
            && cursor_col < next_col
        {
            if let Some(last) = links.last_mut()
                && last.end_col == cursor_col
                && last.url == *url
            {
                last.end_col = next_col;
            } else {
                links.push(MdLinkSpan {
                    start_col: cursor_col,
                    end_col: next_col,
                    url: url.clone(),
                });
            }
        }
        cursor_col = next_col;
        let final_style = match patch_base {
            Some(base) => base.patch(span.style),
            None => span.style,
        };
        if let Some(last) = spans.last_mut()
            && last.1 == final_style
            && prev_link == span_link
        {
            last.0.push_str(&span.text);
        } else {
            spans.push((span.text, final_style));
        }
        prev_link = span_link;
    }
    (spans, links)
}

/// 将单行分段与超链接区间展开为逐字符序列（供软换行切分）。
pub(super) fn cell_links_for_line<'a>(
    spans: &[(String, Style)],
    links: &'a [MdLinkSpan],
) -> Vec<(char, Style, Option<&'a str>)> {
    let mut out = Vec::new();
    let mut col = 0u16;
    for (value, span_style) in spans {
        for ch in value.chars() {
            let w = char_width(ch);
            let link = links
                .iter()
                .find(|l| col >= l.start_col && col < l.end_col)
                .map(|l| l.url.as_str());
            col = col.saturating_add(w);
            out.push((ch, *span_style, link));
        }
    }
    out
}

/// 将软换行切出的字符切片还原为（去尾空白后的着色分段, 超链接列区间）。
pub(super) fn slice_spans_and_links(
    cells: &[(char, Style, Option<&str>)],
) -> (Vec<(String, Style)>, Vec<MdLinkSpan>) {
    let end = cells
        .iter()
        .rposition(|(ch, _, _)| !ch.is_whitespace())
        .map_or(0, |idx| idx + 1);
    let trimmed = &cells[..end];
    let mut spans: Vec<(String, Style)> = Vec::new();
    let mut links: Vec<MdLinkSpan> = Vec::new();
    let mut col = 0u16;
    let mut prev_link: Option<&str> = None;
    for (ch, span_style, link) in trimmed {
        let w = char_width(*ch);
        let next_col = col.saturating_add(w);
        if let Some(url) = link
            && col < next_col
        {
            if let Some(last) = links.last_mut()
                && last.end_col == col
                && last.url == *url
            {
                last.end_col = next_col;
            } else {
                links.push(MdLinkSpan {
                    start_col: col,
                    end_col: next_col,
                    url: (*url).to_string(),
                });
            }
        }
        col = next_col;
        if let Some(last) = spans.last_mut()
            && last.1 == *span_style
            && prev_link == *link
        {
            last.0.push(*ch);
        } else {
            spans.push((ch.to_string(), *span_style));
        }
        prev_link = *link;
    }
    (spans, links)
}

/// 行内渲染（保留超链接目标 URL）：
/// - `[文字](http(s)://...)` 且 `文字` 非空时仅显示 `文字`（链接蓝 + 下划线），隐藏 `(url)` 并携带 `link`；
/// - `[](http(s)://...)`、`<http(s)://...>` 与裸 `http(s)://...` 显示网址本身（链接蓝 + 下划线）并携带 `link`；
/// - 非 `http(s)://` 链接（如本地相对路径）保留显示 `文字 (path)`，`link` 为 `None`。
pub(super) fn inline_spans_with_links(text: &str) -> Vec<InlineSpan> {
    let mut spans = Vec::new();
    let mut plain = String::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let prev = text[..index].chars().next_back();
        let rest = &text[index..];
        // 行内代码：`code` 或 `` `code` ``（内部不再解析）。
        if let Some((inner, consumed)) = code_span(rest) {
            flush(&mut plain, &mut spans);
            spans.push(InlineSpan {
                text: inner.to_string(),
                style: style::md_read_inline_code(),
                link: None,
            });
            index += consumed;
            continue;
        }
        // 粗斜体 `***…***`。
        if let Some((inner, consumed)) = delimited_emphasis(rest, "***", prev) {
            flush(&mut plain, &mut spans);
            push_nested_spans(inner, style::md_read_strong_emphasis(), &mut spans);
            index += consumed;
            continue;
        }
        // 粗体 `**…**` / `__…__`。
        if let Some((inner, consumed)) =
            delimited_emphasis(rest, "**", prev).or_else(|| delimited_emphasis(rest, "__", prev))
        {
            flush(&mut plain, &mut spans);
            push_nested_spans(inner, style::md_read_strong(), &mut spans);
            index += consumed;
            continue;
        }
        // 删除线 `~~…~~`。
        if let Some((inner, consumed)) = delimited_emphasis(rest, "~~", prev) {
            flush(&mut plain, &mut spans);
            push_nested_spans(inner, style::md_read_strikethrough(), &mut spans);
            index += consumed;
            continue;
        }
        // 斜体 `*…*` / `_…_`（排除双写与词内下划线）。
        if let Some((inner, consumed)) = emphasis(rest, prev) {
            flush(&mut plain, &mut spans);
            push_nested_spans(inner, style::md_read_emphasis(), &mut spans);
            index += consumed;
            continue;
        }
        // 链接 `[文字](url)`。
        if rest.starts_with('[')
            && let Some((label, url, consumed)) = link(rest)
        {
            flush(&mut plain, &mut spans);
            let clean_label: String = inline_spans_with_links(label)
                .into_iter()
                .map(|s| s.text)
                .collect();
            let display_label = if !clean_label.trim().is_empty() {
                clean_label
            } else if !label.trim().is_empty() {
                label.to_string()
            } else {
                String::new()
            };
            if is_web_url(url) {
                let target = url.trim().to_string();
                let visible = if display_label.is_empty() {
                    target.clone()
                } else {
                    display_label
                };
                spans.push(InlineSpan {
                    text: visible,
                    style: style::md_read_link_text(),
                    link: Some(target),
                });
            } else if display_label.is_empty() {
                spans.push(InlineSpan {
                    text: format!("({url})"),
                    style: style::md_read_url(),
                    link: None,
                });
            } else {
                spans.push(InlineSpan {
                    text: display_label,
                    style: style::md_read_link_text(),
                    link: None,
                });
                spans.push(InlineSpan {
                    text: format!(" ({url})"),
                    style: style::md_read_url(),
                    link: None,
                });
            }
            index += consumed;
            continue;
        }
        // 自动链接 `<url>`。
        if rest.starts_with('<')
            && let Some(close) = rest.find('>')
        {
            let url = &rest[1..close];
            if is_web_url(url) {
                flush(&mut plain, &mut spans);
                let target = url.trim().to_string();
                spans.push(InlineSpan {
                    text: target.clone(),
                    style: style::md_read_link_text(),
                    link: Some(target),
                });
                index += close + 1;
                continue;
            }
        }
        // 裸 URL（扫描右边界时遇到非 ASCII、空白、控制字符或 HTML/Markdown 分隔符即截断）。
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let raw_end = rest
                .find(|ch: char| {
                    !ch.is_ascii()
                        || ch.is_whitespace()
                        || ch.is_control()
                        || matches!(ch, '<' | '>' | '"' | '`' | '|' | '\\' | '^')
                })
                .unwrap_or(rest.len());
            let end = trim_bare_url_end(&rest[..raw_end]);
            let candidate = &rest[..end];
            if is_web_url(candidate) {
                flush(&mut plain, &mut spans);
                spans.push(InlineSpan {
                    text: candidate.to_string(),
                    style: style::md_read_link_text(),
                    link: Some(candidate.to_string()),
                });
                index += end;
                continue;
            }
        }
        let ch = rest.chars().next().unwrap_or('\0');
        plain.push(ch);
        index += ch.len_utf8();
    }
    flush(&mut plain, &mut spans);
    spans
}

/// 剥离裸 URL 尾随的中英文句读标点（保留平衡的右括号，如 Wikipedia 词条 URL）。
fn trim_bare_url_end(raw: &str) -> usize {
    let mut slice = raw;
    while let Some(ch) = slice.chars().next_back() {
        let strip = matches!(
            ch,
            '。' | '，'
                | '、'
                | '；'
                | '：'
                | '！'
                | '？'
                | '）'
                | '】'
                | '》'
                | '”'
                | '’'
                | '.'
                | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | ']'
                | '>'
                | '"'
                | '\''
        ) || (ch == ')' && {
            let opens = slice.chars().filter(|&c| c == '(').count();
            let closes = slice.chars().filter(|&c| c == ')').count();
            closes > opens
        });
        if strip {
            slice = &slice[..slice.len() - ch.len_utf8()];
        } else {
            break;
        }
    }
    slice.len()
}

/// 递归解析强调块内部的行内分段并合成外层样式。
fn push_nested_spans(inner: &str, outer_style: Style, out: &mut Vec<InlineSpan>) {
    for child in inline_spans_with_links(inner) {
        let merged = if child.style == style::text() {
            outer_style
        } else if child.style == style::md_read_inline_code() {
            child.style
        } else {
            child.style.patch(outer_style)
        };
        if let Some(last) = out.last_mut()
            && last.style == merged
            && last.link == child.link
        {
            last.text.push_str(&child.text);
        } else {
            out.push(InlineSpan {
                text: child.text,
                style: merged,
                link: child.link,
            });
        }
    }
}

/// 把累积的普通文本刷入 spans（正文色）。
fn flush(plain: &mut String, spans: &mut Vec<InlineSpan>) {
    if !plain.is_empty() {
        spans.push(InlineSpan {
            text: std::mem::take(plain),
            style: style::text(),
            link: None,
        });
    }
}

/// 匹配任意长度反引号包裹的行内代码（如 `` `code` `` 或 `` ``code ` inner`` ``）。
fn code_span(rest: &str) -> Option<(&str, usize)> {
    let ticks = rest.bytes().take_while(|&b| b == b'`').count();
    if ticks == 0 {
        return None;
    }
    let after = &rest[ticks..];
    let bytes = after.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'`' {
            let run = after[i..].bytes().take_while(|&b| b == b'`').count();
            if run == ticks {
                let raw = &after[..i];
                if raw.is_empty() {
                    return None;
                }
                let stripped = if ticks > 1
                    && raw.len() >= 2
                    && raw.starts_with(' ')
                    && raw.ends_with(' ')
                    && raw.bytes().any(|b| b != b' ')
                {
                    &raw[1..raw.len() - 1]
                } else {
                    raw
                };
                return Some((stripped, ticks + i + ticks));
            }
            i += run;
        } else {
            i += 1;
        }
    }
    None
}

/// 匹配强调类对称标记（`***` / `**` / `__` / `~~` / `*` / `_`）：
/// - 遵循 CommonMark 规则：开标记后与闭标记前均不得紧邻空白；
/// - 下划线标记（`_` / `__`）不得在 ASCII 字母或数字词内开闭；
/// - 扫描闭合标记时跳过反引号代码片段，并在尾随同字符链（如 `***`）上对齐外层闭合位置。
fn delimited_emphasis<'a>(
    rest: &'a str,
    delim: &str,
    prev: Option<char>,
) -> Option<(&'a str, usize)> {
    let delim_char = delim.chars().next()?;
    let delim_len = delim.len();
    let is_underscore = delim_char == '_';
    if is_underscore && prev.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return None;
    }
    let after = rest.strip_prefix(delim)?;
    let first = after.chars().next()?;
    if first == delim_char || first.is_whitespace() {
        return None;
    }
    let bytes = after.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // 跳过行内代码反引号片段，避免代码内的 `**`/`*`/`_` 误闭合外层强调。
        if bytes[i] == b'`' {
            if let Some((_, consumed)) = code_span(&after[i..]) {
                i += consumed;
                continue;
            }
            let run = after[i..].bytes().take_while(|&b| b == b'`').count();
            i += run.max(1);
            continue;
        }
        if after[i..].starts_with(delim_char) {
            let run = after[i..]
                .chars()
                .take_while(|&ch| ch == delim_char)
                .map(char::len_utf8)
                .sum::<usize>();
            let run_chars = run / delim_char.len_utf8();
            // 单字符 `*`/`_` 不能在双写 `**`/`__` 处闭合（`run_chars == 2` 属于粗体标记）。
            let can_close = if delim_len == 1 {
                run_chars == 1 || run_chars == 3
            } else {
                run >= delim_len
            };
            if can_close {
                let end = i + run - delim_len;
                let inner = &after[..end];
                let tail = &after[end + delim_len..];
                let before_close = inner.chars().next_back();
                let after_close = tail.chars().next();
                let valid_before = before_close.is_some_and(|ch| !ch.is_whitespace())
                    && inner.chars().any(|ch| ch != delim_char);
                let valid_after = after_close.is_none_or(|ch| {
                    ch != delim_char && (!is_underscore || !ch.is_ascii_alphanumeric())
                });
                if valid_before && valid_after {
                    return Some((inner, delim_len + end + delim_len));
                }
            }
            i += run.max(1);
            continue;
        }
        let ch = after[i..].chars().next().unwrap_or('\0');
        i += ch.len_utf8().max(1);
    }
    None
}

/// 斜体 `*…*` / `_…_`；排除双写前缀与词内下划线。
fn emphasis(rest: &str, prev: Option<char>) -> Option<(&str, usize)> {
    let ch = rest.chars().next()?;
    if ch != '*' && ch != '_' {
        return None;
    }
    delimited_emphasis(rest, if ch == '*' { "*" } else { "_" }, prev)
}

/// 链接 `[文字](url)`；支持文字内含反引号代码与配对 `[]`、URL 内含配对 `()`，返回（文字、URL、消耗字节数）。
fn link(rest: &str) -> Option<(&str, &str, usize)> {
    let after = rest.strip_prefix('[')?;
    let bytes = after.as_bytes();
    let mut bracket_depth = 1usize;
    let mut label_close = None;
    let mut idx = 0usize;
    while idx < bytes.len() {
        if bytes[idx] == b'\n' {
            return None;
        }
        if bytes[idx] == b'`' {
            if let Some((_, consumed)) = code_span(&after[idx..]) {
                idx += consumed;
            } else {
                let ticks = after[idx..].bytes().take_while(|&b| b == b'`').count();
                idx += ticks.max(1);
            }
            continue;
        }
        let ch = after[idx..].chars().next()?;
        match ch {
            '[' => bracket_depth += 1,
            ']' => {
                bracket_depth -= 1;
                if bracket_depth == 0 {
                    label_close = Some(idx);
                    break;
                }
            }
            _ => {}
        }
        idx += ch.len_utf8();
    }
    let label_close = label_close?;
    let url_start = after.get(label_close + 1..)?.strip_prefix('(')?;
    let mut paren_depth = 1usize;
    let mut url_close = None;
    for (idx, ch) in url_start.char_indices() {
        if ch == '\n' {
            return None;
        }
        match ch {
            '(' => paren_depth += 1,
            ')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    url_close = Some(idx);
                    break;
                }
            }
            _ => {}
        }
    }
    let url_close = url_close?;
    let url = &url_start[..url_close];
    if url
        .trim()
        .chars()
        .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return None;
    }
    let consumed = 1 + label_close + 1 + 1 + url_close + 1;
    Some((&after[..label_close], url, consumed))
}

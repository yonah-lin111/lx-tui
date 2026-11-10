//! 行内 Markdown 渲染：折叠粗体/斜体/粗斜体/删除线/行内代码/链接标记并生成着色分段。

use ratatui::style::Style;

use crate::ui::style;

/// 行内渲染：吃掉粗体/斜体/粗斜体/删除线/行内代码标记；链接显示 `文字 (url)`。
///
/// 强调类标记（`***`、`**`/`__`、`~~`、`*`/`_`）内部递归解析行内代码、链接与内层强调。
pub(super) fn inline_spans(text: &str) -> Vec<(String, Style)> {
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
            spans.push((inner.to_string(), style::md_read_inline_code()));
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
            let clean_label: String = inline_spans(label).into_iter().map(|(s, _)| s).collect();
            let display_label = if clean_label.is_empty() {
                label.to_string()
            } else {
                clean_label
            };
            spans.push((display_label, style::md_read_link_text()));
            spans.push((format!(" ({url})"), style::md_read_url()));
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
                spans.push((url.to_string(), style::md_read_url()));
                index += close + 1;
                continue;
            }
        }
        // 裸 URL。
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            flush(&mut plain, &mut spans);
            spans.push((rest[..end].to_string(), style::md_read_url()));
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

/// 递归解析强调块内部的行内分段并合成外层样式。
fn push_nested_spans(inner: &str, outer_style: Style, out: &mut Vec<(String, Style)>) {
    for (value, child_style) in inline_spans(inner) {
        let merged = if child_style == style::text() {
            outer_style
        } else if child_style == style::md_read_inline_code() {
            child_style
        } else {
            child_style.patch(outer_style)
        };
        if let Some(last) = out.last_mut()
            && last.1 == merged
        {
            last.0.push_str(&value);
        } else {
            out.push((value, merged));
        }
    }
}

/// 把累积的普通文本刷入 spans（正文色）。
fn flush(plain: &mut String, spans: &mut Vec<(String, Style)>) {
    if !plain.is_empty() {
        spans.push((std::mem::take(plain), style::text()));
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

/// 链接 `[文字](url)`；返回（文字、URL、消耗字节数）。
fn link(rest: &str) -> Option<(&str, &str, usize)> {
    let after = rest.strip_prefix('[')?;
    let label_close = after.find(']')?;
    let url_start = after.get(label_close + 1..)?.strip_prefix('(')?;
    let url_close = url_start.find(')')?;
    let consumed = 1 + label_close + 1 + 1 + url_close + 1;
    Some((&after[..label_close], &url_start[..url_close], consumed))
}

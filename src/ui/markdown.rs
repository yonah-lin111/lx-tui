//! markdown 高亮词法：按逻辑行扫描语法区间，输出与渲染无关的 token。

use std::ops::Range;

use crate::app::markdown::{self, SlashCommandId};

/// 高亮语义分类；样式映射见 `ui/style.rs`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Marker,
    Heading,
    Strong,
    Emphasis,
    Strikethrough,
    InlineCode,
    CodeBlock,
    Quote,
    LinkText,
    Url,
    /// `&&&` 起止行的结构标记（`&&&`、`--start`/`--end`、状态与元数据）。
    TemplateMarker,
    /// 模板块命令名；`None` 为未知命令。
    TemplateCommand(Option<SlashCommandId>),
    /// 标题占位符 `「title: …」`。
    TemplateTitle,
    /// `@path/to/file` 文件提及。
    FileMention,
}

/// 逻辑行上的高亮区间（字节偏移，行内）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

/// 扫描全文，返回每条逻辑行的 token，与 `text.split('\n')` 一一对应。
pub fn scan(text: &str) -> Vec<Vec<Token>> {
    let mut fence = false;
    text.split('\n')
        .map(|line| scan_line(line, &mut fence))
        .collect()
}

/// 扫描单行；`fence` 为跨行围栏代码块状态。
fn scan_line(line: &str, fence: &mut bool) -> Vec<Token> {
    let mut tokens = Vec::new();
    if is_fence(line) {
        *fence = !*fence;
        push(&mut tokens, 0..line.len(), TokenKind::Marker);
        return tokens;
    }
    if *fence {
        push(&mut tokens, 0..line.len(), TokenKind::CodeBlock);
        return tokens;
    }
    if scan_template_end(line, &mut tokens) {
        return tokens;
    }
    if scan_template_start(line, &mut tokens) {
        return tokens;
    }
    if let Some(marker_end) = heading(line) {
        push(&mut tokens, 0..marker_end, TokenKind::Marker);
        push(&mut tokens, marker_end..line.len(), TokenKind::Heading);
        return tokens;
    }
    if is_rule(line) {
        push(&mut tokens, 0..line.len(), TokenKind::Marker);
        return tokens;
    }
    if let Some(prefix_end) = quote_prefix(line) {
        push(&mut tokens, 0..prefix_end, TokenKind::Marker);
        push(&mut tokens, prefix_end..line.len(), TokenKind::Quote);
        return tokens;
    }
    if let Some(prefix_end) = list_prefix(line) {
        push(&mut tokens, 0..prefix_end, TokenKind::Marker);
        scan_inline(line, prefix_end, &mut tokens);
        return tokens;
    }
    scan_inline(line, 0, &mut tokens);
    tokens
}

/// 模板块起始行 token：`&&&` 与 `--start` 为结构标记，命令名按业务分色，标题整体成段。
fn scan_template_start(line: &str, tokens: &mut Vec<Token>) -> bool {
    let Some(parsed) = markdown::parse_template_start_line(line) else {
        return false;
    };
    let spans = word_spans(line);
    if let Some((_, range)) = spans.first() {
        push(tokens, range.clone(), TokenKind::TemplateMarker);
    }
    if let Some((_, range)) = spans.iter().find(|(text, _)| *text == parsed.command) {
        push(
            tokens,
            range.clone(),
            TokenKind::TemplateCommand(SlashCommandId::from_name(parsed.command)),
        );
    }
    if let Some((_, range)) = spans.iter().find(|(text, _)| *text == "--start") {
        push(tokens, range.clone(), TokenKind::TemplateMarker);
    }
    if let Some(open) = line.find('「')
        && let Some(close) = line[open..].find('」')
    {
        push(
            tokens,
            open..open + close + '」'.len_utf8(),
            TokenKind::TemplateTitle,
        );
    }
    true
}

/// 模板块结束行 token：`&&&`、`--end`、状态词与 `{id:}`/`{wt:}` 元数据为结构标记，
/// 命令名按业务分色。
fn scan_template_end(line: &str, tokens: &mut Vec<Token>) -> bool {
    let Some(parsed) = markdown::parse_template_end_line(line) else {
        return false;
    };
    let spans = word_spans(line);
    if let Some((_, range)) = spans.first() {
        push(tokens, range.clone(), TokenKind::TemplateMarker);
    }
    if let Some(command) = parsed.command
        && let Some((_, range)) = spans.iter().find(|(text, _)| *text == command)
    {
        push(
            tokens,
            range.clone(),
            TokenKind::TemplateCommand(SlashCommandId::from_name(command)),
        );
    }
    for (text, range) in &spans {
        if matches!(*text, "--end" | "done" | "in_progress")
            || text.starts_with("{id:")
            || text.starts_with("{wt:")
        {
            push(tokens, range.clone(), TokenKind::TemplateMarker);
        }
    }
    true
}

/// 按 ASCII 空白切分单词及行内字节区间；非 ASCII 视为词内字符。
fn word_spans(line: &str) -> Vec<(&str, Range<usize>)> {
    let bytes = line.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        spans.push((&line[start..index], start..index));
    }
    spans
}

/// 围栏代码块行：至多 3 个前导空格后以 ``` 开头。
fn is_fence(line: &str) -> bool {
    let trimmed = line.trim_start_matches(' ');
    line.len() - trimmed.len() <= 3 && trimmed.starts_with("```")
}

/// ATX 标题的标记段结束偏移（`#` 与后随空白）；非法或非标题返回 None。
fn heading(line: &str) -> Option<usize> {
    let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = &line[hashes..];
    if rest.is_empty() {
        return Some(hashes);
    }
    let spaces = rest
        .bytes()
        .take_while(|byte| *byte == b' ' || *byte == b'\t')
        .count();
    (spaces > 0).then_some(hashes + spaces)
}

/// 水平线：仅由同一种 `-`/`*`/`_`（可夹空白）组成且至少 3 个。
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

/// 引用前缀结束偏移：行首重复的 `>` 链（可含空白与单个后随空格）。
fn quote_prefix(line: &str) -> Option<usize> {
    let mut index = 0;
    let mut found = false;
    while index < line.len() {
        let start = index;
        while line[index..].starts_with(' ') {
            index += 1;
        }
        if line[index..].starts_with('>') {
            index += 1;
            if line[index..].starts_with(' ') {
                index += 1;
            }
            found = true;
        } else {
            index = start;
            break;
        }
    }
    found.then_some(index)
}

/// 列表标记段结束偏移：行首（可缩进）项目符号或序号加后随空白。
fn list_prefix(line: &str) -> Option<usize> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let bullet = ['-', '*', '+']
        .iter()
        .any(|marker| rest.starts_with(*marker));
    let marker_len = if bullet {
        1
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        let delimiter = rest.as_bytes().get(digits).copied();
        if digits == 0 || !matches!(delimiter, Some(b'.') | Some(b')')) {
            return None;
        }
        digits + 1
    };
    let after = &rest[marker_len..];
    let spaces = after
        .bytes()
        .take_while(|byte| *byte == b' ' || *byte == b'\t')
        .count();
    (spaces > 0).then_some(indent + marker_len + spaces)
}

/// 行内语法扫描：代码段优先，内部不再解析其他标记。
fn scan_inline(line: &str, base: usize, tokens: &mut Vec<Token>) {
    let mut index = base;
    while index < line.len() {
        let rest = &line[index..];
        if rest.starts_with('`')
            && let Some(close) = rest[1..].find('`')
        {
            push(tokens, index..index + 1, TokenKind::Marker);
            push(tokens, index + 1..index + 1 + close, TokenKind::InlineCode);
            push(
                tokens,
                index + 1 + close..index + 2 + close,
                TokenKind::Marker,
            );
            index += close + 2;
            continue;
        }
        if rest.starts_with('@')
            && let Some(end) = file_mention_end(line, index)
        {
            push(tokens, index..index + end, TokenKind::FileMention);
            index += end;
            continue;
        }
        if let Some(delimiter) = strong_delimiter(rest)
            && let Some(close) = rest[2..].find(delimiter)
            && close > 0
        {
            push(tokens, index..index + 2, TokenKind::Marker);
            push(tokens, index + 2..index + 2 + close, TokenKind::Strong);
            push(
                tokens,
                index + 2 + close..index + 4 + close,
                TokenKind::Marker,
            );
            index += close + 4;
            continue;
        }
        if rest.starts_with("~~")
            && let Some(close) = rest[2..].find("~~")
            && close > 0
        {
            push(tokens, index..index + 2, TokenKind::Marker);
            push(
                tokens,
                index + 2..index + 2 + close,
                TokenKind::Strikethrough,
            );
            push(
                tokens,
                index + 2 + close..index + 4 + close,
                TokenKind::Marker,
            );
            index += close + 4;
            continue;
        }
        if let Some(delimiter) = emphasis_delimiter(rest)
            && let Some(close) = rest[1..].find(delimiter)
            && close > 0
        {
            push(tokens, index..index + 1, TokenKind::Marker);
            push(tokens, index + 1..index + 1 + close, TokenKind::Emphasis);
            push(
                tokens,
                index + 1 + close..index + 2 + close,
                TokenKind::Marker,
            );
            index += close + 2;
            continue;
        }
        if rest.starts_with('[')
            && let Some(consumed) = link(rest, index, tokens)
        {
            index += consumed;
            continue;
        }
        if rest.starts_with("http://") || rest.starts_with("https://") {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            push(tokens, index..index + end, TokenKind::Url);
            index += end;
            continue;
        }
        index += next_char_len(rest);
    }
}

/// 粗体分隔符；`**` 优先于 `__`。
fn strong_delimiter(rest: &str) -> Option<&str> {
    if rest.starts_with("**") {
        Some("**")
    } else if rest.starts_with("__") {
        Some("__")
    } else {
        None
    }
}

/// 斜体分隔符；排除双写（粗体）前缀。
fn emphasis_delimiter(rest: &str) -> Option<char> {
    let mut chars = rest.chars();
    let ch = chars.next()?;
    if ch != '*' && ch != '_' {
        return None;
    }
    (chars.next() != Some(ch)).then_some(ch)
}

/// 链接 `[文字](URL)`；返回消耗的字节数，不匹配返回 None。
fn link(rest: &str, index: usize, tokens: &mut Vec<Token>) -> Option<usize> {
    let text_close = rest.find(']')?;
    let after = rest.get(text_close + 1..)?;
    if !after.starts_with('(') {
        return None;
    }
    let url_close = after.find(')')?;
    push(tokens, index..index + 1, TokenKind::Marker);
    push(tokens, index + 1..index + text_close, TokenKind::LinkText);
    push(
        tokens,
        index + text_close..index + text_close + 2,
        TokenKind::Marker,
    );
    let url_start = index + text_close + 2;
    push(tokens, url_start..url_start + url_close - 1, TokenKind::Url);
    push(
        tokens,
        index + text_close + 1 + url_close..index + text_close + 2 + url_close,
        TokenKind::Marker,
    );
    Some(text_close + url_close + 2)
}

/// 追加非空区间。
fn push(tokens: &mut Vec<Token>, range: Range<usize>, kind: TokenKind) {
    if range.start < range.end {
        tokens.push(Token { range, kind });
    }
}

/// 下一个字符的字节长度。
fn next_char_len(rest: &str) -> usize {
    rest.chars().next().map(char::len_utf8).unwrap_or(1)
}

/// `@` 起始的文件提及长度（含 `@`）：`@` 位于行首或空白/`[` 之后，
/// 路径由查询字符组成且非空；否则返回 None。
fn file_mention_end(line: &str, index: usize) -> Option<usize> {
    let boundary = index == 0
        || line[..index]
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_whitespace() || ch == '[');
    if !boundary {
        return None;
    }
    let mut end = 1;
    for ch in line[index + 1..].chars() {
        if ch == '@' || !markdown::is_mention_query_char(ch) {
            break;
        }
        end += ch.len_utf8();
    }
    (end > 1).then_some(end)
}

#[cfg(test)]
mod tests;

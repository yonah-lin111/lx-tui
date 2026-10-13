//! `+++` 补充块协议：起止行解析（`&&&` 任务块内部的子块）。

/// 补充块起始行解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuppleStartLine<'a> {
    pub indent: &'a str,
    pub command: &'a str,
    /// 行内兼容标题（`{title: …}` 或 `「title: …」`）；缺省为 None。
    pub title: Option<&'a str>,
}

/// 补充块结束行解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuppleEndLine<'a> {
    pub indent: &'a str,
    pub command: &'a str,
    pub id: Option<&'a str>,
    pub wt: Option<&'a str>,
}

/// `+++` 保留命令名：旧版记录块语法，不作为补充块。
const SUPPLE_RESERVED: [&str; 2] = ["log", "logTemplate"];

/// 解析补充块起始行：`+++ <command> --start [{title: …}|「title: …」]`。
///
/// 命令名非保留词；`--start` 必须存在（无 `--start` 的行不是有效补充块起始，
/// 避免把结束行或旧记录块误判为起始）。
pub fn parse_supple_start_line(line: &str) -> Option<SuppleStartLine<'_>> {
    let trimmed = line.trim_start_matches(' ');
    let indent = &line[..line.len() - trimmed.len()];
    let rest = trimmed.strip_prefix("+++")?;
    if !rest.starts_with(' ') {
        return None;
    }
    let mut rest = rest.trim_start_matches(' ');
    let length = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    let command = rest.get(..length)?;
    if !rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        || SUPPLE_RESERVED.contains(&command)
    {
        return None;
    }
    rest = &rest[length..];
    let after = rest.strip_prefix(" --start")?;
    let title = parse_trailing_title(after)?;
    Some(SuppleStartLine {
        indent,
        command,
        title,
    })
}

/// 解析补充块结束行：`+++ <command> --end [{id: …}] [{wt: …}]`。
pub fn parse_supple_end_line(line: &str) -> Option<SuppleEndLine<'_>> {
    let trimmed = line.trim_start_matches(' ');
    let indent = &line[..line.len() - trimmed.len()];
    let rest = trimmed.strip_prefix("+++")?;
    if !rest.starts_with(' ') {
        return None;
    }
    let mut rest = rest.trim_start_matches(' ');
    let length = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    let command = rest.get(..length)?;
    if !rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        || SUPPLE_RESERVED.contains(&command)
    {
        return None;
    }
    rest = &rest[length..];
    let after = rest.strip_prefix(" --end")?;
    let mut id = None;
    let mut wt = None;
    for token in after.split_whitespace() {
        if let Some(value) = token
            .strip_prefix("{id:")
            .and_then(|value| value.strip_suffix('}'))
        {
            id = Some(value);
        } else if let Some(value) = token
            .strip_prefix("{wt:")
            .and_then(|value| value.strip_suffix('}'))
        {
            wt = Some(value);
        } else {
            return None;
        }
    }
    Some(SuppleEndLine {
        indent,
        command,
        id,
        wt,
    })
}

/// 行尾可选的标题：`{title: …}`（lx-tui）或 `「title: …」`（参考项目）；空行返回 Some(None)。
fn parse_trailing_title(rest: &str) -> Option<Option<&str>> {
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        return Some(None);
    }
    if let Some(inner) = trimmed
        .strip_prefix("{title:")
        .and_then(|value| value.strip_suffix('}'))
    {
        if inner.contains(['{', '}']) {
            return None;
        }
        return Some(Some(inner));
    }
    if let Some(inner) = trimmed
        .strip_prefix("「title:")
        .and_then(|value| value.strip_suffix('」'))
    {
        if inner.contains(['「', '」', '\n']) {
            return None;
        }
        return Some(Some(inner));
    }
    None
}

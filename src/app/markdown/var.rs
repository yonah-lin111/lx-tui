//! `$$$` 变量块协议：起止行解析。

/// 变量块起始行解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarStartLine<'a> {
    pub indent: &'a str,
    /// 显式命令名；裸 `$$$` 为 None。
    pub command: Option<&'a str>,
    /// 行内兼容标题；缺省为 None。
    pub title: Option<&'a str>,
}

/// 变量块结束行解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarEndLine<'a> {
    pub indent: &'a str,
    pub command: Option<&'a str>,
    pub id: Option<&'a str>,
}

/// 变量块命令名（新插入固定使用）。
pub const VAR_COMMAND: &str = "varTemplate";

/// 解析变量块起始行：`$$$ [varTemplate [--start]] [{title: …}|「title: …」]`。
///
/// 裸 `$$$` 亦为合法起始（与结束行同为裸标记，由扫描器按块内外状态判定）。
pub fn parse_var_start_line(line: &str) -> Option<VarStartLine<'_>> {
    let trimmed = line.trim_start_matches(' ');
    let indent = &line[..line.len() - trimmed.len()];
    let mut rest = trimmed.strip_prefix("$$$")?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    rest = rest.trim_start_matches(' ');
    let mut command = None;
    if let Some(after) = strip_word(rest, VAR_COMMAND) {
        command = Some(VAR_COMMAND);
        rest = after.trim_start_matches(' ');
    }
    if let Some(after) = strip_word(rest, "--start") {
        rest = after.trim_start_matches(' ');
    }
    let title = parse_trailing_title(rest)?;
    Some(VarStartLine {
        indent,
        command,
        title,
    })
}

/// 解析变量块结束行：`$$$ [varTemplate --end | --end] [{id: …}]`。
pub fn parse_var_end_line(line: &str) -> Option<VarEndLine<'_>> {
    let trimmed = line.trim_start_matches(' ');
    let indent = &line[..line.len() - trimmed.len()];
    let mut rest = trimmed.strip_prefix("$$$")?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    rest = rest.trim_start_matches(' ');
    let mut command = None;
    if let Some(after) = strip_word(rest, VAR_COMMAND) {
        command = Some(VAR_COMMAND);
        rest = after.trim_start_matches(' ');
    }
    if rest.is_empty() {
        return Some(VarEndLine {
            indent,
            command,
            id: None,
        });
    }
    let after = strip_word(rest, "--end")?;
    let mut id = None;
    for token in after.split_whitespace() {
        if let Some(value) = token
            .strip_prefix("{id:")
            .and_then(|value| value.strip_suffix('}'))
        {
            id = Some(value);
        } else {
            return None;
        }
    }
    Some(VarEndLine {
        indent,
        command,
        id,
    })
}

/// 若 `rest` 以 `word` 开头且后随空白或行尾，返回其后剩余；否则 None。
fn strip_word<'a>(rest: &'a str, word: &str) -> Option<&'a str> {
    let after = rest.strip_prefix(word)?;
    (after.is_empty() || after.starts_with(' ')).then_some(after)
}

/// 行尾可选的标题：`{title: …}` 或 `「title: …」`；空行返回 Some(None)。
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

//! 模板块协议：起止行解析、状态轮转与空项清理。

/// 模板块状态；结束行后缀 ` done` / ` in_progress`，缺省为 `todo`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateStatus {
    Todo,
    InProgress,
    Done,
}

impl TemplateStatus {
    /// 结束行源码后缀；`todo` 无后缀。
    pub fn suffix(self) -> &'static str {
        match self {
            TemplateStatus::Todo => "",
            TemplateStatus::InProgress => " in_progress",
            TemplateStatus::Done => " done",
        }
    }

    /// 循环切换：todo → in_progress → done → todo。
    pub fn next(self) -> Self {
        match self {
            TemplateStatus::Todo => TemplateStatus::InProgress,
            TemplateStatus::InProgress => TemplateStatus::Done,
            TemplateStatus::Done => TemplateStatus::Todo,
        }
    }
}

/// 模板块起始行解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateStartLine<'a> {
    pub indent: &'a str,
    pub command: &'a str,
    pub title: Option<&'a str>,
}

/// 模板块结束行解析结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateEndLine<'a> {
    pub indent: &'a str,
    pub command: Option<&'a str>,
    pub end_flag: bool,
    pub status: TemplateStatus,
    pub id: Option<&'a str>,
    pub wt: Option<&'a str>,
}

/// 模板块范围（逻辑行索引，端点含）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateBlockRange {
    pub start: usize,
    pub end: usize,
}

/// 起始行保留词：状态与子块命令不可作为模板块命令名。
const TEMPLATE_RESERVED: [&str; 6] = [
    "done",
    "in_progress",
    "supple",
    "suppleTemplate",
    "log",
    "logTemplate",
];

/// 解析模板块起始行：`&&& <command> [--start] [「title: …」]`。
pub fn parse_template_start_line(line: &str) -> Option<TemplateStartLine<'_>> {
    let trimmed = line.trim_start_matches(' ');
    let indent = &line[..line.len() - trimmed.len()];
    let rest = trimmed.strip_prefix("&&&")?;
    if !rest.starts_with(' ') {
        return None;
    }
    let rest = rest.trim_start_matches(' ');
    let length = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    let command = rest.get(..length)?;
    if !rest.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        || TEMPLATE_RESERVED.contains(&command)
    {
        return None;
    }
    let mut rest = &rest[length..];
    if let Some(after) = rest.strip_prefix(" --start") {
        rest = after;
    }
    let title = if rest.trim().is_empty() {
        None
    } else {
        let after = rest.trim_start_matches(' ').strip_prefix("「title:")?;
        let close = after.find('」')?;
        if !after[close + '」'.len_utf8()..].trim().is_empty() {
            return None;
        }
        Some(&after[..close])
    };
    Some(TemplateStartLine {
        indent,
        command,
        title,
    })
}

/// 解析模板块结束行：`&&& [command --end] [done|in_progress] [{id:…}] [{wt:…}]`。
pub fn parse_template_end_line(line: &str) -> Option<TemplateEndLine<'_>> {
    let trimmed = line.trim_start_matches(' ');
    let indent = &line[..line.len() - trimmed.len()];
    let rest = trimmed.strip_prefix("&&&")?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    let mut command = None;
    let mut end_flag = false;
    let mut status = TemplateStatus::Todo;
    let mut id = None;
    let mut wt = None;
    for token in rest.split_whitespace() {
        if token == "--end" {
            if end_flag {
                return None;
            }
            end_flag = true;
        } else if token == "done" {
            status = TemplateStatus::Done;
        } else if token == "in_progress" {
            status = TemplateStatus::InProgress;
        } else if let Some(value) = token
            .strip_prefix("{id:")
            .and_then(|value| value.strip_suffix('}'))
        {
            id = Some(value);
        } else if let Some(value) = token
            .strip_prefix("{wt:")
            .and_then(|value| value.strip_suffix('}'))
        {
            wt = Some(value);
        } else if command.is_none() && !end_flag && is_ascii_command(token) {
            command = Some(token);
        } else {
            return None;
        }
    }
    if command.is_some() && !end_flag {
        return None;
    }
    Some(TemplateEndLine {
        indent,
        command,
        end_flag,
        status,
        id,
        wt,
    })
}

/// 解析 `line` 起始的模板块范围；非起始行返回 None，未闭合时 `end` 为文末行。
pub fn parse_template_block_at_line(text: &str, line: usize) -> Option<TemplateBlockRange> {
    let lines: Vec<&str> = text.split('\n').collect();
    parse_template_start_line(lines.get(line)?)?;
    for (index, content) in lines.iter().enumerate().skip(line + 1) {
        if parse_template_end_line(content).is_some() {
            return Some(TemplateBlockRange {
                start: line,
                end: index,
            });
        }
    }
    Some(TemplateBlockRange {
        start: line,
        end: lines.len().saturating_sub(1),
    })
}

/// offset 是否位于模板块内；用于斜杠命令触发抑制。
pub fn inside_template_block(text: &str, offset: usize) -> bool {
    let mut inside = false;
    let mut position = 0;
    for line in text.split('\n') {
        if position >= offset {
            break;
        }
        if parse_template_end_line(line).is_some() {
            inside = false;
        } else if parse_template_start_line(line).is_some() {
            inside = true;
        }
        position += line.len() + 1;
    }
    inside
}

/// 循环切换模板块结束行状态并保留 command、id 与 wt；非结束行返回 None。
pub fn cycle_template_status(line: &str) -> Option<String> {
    use std::fmt::Write;

    let parsed = parse_template_end_line(line)?;
    let mut out = String::from(parsed.indent);
    out.push_str("&&&");
    if let Some(command) = parsed.command {
        out.push(' ');
        out.push_str(command);
        out.push_str(" --end");
    } else if parsed.end_flag {
        out.push_str(" --end");
    }
    out.push_str(parsed.status.next().suffix());
    if let Some(id) = parsed.id {
        let _ = write!(out, " {{id:{id}}}");
    }
    if let Some(wt) = parsed.wt {
        let _ = write!(out, " {{wt:{wt}}}");
    }
    Some(out)
}

/// 过滤模板块正文中未填写的空列表项（含仅剩 `key:` 占位且无有效子项的项）；
/// `+++` / `%%%` 子块内部原样保留；多余空行折叠、首尾空白清理。
pub fn clean_template_content(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut remove = vec![false; lines.len()];
    let mut inside_subblock = false;
    for (index, line) in lines.iter().enumerate() {
        if subblock_start(line) {
            inside_subblock = true;
            continue;
        }
        if inside_subblock {
            if subblock_end(line) {
                inside_subblock = false;
            }
            continue;
        }
        let Some(item) = template_list_item(line) else {
            continue;
        };
        if item.body.is_empty() {
            remove[index] = true;
            continue;
        }
        if !is_placeholder_label(item.body) {
            continue;
        }
        let mut filled = false;
        for child in lines.iter().skip(index + 1) {
            if subblock_start(child) {
                break;
            }
            let Some(child_item) = template_list_item(child) else {
                break;
            };
            if child_item.indent <= item.indent {
                break;
            }
            if !child_item.body.is_empty() && !is_placeholder_label(child_item.body) {
                filled = true;
                break;
            }
        }
        if filled {
            continue;
        }
        remove[index] = true;
        for (child_index, child) in lines.iter().enumerate().skip(index + 1) {
            if subblock_start(child) {
                break;
            }
            let Some(child_item) = template_list_item(child) else {
                break;
            };
            if child_item.indent <= item.indent {
                break;
            }
            remove[child_index] = true;
        }
    }
    let joined = lines
        .iter()
        .enumerate()
        .filter(|(index, _)| !remove[*index])
        .map(|(_, line)| *line)
        .collect::<Vec<_>>()
        .join("\n");
    collapse_blank_lines(&joined)
        .trim_start_matches('\n')
        .trim_end()
        .to_string()
}

/// 列表项：缩进宽度与去空白后的项体。
struct TemplateListItem<'a> {
    indent: usize,
    body: &'a str,
}

/// 解析列表项（`-`/`*`/`+` 加可选任务框）；对齐 lx-agent 的 `isTemplateListItemLine`。
fn template_list_item(line: &str) -> Option<TemplateListItem<'_>> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let rest = &line[indent..];
    let marker = rest.chars().next()?;
    if !matches!(marker, '-' | '*' | '+') {
        return None;
    }
    let mut body = rest[marker.len_utf8()..].trim_start_matches([' ', '\t']);
    if let Some(stripped) = strip_task_checkbox(body) {
        body = stripped.trim_start_matches([' ', '\t']);
    }
    Some(TemplateListItem {
        indent,
        body: body.trim(),
    })
}

/// 去掉任务框 `[ ]`/`[x]`；不是合法任务框返回 None。
fn strip_task_checkbox(text: &str) -> Option<&str> {
    let mut chars = text.chars();
    if chars.next() != Some('[') {
        return None;
    }
    if !matches!(chars.next(), Some(' ' | 'x' | 'X')) {
        return None;
    }
    if chars.next() != Some(']') {
        return None;
    }
    Some(&text[3..])
}

/// 占位标签：以中英冒号结尾且之前不含冒号（如 `Reference:`）。
fn is_placeholder_label(body: &str) -> bool {
    let Some(last) = body.chars().next_back() else {
        return false;
    };
    if last != ':' && last != '：' {
        return false;
    }
    body[..body.len() - last.len_utf8()]
        .chars()
        .all(|ch| ch != ':' && ch != '：')
}

/// `+++ <name> --start` / `%%% <name> --start` 子块起始行。
fn subblock_start(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    matches!(tokens.next(), Some("+++") | Some("%%%")) && tokens.any(|token| token == "--start")
}

/// `+++ <name> --end` / `%%% <name> --end` 子块结束行。
fn subblock_end(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    matches!(tokens.next(), Some("+++") | Some("%%%")) && tokens.any(|token| token == "--end")
}

/// 折叠 3 个及以上连续换行为 2 个（对齐 lx-agent 的 `\n{3,}` 折叠）。
fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0;
    for ch in text.chars() {
        if ch == '\n' {
            newlines += 1;
            continue;
        }
        if newlines > 0 {
            out.push_str(&"\n".repeat(newlines.min(2)));
            newlines = 0;
        }
        out.push(ch);
    }
    if newlines > 0 {
        out.push_str(&"\n".repeat(newlines.min(2)));
    }
    out
}

/// 命令词：ASCII 字母开头，其余为字母数字或下划线。
fn is_ascii_command(token: &str) -> bool {
    let mut bytes = token.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphabetic() && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

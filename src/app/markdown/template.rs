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

/// 解析模板块起始行：`&&& <command> [--start] [{title: …}]`。
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
        let after = rest.trim_start_matches(' ').strip_prefix("{title:")?;
        let close = after.find('}')?;
        if !after[close + 1..].trim().is_empty() || after[..close].contains(['{', '}']) {
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

/// 解析独立的 worktree 绑定行：整行（允许缩进与尾随空白）必须是 `{wt:<branch>}`；
/// 分支名非空且不含空白与花括号。
pub fn parse_worktree_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?;
    let branch = inner.strip_prefix("wt:")?;
    let valid = !branch.is_empty()
        && !branch
            .chars()
            .any(|ch| ch.is_whitespace() || ch == '{' || ch == '}');
    valid.then_some(branch)
}

/// 解析 `line` 起始的模板块范围；非起始行返回 None，未闭合时 `end` 为文末行。
pub fn parse_template_block_at_line(text: &str, line: usize) -> Option<TemplateBlockRange> {
    super::blocks::block_scan(text)
        .spans
        .into_iter()
        .find(|span| span.kind == super::blocks::BlockKind::Template && span.start == line)
        .map(|span| TemplateBlockRange {
            start: span.start,
            end: span.end,
        })
}

/// offset 所在模板块的开始行；不在块内返回 None（含起止行自身）。
pub fn template_block_start_at(text: &str, offset: usize) -> Option<usize> {
    super::blocks::block_at(text, offset)
        .filter(|info| info.kind == super::blocks::BlockKind::Template)
        .map(|info| info.start)
}

/// 模板块行的渲染角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateLineRole {
    Start,
    Middle,
    End,
}

/// 模板块行的渲染信息：角色与所属块状态（未闭合块按 `todo`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemplateLineInfo {
    pub role: TemplateLineRole,
    pub status: TemplateStatus,
    /// 是否为结构分割行：标题行之下或 `{wt:}` 绑定行之上的空行，渲染为块边框横线。
    pub separator: bool,
}

/// 逐行扫描模板块归属（由 `block_scan` 投影）；不在块内的行为 None，未闭合块延伸至文末。
pub fn template_line_infos(text: &str) -> Vec<Option<TemplateLineInfo>> {
    super::blocks::block_scan(text)
        .infos
        .into_iter()
        .map(|info| {
            let info = info.filter(|info| info.kind == super::blocks::BlockKind::Template)?;
            let role = match info.role {
                super::blocks::BlockRole::Start => TemplateLineRole::Start,
                super::blocks::BlockRole::Middle => TemplateLineRole::Middle,
                super::blocks::BlockRole::End => TemplateLineRole::End,
            };
            Some(TemplateLineInfo {
                role,
                status: info.status,
                separator: info.separator,
            })
        })
        .collect()
}

/// `line` 是否位于模板块内（含起止行）；用于渲染与光标/鼠标坐标映射。
pub fn is_template_block_line(text: &str, line: usize) -> bool {
    template_line_infos(text)
        .get(line)
        .copied()
        .flatten()
        .is_some()
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

/// 清理后的块正文补齐底部结构空行：末行为 `{wt:}` 绑定行时保持原样；
/// 否则在结束行前保证一空行（块底部留白；空正文保持不变）。
pub fn ensure_bottom_gap(body: String) -> String {
    let trimmed = body.trim_end_matches('\n');
    if trimmed.is_empty()
        || trimmed
            .lines()
            .last()
            .is_some_and(|line| parse_worktree_line(line).is_some())
    {
        return trimmed.to_string();
    }
    format!("{trimmed}\n")
}

/// 复制用正文（对齐 lx-agent 的复制语义，不修改原文）：
/// 剔除标题占位行（`--start` 下一行）、`{wt:}` 绑定行，
/// 保留 `%%%` / `+++ log` 记录子块内容但剔除其起止标记行、剔除 `//` 注释行，最后清理未填写空项。
pub fn copy_template_content(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut kept: Vec<&str> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index == 0 && is_title_line(line) {
            continue;
        }
        if parse_worktree_line(line).is_some() {
            continue;
        }
        if is_log_subblock_start(line) || subblock_end(line) || is_comment_line(line) {
            continue;
        }
        kept.push(line);
    }
    clean_template_content(&kept.join("\n"))
}

/// 解析独立标题行 `{title: …}`（允许缩进与尾随空白）；返回冒号后的原始内容。
pub fn parse_title_line(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let inner = trimmed.strip_prefix('{')?.strip_suffix('}')?;
    let title = inner.strip_prefix("title:")?;
    (!title.contains(['{', '}'])).then_some(title)
}

/// `line` 是否为独立标题行 `{title: …}`。
pub fn is_title_line(line: &str) -> bool {
    parse_title_line(line).is_some()
}

/// 内置模板默认章节标题：单独存在且无其它实质内容时不视为有效标题提炼输入。
const DEFAULT_TEMPLATE_HEADINGS: [&str; 6] = [
    "# Add Requirement",
    "# Fix Bug",
    "# Execute Task",
    "# Refactor Feature",
    "# Design Style",
    "## Supplementary Requirements",
];

/// 提取用于 `/title` 生成的模板块有效正文：
/// 经过 `copy_template_content` 清洗后，进一步剔除仅存的内置模板默认大标题行。
pub fn extract_block_title_source(body: &str) -> String {
    let copied = copy_template_content(body);
    copied
        .lines()
        .filter(|line| !DEFAULT_TEMPLATE_HEADINGS.contains(&line.trim()))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// 读取目标块起始行下一行的 `{title: ...}` 标题值（或起始行内联标题）；若不存在标题返回 None。
pub fn block_title_value_at_line(text: &str, start_line: usize) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    if let Some(next_line) = lines.get(start_line + 1)
        && let Some(title) = parse_title_line(next_line)
    {
        return Some(title.trim().to_string());
    }
    let start_str = lines.get(start_line)?;
    if let Some(parsed) = parse_template_start_line(start_str)
        && let Some(title) = parsed.title
    {
        return Some(title.trim().to_string());
    }
    if let Some(parsed) = super::supple::parse_supple_start_line(start_str)
        && let Some(title) = parsed.title
    {
        return Some(title.trim().to_string());
    }
    None
}

/// 计算设置/替换/移除块标题行所需的替换字节区间与替换文本（同时兼容起始行内联标题与下一行独立标题行）。
pub fn set_block_title_edit(
    text: &str,
    start_line: usize,
    new_title: Option<&str>,
) -> Option<(std::ops::Range<usize>, String)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let start_str = lines.get(start_line)?;
    let inline_title_idx = if parse_template_start_line(start_str)
        .is_some_and(|s| s.title.is_some())
        || super::supple::parse_supple_start_line(start_str).is_some_and(|s| s.title.is_some())
    {
        start_str
            .find("{title:")
            .or_else(|| start_str.find("「title:"))
    } else if parse_template_start_line(start_str).is_some()
        || super::supple::parse_supple_start_line(start_str).is_some()
    {
        None
    } else {
        return None;
    };
    let indent = &start_str[..start_str.len() - start_str.trim_start_matches(' ').len()];

    let mut line_starts = Vec::with_capacity(lines.len() + 1);
    let mut offset = 0;
    for line in &lines {
        line_starts.push(offset);
        offset += line.len() + 1;
    }
    line_starts.push(offset);

    let has_title_next = lines.get(start_line + 1).is_some_and(|l| is_title_line(l));

    match new_title {
        Some(raw_title) => {
            let safe_title = raw_title.replace(['{', '}'], "");
            let safe_title = safe_title.trim();
            if has_title_next {
                let t_start = line_starts[start_line + 1];
                let t_end = t_start + lines[start_line + 1].len();
                Some((t_start..t_end, format!("{indent}{{title: {safe_title}}}")))
            } else if let Some(inline_idx) = inline_title_idx {
                let head = start_str[..inline_idx].trim_end_matches(' ');
                let s_start = line_starts[start_line];
                let s_end = s_start + start_str.len();
                Some((
                    s_start..s_end,
                    format!("{head}\n{indent}{{title: {safe_title}}}"),
                ))
            } else {
                let insert_at = line_starts[start_line] + start_str.len();
                Some((
                    insert_at..insert_at,
                    format!("\n{indent}{{title: {safe_title}}}"),
                ))
            }
        }
        None => {
            if has_title_next {
                let t_start = line_starts[start_line + 1];
                let t_end = line_starts[start_line + 2].min(text.len());
                Some((t_start..t_end, String::new()))
            } else if let Some(inline_idx) = inline_title_idx {
                let head = start_str[..inline_idx].trim_end_matches(' ');
                let s_start = line_starts[start_line];
                let s_end = s_start + start_str.len();
                Some((s_start..s_end, head.to_string()))
            } else {
                None
            }
        }
    }
}

/// 结构性标题行（模板块起始行下一行）的标记区间：受保护的 `{title:` 前缀
/// 与收尾 `}` 及其后区域，之间的值为可编辑区。
struct TitleMarker {
    line_start: usize,
    prefix_end: usize,
    brace: usize,
    line_end: usize,
}

/// 全部结构性标题行的标记区间。
fn title_markers(text: &str) -> Vec<TitleMarker> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut offsets = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in &lines {
        offsets.push(offset);
        offset += line.len() + 1;
    }
    let mut markers = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if parse_template_start_line(line).is_none()
            && super::supple::parse_supple_start_line(line).is_none()
        {
            continue;
        }
        let Some(next) = lines.get(index + 1) else {
            continue;
        };
        if !is_title_line(next) {
            continue;
        }
        let trimmed_len = next.trim_end().len();
        let Some(line_start) = offsets.get(index + 1).copied() else {
            continue;
        };
        let indent = next.len() - next.trim_start().len();
        markers.push(TitleMarker {
            line_start,
            prefix_end: line_start + indent + "{title:".len(),
            brace: line_start + trimmed_len.saturating_sub(1),
            line_end: line_start + next.len(),
        });
    }
    markers
}

/// 目标编辑是否移除或改写结构性标题行 `{title: …}` 的标记：值内编辑放行；
/// 触碰 `{title:` 前缀、收尾 `}` 或相邻换行被拒绝；严格覆盖整行以上范围
/// （清空全文、删整块）放行。
pub fn edit_touches_title_line(text: &str, from: usize, to: usize) -> bool {
    let insertion = from == to;
    for marker in title_markers(text) {
        if insertion {
            // 只禁止写入前缀区（`{title:` 及其之前）；值内与 `}` 之后的插入放行，
            // 换行不区分内容，避免误伤行尾回车。
            if from >= marker.line_start && from < marker.prefix_end {
                return true;
            }
            continue;
        }
        if from < marker.line_start && to > marker.line_end {
            continue;
        }
        if to == marker.line_start || from == marker.line_end {
            return true;
        }
        if from < marker.prefix_end && to > marker.line_start {
            return true;
        }
        if to > marker.brace && from < marker.line_end {
            return true;
        }
    }
    false
}

/// 记录子块起始行：`%%% <name> --start` 或旧版 `+++ log` / `+++ logTemplate --start`（保留内容、剔除标记）。
fn is_log_subblock_start(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    let Some(marker) = tokens.next() else {
        return false;
    };
    let Some(name) = tokens.next() else {
        return false;
    };
    if !tokens.any(|token| token == "--start") {
        return false;
    }
    matches!((marker, name), ("%%%", _) | ("+++", "log" | "logTemplate"))
}

/// `//` 注释行（允许前置缩进）。
fn is_comment_line(line: &str) -> bool {
    line.trim_start().starts_with("//")
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

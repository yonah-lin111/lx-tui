//! 变量块内容解析与文本变换：从已闭合 `$$$` 块提取变量、清理空项、合并与置顶。
//!
//! 纯函数、无 IO；解析兼容参考项目的单行引号、三引号、`|`/`>` 多行、层级点号与
//! `vars:` 平级映射。当前 Prompt 文档内变量以正文为唯一真源，同名定义由最后一项覆盖。

use std::collections::HashMap;

use super::blocks::{
    BlockKind, BlockSpan, block_deletion_range, block_spans, line_end_at, line_start_at,
};
use super::var::{parse_var_end_line, parse_var_start_line};

/// 一条变量：名称（可含点号层级）与值（多行以 `\n` 连接）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableEntry {
    pub name: String,
    pub value: String,
}

/// 把多段文本按行拆分（兼容 CRLF）。
fn split_lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// 前导空白列数（按字符计；与参考实现的 `\s` 语义近似）。
fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches([' ', '\t']).len()
}

/// 解析 `$$$` 块正文中的变量条目（对齐参考实现）。
pub fn parse_var_content(raw: &str) -> Vec<VariableEntry> {
    let lines = split_lines(raw);
    let mut entries: Vec<VariableEntry> = Vec::new();
    let mut stack: Vec<(String, usize)> = Vec::new();

    let mut current_key: Option<String> = None;
    let mut current_value: Vec<String> = Vec::new();
    let mut is_multi = false;
    let mut is_triple = false;
    let mut multi_indent: isize = -1;

    let flush = |entries: &mut Vec<VariableEntry>,
                 current_key: &mut Option<String>,
                 current_value: &mut Vec<String>,
                 is_multi: &mut bool,
                 is_triple: &mut bool,
                 multi_indent: &mut isize| {
        if let Some(key) = current_key.take() {
            let value = if *is_multi {
                current_value.join("\n").trim_end().to_string()
            } else {
                current_value.join(" ").trim().to_string()
            };
            entries.push(VariableEntry { name: key, value });
        }
        current_value.clear();
        *is_multi = false;
        *is_triple = false;
        *multi_indent = -1;
    };

    let full_key = |stack: &[(String, usize)], key: &str| -> String {
        let start = usize::from(stack.first().is_some_and(|(k, _)| k == "vars"));
        let mut parts: Vec<&str> = stack[start..].iter().map(|(k, _)| k.as_str()).collect();
        parts.push(key);
        parts.join(".")
    };

    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let indent = indent_of(line);

        if is_triple {
            if line.trim() == "\"\"\"" {
                flush(
                    &mut entries,
                    &mut current_key,
                    &mut current_value,
                    &mut is_multi,
                    &mut is_triple,
                    &mut multi_indent,
                );
                index += 1;
                continue;
            }
            let stripped = if indent > multi_indent as usize {
                line.get(multi_indent as usize..).unwrap_or(line)
            } else {
                line.trim_start()
            };
            current_value.push(stripped.to_string());
            index += 1;
            continue;
        }

        if is_multi {
            if line.trim().is_empty() {
                current_value.push(String::new());
                index += 1;
                continue;
            }
            if indent > multi_indent as usize {
                current_value.push(
                    line.get(multi_indent as usize + 2..)
                        .unwrap_or("")
                        .to_string(),
                );
                index += 1;
                continue;
            }
            flush(
                &mut entries,
                &mut current_key,
                &mut current_value,
                &mut is_multi,
                &mut is_triple,
                &mut multi_indent,
            );
        }

        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with("//")
            || trimmed.starts_with("+++")
            || trimmed.starts_with("%%%")
            || trimmed.starts_with("---")
        {
            index += 1;
            continue;
        }

        while stack.last().is_some_and(|(_, level)| *level >= indent) {
            stack.pop();
        }

        let Some(colon) = line.find(':') else {
            index += 1;
            continue;
        };
        let key = line[..colon].trim();
        let mut raw_value = line[colon + 1..].trim();
        if key.is_empty()
            || key.starts_with(['#', '-'])
            || key.starts_with("+++")
            || key.starts_with("%%%")
        {
            index += 1;
            continue;
        }

        flush(
            &mut entries,
            &mut current_key,
            &mut current_value,
            &mut is_multi,
            &mut is_triple,
            &mut multi_indent,
        );
        let full = full_key(&stack, key);

        if raw_value.is_empty() || raw_value.starts_with('#') {
            let mut next = index + 1;
            while next < lines.len() && lines[next].trim().is_empty() {
                next += 1;
            }
            if next < lines.len() && lines[next].trim().starts_with("\"\"\"") {
                current_key = Some(full);
                is_multi = true;
                is_triple = true;
                multi_indent = indent_of(lines[next]) as isize;
                index = next + 1;
                continue;
            }
            stack.push((key.to_string(), indent));
            index += 1;
            continue;
        }

        if let Some(rest) = raw_value.strip_prefix("\"\"\"") {
            let rest = rest.trim();
            if let Some(inner) = rest.strip_suffix("\"\"\"") {
                entries.push(VariableEntry {
                    name: full,
                    value: inner.trim().to_string(),
                });
            } else {
                current_key = Some(full);
                is_multi = true;
                is_triple = true;
                multi_indent = indent as isize;
                if !rest.is_empty() {
                    current_value.push(rest.to_string());
                }
            }
            index += 1;
            continue;
        }

        if matches!(raw_value, "|" | ">" | "|-" | ">-") {
            current_key = Some(full);
            is_multi = true;
            multi_indent = indent as isize;
            index += 1;
            continue;
        }

        if let Some(rest) = raw_value.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                raw_value = &rest[..end];
            }
        } else if let Some(rest) = raw_value.strip_prefix('\'') {
            if let Some(end) = rest.find('\'') {
                raw_value = &rest[..end];
            }
        } else if let Some(comment) = raw_value.find('#') {
            raw_value = raw_value[..comment].trim();
        }
        entries.push(VariableEntry {
            name: full,
            value: raw_value.to_string(),
        });
        index += 1;
    }

    flush(
        &mut entries,
        &mut current_key,
        &mut current_value,
        &mut is_multi,
        &mut is_triple,
        &mut multi_indent,
    );
    entries
}

/// 抽取全部已闭合 `$$$` 块的正文。
fn var_block_contents(text: &str) -> Vec<String> {
    let lines = split_lines(text);
    let mut blocks = Vec::new();
    let mut inside = false;
    let mut current: Vec<&str> = Vec::new();
    for line in lines {
        if !inside {
            if parse_var_start_line(line).is_some() {
                inside = true;
                current.clear();
            }
        } else if parse_var_end_line(line).is_some() {
            inside = false;
            blocks.push(current.join("\n"));
            current.clear();
        } else {
            current.push(line);
        }
    }
    blocks
}

/// 当前文档变量：从已闭合 `$$$` 块收集；无变量块时向下兼容文首 `---` frontmatter。
///
/// 同名定义由最后一项覆盖，候选保留首次出现顺序、取最后一次的值。
pub fn document_variables(text: &str) -> Vec<VariableEntry> {
    let blocks = var_block_contents(text);
    let entries = if !blocks.is_empty() {
        blocks
            .iter()
            .flat_map(|block| parse_var_content(block))
            .collect::<Vec<_>>()
    } else {
        match frontmatter(text) {
            Some(front) => parse_var_content(front),
            None => Vec::new(),
        }
    };
    dedupe_last_wins(entries)
}

/// 同名去重：保留首次出现位置，值取最后一次定义。
fn dedupe_last_wins(entries: Vec<VariableEntry>) -> Vec<VariableEntry> {
    let mut result: Vec<VariableEntry> = Vec::new();
    let mut index_of: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        if let Some(&position) = index_of.get(&entry.name) {
            result[position].value = entry.value;
        } else {
            index_of.insert(entry.name.clone(), result.len());
            result.push(entry);
        }
    }
    result
}

/// 文首 `---` frontmatter 正文；不存在返回 None。
fn frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']).trim() == "---" {
            return Some(&rest[..offset]);
        }
        offset += line.len();
    }
    None
}

/// 清理变量块正文中未填写的条目：空引号、默认 `"var"`、空三引号与无实际值的项。
///
/// 保留 `@content:` 保留键、已填键值与多行正文；`+++`/`%%%` 子块原样保留。
pub fn clean_var_content(block: &str) -> String {
    let lines = split_lines(block);
    let mut preserved: Vec<&str> = Vec::new();
    let mut index = 0;
    let mut in_subblock = false;

    while index < lines.len() {
        let line = lines[index];
        if !in_subblock && is_subblock_start(line) {
            in_subblock = true;
            preserved.push(line);
            index += 1;
            continue;
        }
        if in_subblock {
            if is_subblock_end(line) {
                in_subblock = false;
            }
            preserved.push(line);
            index += 1;
            continue;
        }
        if is_content_key(line) {
            preserved.push(line);
            index += 1;
            continue;
        }
        if is_placeholder_entry(line) {
            index += 1;
            continue;
        }
        if is_bare_key(line) {
            let indent = indent_of(line);
            if index + 1 < lines.len() && lines[index + 1].trim() == "\"\"\"" {
                let mut j = index + 2;
                let mut sub: Vec<String> = Vec::new();
                while j < lines.len() && lines[j].trim() != "\"\"\"" {
                    sub.push(lines[j].trim().to_string());
                    j += 1;
                }
                if j < lines.len() {
                    let unfilled = sub.is_empty()
                        || (sub.len() == 1 && (sub[0] == "var" || sub[0].is_empty()));
                    if unfilled {
                        index = j + 1;
                        continue;
                    }
                    preserved.extend(lines[index..=j].iter().copied());
                    index = j + 1;
                    continue;
                }
            }
            let mut has_child = false;
            for child in lines.iter().skip(index + 1) {
                if !child.trim().is_empty() {
                    has_child = indent_of(child) > indent;
                    break;
                }
            }
            if !has_child {
                index += 1;
                continue;
            }
        }
        preserved.push(line);
        index += 1;
    }

    let result = drop_empty_parents(&preserved);
    result.join("\n").trim_end().to_string()
}

/// 剔除因清理而变空的父级命名空间（其后无更深缩进子项）。
fn drop_empty_parents(lines: &[&str]) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if let Some(indent) = bare_key_indent(line) {
            if lines
                .get(index + 1)
                .is_some_and(|next| next.trim() == "\"\"\"")
            {
                result.push((*line).to_string());
                continue;
            }
            let mut has_child = false;
            for next in lines.iter().skip(index + 1) {
                if !next.trim().is_empty() {
                    has_child = indent_of(next) > indent;
                    break;
                }
            }
            if !has_child {
                continue;
            }
        }
        result.push((*line).to_string());
    }
    result
}

/// `key:` 形式（无行内值）且键名合法时返回其缩进。
fn bare_key_indent(line: &str) -> Option<usize> {
    let trimmed = line.trim_end();
    let rest = trimmed.strip_suffix(':')?;
    let key = rest.trim_start();
    let indent = rest.len() - key.len();
    let valid = !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'));
    valid.then_some(indent)
}

/// 单行占位/空值：`key: "var"` / `key: 'var'` / `key: ""` / `key: ''`。
fn is_placeholder_entry(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(colon) = trimmed.find(':') else {
        return false;
    };
    let key = trimmed[..colon].trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'))
    {
        return false;
    }
    matches!(
        trimmed[colon + 1..].trim(),
        "\"var\"" | "'var'" | "\"\"" | "''"
    )
}

/// `key:` 形式（无行内值）。
fn is_bare_key(line: &str) -> bool {
    bare_key_indent(line).is_some()
}

/// `@content:` 保留键行。
fn is_content_key(line: &str) -> bool {
    line.trim() == "@content:"
}

/// `+++`/`%%%...--start` 子块起始行。
fn is_subblock_start(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    matches!(tokens.next(), Some("+++") | Some("%%%")) && tokens.any(|token| token == "--start")
}

/// `+++`/`%%%...--end` 子块结束行。
fn is_subblock_end(line: &str) -> bool {
    let mut tokens = line.split_whitespace();
    matches!(tokens.next(), Some("+++") | Some("%%%")) && tokens.any(|token| token == "--end")
}

/// 首个 `$$$` 变量块范围。
pub fn first_var_span(text: &str) -> Option<BlockSpan> {
    block_spans(text)
        .into_iter()
        .find(|span| span.kind == BlockKind::Var)
}
/// 全部 `$$$` 变量块范围。
pub fn var_spans(text: &str) -> Vec<BlockSpan> {
    block_spans(text)
        .into_iter()
        .filter(|span| span.kind == BlockKind::Var)
        .collect()
}

/// 光标是否位于某个变量块的三引号多行字符串内部。
pub fn var_in_triple_quotes(text: &str, cursor: usize) -> bool {
    let Some(span) = super::blocks::block_span_at(text, cursor) else {
        return false;
    };
    if span.kind != BlockKind::Var {
        return false;
    }
    let lines = split_lines(text);
    let cursor_line = text[..cursor.min(text.len())].matches('\n').count();
    let last = cursor_line.min(span.end);
    let mut in_triple = false;
    for (index, line) in lines.iter().enumerate().take(last + 1).skip(span.start + 1) {
        if line.matches("\"\"\"").count() % 2 == 1 {
            in_triple = !in_triple;
        }
        if index == last {
            break;
        }
    }
    in_triple
}

/// 变量块内 Tab 跳转目标：每个键行的键名结束字节偏移，按文档顺序。
pub fn var_tab_targets(text: &str, cursor: usize) -> Vec<usize> {
    let Some(span) = super::blocks::block_span_at(text, cursor) else {
        return Vec::new();
    };
    if span.kind != BlockKind::Var {
        return Vec::new();
    }
    let mut targets = Vec::new();
    let mut offset = 0;
    for (index, line) in text.split('\n').enumerate() {
        if index > span.end {
            break;
        }
        if index > span.start && index < span.end {
            let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
            let rest = &line[indent..];
            if let Some(colon) = rest.find(':') {
                let key = rest[..colon].trim_end();
                let valid = !key.is_empty()
                    && !key.starts_with('@')
                    && key
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '-'));
                if valid {
                    targets.push(offset + indent + key.len());
                }
            }
        }
        offset += line.len() + 1;
    }
    targets
}

/// 合并结果：当前块已是首块返回 `None`，否则返回新文本。
pub fn merge_var_to_first(text: &str, span: BlockSpan) -> Option<String> {
    let first = first_var_span(text)?;
    if span.start <= first.start {
        return None;
    }
    let lines = split_lines(text);
    if span.end >= lines.len() {
        return None;
    }
    let inner = lines[span.start + 1..span.end].join("\n");
    let raw = inner.trim();
    let insert_pos = line_start_at(text, first.end);
    let delete = block_deletion_range(text, span);
    let mut edits = Vec::new();
    if !raw.is_empty() {
        edits.push((insert_pos..insert_pos, format!("{raw}\n")));
    }
    edits.push((delete.clone(), String::new()));
    Some(apply_edits(text, edits))
}

/// 置顶结果：当前块已在顶部连续块组返回 `None`，否则返回新文本。
pub fn move_var_to_top(text: &str, span: BlockSpan) -> Option<String> {
    let lines = split_lines(text);
    if span.end >= lines.len() {
        return None;
    }
    // 顶部连续变量块组末行（忽略开头空行）。
    let mut last_top_end: Option<usize> = None;
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim().is_empty() {
            index += 1;
            continue;
        }
        if parse_var_start_line(lines[index]).is_some() {
            let mut end = None;
            for (j, line) in lines.iter().enumerate().skip(index + 1) {
                if parse_var_end_line(line).is_some() {
                    end = Some(j);
                    break;
                }
            }
            if let Some(end) = end {
                last_top_end = Some(end);
                index = end + 1;
                continue;
            }
        }
        break;
    }
    if last_top_end.is_some_and(|end| span.start <= end) {
        return None;
    }
    let block_text = lines[span.start..=span.end].join("\n");
    let delete = block_deletion_range(text, span);
    let mut edits = Vec::new();
    match last_top_end {
        None => {
            let suffix = if text.is_empty() { "" } else { "\n\n" };
            edits.push((0..0, format!("{block_text}{suffix}")));
        }
        Some(end) => {
            let insert_pos = line_end_at(text, end);
            let has_empty_after = lines
                .get(end + 1)
                .is_some_and(|line| line.trim().is_empty());
            let suffix = if has_empty_after { "" } else { "\n" };
            edits.push((insert_pos..insert_pos, format!("\n\n{block_text}{suffix}")));
        }
    }
    edits.push((delete, String::new()));
    Some(apply_edits(text, edits))
}

/// 应用若干互不重叠的替换：按起点降序执行，避免偏移互相影响。
fn apply_edits(text: &str, mut edits: Vec<(std::ops::Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut result = text.to_string();
    for (range, insert) in edits {
        if range.start <= range.end && range.end <= result.len() {
            result.replace_range(range, &insert);
        }
    }
    result
}

/// `/addVarContent` 追加位置：首个闭合变量块的 `@content:` 列表末尾。
///
/// 返回（插入字节偏移, 插入文本）。无闭合变量块返回 None；块内无 `@content:` 时
/// 在起始行后新建保留键。插入文本以 `@` 结尾，调用方据此打开文件提及面板。
pub fn var_content_append(text: &str) -> Option<(usize, String)> {
    let span = block_spans(text)
        .into_iter()
        .find(|span| span.kind == BlockKind::Var && span.closed)?;
    let lines = split_lines(text);
    let mut offsets = Vec::with_capacity(lines.len());
    let mut offset = 0;
    for line in &lines {
        offsets.push(offset);
        offset += line.len() + 1;
    }
    let line_end = |index: usize| offsets[index] + lines[index].len();
    let indent_of = |line: &str| line.len() - line.trim_start_matches([' ', '\t']).len();

    let mut content_index = None;
    let mut in_triple = false;
    for (index, line) in lines.iter().enumerate().take(span.end).skip(span.start + 1) {
        let count = line.matches("\"\"\"").count();
        if in_triple {
            if count % 2 == 1 {
                in_triple = false;
            }
            continue;
        }
        if count % 2 == 1 {
            in_triple = true;
            continue;
        }
        if line.trim() == "@content:" {
            content_index = Some(index);
            break;
        }
    }

    match content_index {
        Some(content) => {
            let indent = &lines[content][..indent_of(lines[content])];
            let mut anchor = content;
            for (index, line) in lines.iter().enumerate().take(span.end).skip(content + 1) {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let is_item = trimmed.starts_with("- ")
                    || trimmed.starts_with("* ")
                    || trimmed.starts_with("+ ");
                if is_item {
                    anchor = index;
                } else {
                    break;
                }
            }
            Some((line_end(anchor), format!("\n{indent}  - @")))
        }
        None => {
            let indent = &lines[span.start][..indent_of(lines[span.start])];
            Some((
                line_end(span.start),
                format!("\n{indent}@content:\n{indent}  - @"),
            ))
        }
    }
}

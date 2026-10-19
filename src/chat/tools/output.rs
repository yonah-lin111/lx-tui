//! 结构化结果的三种呈现：模型文本、UI 摘要、DB 落库 JSON。
//!
//! 三者都在本文件生成，保证截断标志、`next_offset` 与部分成功语义只有一处口径。
//! 摘要绝不包含文件内容；落库 JSON 超限时截断正文并显式写 `truncated: true`。

use serde_json::{Value, json};

use super::MAX_TOOL_RESULT_BYTES;
use super::types::{
    FileChange, FileChangeKind, ReadKind, SearchMatch, ToolExecutionResult, ToolSummary,
};

/// 把结构化结果转成给模型的唯一文本表示。
///
/// 所有截断都显式呈现（`truncated` / `next_offset` / 行截断标记），不静默省略。
pub fn format_for_model(result: &ToolExecutionResult) -> String {
    match result {
        ToolExecutionResult::Read {
            path,
            kind,
            content,
            start_line,
            end_line,
            next_offset,
            truncated,
            total_lines,
        } => format_read(
            path,
            *kind,
            content,
            *start_line,
            *end_line,
            *next_offset,
            *truncated,
            *total_lines,
        ),
        ToolExecutionResult::Search {
            matches,
            truncated,
            files_scanned,
        } => format_search(matches, *truncated, *files_scanned),
        ToolExecutionResult::Edit { changes, partial } => format_edit(changes, *partial),
        ToolExecutionResult::Write {
            path,
            created,
            added,
            removed,
        } => {
            let action = if *created { "Created" } else { "Overwrote" };
            format!(
                "{action} {}\nadded {added} line(s), removed {removed} line(s)",
                path.display()
            )
        }
        ToolExecutionResult::Failed {
            code,
            message,
            changes,
        } => format_failure(*code, message, changes),
    }
}

/// `read` 的文本表示：首行给出范围与文件总行数，末行给出截断续读信息。
#[allow(clippy::too_many_arguments)]
fn format_read(
    path: &std::path::Path,
    kind: ReadKind,
    content: &str,
    start_line: usize,
    end_line: usize,
    next_offset: Option<usize>,
    truncated: bool,
    total_lines: usize,
) -> String {
    let mut text = String::new();
    match kind {
        ReadKind::Directory => {
            text.push_str(&format!(
                "Listed {} entries in directory {}\n",
                total_lines,
                path.display()
            ));
            if truncated {
                text.push_str("truncated: true (only the first entries are shown)\n");
            }
        }
        ReadKind::File => {
            if start_line == 0 {
                text.push_str(&format!("Read {} (empty file)\n", path.display()));
                return text;
            }
            text.push_str(&format!(
                "Read {} lines {}-{} of {} total lines\n",
                path.display(),
                start_line,
                end_line,
                total_lines
            ));
            if truncated {
                text.push_str("truncated: true (long lines and/or output size were cut)\n");
            }
        }
    }
    text.push_str("---\n");
    text.push_str(content);
    if !content.ends_with('\n') && !content.is_empty() {
        text.push('\n');
    }
    if let Some(next) = next_offset {
        text.push_str(&format!(
            "next_offset: {next} (call read again with this offset to continue)\n"
        ));
    }
    text
}

/// `search` 的文本表示：每行 `path:line: text`，行截断在末尾显式标注。
fn format_search(matches: &[SearchMatch], truncated: bool, files_scanned: usize) -> String {
    let mut text = String::new();
    text.push_str(&format!(
        "Found {} match(es) in {files_scanned} file(s)\n",
        matches.len()
    ));
    if truncated {
        text.push_str("truncated: true (result/limit/time or file cap reached)\n");
    }
    for hit in matches {
        text.push_str(&format!(
            "{}:{}: {}\n",
            hit.path.display(),
            hit.line,
            hit.text
        ));
    }
    if matches.is_empty() && !truncated {
        text.push_str("no matches\n");
    }
    text
}

/// `edit` 的文本表示：逐文件列出动作与增删行数。
fn format_edit(changes: &[FileChange], partial: bool) -> String {
    let mut text = String::new();
    text.push_str(&format!("Applied patch to {} file(s)\n", changes.len()));
    if partial {
        text.push_str("partial: true (some hunks produced no content change)\n");
    }
    for change in changes {
        text.push_str(&format!("{}\n", describe_change(change)));
    }
    text
}

/// 失败表示：错误码 + 说明 +（多文件补丁时）已提交变更。
fn format_failure(
    code: super::types::ToolErrorCode,
    message: &str,
    changes: &[FileChange],
) -> String {
    let mut text = format!("Tool failed [{}]: {message}\n", code.as_str());
    if changes.is_empty() {
        text.push_str("no files were changed\n");
        return text;
    }
    text.push_str(&format!(
        "partially applied: {} file(s) were already changed on disk (this is not a rollback)\n",
        changes.len()
    ));
    for change in changes {
        text.push_str(&format!("{}\n", describe_change(change)));
    }
    text
}

/// 单个变更的单行描述（供模型与失败结果复用）。
fn describe_change(change: &FileChange) -> String {
    match &change.kind {
        FileChangeKind::Added => format!(
            "added {} (+{} -{})",
            change.path.display(),
            change.added,
            change.removed
        ),
        FileChangeKind::Updated => format!(
            "updated {} (+{} -{})",
            change.path.display(),
            change.added,
            change.removed
        ),
        FileChangeKind::Deleted => {
            format!("deleted {} (-{})", change.path.display(), change.removed)
        }
        FileChangeKind::Moved { to } => format!(
            "moved {} -> {} (+{} -{})",
            change.path.display(),
            to.display(),
            change.added,
            change.removed
        ),
    }
}

/// 结构化结果的 UI 摘要：只有路径/范围/计数，**不含**内容。
pub fn to_summary(result: &ToolExecutionResult) -> ToolSummary {
    match result {
        ToolExecutionResult::Read {
            path,
            kind,
            start_line,
            end_line,
            truncated,
            total_lines,
            ..
        } => {
            // 文件读取才有闭区间与「实际读到的行数」；目录读取的 start/end 是条目序号。
            let file_range =
                (*kind == ReadKind::File && *start_line > 0).then_some((*start_line, *end_line));
            ToolSummary {
                path: Some(path.display().to_string()),
                range: file_range,
                read_lines: file_range
                    .map(|(start, end)| end.saturating_sub(start).saturating_add(1)),
                matches: (*kind == ReadKind::Directory).then_some(*total_lines),
                total_lines: Some(*total_lines),
                truncated: *truncated,
                ..ToolSummary::default()
            }
        }
        ToolExecutionResult::Search {
            matches, truncated, ..
        } => ToolSummary {
            matches: Some(matches.len()),
            truncated: *truncated,
            ..ToolSummary::default()
        },
        ToolExecutionResult::Edit { changes, partial } => {
            let path = single_path(changes);
            let (added, removed) = sum_counts(changes);
            ToolSummary {
                path,
                added: Some(added),
                removed: Some(removed),
                partial: *partial,
                ..ToolSummary::default()
            }
        }
        ToolExecutionResult::Write {
            path,
            added,
            removed,
            ..
        } => ToolSummary {
            path: Some(path.display().to_string()),
            added: Some(*added),
            removed: Some(*removed),
            ..ToolSummary::default()
        },
        ToolExecutionResult::Failed {
            code: _code,
            message: _message,
            changes,
        } => {
            let path = single_path(changes);
            let (added, removed) = sum_counts(changes);
            ToolSummary {
                path,
                added: (!changes.is_empty()).then_some(added),
                removed: (!changes.is_empty()).then_some(removed),
                ..ToolSummary::default()
            }
        }
    }
}

/// 变更列表里唯一涉及的文件路径（多文件补丁时为 None，交由 UI 显示计数）。
fn single_path(changes: &[FileChange]) -> Option<String> {
    match changes {
        [single] => Some(single.path.display().to_string()),
        _ => None,
    }
}

/// 累计增删行数。
fn sum_counts(changes: &[FileChange]) -> (usize, usize) {
    changes
        .iter()
        .fold((0usize, 0usize), |(added, removed), change| {
            (
                added.saturating_add(change.added),
                removed.saturating_add(change.removed),
            )
        })
}

/// 规范化结果 JSON（供 DB `result_json` 落库）。
///
/// 超 [`MAX_TOOL_RESULT_BYTES`] 时把正文替换为截断内容并显式置 `truncated: true`。
/// 失败结果也返回 JSON（含错误码与已提交变更），便于回放与 UI 摘要。
pub fn to_result_json(result: &ToolExecutionResult) -> Option<String> {
    let value = match result {
        ToolExecutionResult::Read {
            path,
            kind,
            content,
            start_line,
            end_line,
            next_offset,
            truncated,
            total_lines,
        } => json!({
            "tool": "read",
            "path": path.display().to_string(),
            "kind": match kind {
                ReadKind::File => "file",
                ReadKind::Directory => "directory",
            },
            "content": content,
            "start_line": start_line,
            "end_line": end_line,
            "next_offset": next_offset,
            "truncated": truncated,
            "total_lines": total_lines,
        }),
        ToolExecutionResult::Search {
            matches,
            truncated,
            files_scanned,
        } => json!({
            "tool": "search",
            "matches": matches
                .iter()
                .map(|hit| json!({
                    "path": hit.path.display().to_string(),
                    "line": hit.line,
                    "text": hit.text,
                    "line_truncated": hit.line_truncated,
                }))
                .collect::<Vec<_>>(),
            "truncated": truncated,
            "files_scanned": files_scanned,
        }),
        ToolExecutionResult::Edit { changes, partial } => json!({
            "tool": "edit",
            "changes": changes.iter().map(change_json).collect::<Vec<_>>(),
            "partial": partial,
        }),
        ToolExecutionResult::Write {
            path,
            created,
            added,
            removed,
        } => json!({
            "tool": "write",
            "path": path.display().to_string(),
            "created": created,
            "added": added,
            "removed": removed,
        }),
        ToolExecutionResult::Failed {
            code,
            message,
            changes,
        } => json!({
            "tool": "error",
            "code": code.as_str(),
            "message": message,
            "changes": changes.iter().map(change_json).collect::<Vec<_>>(),
        }),
    };
    let text = serde_json::to_string(&value).ok()?;
    if text.len() <= MAX_TOOL_RESULT_BYTES {
        return Some(text);
    }
    shrink_json_text(value)
}

/// 单个变更的 JSON 表示。
fn change_json(change: &FileChange) -> Value {
    let mut object = serde_json::Map::new();
    object.insert(
        "path".into(),
        Value::String(change.path.display().to_string()),
    );
    object.insert("added".into(), json!(change.added));
    object.insert("removed".into(), json!(change.removed));
    object.insert(
        "kind".into(),
        match &change.kind {
            FileChangeKind::Added => Value::String("added".into()),
            FileChangeKind::Updated => Value::String("updated".into()),
            FileChangeKind::Deleted => Value::String("deleted".into()),
            FileChangeKind::Moved { .. } => Value::String("moved".into()),
        },
    );
    if let FileChangeKind::Moved { to } = &change.kind {
        object.insert("to".into(), Value::String(to.display().to_string()));
    }
    Value::Object(object)
}

/// 结果 JSON 超限：把 `content` / `text` 段落截断并显式标记 `truncated: true`。
///
/// 只截断承载正文的字段；结构、路径与计数保留，保证回放仍可判断发生了什么。
fn shrink_json_text(mut value: Value) -> Option<String> {
    let budget = MAX_TOOL_RESULT_BYTES / 2;
    if let Value::Object(object) = &mut value {
        if let Some(Value::String(content)) = object.get_mut("content") {
            *content = truncate_chars(content, budget);
        }
        if let Some(Value::Array(matches)) = object.get_mut("matches") {
            let mut used = 0usize;
            let mut kept: Vec<Value> = Vec::new();
            for entry in matches.iter() {
                let size = serde_json::to_string(entry)
                    .map(|text| text.len())
                    .unwrap_or(0);
                if used + size > budget {
                    break;
                }
                used += size;
                kept.push(entry.clone());
            }
            *matches = kept;
        }
        object.insert("truncated".into(), Value::Bool(true));
    }
    let text = serde_json::to_string(&value).ok()?;
    if text.len() <= MAX_TOOL_RESULT_BYTES {
        Some(text)
    } else {
        // 极端情况（大量命中条目）：退化为只留结构与标志的 JSON。
        serde_json::to_string(&json!({
            "truncated": true,
            "note": "result exceeded the storage limit and was reduced to metadata",
        }))
        .ok()
    }
}

/// 按字符截断到 `budget` 字节以内（不切断 UTF-8）。
fn truncate_chars(text: &str, budget: usize) -> String {
    if text.len() <= budget {
        return text.to_string();
    }
    let mut end = budget.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str("\n…[truncated]");
    truncated
}

#[cfg(test)]
mod tests;

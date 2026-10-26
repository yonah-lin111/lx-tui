//! 结构化结果的三种呈现：模型文本、UI 摘要、DB 落库 JSON。
//!
//! 三者都在本文件生成，保证截断标志、`next_offset` 与部分成功语义只有一处口径。
//! 摘要绝不包含文件内容；落库 JSON 超限时截断正文并显式写 `truncated: true`。

use serde_json::{Value, json};

use super::MAX_TOOL_RESULT_BYTES;
use super::diff::FileDiff;
use super::types::{
    FileChange, FileChangeKind, ReadKind, SearchMatch, ToolExecutionResult, ToolSummary,
};

mod file;

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
            version,
        } => format_read(
            path,
            *kind,
            content,
            *start_line,
            *end_line,
            *next_offset,
            *truncated,
            *total_lines,
            version.as_deref(),
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
            ..
        } => {
            let action = if *created { "Created" } else { "Overwrote" };
            format!(
                "{action} {}\nadded {added} line(s), removed {removed} line(s)",
                path.display()
            )
        }
        ToolExecutionResult::Process(result) => format_process(result),
        ToolExecutionResult::Plan(snapshot) => format_plan(snapshot),
        ToolExecutionResult::Ask { questions } => format_ask(questions),
        ToolExecutionResult::Subagent(outcome) => format_subagent(outcome),
        ToolExecutionResult::Failed {
            code,
            message,
            changes,
        } => format_failure(*code, message, changes),
    }
}

/// 进程工具结果：状态 / 退出码 / 输出区间 / 输出正文。
///
/// 非零退出、超时与中断都写成明确的失败行，绝不让模型把它读成「命令成功」。
fn format_process(result: &crate::chat::process::ProcessResult) -> String {
    use crate::chat::process::ProcessState;
    let mut text = String::new();
    match &result.state {
        ProcessState::Exited { code } => {
            text.push_str(&format!(
                "{} finished with exit code {}\n",
                result.command,
                code.map_or_else(|| "unknown".to_string(), |code| code.to_string())
            ));
        }
        ProcessState::Running | ProcessState::Starting => {
            text.push_str(&format!(
                "{} is still running; session_id={} (use write_stdin to read more output or send input)\n",
                result.command, result.session_id
            ));
        }
        ProcessState::Stopped => {
            text.push_str(&format!("{} was stopped\n", result.command));
        }
        ProcessState::TimedOut => {
            text.push_str(&format!("{} timed out and was stopped\n", result.command));
        }
        ProcessState::Interrupted => {
            text.push_str(&format!(
                "{} was interrupted; its final state is unknown\n",
                result.command
            ));
        }
        ProcessState::Failed { code } => {
            text.push_str(&format!("{} failed to run: {code}\n", result.command));
        }
    }
    text.push_str(&format!(
        "session_id: {}\nprocess_state: {}\n",
        result.session_id,
        result.state.as_str()
    ));
    // 与 codex 的响应头同口径：分块标识 + 实际墙钟时间（秒），模型据此判断
    // 「这次是不是刚好卡在等待窗口边界」，以及把本次输出与后续轮次区分开。
    text.push_str(&format!("chunk_id: {}\n", result.chunk_id));
    text.push_str(&format!(
        "wall_time_seconds: {:.4}\n",
        result.wall_time_ms as f64 / 1_000.0
    ));
    if let Some(code) = result.exit_code {
        text.push_str(&format!("exit_code: {code}\n"));
    }
    text.push_str(&format!(
        "output: bytes [{}, {}){}\n",
        result.output_start,
        result.output_end,
        if result.truncated {
            " · more output pending"
        } else {
            ""
        }
    ));
    if result.dropped_bytes > 0 {
        text.push_str(&format!(
            "dropped_bytes: {} (earlier output was discarded before it could be read)\n",
            result.dropped_bytes
        ));
    }
    if let Some(input) = &result.input {
        text.push_str(&format!("input: {input}\n"));
    }
    // 可疑的后台任务：把「shell 返回 0 ≠ 后台任务已启动」这件事说给模型，
    // 否则它会直接向用户汇报一个并不存在的后台任务。
    if let Some(warning) = result.background_job_warning() {
        text.push_str(&format!("warning: {warning}\n"));
    }
    if result.output.is_empty() {
        text.push_str("no output\n");
    } else {
        text.push_str("---\n");
        text.push_str(&result.output);
        if !result.output.ends_with('\n') {
            text.push('\n');
        }
    }
    text
}

/// 计划结果：已提交的完整快照（模型据此确认替换生效）。
fn format_plan(snapshot: &crate::chat::plan::PlanSnapshot) -> String {
    if snapshot.is_empty() {
        return format!(
            "Plan updated (revision {}): the plan is now empty\n",
            snapshot.revision
        );
    }
    let mut text = format!(
        "Plan updated (revision {}): {}/{} completed\n",
        snapshot.revision,
        snapshot.completed(),
        snapshot.steps.len()
    );
    for step in &snapshot.steps {
        let marker = match step.status {
            crate::chat::plan::PlanStepStatus::Completed => "[x]",
            crate::chat::plan::PlanStepStatus::InProgress => "[>]",
            crate::chat::plan::PlanStepStatus::Pending => "[ ]",
        };
        text.push_str(&format!("{marker} {}\n", step.step));
    }
    if let Some(explanation) = &snapshot.explanation {
        text.push_str(&format!("note: {explanation}\n"));
    }
    text
}

/// 子 agent 控制工具的结果文本（模型可见，英文）。
///
/// 报告正文只在本次领取到时出现一次；`wait_agent` 超时不是错误。
fn format_subagent(outcome: &crate::chat::agent::SubagentOutcome) -> String {
    use crate::chat::agent::SubagentOutcome;
    match outcome {
        SubagentOutcome::Spawned { agent_id, role } => format!(
            "Spawned subagent `{agent_id}` ({}). It runs in the background; use `wait_agent` \
             when you need its result.\n",
            role.as_str()
        ),
        SubagentOutcome::Waited { timed_out, agents } => {
            let mut text = format!("timed_out: {timed_out}\n");
            for agent in agents {
                text.push_str(&format!(
                    "- agent_id={} role={} status={}\n",
                    agent.agent_id,
                    agent.role.as_str(),
                    agent.status
                ));
                if let Some(report) = &agent.report {
                    text.push_str("report:\n");
                    text.push_str(report);
                    if !report.ends_with('\n') {
                        text.push('\n');
                    }
                }
            }
            text
        }
        SubagentOutcome::Closed {
            agent_id,
            previous_status,
        } => format!(
            "Closed subagent `{agent_id}` (was {previous_status}). It produced no report.\n"
        ),
    }
}

/// `ask` 的文本表示：与 codex 同构的 `{"answers":{"<id>":{"answers":[...]}}}`。
///
/// 只回答案、不回题干：题干在模型的调用参数里，答案才是这次调用的新鲜信息。
fn format_ask(questions: &[crate::chat::tools::AnsweredQuestion]) -> String {
    let answers = questions
        .iter()
        .map(|question| (question.id.clone(), json!({ "answers": question.answers })))
        .collect::<serde_json::Map<_, _>>();
    json!({ "answers": answers }).to_string()
}

/// `read` 的文本表示：首行给出范围与文件总行数，末行给出截断续读信息。
///
/// 文件读取额外给出 `file_version`：修改该文件时必须把它作为 `read_version`
/// 参数交给 `edit`/`write`，否则写入会被拒绝。
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
    version: Option<&str>,
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
                if let Some(version) = version {
                    text.push_str(&format!("file_version: {version}\n"));
                }
                return text;
            }
            text.push_str(&format!(
                "Read {} lines {}-{} of {} total lines\n",
                path.display(),
                start_line,
                end_line,
                total_lines
            ));
            if let Some(version) = version {
                text.push_str(&format!(
                    "file_version: {version} (pass this as read_version when editing this file)\n"
                ));
            }
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
            let counts_known = changes
                .iter()
                .all(|change| change.diff.as_ref().is_none_or(FileDiff::counts_known));
            ToolSummary {
                path,
                added: counts_known.then_some(added),
                removed: counts_known.then_some(removed),
                counts_known,
                partial: *partial,
                ..ToolSummary::default()
            }
        }
        ToolExecutionResult::Write {
            path,
            added,
            removed,
            diff,
            ..
        } => {
            let counts_known = diff.as_ref().is_none_or(FileDiff::counts_known);
            ToolSummary {
                path: Some(path.display().to_string()),
                added: counts_known.then_some(*added),
                removed: counts_known.then_some(*removed),
                counts_known,
                ..ToolSummary::default()
            }
        }
        ToolExecutionResult::Failed {
            code: _code,
            message: _message,
            changes,
        } => {
            let path = single_path(changes);
            let (added, removed) = sum_counts(changes);
            let counts_known = changes
                .iter()
                .all(|change| change.diff.as_ref().is_none_or(FileDiff::counts_known));
            ToolSummary {
                path,
                added: (counts_known && !changes.is_empty()).then_some(added),
                removed: (counts_known && !changes.is_empty()).then_some(removed),
                ..ToolSummary::default()
            }
        }
        // 进程工具与计划工具的展示数据走各自的类型化视图（`ProcessView` / `PlanSnapshotView`），
        // 不塞进文件摘要里：摘要只描述“哪个文件、多少行”，进程/计划没有这个形状。
        ToolExecutionResult::Process(_)
        | ToolExecutionResult::Plan(_)
        | ToolExecutionResult::Ask { .. }
        | ToolExecutionResult::Subagent(_) => ToolSummary::default(),
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
            version,
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
            "file_version": version,
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
        ToolExecutionResult::Edit { changes, partial } => file::edit_json(changes, *partial),
        ToolExecutionResult::Write {
            path,
            created,
            added,
            removed,
            diff,
        } => file::write_json(path, *created, *added, *removed, diff.as_ref()),
        ToolExecutionResult::Failed {
            code,
            message,
            changes,
        } => file::failure_json(*code, message, changes),
        ToolExecutionResult::Process(result) => result.to_json(),
        ToolExecutionResult::Plan(snapshot) => snapshot.to_json(),
        ToolExecutionResult::Subagent(outcome) => outcome.to_json(),
        ToolExecutionResult::Ask { questions } => json!({
            "tool": "ask",
            "questions": questions
                .iter()
                .map(|question| json!({
                    "id": question.id,
                    "header": question.header,
                    "question": question.question,
                    "answers": question.answers,
                }))
                .collect::<Vec<_>>(),
        }),
    };
    let text = serde_json::to_string(&value).ok()?;
    if text.len() <= MAX_TOOL_RESULT_BYTES {
        return Some(text);
    }
    shrink_json_text(value)
}

/// 结果 JSON 超限：把 `content` / `text` 段落截断并显式标记 `truncated: true`。
///
/// 只截断承载正文的字段；结构、路径与计数保留，保证回放仍可判断发生了什么。
fn shrink_json_text(mut value: Value) -> Option<String> {
    // 文件工具：先按最终 JSON 字节裁差异正文与文件摘要前缀（含 omitted_files），
    // 不能因为 UI 差异过大把原本成功的文件操作变成落库失败。
    if value
        .get("tool")
        .and_then(Value::as_str)
        .is_some_and(|tool| {
            matches!(tool, "edit" | "write" | "error") && value.get("changes").is_some()
                || tool == "write"
        })
    {
        return file::shrink(value, MAX_TOOL_RESULT_BYTES);
    }
    let budget = MAX_TOOL_RESULT_BYTES / 2;
    if let Value::Object(object) = &mut value {
        if let Some(Value::String(content)) = object.get_mut("content") {
            *content = truncate_chars(content, budget);
        }
        // 进程结果：正文在 `output` 字段，同样只截断正文、保留区间与状态。
        if let Some(Value::String(output)) = object.get_mut("output") {
            *output = truncate_chars(output, budget);
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

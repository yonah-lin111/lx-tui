//! 文件工具（`edit` / `write` / 失败带已提交变更）的结果 JSON：差异正文、调用总计与预算裁切。
//!
//! 预算规则（见 `docs/feature/lx-file-tool-diff/design.md` §4.3、§7.1）：
//!
//! 1. 先按最终 JSON 的 UTF-8 字节判断，计入转义、行结构与路径开销；
//! 2. 超限时**从尾部移除完整差异行/hunk**，并把被裁文件的 `status` 置为 `truncated`；
//! 3. 正文裁光仍超限（极端：文件元数据本身放不下）时，保留可容纳的文件摘要前缀并记录
//!    `omitted_files`、`files` 与完整总计——绝不缩成只剩一句 `note`；
//! 4. 计数始终来自完整编辑脚本：裁正文不改 `file_totals`，未知计数写 `null` 而不是 0。

use serde_json::{Value, json};

use super::super::diff::{DiffHunk, DiffLine, DiffLineKind, FileDiff};
use super::super::types::{FileChange, FileChangeKind, ToolErrorCode};

/// 单次调用保留的差异正文上限（512 KiB）。
pub const MAX_DIFF_JSON_BYTES: usize = 512 * 1024;

/// 单次调用保留的差异行对象上限。
pub const MAX_DIFF_JSON_LINES: usize = 10_000;

/// `edit` 成功结果。
pub(super) fn edit_json(changes: &[FileChange], partial: bool) -> Value {
    json!({
        "tool": "edit",
        "changes": changes_json(changes),
        "partial": partial,
        "file_totals": totals_json(changes),
    })
}

/// `write` 成功结果。
pub(super) fn write_json(
    path: &std::path::Path,
    created: bool,
    added: usize,
    removed: usize,
    diff: Option<&FileDiff>,
) -> Value {
    let counts_known = diff.is_none_or(FileDiff::counts_known);
    let mut object = json!({
        "tool": "write",
        "path": path.display().to_string(),
        "created": created,
        "added": added,
        "removed": removed,
        "counts_known": counts_known,
        "simplified": diff.is_some_and(FileDiff::simplified),
        "file_totals": {
            "files": 1,
            "added": if counts_known { json!(added) } else { Value::Null },
            "removed": if counts_known { json!(removed) } else { Value::Null },
            "simplified": diff.is_some_and(FileDiff::simplified),
            "omitted_files": 0,
        },
    });
    if let Some(diff) = diff
        && let (Some(object), Some(entry)) = (object.as_object_mut(), diff_json(diff))
    {
        object.insert("diff".into(), entry);
    }
    object
}

/// 失败结果（含已提交变更）。
///
/// 没有任何已提交文件时不写 `file_totals`：空变更列表不是「0 个文件的完整总计」，
/// 摘要必须保持"无计数"而不是显示 +0 -0。
pub(super) fn failure_json(code: ToolErrorCode, message: &str, changes: &[FileChange]) -> Value {
    let mut object = json!({
        "tool": "error",
        "code": code.as_str(),
        "message": message,
        "changes": changes_json(changes),
    });
    if !changes.is_empty()
        && let Some(map) = object.as_object_mut()
    {
        map.insert("file_totals".into(), totals_json(changes));
    }
    object
}

/// 全部变更的 JSON（按提交顺序，便于对应部分失败）。
fn changes_json(changes: &[FileChange]) -> Vec<Value> {
    let mut budget = DiffBudget::new();
    changes
        .iter()
        .map(|change| change_json(change, &mut budget))
        .collect()
}

/// 单个变更的 JSON；`diff` 只在有正文/状态可表达时出现。
fn change_json(change: &FileChange, budget: &mut DiffBudget) -> Value {
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
    let counts_known = change.diff.as_ref().is_none_or(FileDiff::counts_known);
    object.insert("counts_known".into(), Value::Bool(counts_known));
    object.insert(
        "simplified".into(),
        Value::Bool(change.diff.as_ref().is_some_and(FileDiff::simplified)),
    );
    if let Some(diff) = &change.diff
        && let Some(entry) = diff_json_budgeted(diff, budget)
    {
        object.insert("diff".into(), entry);
    }
    Value::Object(object)
}

/// 调用总计：在**裁切之前**按完整结果计算。
fn totals_json(changes: &[FileChange]) -> Value {
    let mut added = 0usize;
    let mut removed = 0usize;
    let mut known = true;
    let mut simplified = false;
    for change in changes {
        match &change.diff {
            Some(diff) if !diff.counts_known() => known = false,
            _ => {
                added = added.saturating_add(change.added);
                removed = removed.saturating_add(change.removed);
            }
        }
        simplified |= change.diff.as_ref().is_some_and(FileDiff::simplified);
    }
    json!({
        "files": changes.len(),
        "added": if known { json!(added) } else { Value::Null },
        "removed": if known { json!(removed) } else { Value::Null },
        "simplified": simplified,
        "omitted_files": 0,
    })
}

/// 单次调用内跨文件共享的差异正文预算。
struct DiffBudget {
    bytes: usize,
    lines: usize,
}

impl DiffBudget {
    fn new() -> Self {
        Self {
            bytes: MAX_DIFF_JSON_BYTES,
            lines: MAX_DIFF_JSON_LINES,
        }
    }

    /// 扣减：成功返回 true。
    fn take(&mut self, bytes: usize, lines: usize) -> bool {
        if lines > self.lines || bytes > self.bytes {
            return false;
        }
        self.bytes -= bytes;
        self.lines -= lines;
        true
    }
}

/// 未做跨文件预算的差异 JSON（`write` 单文件路径）。
fn diff_json(diff: &FileDiff) -> Option<Value> {
    diff_json_budgeted(diff, &mut DiffBudget::new())
}

/// 差异 JSON：超预算时从尾部裁掉完整行，并把状态置为 `truncated`。
fn diff_json_budgeted(diff: &FileDiff, budget: &mut DiffBudget) -> Option<Value> {
    match diff {
        FileDiff::Unavailable { reason } => Some(json!({
            "version": 1,
            "status": "unavailable",
            "reason": reason.as_str(),
            "simplified": false,
            "hunks": [],
        })),
        FileDiff::Complete { hunks, simplified } | FileDiff::Truncated { hunks, simplified } => {
            let (kept, truncated) = fit_hunks(hunks, budget);
            let status = if truncated || diff.truncated() {
                "truncated"
            } else {
                "complete"
            };
            Some(json!({
                "version": 1,
                "status": status,
                "simplified": simplified,
                "hunks": kept,
            }))
        }
    }
}

/// 按预算裁剪 hunks（从尾部丢完整行）；返回保留正文与是否截断。
fn fit_hunks(hunks: &[DiffHunk], budget: &mut DiffBudget) -> (Vec<Value>, bool) {
    let mut kept = Vec::with_capacity(hunks.len());
    let mut truncated = false;
    for hunk in hunks {
        if truncated {
            break;
        }
        let mut lines = Vec::new();
        for line in &hunk.lines {
            let value = line_json(line);
            let size = serde_json::to_string(&value).map_or(0, |text| text.len());
            if !budget.take(size, 1) {
                truncated = true;
                break;
            }
            lines.push(value);
        }
        if !lines.is_empty() {
            kept.push(json!({ "lines": lines }));
        }
    }
    (kept, truncated)
}

/// 差异行 JSON。
fn line_json(line: &DiffLine) -> Value {
    json!({
        "kind": match line.kind {
            DiffLineKind::Context => "context",
            DiffLineKind::Added => "added",
            DiffLineKind::Removed => "removed",
        },
        "old_line": line.old_line,
        "new_line": line.new_line,
        "text": line.text,
        "no_final_newline": line.no_final_newline,
    })
}

/// 最终兜底：按最终 JSON 字节裁剪差异正文，必要时省略文件摘要前缀。
///
/// 只在 [`super::to_result_json`] 判断文本超限后调用；返回的 JSON 保证 ≤ `max_bytes`
/// （除非单个文件路径本身都放不下，此时仍保留总计与 `omitted_files`）。
pub(super) fn shrink(mut value: Value, max_bytes: usize) -> Option<String> {
    // 1) 从尾部丢差异行（每轮丢一成），保留文件摘要与总计。
    loop {
        let text = serde_json::to_string(&value).ok()?;
        if text.len() <= max_bytes || !drop_last_diff_line(&mut value) {
            break;
        }
    }
    // 2) 差异已裁光仍超限：从尾部省略文件摘要，记录 omitted_files 与总文件数。
    let total_files = file_count(&value);
    loop {
        let text = serde_json::to_string(&value).ok()?;
        if text.len() <= max_bytes {
            return Some(text);
        }
        if !drop_last_change(&mut value) {
            break;
        }
        let omitted = total_files.saturating_sub(file_count(&value));
        mark_omitted(&mut value, omitted);
    }
    let text = serde_json::to_string(&value).ok()?;
    (text.len() <= max_bytes).then_some(text)
}

/// 从 `changes` 末尾丢一个差异行；返回是否发生了删除。
fn drop_last_diff_line(value: &mut Value) -> bool {
    let Some(changes) = value.get_mut("changes").and_then(Value::as_array_mut) else {
        // `write`：diff 位于顶层。
        let Some(diff) = value.get_mut("diff") else {
            return false;
        };
        return drop_last_hunk_line(diff);
    };
    for change in changes.iter_mut().rev() {
        if let Some(diff) = change.get_mut("diff")
            && drop_last_hunk_line(diff)
        {
            mutate_diff_status(diff, "truncated");
            return true;
        }
    }
    false
}

/// 从单个 `diff` 的最后一个 hunk 末尾删一行；返回是否删到。
fn drop_last_hunk_line(diff: &mut Value) -> bool {
    let Some(hunks) = diff.get_mut("hunks").and_then(Value::as_array_mut) else {
        return false;
    };
    // 一次删掉尾部 1/8（至少 1 行）：避免 10k 行逐行重序列化的平方级开销。
    let total: usize = hunks
        .iter()
        .filter_map(|hunk| hunk.get("lines").and_then(Value::as_array))
        .map(Vec::len)
        .sum();
    if total == 0 {
        return false;
    }
    let mut drop = (total / 8).max(1);
    let mut dropped = false;
    while drop > 0 && !hunks.is_empty() {
        let last = hunks.len() - 1;
        let Some(lines) = hunks[last].get_mut("lines").and_then(Value::as_array_mut) else {
            hunks.pop();
            continue;
        };
        let take = drop.min(lines.len());
        let new_len = lines.len() - take;
        lines.truncate(new_len);
        drop -= take;
        dropped = true;
        if lines.is_empty() {
            hunks.pop();
        }
    }
    dropped
}

/// 强制把 `diff.status` 置为截断。
fn mutate_diff_status(diff: &mut Value, status: &str) {
    if let Some(object) = diff.as_object_mut() {
        object.insert("status".into(), Value::String(status.into()));
    }
}

/// 从 `changes` 末尾省略一个文件摘要；返回是否发生了删除。
fn drop_last_change(value: &mut Value) -> bool {
    let Some(changes) = value.get_mut("changes").and_then(Value::as_array_mut) else {
        return false;
    };
    changes.pop().is_some()
}

/// 结果中当前保留的文件数。
fn file_count(value: &Value) -> usize {
    match value.get("changes").and_then(Value::as_array) {
        Some(changes) => changes.len(),
        // `write` 的单个文件只要还有 `path` 就算保留。
        None => usize::from(value.get("path").is_some()),
    }
}

/// 记录省略文件数（总计里的 `omitted_files`）。
fn mark_omitted(value: &mut Value, omitted: usize) {
    if let Some(totals) = value.get_mut("file_totals").and_then(Value::as_object_mut) {
        totals.insert("omitted_files".into(), json!(omitted));
    }
}

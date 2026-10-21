//! `edit` 工具：执行 Codex 风格补丁（Add / Update / Delete / Move）。
//!
//! 两阶段执行：
//! 1. **解析与路径校验**（无写入）：解析补丁文本、解析全部目标路径，并做纯路径级检查
//!    （Add 目标不能已存在、Update/Delete 目标必须是已存在的普通文件、Move 目标既不能是
//!    目录也不能已存在）。补丁格式与路径问题都在此阶段返回，此时磁盘没有任何变化。
//! 2. **逐个提交**：按补丁顺序读取文件、匹配上下文、落盘。多文件补丁**非原子**，
//!    中途失败会返回失败原因与**已提交**的 [`FileChange`]，绝不谎报整体回滚。
//!
//! 上下文匹配要求唯一：每个块的定位行与增删行必须在文件（及前一块之后的位置）中
//! **恰好匹配一次**，否则 `PatchContextMismatch`；绝不猜测替换位置。匹配先按精确
//! 文本比较，失配后再按「两侧 `trim_end` 后相等」回退一档（模型产出的补丁常丢行尾空白）。
//!
//! 纯插入块（块内只有 `+` 行）的落点：带 `@@ <context>` 定位行时紧跟该定位行之后；
//! 只有裸 `@@` 或完全没有 `@@` 时追加到**文件末尾**（与 codex `file_update.rs` 一致）。

use std::path::{Path, PathBuf};

use super::args::Args;
use super::diff::{FileDiff, diff_lines};
use super::lock;
use super::patch::{self, Block, BodyLineKind, Hunk};
use super::path;
use super::text::{Newline, SourceText};
use super::types::{
    FileChange, FileChangeKind, ToolContext, ToolErrorCode, ToolExecutionResult, ToolFailure,
    ToolResult,
};

/// `edit` 只允许一个字段：完整补丁文本。
const ALLOWED: [&str; 1] = ["patch"];

/// 已解析路径的 hunk（路径校验完成，尚未读写内容）。
enum Resolved<'a> {
    Add {
        hunk: &'a Hunk,
        target: PathBuf,
    },
    Delete {
        target: PathBuf,
    },
    Update {
        hunk: &'a Hunk,
        target: PathBuf,
        move_to: Option<PathBuf>,
    },
}

/// 执行 `edit`。
pub(crate) fn run(ctx: &ToolContext, arguments_json: &str) -> ToolExecutionResult {
    let args = match Args::parse(arguments_json, &ALLOWED) {
        Ok(args) => args,
        Err(failure) => return failure.into_result(Vec::new()),
    };
    let patch_text = match args.required_text("patch") {
        Ok(text) => text,
        Err(failure) => return failure.into_result(Vec::new()),
    };
    let hunks = match patch::parse(patch_text) {
        Ok(hunks) => hunks,
        Err(failure) => return failure.into_result(Vec::new()),
    };
    let resolved = match resolve_paths(ctx, &hunks) {
        Ok(resolved) => resolved,
        Err(failure) => return failure.into_result(Vec::new()),
    };
    commit(&resolved)
}

/// 解析全部路径并做路径级检查（是否存在、是否普通文件、移动目标是否目录或已存在）。
fn resolve_paths<'a>(ctx: &ToolContext, hunks: &'a [Hunk]) -> ToolResult<Vec<Resolved<'a>>> {
    let mut resolved = Vec::with_capacity(hunks.len());
    for hunk in hunks {
        match hunk {
            Hunk::Add { path: raw, .. } => {
                let target = path::resolve(ctx, &raw.to_string_lossy())?;
                if target.exists() {
                    return Err(ToolFailure::new(
                        ToolErrorCode::PatchInvalid,
                        format!(
                            "'*** Add File: {}' failed: file already exists; use '*** Update File' instead",
                            path::display(&target)
                        ),
                    ));
                }
                resolved.push(Resolved::Add { hunk, target });
            }
            Hunk::Delete { path: raw } => {
                let target = path::resolve(ctx, &raw.to_string_lossy())?;
                require_regular_file(&target, "*** Delete File")?;
                resolved.push(Resolved::Delete { target });
            }
            Hunk::Update {
                path: raw, move_to, ..
            } => {
                let target = path::resolve(ctx, &raw.to_string_lossy())?;
                require_regular_file(&target, "*** Update File")?;
                let move_to = match move_to {
                    Some(raw) => {
                        let destination = path::resolve(ctx, &raw.to_string_lossy())?;
                        if destination.is_dir() {
                            return Err(ToolFailure::new(
                                ToolErrorCode::NotAFile,
                                format!(
                                    "cannot move {} to {}: destination is a directory",
                                    path::display(&target),
                                    path::display(&destination)
                                ),
                            ));
                        }
                        // 与 `*** Add File` 同口径：目标已存在就拒绝，避免静默覆盖。
                        // 消息只含路径，不含被覆盖内容。
                        if destination.exists() {
                            return Err(ToolFailure::new(
                                ToolErrorCode::PatchInvalid,
                                format!(
                                    "cannot move {} to {}: destination already exists; delete it first or choose another path",
                                    path::display(&target),
                                    path::display(&destination)
                                ),
                            ));
                        }
                        Some(destination)
                    }
                    None => None,
                };
                resolved.push(Resolved::Update {
                    hunk,
                    target,
                    move_to,
                });
            }
        }
    }
    Ok(resolved)
}

/// 目标必须是已存在的普通文件（符号链接按目标类型判断）。
fn require_regular_file(target: &Path, marker: &str) -> ToolResult<()> {
    let metadata = std::fs::metadata(target).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ToolFailure::new(
            ToolErrorCode::NotFound,
            format!(
                "'{marker}: {}' failed: file does not exist",
                path::display(target)
            ),
        ),
        _ => path::map_io_error(&error, "stat", target),
    })?;
    if !metadata.is_file() {
        return Err(ToolFailure::new(
            ToolErrorCode::NotAFile,
            format!(
                "'{marker}: {}' failed: not a regular file",
                path::display(target)
            ),
        ));
    }
    Ok(())
}

/// 提交阶段：逐个 hunk 落盘，累计已提交变更。
fn commit(resolved: &[Resolved<'_>]) -> ToolExecutionResult {
    let mut changes: Vec<FileChange> = Vec::new();
    let mut partial = false;
    for plan in resolved {
        match commit_one(plan) {
            Ok(Some(change)) => changes.push(change),
            // 无实际变更：不写盘，但如实标记 partial。
            Ok(None) => partial = true,
            Err(failure) => {
                // 源文件可能**已经写入**（随后目录/rename 失败）：先计入已提交变更，
                // 再报失败——否则 UI 会漏掉这次真实发生的写盘。
                if let Some(change) = failure.committed {
                    changes.push(*change);
                }
                return failure.cause.into_result(changes);
            }
        }
    }
    if changes.is_empty() {
        return ToolFailure::new(
            ToolErrorCode::NoChanges,
            "patch produced no changes; file content already matches the patch result",
        )
        .into_result(Vec::new());
    }
    ToolExecutionResult::Edit { changes, partial }
}

/// 提交阶段失败：普通错误，或「源文件已写入、后续移动失败」时附带的已提交变更。
struct CommitFailure {
    cause: ToolFailure,
    /// 已落盘但后续步骤失败的文件变更。装箱：`FileChange` 现在携带差异正文，
    /// 直接内联会让 `Result<_, CommitFailure>` 的错误变体过大（clippy）。
    committed: Option<Box<FileChange>>,
}

impl From<ToolFailure> for CommitFailure {
    fn from(cause: ToolFailure) -> Self {
        Self {
            cause,
            committed: None,
        }
    }
}

/// 落盘单个 hunk；无实际变更时返回 `Ok(None)`。
fn commit_one(plan: &Resolved<'_>) -> Result<Option<FileChange>, CommitFailure> {
    match plan {
        Resolved::Add { hunk, target } => {
            let Hunk::Add { lines, .. } = hunk else {
                return Err(internal_error("add hunk mismatch").into());
            };
            if lines.is_empty() {
                return Ok(None);
            }
            let source = SourceText {
                bom: false,
                newline: Newline::Lf,
                lines: Vec::new(),
                trailing_newline: true,
            };
            let bytes = source.encode(lines);
            let (diff, counts) = diff_lines(&[], lines, true, true, 0, bytes.len());
            write_bytes(target, &bytes)?;
            Ok(Some(FileChange {
                path: target.clone(),
                added: counts.added,
                removed: counts.removed,
                kind: FileChangeKind::Added,
                diff: Some(diff),
            }))
        }
        Resolved::Delete { target } => {
            // 旧内容不可读（二进制/非 UTF-8）不改变删除能力：仍删，差异标为不可用。
            let (diff, removed) = match read_source(target) {
                Ok((lines, bytes, _)) => {
                    let (diff, counts) = diff_lines(&lines, &[], true, true, bytes, 0);
                    (diff, counts.removed)
                }
                Err(failure) => (
                    FileDiff::Unavailable {
                        reason: unavailable_reason(&failure),
                    },
                    0,
                ),
            };
            lock::with_path_lock(target, || std::fs::remove_file(target))
                .map_err(|error| path::map_io_error(&error, "delete", target))?;
            Ok(Some(FileChange {
                path: target.clone(),
                added: 0,
                removed,
                kind: FileChangeKind::Deleted,
                diff: Some(diff),
            }))
        }
        Resolved::Update {
            hunk,
            target,
            move_to,
        } => {
            let Hunk::Update { blocks, .. } = hunk else {
                return Err(internal_error("update hunk mismatch").into());
            };
            // 一次读 + 一次解析：`before` 与写回编码共用同一份 `SourceText`。
            // （旧实现先 `read_text` 再 `std::fs::read` + `parse`：两次读之间有窗口，
            // 解析也做了两遍。）
            let bytes = std::fs::read(target)
                .map_err(|error| path::map_io_error(&error, "read", target))?;
            let source = SourceText::parse(&bytes)?;
            let before = source.lines.clone();
            let after = apply_blocks(&before, blocks, target)?;
            let encoded = source.encode(&after);
            // 差异与计数同源：都来自本次真实 before/after，不按输入 patch 统计。
            let (diff, counts) = diff_lines(
                &before,
                &after,
                source.trailing_newline,
                source.trailing_newline,
                bytes.len(),
                encoded.len(),
            );
            let same_content = before == after;
            match move_to {
                None => {
                    // 内容没有实际变化：不写盘，如实标记 partial。
                    if same_content {
                        return Ok(None);
                    }
                    write_bytes(target, &encoded)?;
                    Ok(Some(FileChange {
                        path: target.clone(),
                        added: counts.added,
                        removed: counts.removed,
                        kind: FileChangeKind::Updated,
                        diff: Some(diff),
                    }))
                }
                Some(destination) => {
                    // 纯移动不重写源文件；只有内容变化时才写。
                    if !same_content {
                        write_bytes(target, &encoded)?;
                    }
                    // 源文件内容已经更新（或本来就没变）：此后任何失败都必须带上这份
                    // 已提交的 Updated 变更，不能漏报。
                    let committed = FileChange {
                        path: target.clone(),
                        added: counts.added,
                        removed: counts.removed,
                        kind: FileChangeKind::Updated,
                        diff: Some(diff.clone()),
                    };
                    if let Some(parent) = destination.parent()
                        && !parent.as_os_str().is_empty()
                    {
                        std::fs::create_dir_all(parent).map_err(|error| CommitFailure {
                            committed: (!same_content).then(|| Box::new(committed.clone())),
                            cause: path::map_io_error(&error, "create directory", parent),
                        })?;
                    }
                    lock::with_path_lock(target, || std::fs::rename(target, destination))
                        .map_err(|error| CommitFailure {
                            committed: (!same_content).then(|| Box::new(committed.clone())),
                            cause: ToolFailure::new(
                                io_code(&error),
                                format!(
                                    "wrote {} but failed to move it to {}: {error}; the source file already has the new content",
                                    path::display(target),
                                    path::display(destination)
                                ),
                            ),
                        })?;
                    Ok(Some(FileChange {
                        path: target.clone(),
                        added: counts.added,
                        removed: counts.removed,
                        kind: FileChangeKind::Moved {
                            to: destination.clone(),
                        },
                        diff: Some(diff),
                    }))
                }
            }
        }
    }
}

/// 读取文本源：行、原始字节数、是否以换行结尾（删除前统计差异用）。
fn read_source(target: &Path) -> ToolResult<(Vec<String>, usize, bool)> {
    let bytes =
        std::fs::read(target).map_err(|error| path::map_io_error(&error, "read", target))?;
    let source = SourceText::parse(&bytes)?;
    Ok((source.lines, bytes.len(), source.trailing_newline))
}

/// 读取失败 → 差异不可用的稳定原因。
fn unavailable_reason(failure: &ToolFailure) -> crate::chat::tools::DiffUnavailableReason {
    match failure.code {
        ToolErrorCode::Binary => crate::chat::tools::DiffUnavailableReason::Binary,
        ToolErrorCode::NotUtf8 => crate::chat::tools::DiffUnavailableReason::NotUtf8,
        _ => crate::chat::tools::DiffUnavailableReason::ReadFailed,
    }
}

/// 应用全部 `@@` 块到行集合；返回更新后的行。
fn apply_blocks(lines: &[String], blocks: &[Block], target: &Path) -> ToolResult<Vec<String>> {
    let mut current: Vec<String> = lines.to_vec();
    let mut search_start = 0usize;
    for block in blocks {
        let position = locate_block(&current, block, search_start, target)?;
        let old_lines = block_old_lines(block);
        let new_lines = block_new_lines(block);
        let inserted = new_lines.len();
        // 追加到文件末尾的纯插入不移动搜索游标（与 codex 一致：空 `old_lines` 时保持
        // `line_index` 不变），否则排在它之后的块会从文件末尾之外开始搜索。
        let appended_tail = old_lines.is_empty() && position == current.len();
        current.splice(position..position + old_lines.len(), new_lines);
        if !appended_tail {
            search_start = position + inserted;
        }
    }
    Ok(current)
}

/// 定位一个块在文件中的唯一替换位置。
///
/// 规则按顺序生效：
///
/// 1. **`@@ <context>` 定位行**（若存在）先缩小范围：该行本身也必须在范围内唯一，
///    之后的匹配从它的下一行开始。
/// 2. **纯插入块**（块内只有 `+` 行，`old_lines` 为空）：带 `@@ <context>` 时紧跟
///    该定位行之后；只有裸 `@@` 或没有 `@@` 时**追加到文件末尾**（与 codex 一致）。
/// 3. **`*** End of File` 块**（[`Block::at_eof`]）：位置只能由「块的旧行贴住文件
///    末尾」唯一确定（`lines.len() - old_lines.len()`），不做搜索；该位置不匹配即
///    `PatchContextMismatch`。
/// 4. **普通块**：块的旧行序列（上下文 + 删除行）在范围内必须恰好出现一次。先按
///    精确文本匹配，失败后再按「两侧 `trim_end` 后相等」回退一档；两档都不唯一时
///    报 `PatchContextMismatch` 并列出候选行号。
fn locate_block(
    lines: &[String],
    block: &Block,
    search_start: usize,
    target: &Path,
) -> ToolResult<usize> {
    let mut start = search_start;
    if let Some(context) = &block.context {
        let needle = context.trim_end();
        let hits: Vec<usize> = (start..lines.len())
            .filter(|index| lines[*index].trim_end() == needle)
            .collect();
        match hits.len() {
            0 => {
                return Err(ToolFailure::new(
                    ToolErrorCode::PatchContextMismatch,
                    format!("context '{needle}' not found in {}", path::display(target)),
                ));
            }
            1 => start = hits[0] + 1,
            count => {
                return Err(ToolFailure::new(
                    ToolErrorCode::PatchContextMismatch,
                    format!(
                        "context '{needle}' matches {count} places in {}; add surrounding lines to make it unique",
                        path::display(target)
                    ),
                ));
            }
        }
    }
    let old_lines = block_old_lines(block);
    if block.at_eof {
        // 唯一合法位置由「旧行贴住文件末尾」确定（空旧行即 `lines.len()`）。
        if let Some(candidate) = eof_candidate(lines, &old_lines, start) {
            return Ok(candidate);
        }
        return Err(ToolFailure::new(
            ToolErrorCode::PatchContextMismatch,
            format!(
                "block is marked '*** End of File' but the file does not end with it: \
                 the last {} line(s) of {} do not match the hunk",
                old_lines.len(),
                path::display(target)
            ),
        ));
    }
    if old_lines.is_empty() {
        // 纯插入块：无 `@@ <context>` 时追加到文件末尾（codex 对空 `old_lines` 一律
        // `replacements.push((original_lines.len(), 0, …))`）。
        if block.context.is_none() {
            return Ok(lines.len());
        }
        return Ok(start.min(lines.len()));
    }
    if old_lines.len() > lines.len() {
        return Err(context_mismatch(block, target));
    }
    // 两档匹配：先精确，再「两侧 trim_end 后相等」（rstrip 回退）。
    for rstrip in [false, true] {
        let hits = find_matches(lines, &old_lines, start, rstrip);
        match hits.len() {
            1 => return Ok(hits[0]),
            0 => continue,
            count => {
                return Err(ToolFailure::new(
                    ToolErrorCode::PatchContextMismatch,
                    format!(
                        "the changed lines of a hunk match {count} places in {} ({}); extend the hunk with more context lines",
                        path::display(target),
                        format_candidate_lines(&hits),
                    ),
                ));
            }
        }
    }
    Err(context_mismatch(block, target))
}

/// 在 `start..=lines.len()-expected.len()` 范围内找出 `expected` 的所有起始位置。
///
/// `rstrip` 为真时按「两侧 `trim_end` 后相等」比较（容忍补丁与文件的行尾空白差异）。
fn find_matches(lines: &[String], expected: &[String], start: usize, rstrip: bool) -> Vec<usize> {
    let Some(limit) = lines.len().checked_sub(expected.len()) else {
        return Vec::new();
    };
    (start..=limit)
        .filter(|candidate| {
            expected.iter().enumerate().all(|(offset, line)| {
                let actual = &lines[candidate + offset];
                if rstrip {
                    actual.trim_end() == line.trim_end()
                } else {
                    actual == line
                }
            })
        })
        .collect()
}

/// `*** End of File` 块的唯一合法位置：块的旧行贴住文件末尾（且不早于上一块的搜索起点）。
///
/// 旧行为空时该位置就是 `lines.len()`（纯插入到末尾）。返回 `None` 表示位置不存在或
/// 上下文不匹配。
fn eof_candidate(lines: &[String], old_lines: &[String], start: usize) -> Option<usize> {
    let candidate = lines.len().checked_sub(old_lines.len())?;
    if candidate < start {
        return None;
    }
    let exact = old_lines
        .iter()
        .enumerate()
        .all(|(offset, line)| lines[candidate + offset] == *line);
    let rstrip = old_lines
        .iter()
        .enumerate()
        .all(|(offset, line)| lines[candidate + offset].trim_end() == line.trim_end());
    (exact || rstrip).then_some(candidate)
}

/// 候选行号的 1-based 展示（最多 5 个，其余以 `…` 表示）。
fn format_candidate_lines(hits: &[usize]) -> String {
    const LIMIT: usize = 5;
    let shown: Vec<String> = hits
        .iter()
        .take(LIMIT)
        .map(|index| (index + 1).to_string())
        .collect();
    let mut text = format!("lines {}", shown.join(", "));
    if hits.len() > LIMIT {
        text.push_str(", …");
    }
    text
}

/// 上下文失配错误（只描述位置，不含文件内容）。
fn context_mismatch(block: &Block, target: &Path) -> ToolFailure {
    let hint = block
        .context
        .as_deref()
        .map(|context| format!(" after context '{context}'"))
        .unwrap_or_default();
    ToolFailure::new(
        ToolErrorCode::PatchContextMismatch,
        format!(
            "hunk lines{hint} do not match the current content of {}",
            path::display(target)
        ),
    )
}

/// 块的旧行（上下文 + 删除行）。
fn block_old_lines(block: &Block) -> Vec<String> {
    block
        .lines
        .iter()
        .filter(|line| line.kind != BodyLineKind::Add)
        .map(|line| line.text.clone())
        .collect()
}

/// 块的新行（上下文 + 新增行）。
fn block_new_lines(block: &Block) -> Vec<String> {
    block
        .lines
        .iter()
        .filter(|line| line.kind != BodyLineKind::Remove)
        .map(|line| line.text.clone())
        .collect()
}

/// 原子写入单个文件（同目录临时文件 + rename），并按路径加短时写锁。
///
/// 目标是符号链接时写入其真实文件（原子替换不能作用于链接本身）。
fn write_bytes(target: &Path, bytes: &[u8]) -> ToolResult<()> {
    let resolved = path::write_target(target);
    if let Some(parent) = resolved.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| path::map_io_error(&error, "create directory", parent))?;
    }
    lock::with_path_lock(&resolved, || {
        crate::platform::write_file_atomic(&resolved, bytes)
    })
    .map_err(|error| path::map_io_error(&error, "write", target))
}

/// IO 错误 → 工具错误码。
fn io_code(error: &std::io::Error) -> ToolErrorCode {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => ToolErrorCode::PermissionDenied,
        std::io::ErrorKind::NotFound => ToolErrorCode::NotFound,
        _ => ToolErrorCode::Io,
    }
}

/// 内部一致性错误（hunk 类型与解析结果不匹配，理论上不可达）。
fn internal_error(message: impl Into<String>) -> ToolFailure {
    ToolFailure::new(ToolErrorCode::Io, message)
}

#[cfg(test)]
mod tests;

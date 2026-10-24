//! `write` 工具：创建或**完整覆盖** UTF-8 文本文件。
//!
//! 覆盖语义：`content` 即替换后的完整文件内容（空字符串等于清空文件），绝不追加。
//! 父目录按需创建；写入用同目录临时文件 + rename 原子替换，并按路径加短时写锁。
//!
//! 读后版本保护（对应 design §2.1）：
//! - 覆盖已有文件必须带 `read_version`，且与**本会话读到的版本**和**当前磁盘版本**
//!   都一致；任一方不一致都在写盘前失败（零写入）。
//! - 新建文件要求目标仍不存在；目标已出现则拒绝并提示先读。
//!
//! 快照顺序：读取 before、生成 planned-after、持久化 ACK 全部在**同一临界区**内完成，
//! 之后才真正写盘；锁外不保留任何旧 preimage。

use std::path::Path;

use super::args::Args;
use super::diff::diff_lines;
use super::lock;
use super::mutation::{self, MutationOperation, MutationRecorder, PathMutation};
use super::path;
use super::read_version;
use super::text::{SourceText, split_lines};
use super::types::{ToolContext, ToolErrorCode, ToolExecutionResult, ToolFailure};

/// `write` 允许的参数集合。
const ALLOWED: [&str; 3] = ["path", "content", "read_version"];

/// 执行 `write`，可注入 mutation recorder。
pub(crate) fn run_with(
    ctx: &ToolContext,
    arguments_json: &str,
    recorder: Option<&dyn MutationRecorder>,
) -> ToolExecutionResult {
    match run_inner(ctx, arguments_json, recorder) {
        Ok(result) => result,
        Err(failure) => failure.into_result(Vec::new()),
    }
}

fn run_inner(
    ctx: &ToolContext,
    arguments_json: &str,
    recorder: Option<&dyn MutationRecorder>,
) -> super::types::ToolResult<ToolExecutionResult> {
    let args = Args::parse(arguments_json, &ALLOWED)?;
    let raw_path = args.required_str("path")?;
    // 空内容是合法输入（清空文件），因此用允许空串的读取方式。
    let content = args.required_text("content")?;
    let expected = args.optional_str("read_version")?;
    let target = path::resolve(ctx, raw_path)?;
    let resolved = path::write_target(&target);
    // 目标路径与物理目标都要锁：链接路径与物理路径是两个入口。
    let paths: Vec<&Path> = if resolved == target {
        vec![target.as_path()]
    } else {
        vec![target.as_path(), resolved.as_path()]
    };
    let _cross = mutation::lock_external(&paths)?;
    lock::with_path_locks(&paths, || {
        run_locked(
            ctx,
            target.as_path(),
            resolved.as_path(),
            content,
            expected,
            recorder,
        )
    })
}

/// 临界区内：读 before → 版本核对 → 规划 after → Prepare ACK → 写盘 → Commit ACK。
fn run_locked(
    ctx: &ToolContext,
    target: &Path,
    resolved: &Path,
    content: &str,
    expected: Option<&str>,
    recorder: Option<&dyn MutationRecorder>,
) -> super::types::ToolResult<ToolExecutionResult> {
    let before = mutation::read_write_target(resolved).map_err(|error| match error.kind() {
        std::io::ErrorKind::InvalidInput => ToolFailure::new(
            ToolErrorCode::NotAFile,
            format!(
                "{} is not a regular file; write only replaces regular files",
                path::display(target)
            ),
        ),
        _ => path::map_io_error(&error, "read", resolved),
    })?;
    mutation::ensure_single_link(resolved)?;
    let created = before.kind == crate::platform::RawKind::Missing;
    // 读后版本保护：新建要求目标不存在；覆盖要求带本会话读到的版本且磁盘未变。
    //
    // 没有登记表时（只读调用/单元测试）不做 token 核对：生产执行器一律注入登记表，
    // 持久层的 `expected_version` 核对是第二道保险，因此这条放行不会出现在生产轮次。
    let expected_version = if created {
        read_version::require_missing(target)?;
        None
    } else if let Some(registered) = ctx.reads.as_ref() {
        // 模型声明的 token 必须与本会话登记、与当前磁盘版本三方一致。
        let declared = expected.ok_or_else(|| {
            ToolFailure::new(
                ToolErrorCode::StaleRead,
                format!(
                    "{} must be read before it is overwritten; call read and pass the returned \
                     file_version as read_version",
                    path::display(target)
                ),
            )
        })?;
        let required = read_version::require_read_version(
            registered,
            &read_version::canonical_key(resolved),
            &path::display(target),
            &before,
        )?;
        if required != declared {
            return Err(ToolFailure::new(
                ToolErrorCode::StaleRead,
                format!(
                    "{} was changed after it was read; read it again and use the new file_version",
                    path::display(target)
                ),
            ));
        }
        Some(required)
    } else {
        None
    };

    // 保留既有文件的换行风格与 BOM；新建文件统一 LF、无 BOM。
    let (before_lines, before_bytes, before_final_newline, newline, bom) = if created {
        (
            Vec::new(),
            0,
            true,
            crate::chat::tools::text::Newline::Lf,
            false,
        )
    } else {
        let source = SourceText::parse(&before.bytes)?;
        (
            source.lines.clone(),
            before.bytes.len(),
            source.trailing_newline,
            source.newline,
            source.bom,
        )
    };
    let after_lines = split_lines(content);
    // 差异与计数同源：都来自这份真实 before/after，不在写盘后重读。
    let (diff, counts) = diff_lines(
        &before_lines,
        &after_lines,
        before_final_newline,
        content.ends_with('\n'),
        before_bytes,
        content.len(),
    );
    let source = SourceText {
        bom,
        newline,
        lines: Vec::new(),
        // 内容是否以换行结尾由输入本身决定。
        trailing_newline: content.ends_with('\n'),
    };
    let bytes = source.encode(&after_lines);
    let planned = mutation::planned_regular(&before, bytes.clone());
    let change = PathMutation {
        operation: if created {
            MutationOperation::Create
        } else {
            MutationOperation::Update
        },
        path: target.to_path_buf(),
        target: resolved.to_path_buf(),
        before,
        planned_after: planned,
        expected_version,
    };
    mutation::capture(recorder, vec![change], || write_content(resolved, &bytes))?;
    Ok(ToolExecutionResult::Write {
        path: target.to_path_buf(),
        created,
        added: counts.added,
        removed: counts.removed,
        diff: Some(diff),
    })
}

/// 原子写入（调用方已持有路径锁）；父目录缺失时创建。
fn write_content(resolved: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = resolved.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    crate::platform::write_file_atomic(resolved, bytes)
}

#[cfg(test)]
mod tests;

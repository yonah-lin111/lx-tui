//! `write` 工具：创建或**完整覆盖** UTF-8 文本文件。
//!
//! 覆盖语义：`content` 即替换后的完整文件内容（空字符串等于清空文件），绝不追加。
//! 父目录按需创建；写入用同目录临时文件 + rename 原子替换，并按路径加短时写锁。

use std::path::Path;

use super::args::Args;
use super::lock;
use super::path;
use super::text::{SourceText, line_diff_counts, split_lines};
use super::types::{ToolContext, ToolErrorCode, ToolExecutionResult, ToolFailure, ToolResult};

/// `write` 允许的参数集合。
const ALLOWED: [&str; 2] = ["path", "content"];

/// 执行 `write`。
pub(crate) fn run(ctx: &ToolContext, arguments_json: &str) -> ToolResult<ToolExecutionResult> {
    let args = Args::parse(arguments_json, &ALLOWED)?;
    let raw_path = args.required_str("path")?;
    // 空内容是合法输入（清空文件），因此用允许空串的读取方式。
    let content = args.required_text("content")?;
    let target = path::resolve(ctx, raw_path)?;

    let existing = match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            let metadata = if metadata.file_type().is_symlink() {
                std::fs::metadata(&target)
                    .map_err(|error| path::map_io_error(&error, "stat", &target))?
            } else {
                metadata
            };
            if metadata.is_dir() {
                return Err(ToolFailure::new(
                    ToolErrorCode::NotAFile,
                    format!(
                        "{} is a directory; write only replaces regular files",
                        path::display(&target)
                    ),
                ));
            }
            if !metadata.is_file() {
                return Err(ToolFailure::new(
                    ToolErrorCode::NotAFile,
                    format!("{} is not a regular file", path::display(&target)),
                ));
            }
            Some(metadata)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(path::map_io_error(&error, "stat", &target)),
    };

    // 保留既有文件的换行风格与 BOM；新建文件统一 LF、无 BOM。
    let (before_lines, newline, bom) = match &existing {
        Some(_) => {
            let bytes = std::fs::read(&target)
                .map_err(|error| path::map_io_error(&error, "read", &target))?;
            let source = SourceText::parse(&bytes)?;
            (source.lines.clone(), source.newline, source.bom)
        }
        None => (Vec::new(), crate::chat::tools::text::Newline::Lf, false),
    };
    let after_lines = split_lines(content);
    let (added, removed) = line_diff_counts(&before_lines, &after_lines);
    let created = existing.is_none();
    let source = SourceText {
        bom,
        newline,
        lines: Vec::new(),
        // 内容是否以换行结尾由输入本身决定。
        trailing_newline: content.ends_with('\n'),
    };
    let bytes = source.encode(&after_lines);
    write_content(&target, &bytes)?;
    Ok(ToolExecutionResult::Write {
        path: target,
        created,
        added,
        removed,
    })
}

/// 原子写入并按路径加短时写锁；父目录缺失时创建。
///
/// 目标是符号链接时写入其真实文件（原子替换不能作用于链接本身，否则会把链接
/// 变成普通文件）。
fn write_content(target: &Path, bytes: &[u8]) -> ToolResult<()> {
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

#[cfg(test)]
mod tests;

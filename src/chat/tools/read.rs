//! `read` 工具：读文本文件（带 1-based 行号与截断元数据）或列目录条目。
//!
//! `offset` 与返回行号都是 1-based；`offset` 超出文件行数返回 `OffsetOutOfRange`。
//! 空文件、末行无换行、CRLF、Unicode 宽字符都按原样保留（行号只加前缀，不改内容）。

use std::path::Path;

use super::args::{Args, clamp_limit};
use super::path;
use super::text::SourceText;
use super::types::{
    ReadKind, ToolContext, ToolErrorCode, ToolExecutionResult, ToolFailure, ToolResult,
};
use super::{MAX_READ_BYTES, MAX_READ_LINE_CHARS, MAX_READ_LINES};

/// `read` 允许的参数集合。
const ALLOWED: [&str; 3] = ["path", "offset", "limit"];

/// 执行 `read`。
pub(crate) fn run(ctx: &ToolContext, arguments_json: &str) -> ToolResult<ToolExecutionResult> {
    let args = Args::parse(arguments_json, &ALLOWED)?;
    let raw_path = args.required_str("path")?;
    let offset = args.optional_positive("offset")?.unwrap_or(1);
    let limit = clamp_limit(args.optional_positive("limit")?, MAX_READ_LINES);
    let target = path::resolve(ctx, raw_path)?;

    let metadata = std::fs::symlink_metadata(&target).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ToolFailure::new(
            ToolErrorCode::NotFound,
            format!("{} does not exist", path::display(&target)),
        ),
        _ => path::map_io_error(&error, "stat", &target),
    })?;
    // 符号链接按目标类型处理（允许链接指向任意位置）。
    let metadata = if metadata.file_type().is_symlink() {
        std::fs::metadata(&target).map_err(|error| path::map_io_error(&error, "stat", &target))?
    } else {
        metadata
    };
    if metadata.is_dir() {
        return read_directory(&target, offset, limit);
    }
    if !metadata.is_file() {
        return Err(ToolFailure::new(
            ToolErrorCode::NotAFile,
            format!("{} is not a regular file", path::display(&target)),
        ));
    }
    read_file(&target, offset, limit)
}

/// 读取文本文件的一段。
fn read_file(target: &Path, offset: usize, limit: usize) -> ToolResult<ToolExecutionResult> {
    let bytes =
        std::fs::read(target).map_err(|error| path::map_io_error(&error, "read", target))?;
    let source = SourceText::parse(&bytes)?;
    let total_lines = source.lines.len();
    if total_lines == 0 {
        // 空文件：offset 只允许 1（从首行开始读，读到 0 行）。
        if offset > 1 {
            return Err(offset_error(offset, total_lines));
        }
        return Ok(ToolExecutionResult::Read {
            path: target.to_path_buf(),
            kind: ReadKind::File,
            content: String::new(),
            start_line: 0,
            end_line: 0,
            next_offset: None,
            truncated: false,
            total_lines: 0,
        });
    }
    if offset > total_lines {
        return Err(offset_error(offset, total_lines));
    }

    let start_index = offset - 1;
    let mut content = String::new();
    let mut used_bytes = 0usize;
    let mut emitted = 0usize;
    let mut line_truncated = false;
    for index in start_index..total_lines {
        if emitted >= limit {
            break;
        }
        let (rendered, truncated) = render_line(index + 1, &source.lines[index]);
        let cost = rendered.len() + 1;
        // 至少输出一行，避免单行超限时死循环或空结果。
        if emitted > 0 && used_bytes + cost > MAX_READ_BYTES {
            break;
        }
        line_truncated |= truncated;
        content.push_str(&rendered);
        content.push('\n');
        used_bytes += cost;
        emitted += 1;
    }
    let end_line = start_index + emitted;
    let next_offset = (end_line < total_lines).then_some(end_line + 1);
    Ok(ToolExecutionResult::Read {
        path: target.to_path_buf(),
        kind: ReadKind::File,
        content,
        start_line: start_index + 1,
        end_line,
        next_offset,
        truncated: next_offset.is_some() || line_truncated,
        total_lines,
    })
}

/// 单行输出：`{n}: {text}`，超长按字符截断并标记。
fn render_line(number: usize, line: &str) -> (String, bool) {
    let count = line.chars().count();
    if count <= MAX_READ_LINE_CHARS {
        return (format!("{number}: {line}"), false);
    }
    let head: String = line.chars().take(MAX_READ_LINE_CHARS).collect();
    (format!("{number}: {head}…[truncated]"), true)
}

/// `offset` 越界错误（消息含允许范围，不含文件内容）。
fn offset_error(offset: usize, total_lines: usize) -> ToolFailure {
    ToolFailure::new(
        ToolErrorCode::OffsetOutOfRange,
        format!("offset {offset} is out of range: file has {total_lines} lines"),
    )
}

/// 列目录条目：按名称排序，`[dir]` / `[file]` 前缀，每行一个。
///
/// `offset` / `limit` 与文件读取同口径（1-based 条目序号），便于继续列出大型目录。
fn read_directory(target: &Path, offset: usize, limit: usize) -> ToolResult<ToolExecutionResult> {
    let entries = std::fs::read_dir(target)
        .map_err(|error| path::map_io_error(&error, "list directory", target))?;
    let mut names: Vec<(String, bool)> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            // 单个条目读取失败按跳过处理（父目录仍可列）。
            Err(_) => continue,
        };
        let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
        let name = entry.file_name().to_string_lossy().into_owned();
        names.push((name, is_dir));
    }
    names.sort_by(|left, right| left.0.cmp(&right.0));
    let total_lines = names.len();
    if total_lines == 0 {
        if offset > 1 {
            return Err(offset_error(offset, 0));
        }
        return Ok(ToolExecutionResult::Read {
            path: target.to_path_buf(),
            kind: ReadKind::Directory,
            content: String::new(),
            start_line: 0,
            end_line: 0,
            next_offset: None,
            truncated: false,
            total_lines: 0,
        });
    }
    if offset > total_lines {
        return Err(offset_error(offset, total_lines));
    }
    let start_index = offset - 1;
    let mut content = String::new();
    let mut used_bytes = 0usize;
    let mut emitted = 0usize;
    for (name, is_dir) in names.iter().skip(start_index) {
        if emitted >= limit {
            break;
        }
        let rendered = format!("{} {name}", if *is_dir { "[dir]" } else { "[file]" });
        let cost = rendered.len() + 1;
        if emitted > 0 && used_bytes + cost > MAX_READ_BYTES {
            break;
        }
        content.push_str(&rendered);
        content.push('\n');
        used_bytes += cost;
        emitted += 1;
    }
    let end_line = start_index + emitted;
    let next_offset = (end_line < total_lines).then_some(end_line + 1);
    Ok(ToolExecutionResult::Read {
        path: target.to_path_buf(),
        kind: ReadKind::Directory,
        content,
        start_line: start_index + 1,
        end_line,
        next_offset,
        truncated: next_offset.is_some(),
        total_lines,
    })
}

#[cfg(test)]
mod tests;

//! 路径解析与 IO 错误归一化：工具层的所有文件访问都经这里。
//!
//! 相对路径一律以冻结的 [`ToolContext::cwd`] 为基准解析，绝不读取进程 cwd；
//! 绝对路径原样使用（允许符号链接指向任意位置，访问范围由 OS 权限决定）。

use std::io;
use std::path::{Path, PathBuf};

use super::types::{ToolContext, ToolErrorCode, ToolFailure};

/// 解析模型给出的路径。
///
/// 先校验上下文 cwd（非空、绝对、存在且是目录），不满足即返回 `PathUnresolvable`：
/// 工作区已消失时本轮工具一律失败，绝不退回进程 cwd 随机执行。校验通过后，
/// 绝对路径原样返回（允许访问工作区外），相对路径拼接到 `ctx.cwd`。
pub(crate) fn resolve(ctx: &ToolContext, raw: &str) -> Result<PathBuf, ToolFailure> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ToolFailure::invalid("path must not be empty"));
    }
    if ctx.cwd.as_os_str().is_empty() {
        return Err(ToolFailure::new(
            ToolErrorCode::PathUnresolvable,
            "workspace cwd is empty; cannot resolve paths for this turn",
        ));
    }
    if !ctx.cwd.is_absolute() {
        return Err(ToolFailure::new(
            ToolErrorCode::PathUnresolvable,
            "workspace cwd is not an absolute path",
        ));
    }
    if !ctx.cwd.is_dir() {
        return Err(ToolFailure::new(
            ToolErrorCode::PathUnresolvable,
            "workspace cwd does not exist or is not a directory",
        ));
    }
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return Ok(candidate.to_path_buf());
    }
    Ok(ctx.cwd.join(candidate))
}

/// 按操作场景把 `std::io::Error` 归一化成工具错误码。
///
/// `action` 只描述动作（如 `read`、`create directory`），不包含文件内容。
pub(crate) fn map_io_error(error: &io::Error, action: &str, path: &Path) -> ToolFailure {
    let code = match error.kind() {
        io::ErrorKind::NotFound => ToolErrorCode::NotFound,
        io::ErrorKind::PermissionDenied => ToolErrorCode::PermissionDenied,
        io::ErrorKind::AlreadyExists => ToolErrorCode::Io,
        _ => ToolErrorCode::Io,
    };
    ToolFailure::new(code, format!("{action} {} failed: {error}", display(path)))
}

/// 路径展示：用于错误消息与结果文本（不涉及文件内容）。
pub(crate) fn display(path: &Path) -> String {
    path.display().to_string()
}

/// 写入目标：目标是符号链接时解析到真实文件，避免原子替换把链接本身替换成普通文件。
///
/// 只在链接已存在且能解析时改写；新建路径原样返回（链接不会在写入时凭空出现）。
pub(crate) fn write_target(path: &Path) -> PathBuf {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests;

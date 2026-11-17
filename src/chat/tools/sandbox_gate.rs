//! 文件工具沙箱闸门：所有写工具在落盘前调用同一判定。
//!
//! 只读 `ToolContext` 的策略快照，不访问全局状态；策略在 turn 起点冻结。
//! 有效权限由 [`crate::chat::sandbox::permissions::FileSystemPolicy`] 解析
//! （`Deny > Write > Read`，字面路径按组件边界优先）。

use std::path::Path;

use crate::chat::sandbox::FileSystemAccessMode;
use crate::config::sandbox::SandboxMode;
use crate::platform::sandbox::roots;

use super::types::{ToolContext, ToolErrorCode, ToolFailure};

/// 校验多个逻辑目标可写（任一失败即整体拒绝）。
pub(crate) fn ensure_writable(ctx: &ToolContext, targets: &[&Path]) -> Result<(), ToolFailure> {
    if ctx.sandbox.mode == SandboxMode::DangerFullAccess {
        return Ok(());
    }
    if ctx.sandbox.mode == SandboxMode::ReadOnly {
        return Err(read_only());
    }
    for target in targets {
        let resolved = roots::resolve_write_target(target).ok_or_else(|| outside(target))?;
        ensure_path(ctx, &resolved)?;
    }
    Ok(())
}

/// 解析后的目标判定（符号链接指向根外时由解析路径拒绝）。
pub(crate) fn ensure_writable_resolved(
    ctx: &ToolContext,
    _logical: &Path,
    resolved: &Path,
) -> Result<(), ToolFailure> {
    if ctx.sandbox.mode == SandboxMode::DangerFullAccess {
        return Ok(());
    }
    if ctx.sandbox.mode == SandboxMode::ReadOnly {
        return Err(read_only());
    }
    let resolved = roots::resolve_write_target(resolved).ok_or_else(|| outside(resolved))?;
    ensure_path(ctx, &resolved)
}

fn ensure_path(ctx: &ToolContext, path: &Path) -> Result<(), ToolFailure> {
    match ctx.sandbox.policy.access_for(path) {
        FileSystemAccessMode::Write => Ok(()),
        FileSystemAccessMode::Deny => Err(ToolFailure::new(
            ToolErrorCode::SandboxDenied,
            format!(
                "path \"{}\" is denied by the current sandbox policy.",
                path.display()
            ),
        )),
        FileSystemAccessMode::Read => match ctx.sandbox.policy.explain(path) {
            // 基准 `(/, Read)` 之外的命中是显式 read carveout / 保护路径。
            Some((_, crate::chat::sandbox::FileSystemPath::Path(entry)))
                if entry != Path::new("/") =>
            {
                Err(ToolFailure::new(
                    ToolErrorCode::SandboxDenied,
                    format!(
                        "path \"{}\" is read-only in the current sandbox policy.",
                        path.display()
                    ),
                ))
            }
            _ => Err(outside(path)),
        },
    }
}

fn read_only() -> ToolFailure {
    ToolFailure::new(
        ToolErrorCode::SandboxDenied,
        "sandbox mode \"read-only\" blocks file writes; switch mode with /permission or ask the user to change it.",
    )
}

fn outside(path: &Path) -> ToolFailure {
    ToolFailure::new(
        ToolErrorCode::SandboxDenied,
        format!(
            "path \"{}\" is outside the writable roots of the current sandbox mode.",
            path.display()
        ),
    )
}

#[cfg(test)]
mod tests;

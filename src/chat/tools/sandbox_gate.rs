//! 文件工具沙箱闸门：所有写工具在落盘前调用同一判定。
//!
//! 只读 `ToolContext` 的快照，不访问全局状态；策略在 turn 起点冻结。

use std::path::Path;

use crate::config::sandbox::SandboxMode;
use crate::platform::sandbox::roots;

use super::types::{ToolContext, ToolErrorCode, ToolFailure};

/// 校验多个逻辑目标可写（任一失败即整体拒绝）。
pub(crate) fn ensure_writable(ctx: &ToolContext, targets: &[&Path]) -> Result<(), ToolFailure> {
    if ctx.sandbox.mode == SandboxMode::DangerFullAccess {
        return Ok(());
    }
    if ctx.sandbox.mode == SandboxMode::ReadOnly {
        return Err(ToolFailure::new(
            ToolErrorCode::SandboxDenied,
            "sandbox mode \"read-only\" blocks file writes; switch mode with /permission or ask the user to change it.",
        ));
    }
    for target in targets {
        let resolved = roots::resolve_write_target(target).ok_or_else(|| outside(target))?;
        ensure_path(ctx, target)?;
        ensure_path(ctx, &resolved)?;
    }
    Ok(())
}

/// 逻辑路径与解析路径分别判定（符号链接指向根外时由解析路径拒绝）。
pub(crate) fn ensure_writable_resolved(
    ctx: &ToolContext,
    logical: &Path,
    resolved: &Path,
) -> Result<(), ToolFailure> {
    if ctx.sandbox.mode == SandboxMode::DangerFullAccess {
        return Ok(());
    }
    if ctx.sandbox.mode == SandboxMode::ReadOnly {
        return Err(ToolFailure::new(
            ToolErrorCode::SandboxDenied,
            "sandbox mode \"read-only\" blocks file writes; switch mode with /permission or ask the user to change it.",
        ));
    }
    ensure_path(ctx, logical)?;
    ensure_path(ctx, resolved)
}

fn ensure_path(ctx: &ToolContext, path: &Path) -> Result<(), ToolFailure> {
    if roots::is_protected(path, &ctx.sandbox.protected) {
        return Err(ToolFailure::new(
            ToolErrorCode::SandboxDenied,
            format!(
                "path \"{}\" is protected by the current sandbox mode.",
                path.display()
            ),
        ));
    }
    if ctx
        .sandbox
        .writable_roots
        .iter()
        .any(|root| roots::path_within(path, root))
    {
        return Ok(());
    }
    Err(outside(path))
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

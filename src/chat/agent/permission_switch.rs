//! 会话中途切换沙箱权限模式时的 `<system-update>` 英文提示词构建。

use std::path::Path;

use crate::config::sandbox::{SandboxMode, SandboxSettings};

/// 构造主会话或子 agent 切换权限模式时注入模型的英文更新文本。
pub fn render_permission_switch_update(
    from_mode: SandboxMode,
    to_mode: SandboxMode,
    sandbox: &SandboxSettings,
    workspace: Option<&Path>,
    cwd: Option<&Path>,
    approvals_cleared: bool,
) -> String {
    let mut effective = sandbox.clone();
    effective.mode = to_mode;
    let xml = crate::chat::prompt::sandbox_context_xml(&effective, workspace, cwd);
    let mut lines = vec![format!(
        "Sandbox permissions switched from {} ({}) to {} ({}).",
        from_mode.short_label(),
        from_mode.as_str(),
        to_mode.short_label(),
        to_mode.as_str(),
    )];
    if approvals_cleared {
        lines.push(
            "Session-level command approvals from the previous mode were cleared.".to_string(),
        );
    }
    lines.push(
        "Earlier tool results or denials in this conversation reflected the previous mode; follow the active sandbox boundaries below from now on."
            .to_string(),
    );
    lines.push(String::new());
    lines.push(xml);
    lines.join("\n")
}

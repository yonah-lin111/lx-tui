//! 沙箱与权限面板用户可见英文文案。

/// 权限切换提示条目标题（独立时间线条目）。
pub const LX_PERMISSION_SWITCH_NOTICE_TITLE: &str = "Permission switched";

/// 权限切换提示条目标题：连续多条合并时带数量（`Permission switched x2`）。
pub fn permission_switch_notice_title(count: usize) -> String {
    if count > 1 {
        format!("Permission switched x{count}")
    } else {
        LX_PERMISSION_SWITCH_NOTICE_TITLE.to_string()
    }
}

/// 权限切换提示条目的目标行：`mode: Workspace · 16:59`。
pub fn permission_switch_notice_head(mode_label: &str, time: &str) -> String {
    format!("mode: {mode_label} · {time}")
}

/// `/permission` 命令描述。
pub const LX_PERMISSION_COMMAND_DESC: &str = "Switch the sandbox mode for this session";
/// `/permission` 命令面板标题。
pub const LX_PERMISSION_PANEL_TITLE: &str = "Permission";
/// `/permission` 候选模式说明。
pub const LX_PERMISSION_READ_ONLY_DESC: &str = "Model can read, shell cannot write";
pub const LX_PERMISSION_WORKSPACE_DESC: &str = "Only the workspace is writable (default)";
pub const LX_PERMISSION_FULL_DESC: &str = "No sandbox at all";

/// `/permission` 切档后的窗格内提示文案。
pub fn permission_switched_toast(mode_label: &str) -> String {
    format!("Permission: {mode_label} (applies to the next turn)")
}

/// 权限切换卡片展开首行的前后跃迁描述。
pub fn permission_switch_transition(from_label: &str, to_label: &str) -> String {
    format!("switched {from_label} -> {to_label}")
}

/// 沙箱审批命令面板：显式提权标题。
pub const LX_APPROVAL_ESCALATION: &str = "Sandbox escalation";
/// 沙箱审批命令面板：on-failure 重试标题。
pub const LX_APPROVAL_RETRY: &str = "Sandbox blocked command";
/// 沙箱审批命令面板：规则文件 `prompt` 标题。
pub const LX_APPROVAL_RULE: &str = "Sandbox rule requires approval";
/// 沙箱审批命令面板：子 agent 来源角标与上下文标签。
pub const LX_APPROVAL_SUBAGENT_TAG: &str = "[Subagent]";
pub const LX_APPROVAL_SUBAGENT_LABEL: &str = "Subagent:";
/// 沙箱审批命令面板：上下文原因/命令/申请摘要标签与默认拒绝文案。
pub const LX_APPROVAL_BLOCKED_LABEL: &str = "Blocked:";
pub const LX_APPROVAL_REASON_LABEL: &str = "Reason:";
pub const LX_APPROVAL_DENIED_FALLBACK: &str = "sandbox denied";
pub const LX_APPROVAL_COMMAND_LABEL: &str = "Command:";
pub const LX_APPROVAL_REQUESTED_LABEL: &str = "Requested:";

/// 沙箱审批面板右上角来源与排队余量标签。
pub fn approval_right_title(is_subagent: bool, pending_after_front: usize) -> Option<String> {
    match (is_subagent, pending_after_front) {
        (false, 0) => None,
        (true, 0) => Some(LX_APPROVAL_SUBAGENT_TAG.to_string()),
        (false, count) => Some(format!("+{count} pending")),
        (true, count) => Some(format!("{LX_APPROVAL_SUBAGENT_TAG} +{count} pending")),
    }
}

/// 审批选项文案。
pub const LX_APPROVAL_OPT_ONCE: &str = "Approve once";
pub const LX_APPROVAL_OPT_SESSION: &str = "Approve for session";
pub const LX_APPROVAL_OPT_ALWAYS_ALLOW: &str = "Always allow prefix";
pub const LX_APPROVAL_OPT_DENY: &str = "Deny";
pub const LX_APPROVAL_OPT_RETRY_ONCE: &str = "Retry once";
pub const LX_APPROVAL_OPT_RETRY_SESSION: &str = "Always retry this command";
pub const LX_APPROVAL_OPT_SESSION_PREVIEW: &str = "this session";
pub const LX_APPROVAL_OPT_KEEP_BLOCKED: &str = "Keep blocked";

/// 审批命令面板底栏提示（按是否支持会话级与是否重试区分快捷键）。
pub const LX_APPROVAL_FOOTER_ESCALATION: &str = "↑↓ select · ↵ confirm · y/a/n · esc deny";
pub const LX_APPROVAL_FOOTER_ONCE: &str = "↑↓ select · ↵ confirm · y/n · esc deny";
pub const LX_APPROVAL_FOOTER_RETRY: &str = "↑↓ select · ↵ confirm · r/a/n · esc deny";

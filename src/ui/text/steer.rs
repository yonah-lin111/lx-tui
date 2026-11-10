//! 流中 steer（输入框上方 pending 提示区与拦截反馈）英文文案。

/// pending 提示区行首标签。
pub const LX_STEER_LABEL: &str = "Steering";
/// 单附件计数后缀（`1 attachment`）。
pub const LX_STEER_ATTACHMENT_SINGULAR: &str = "attachment";
/// 多附件计数后缀（`N attachments`）。
pub const LX_STEER_ATTACHMENTS: &str = "attachments";
/// 超过可见条数时末行提示的后缀（`+N more pending`）。
pub const LX_STEER_MORE: &str = "more pending";
/// 丢弃未生效 steer 的提示前缀。
pub const LX_STEER_DISCARDED_PREFIX: &str = "Discarded";
pub const LX_STEER_DISCARDED: &str = "undelivered steer(s)";
/// 流中 `/` 命令拒绝提示。
pub const LX_STEER_SLASH_UNSUPPORTED: &str = "Slash commands can't be sent while steering";
/// pending 条数达到上限。
pub const LX_STEER_TOO_MANY: &str = "Too many pending steers";
/// pending 图片总字节超预算。
pub const LX_STEER_IMAGE_LIMIT: &str = "Too many pending images";
/// 单条 steer 文本超过消息上限。
pub const LX_STEER_TEXT_LIMIT: &str = "Steer is too large";

/// `+N more pending`：超出提示区可见条数时的末行文案。
pub fn lx_steer_more(count: usize) -> String {
    format!("+{count} {LX_STEER_MORE}")
}

/// `1 attachment` / `N attachments`：单条 pending steer 的附件数量。
pub fn lx_steer_attachments(count: usize) -> String {
    if count == 1 {
        format!("1 {LX_STEER_ATTACHMENT_SINGULAR}")
    } else {
        format!("{count} {LX_STEER_ATTACHMENTS}")
    }
}

/// `Discarded N undelivered steer(s)`：会话切换/关闭时丢弃 pending 的提示。
pub fn lx_steer_discarded(count: usize) -> String {
    format!("{LX_STEER_DISCARDED_PREFIX} {count} {LX_STEER_DISCARDED}")
}

//! 详情输入框 `/` 命令、撤销确认与汇报/压缩反馈（英文）。

/// 撤销确认面板标题。
pub const LX_DETAIL_REWIND_PANEL_TITLE: &str = "Rewind";
/// Agents 撤销范围（右端标签）。
pub const LX_DETAIL_REWIND_SCOPE: &str = "last private turn";
/// BTW 撤销范围（右端标签）。
pub const LX_DETAIL_REWIND_SCOPE_BTW: &str = "last side turn";
/// 撤销确认面板底边提示。
pub const LX_DETAIL_REWIND_FOOTER: &str = "↑↓ select · ↵ confirm · esc cancel";
/// 撤销失败。
pub const LX_DETAIL_REWIND_FAILED: &str = "Rewind failed";
/// 撤销已提交，但重载详情视图失败。
pub const LX_DETAIL_REWIND_RELOAD_FAILED: &str = "Rewound, but reload failed";
/// 目标仍在运行/排队。
pub const LX_DETAIL_REWIND_BUSY: &str = "Wait for the current turn to finish";
/// 没有可撤回的轮次或检查点。
pub const LX_DETAIL_REWIND_NOTHING: &str = "Nothing to rewind";
/// 目标已经变化，无法安全撤销。
pub const LX_DETAIL_REWIND_UNAVAILABLE: &str = "Cannot rewind this turn";

/// 详情会话不可用（已被移除 / 尚未落库）。
pub const LX_COMPACT_UNAVAILABLE: &str = "Cannot compact this conversation";
/// 汇报目标不可用。
pub const LX_DETAIL_REPORT_UNAVAILABLE: &str = "Cannot report from this agent";

/// 汇报目标仍在运行。
pub const LX_DETAIL_REPORT_BUSY: &str = "Wait for the current turn to finish";
/// 汇报备注超过单条消息上限。
pub const LX_REPORT_NOTE_LIMIT: &str = "Report note is too long";

/// 侧线程已清空。
pub const LX_BTW_RESET_DONE: &str = "Side thread cleared";
/// 侧线程删除失败。
pub const LX_BTW_RESET_FAILED: &str = "Could not clear the side thread";
/// 回答仍在进行。
pub const LX_BTW_RESET_BUSY: &str = "Wait for the answer to finish";
/// 当前 QA 没有侧线程。
pub const LX_BTW_RESET_EMPTY: &str = "No side thread to clear";

/// 详情命令必须整段独立输入。
pub const LX_DETAIL_COMMAND_STANDALONE: &str = "Run this command on its own line";

// ===== 报告二级面板 =====

/// 报告面板标题。
pub const LX_REPORT_PANEL_TITLE: &str = "Report";
/// 报告面板底边快捷键提示。
pub const LX_REPORT_PANEL_FOOTER: &str = "↑↓ select · ↵ run · esc close";
/// 报告面板右端来源（Agents）。
pub const LX_REPORT_SCOPE_AGENT: &str = "subagent";
/// 报告面板右端来源（BTW）。
pub const LX_REPORT_SCOPE_BTW: &str = "btw";
/// 三项动作文案（顺序与 `ReportAction::ALL` 一致）。
pub const LX_REPORT_ACTION_RESEND: &str = "Resend last report";
pub const LX_REPORT_ACTION_REGENERATE: &str = "Regenerate report";
pub const LX_REPORT_ACTION_CANCEL: &str = "Cancel";
/// `Resend` 不可用原因。
pub const LX_REPORT_NO_PREVIOUS: &str = "No previous report";
/// 打开面板后目标被切走。
pub const LX_REPORT_TARGET_CHANGED: &str = "Report target changed";
/// 报告生成失败或正文为空。
pub const LX_REPORT_UNAVAILABLE: &str = "Report unavailable";
/// BTW 合并报告全部失败。
pub const LX_BTW_REPORT_UNAVAILABLE: &str = "BTW report unavailable";

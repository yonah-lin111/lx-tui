//! 详情输入框 `/` 命令、撤销确认与汇报/压缩反馈（英文）。

/// 撤销确认面板标题。
pub const LX_DETAIL_UNDO_PANEL_TITLE: &str = "Undo";
/// Agents 撤销范围（右端标签）。
pub const LX_DETAIL_UNDO_SCOPE: &str = "last private turn";
/// BTW 撤销范围（右端标签）。
pub const LX_DETAIL_UNDO_SCOPE_BTW: &str = "last side turn";
/// 撤销确认面板底边提示。
pub const LX_DETAIL_UNDO_FOOTER: &str = "↑↓ select · ↵ confirm · esc cancel";
/// 撤销成功。
pub const LX_DETAIL_UNDO_DONE: &str = "Undid the last turn";
/// 撤销失败。
pub const LX_DETAIL_UNDO_FAILED: &str = "Undo failed";
/// 目标仍在运行/排队。
pub const LX_DETAIL_UNDO_BUSY: &str = "Wait for the current turn to finish";
/// 没有可撤回的轮次。
pub const LX_DETAIL_UNDO_NOTHING: &str = "Nothing to undo";
/// 文件快照不可用。
pub const LX_DETAIL_UNDO_UNAVAILABLE: &str = "Cannot restore files for this turn";

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
/// 命令尚未接入执行（过渡占位，任务 T3–T7 会替换）。
pub const LX_DETAIL_COMMAND_UNAVAILABLE: &str = "This command is not available yet";

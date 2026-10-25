//! 右侧容器 Todo 面板的全部文案。

/// 工具栏按钮。
pub const TODO_NEW_LABEL: &str = "[new]";
pub const TODO_EDIT_LABEL: &str = "[edit]";
pub const TODO_DELETE_LABEL: &str = "[del]";
pub const TODO_CLEAR_DONE_LABEL: &str = "[clear done]";
/// 状态框（三态各 3 列；`[ ]` 内含一个空格，与 markdown 任务列表口径一致）。
pub const TODO_STATUS_TODO: &str = "[ ]";
pub const TODO_STATUS_IN_PROGRESS: &str = "[~]";
pub const TODO_STATUS_DONE: &str = "[x]";
/// 空态文案。
pub const TODO_EMPTY: &str = "no todos yet";
/// 完成计数分隔符（`done/total` 的 `/`）。
pub const TODO_COUNT_SEPARATOR: &str = "/";
/// 写库失败提示。
pub const TOAST_TODO_SAVE_FAILED: &str = "todo save failed";

/// 工具栏按钮 hover 提示。
pub const TODO_TOOLTIP_NEW: &str = "new todo";
pub const TODO_TOOLTIP_EDIT: &str = "edit todo";
pub const TODO_TOOLTIP_DELETE: &str = "delete todo";
pub const TODO_TOOLTIP_CLEAR_DONE: &str = "clear done todos";

#[cfg(test)]
mod tests;

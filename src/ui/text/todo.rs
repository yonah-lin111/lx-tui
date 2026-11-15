//! 右侧容器 Todo 面板的全部文案。

/// 工具栏按钮。
pub const TODO_NEW_LABEL: &str = "[+]";
pub const TODO_SORT_LABEL: &str = "[sort]";

/// 状态框（三态各 3 列；`[ ]` 内含一个空格，与 markdown 任务列表口径一致）。
pub const TODO_STATUS_TODO: &str = "[ ]";
pub const TODO_STATUS_IN_PROGRESS: &str = "[~]";
pub const TODO_STATUS_DONE: &str = "[x]";

/// 空态文案。
pub const TODO_EMPTY: &str = "no todos yet";
/// 新建编辑行的占位文案（输入框为空时显示，与页重命名同口径）。
pub const TODO_EDIT_PLACEHOLDER: &str = "new todo";

/// 一周快速切日带（Week Strip）控件。
pub const TODO_WEEK_PREV_LABEL: &str = "‹";
pub const TODO_WEEK_NEXT_LABEL: &str = "›";
pub const TODO_TODAY_DOT: &str = "•";

/// 历史未完成顺延横幅（Rollover Banner）。
pub const TODO_ROLLOVER_ACTION_LABEL: &str = "[move to today]";
pub const TODO_ROLLOVER_SHORT_ACTION_LABEL: &str = "[move]";
pub const TODO_ROLLOVER_DISMISS_LABEL: &str = "[x]";

/// 顺延完成后的 Toast 提示文案。
pub fn toast_todo_rollover(count: usize) -> String {
    if count == 1 {
        "Moved 1 uncompleted todo to today".to_string()
    } else {
        format!("Moved {count} uncompleted todos to today")
    }
}

/// 写库失败提示。
pub const TOAST_TODO_SAVE_FAILED: &str = "todo save failed";
/// 文本超出上限提示（提交前拦下，条目不会进列表）。
pub const TOAST_TODO_TOO_LONG: &str = "todo too long (max 500 chars)";

/// 工具栏按钮 hover 提示。
pub const TODO_TOOLTIP_NEW: &str = "new todo";
pub const TODO_TOOLTIP_SORT: &str = "sort by priority";

/// Todo 右键菜单项与回退标题。
pub const MENU_TODO_NEW: &str = "New todo";
pub const MENU_TODO_EDIT: &str = "Edit";
pub const MENU_TODO_GROUP_NEXT: &str = "Group with next";
pub const MENU_TODO_SPLIT_GROUP: &str = "Split from group";
pub const MENU_TODO_UNGROUP_ALL: &str = "Ungroup all";
pub const MENU_TODO_PRIORITY: &str = "Cycle priority";
pub const MENU_TODO_DELETE: &str = "Delete";
pub const MENU_TODO_SORT: &str = "Sort by priority";
pub const MENU_TITLE_TODO_ITEM: &str = "Todo";
pub const MENU_TITLE_TODO_BLANK: &str = "Todos";

#[cfg(test)]
mod tests;

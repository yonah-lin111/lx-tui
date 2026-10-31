//! 右侧容器 Todo 面板的全部文案。

/// 工具栏按钮。
pub const TODO_NEW_LABEL: &str = "[+]";
pub const TODO_EDIT_LABEL: &str = "[edit]";
pub const TODO_DELETE_LABEL: &str = "[del]";
pub const TODO_SORT_LABEL: &str = "[sort]";
pub const TODO_CLEAR_DONE_LABEL: &str = "[clear done]";

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

/// 底部统计：标签、段间分隔符、百分比后缀与柱图字形。
pub const TODO_STATS_TOTAL_LABEL: &str = "total";
pub const TODO_STATS_OPEN_LABEL: &str = "open";
pub const TODO_STATS_DOING_LABEL: &str = "doing";
pub const TODO_STATS_DONE_LABEL: &str = "done";
pub const TODO_STATS_PLANNED_LABEL: &str = "plan";
pub const TODO_STATS_SEPARATOR: &str = " · ";
pub const TODO_STATS_PERCENT_SUFFIX: &str = "%";

/// 图表切换控件与标题。
pub const TODO_CHART_PREV_LABEL: &str = "‹";
pub const TODO_CHART_NEXT_LABEL: &str = "›";
pub const TODO_CHART_STATUS_TITLE: &str = "status";
pub const TODO_CHART_PRIORITY_TITLE: &str = "priority";
pub const TODO_CHART_TREND_TITLE: &str = "7d trend";
pub const TODO_CHART_TODAY_PREFIX: &str = "today";
pub const TODO_CHART_DAY_PREFIX: &str = "day";
pub const TODO_CHART_WEEK_PREFIX: &str = "week";

/// 柱图实心格与计划浅格。
pub const TODO_BAR_FILLED: &str = "█";
pub const TODO_BAR_PLANNED: &str = "░";

/// 写库失败提示。
pub const TOAST_TODO_SAVE_FAILED: &str = "todo save failed";
/// 文本超出上限提示（提交前拦下，条目不会进列表）。
pub const TOAST_TODO_TOO_LONG: &str = "todo too long (max 500 chars)";

/// 工具栏按钮 hover 提示。
pub const TODO_TOOLTIP_NEW: &str = "new todo";
pub const TODO_TOOLTIP_EDIT: &str = "edit todo";
pub const TODO_TOOLTIP_DELETE: &str = "delete todo";
pub const TODO_TOOLTIP_SORT: &str = "sort by priority";
pub const TODO_TOOLTIP_CLEAR_DONE: &str = "clear done todos";

#[cfg(test)]
mod tests;

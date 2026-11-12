//! `/btw`（旁路问答）的全部用户可见文案（英文）。

/// `/btw` 在 `/` 命令面板里的一句话描述。
pub const LX_BTW_COMMAND_DESC: &str = "Ask a read-only side question about the current exchange";
/// 等待参数时的灰色提示。
pub const LX_BTW_ARG_HINT: &str = "[prompt]";
/// 后台面板 tab 标签。
pub const LX_TASK_TAB_BTW: &str = "BTW";
/// 空 `/btw` 提交时的提示。
pub const LX_BTW_EMPTY_PROMPT: &str = "Enter a question after /btw";
/// 没有主 QA 时的提示。
pub const LX_BTW_NO_QA: &str = "Start a main conversation first";
/// 只有 Native 入口支持。
pub const LX_BTW_ENTRY_UNAVAILABLE: &str = "BTW is available in Native Agent only";
/// 会话或工作区缺失。
pub const LX_BTW_NO_SESSION: &str = "Open a saved main conversation first";
/// QA 列表为空。
pub const LX_BTW_QA_EMPTY: &str = "No main question yet";
/// 线程内容尚未加载。
pub const LX_BTW_THREAD_LOADING: &str = "Loading side conversation…";
/// 没有匹配实例（D17：不自动创建任务）。
pub const LX_BTW_NO_MATCHING_AGENT: &str = "No matching subagent; start a task from the main agent";
/// 显式选择与已有实例冲突时。
pub const LX_BTW_AMBIGUOUS_AGENT: &str = "Multiple subagents match; pick one from the @ list";
/// 提交失败（保留草稿）。
pub const LX_BTW_SUBMIT_FAILED: &str = "BTW request failed; your draft was kept";
/// 回答队列已满 / 达到并发上限时的提示。
pub const LX_BTW_QUEUED: &str = "BTW request queued";
/// 停止成功。
pub const LX_BTW_STOPPED: &str = "BTW answer stopped";
/// 目标集合重置提示。
pub const LX_BTW_TARGETS_RESET: &str = "BTW targets reset to lx";
/// 输入框左侧目标标签。
pub const LX_BTW_TARGETS_PREFIX: &str = "To: ";
/// 主问题标题为空时的占位。
pub const LX_BTW_QA_UNTITLED: &str = "(untitled question)";
/// 空对话提示。
pub const LX_BTW_EMPTY_THREAD: &str = "Ask a side question about this exchange";
/// QA 列表标题。
pub const LX_BTW_QA_TITLE: &str = "QA";
/// 底边框键位提示（顺序即展示顺序，窄窗格整项丢弃）。
pub const LX_BTW_HINTS: [crate::ui::text::Hint; 4] = [
    crate::ui::text::hint("focus", "tab"),
    crate::ui::text::hint("send", "↵"),
    crate::ui::text::hint("stop", "^d"),
    crate::ui::text::hint("back", "esc"),
];

/// 侧线程状态标签。
pub fn btw_status_label(status: &str) -> &'static str {
    match status {
        "queued" => "queued",
        "running" => "running",
        "completed" => "done",
        "partial" => "partial",
        "failed" => "failed",
        "cancelled" => "stopped",
        "interrupted" => "interrupted",
        _ => "idle",
    }
}

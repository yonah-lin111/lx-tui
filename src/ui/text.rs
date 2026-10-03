//! 全部用户可见文案；组件与逻辑禁止散落硬编码字符串。

use crate::layout::PaneId;

pub const APP_NAME: &str = "lx-tui";
pub const SIDEBAR_TITLE: &str = "Workspaces";
pub const STATUS_HINT: &str = "Ctrl+b: ? help · b sidebar · q quit";
pub const HELP_TITLE: &str = "Keybindings";
pub const MIN_SIZE_HINT: &str = "terminal too small";

/// 快捷键帮助条目；应用动作都在 Ctrl+b 前缀之后。
pub const HELP_KEYS: &[(&str, &str)] = &[
    ("Ctrl+b ?", "toggle help"),
    ("Ctrl+b h / j / k / l", "focus pane"),
    ("Ctrl+b Tab / Shift+Tab", "next / prev pane"),
    ("Ctrl+b [ / ]", "prev / next tab"),
    ("Ctrl+b 1-9", "select workspace"),
    ("Ctrl+b b", "toggle sidebar"),
    ("Ctrl+b Ctrl+b", "send prefix to pane"),
    ("Ctrl+b q", "quit"),
    ("Esc", "close overlay"),
];

/// 窗格标题：优先 OSC 标题，否则按标识生成。
pub fn pane_title(id: PaneId, osc_title: Option<&str>) -> String {
    match osc_title {
        Some(title) if !title.trim().is_empty() => title.to_string(),
        _ => format!("pane {}", id.raw()),
    }
}

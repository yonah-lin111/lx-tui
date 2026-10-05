//! 全部用户可见文案；组件与逻辑禁止散落硬编码字符串。

use crate::layout::PaneId;

pub const SIDEBAR_TITLE: &str = "Workspaces";
pub const MIN_SIZE_HINT: &str = "terminal too small";

/// prompt 占位窗格。
pub const PROMPT_TITLE: &str = "prompt";
/// 折叠按钮与折叠窄条。
pub const COLLAPSE_LABEL: &str = "[-]";
pub const EXPAND_LABEL: &str = "[+]";
pub const STRIP_LINE: &str = "│";

/// 窗格标题：优先 OSC 标题，否则按标识生成。
pub fn pane_title(id: PaneId, osc_title: Option<&str>) -> String {
    match osc_title {
        Some(title) if !title.trim().is_empty() => title.to_string(),
        _ => format!("pane {}", id.raw()),
    }
}

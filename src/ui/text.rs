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

/// 复制反馈 toast。
pub const TOAST_COPIED: &str = "Copied to clipboard";
pub const TOAST_COPY_FAILED: &str = "Copy failed";

/// 单行截断：按字符数近似（现有文案为 ASCII），超出以省略号收尾。
pub fn ellipsize(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars - 1).collect();
    truncated.push('…');
    truncated
}

/// 窗格标题：优先 OSC 标题，否则按标识生成。
pub fn pane_title(id: PaneId, osc_title: Option<&str>) -> String {
    match osc_title {
        Some(title) if !title.trim().is_empty() => title.to_string(),
        _ => format!("pane {}", id.raw()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsize_keeps_text_within_limit() {
        assert_eq!(ellipsize("abc", 3), "abc");
        assert_eq!(ellipsize("abc", 5), "abc");
    }

    #[test]
    fn ellipsize_marks_truncation() {
        assert_eq!(ellipsize("Copied to clipboard", 5), "Copi…");
        assert_eq!(ellipsize("abc", 1), "…");
        assert_eq!(ellipsize("abc", 0), "");
    }
}

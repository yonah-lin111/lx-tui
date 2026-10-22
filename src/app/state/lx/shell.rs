//! lx 输入框的 shell 模式判定与文本序列化。
//!
//! 唯一规则：**整段输入的首个非空白字符**是 `!` 或全角 `！` 时进入 shell 模式；
//! 正文中任何其它位置的感叹号、货币符号 `¥`/`￥` 一律原样保留。
//! 模式字符本身不进入 `Lx::input`（正文只存脚本），序列化时再统一补回 ASCII `!`。

/// 输入框编辑模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LxInputMode {
    /// 普通聊天草稿。
    #[default]
    Chat,
    /// 本地 shell 命令（正文不含模式前缀）。
    Shell,
}

/// 整段文本中首个非空白字符若是模式标记，返回该标记的字节偏移。
///
/// 只认 `!` 与 `！`；`¥`/`￥` 明确不转换。返回 None 表示普通文本。
pub fn marker_offset(text: &str) -> Option<usize> {
    text.char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .and_then(|(offset, ch)| matches!(ch, '!' | '！').then_some(offset))
}

/// 把完整文本拆成（模式, 正文）：首个非空白标记被吸收，其余内容逐字保留。
pub fn split_mode(text: &str) -> (LxInputMode, String) {
    match marker_offset(text) {
        Some(offset) => {
            let mut body = text.to_string();
            body.remove(offset);
            (LxInputMode::Shell, body)
        }
        None => (LxInputMode::Chat, text.to_string()),
    }
}

/// 序列化输入框完整文本（输入历史、排队快照与草稿比较共用）。
pub fn serialize(mode: LxInputMode, body: &str) -> String {
    match mode {
        LxInputMode::Chat => body.to_string(),
        LxInputMode::Shell => format!("!{body}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_non_blank_marker_enters_shell() {
        assert_eq!(marker_offset("!pwd"), Some(0));
        assert_eq!(marker_offset("  ！pwd"), Some(2));
        assert_eq!(marker_offset("  !pwd"), Some(2));
    }

    #[test]
    fn marker_later_in_text_is_not_a_shell_prefix() {
        assert_eq!(marker_offset("你好！"), None);
        assert_eq!(marker_offset("说明\n！pwd"), None);
        assert_eq!(marker_offset("¥100"), None);
        assert_eq!(marker_offset("￥100"), None);
    }

    #[test]
    fn split_only_strips_the_prefix_marker() {
        assert_eq!(
            split_mode("  ！echo 好！ ¥HOME"),
            (LxInputMode::Shell, "  echo 好！ ¥HOME".to_string())
        );
        assert_eq!(split_mode("!pwd"), (LxInputMode::Shell, "pwd".to_string()));
        assert_eq!(
            split_mode("hi !there"),
            (LxInputMode::Chat, "hi !there".to_string())
        );
    }

    #[test]
    fn serialize_round_trips_shell_body() {
        assert_eq!(serialize(LxInputMode::Chat, "hi"), "hi");
        assert_eq!(serialize(LxInputMode::Shell, "pwd"), "!pwd");
    }
}

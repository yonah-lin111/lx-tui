//! lx 聊天输入框相关的动态提示文案拼接。
//!
//! 只有**需要拼串**的少数几条放这里，纯字面量常量仍在 `ui/text.rs` 的
//! `LX_CHAT_*` 区（那里贴着 1000 行上限，新增拼串逻辑挪进本模块）。

/// lx 输入框上方聊天栏的 loading 文案。
pub const LOADING_LABEL: &str = "Working...";
/// 未知命令提示前缀（后面直接跟命令名，整体保持一行短语）。
pub const UNKNOWN_COMMAND_PREFIX: &str = "Unknown command ";
/// 未知命令提示里的可用命令示例（短到能塞进一行）。
pub const UNKNOWN_COMMAND_HINT: &str = " · try /model";
/// 未知命令提示里命令名的最大展示字符数。
pub const UNKNOWN_COMMAND_SAMPLE: usize = 24;

/// 拼一条「未知命令」提示，**极简**：`Unknown command /foo · try /model`。
///
/// 命令名按字符取样后截断（不做宽字符半写），并去掉首尾空白；过长时附 `…`。
/// 整串控制在 ~40 字符内，输入框上方那一行放得下；再窄由渲染层的 `…` 兜底。
pub fn unknown_command(command: &str) -> String {
    let trimmed = command.trim();
    let mut sample: String = trimmed.chars().take(UNKNOWN_COMMAND_SAMPLE).collect();
    if sample.chars().count() < trimmed.chars().count() {
        sample.push('…');
    }
    format!("{UNKNOWN_COMMAND_PREFIX}{sample}{UNKNOWN_COMMAND_HINT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_command_is_short_enough_for_one_line() {
        let message = unknown_command("/foo");
        assert_eq!(message, "Unknown command /foo · try /model");
        // 极简：一行内放得下（输入框上方那一行会再按实际宽度截断）。
        assert!(message.chars().count() <= 40, "{message}");
    }

    #[test]
    fn long_commands_are_truncated_with_ellipsis() {
        let long = format!("/{}", "x".repeat(UNKNOWN_COMMAND_SAMPLE + 10));
        let message = unknown_command(&long);
        assert!(message.contains('…'), "超长应截断：{message}");
        // 命令名部分不超过取样上限 + 省略号。
        let name = message
            .strip_prefix(UNKNOWN_COMMAND_PREFIX)
            .and_then(|rest| rest.strip_suffix(UNKNOWN_COMMAND_HINT))
            .expect("前缀/尾巴完整");
        assert_eq!(name.chars().count(), UNKNOWN_COMMAND_SAMPLE + 1);
    }

    #[test]
    fn short_commands_are_not_truncated() {
        let message = unknown_command("/model2");
        assert!(!message.contains('…'), "{message}");
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(
            unknown_command("  /foo  "),
            "Unknown command /foo · try /model"
        );
    }
}

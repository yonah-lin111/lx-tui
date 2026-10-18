//! lx 聊天输入框相关的动态提示文案拼接。
//!
//! 只有**需要拼串**的少数几条放这里，纯字面量常量仍在 `ui/text.rs` 的
//! `LX_CHAT_*` 区（那里是 999 行的上限边界，新增拼串逻辑挪进本模块）。

/// 输入以 `/` 开头但既不是命令面板项、也不是任何内置命令时的警告前缀。
pub const UNKNOWN_COMMAND_PREFIX: &str = "Unknown command";
/// 未知命令提示里给出的可用命令提示尾巴。
pub const UNKNOWN_COMMAND_HINT: &str = "try /model, /resume or /new";
/// 未知命令提示里命令名的最大展示字符数（超长命令截断，避免撑破 toast）。
pub const UNKNOWN_COMMAND_SAMPLE: usize = 32;

/// 拼一条「未知命令」完整提示：`Unknown command `/foo`; try /model, /resume or /new`。
///
/// 命令名按字符取样后截断（不做宽字符半写），并去掉首尾空白；过长时附 `…`。
pub fn unknown_command(command: &str) -> String {
    let trimmed = command.trim();
    let mut sample: String = trimmed.chars().take(UNKNOWN_COMMAND_SAMPLE).collect();
    if sample.chars().count() < trimmed.chars().count() {
        sample.push('…');
    }
    format!("{UNKNOWN_COMMAND_PREFIX} `{sample}`; {UNKNOWN_COMMAND_HINT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_command_echoes_the_command_and_hints_available_ones() {
        let message = unknown_command("/foo");
        assert_eq!(
            message,
            "Unknown command `/foo`; try /model, /resume or /new"
        );
    }

    #[test]
    fn unknown_command_preserves_arguments_and_trims_whitespace() {
        let message = unknown_command("  /foo 参数  ");
        assert!(message.contains("/foo 参数"), "{message}");
    }

    #[test]
    fn long_commands_are_truncated_with_ellipsis() {
        let long = format!("/{}", "x".repeat(UNKNOWN_COMMAND_SAMPLE + 10));
        let message = unknown_command(&long);
        assert!(message.contains('…'), "超长应截断：{message}");
        // 截断后命令名部分不超过取样上限 + 省略号。
        let start = message.find('`').expect("有反引号") + 1;
        let end = message[start..].find('`').expect("有收尾反引号") + start;
        assert_eq!(
            message[start..end].chars().count(),
            UNKNOWN_COMMAND_SAMPLE + 1
        );
    }

    #[test]
    fn short_commands_are_not_truncated() {
        let message = unknown_command("/model2");
        assert!(!message.contains('…'), "{message}");
    }
}

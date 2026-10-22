//! lx 聊天输入框相关的动态提示文案拼接。
//!
//! 只有**需要拼串**的少数几条放这里，纯字面量常量仍在 `ui/text.rs` 的
//! `LX_CHAT_*` 区（那里贴着 1000 行上限，新增拼串逻辑挪进本模块）。

/// lx 输入框上方聊天栏的 loading 文案。
pub const LOADING_LABEL: &str = "Working...";
/// loading 文案右侧的括号统计：`(2.7s · ↑1.6k ↓320 tok)`。
///
/// 三部分都可缺失（缺哪段就少哪段，全缺返回 None 整段不画）：
/// - 耗时 = 本轮已耗时（`LxChat.turn_elapsed_ms`，事件循环每帧刷新）；
/// - `↑N` = 本轮输入估算（会话里已展示内容折算），`↓N` = 已产出输出估算；
/// - ` tok` 只挂在最后一个数字后，避免 `↑1.6k tok ↓320 tok` 的啰嗦写法。
///
/// 箭头沿用「↓ = 往外吐、↑ = 往里送」的直觉；与「回到底部」的 `↓` 靠括号与 `tok`
/// 单位区分，不会混淆。
pub fn stream_stats_label(
    elapsed_ms: Option<i64>,
    input: Option<u64>,
    output: Option<u64>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(elapsed) = elapsed_ms.filter(|value| *value > 0) {
        parts.push(crate::ui::text::format_duration(elapsed));
    }
    let mut tokens = String::new();
    if let Some(input) = input {
        tokens.push_str(&format!("↑{}", crate::ui::text::token_count_precise(input)));
    }
    if let Some(output) = output {
        if !tokens.is_empty() {
            tokens.push(' ');
        }
        tokens.push_str(&format!(
            "↓{}",
            crate::ui::text::token_count_precise(output)
        ));
    }
    if !tokens.is_empty() {
        tokens.push_str(" tok");
        parts.push(tokens);
    }
    (!parts.is_empty()).then(|| format!("({})", parts.join(" · ")))
}

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

#[cfg(test)]
mod stats_tests {
    use super::*;

    /// 三段齐全：`(2.7s · ↑1.6k ↓320 tok)`。
    #[test]
    fn stats_label_joins_time_and_both_token_directions() {
        assert_eq!(
            stream_stats_label(Some(2_700), Some(1_600), Some(320)).as_deref(),
            Some("(2.7s · ↑1.6k ↓320 tok)")
        );
    }

    /// 只有 token：没有耗时就不画时间，` tok` 只挂一次。
    #[test]
    fn stats_label_omits_missing_parts() {
        assert_eq!(
            stream_stats_label(None, Some(1_600), Some(320)).as_deref(),
            Some("(↑1.6k ↓320 tok)")
        );
        assert_eq!(
            stream_stats_label(Some(820), None, Some(12)).as_deref(),
            Some("(820ms · ↓12 tok)")
        );
        assert_eq!(
            stream_stats_label(None, Some(900), None).as_deref(),
            Some("(↑900 tok)")
        );
        assert_eq!(stream_stats_label(None, None, None), None, "全空整段不画");
        assert_eq!(
            stream_stats_label(Some(0), None, None),
            None,
            "0ms 不算耗时"
        );
    }
}

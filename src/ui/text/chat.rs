//! lx 聊天输入框相关的动态提示文案拼接。
//!
//! 只有**需要拼串**的少数几条放这里，纯字面量常量仍在 `ui/text.rs` 的
//! `LX_CHAT_*` 区（那里贴着 1000 行上限，新增拼串逻辑挪进本模块）。

use std::borrow::Cow;

use unicode_width::UnicodeWidthStr;

/// lx 输入框上方聊天栏的兜底 loading 文案（没有可展示的任务或阶段）。
pub const LOADING_LABEL: &str = "Working...";
/// 当前 assistant 仍有**开放**思考块时的阶段文案。
pub const THINKING_LABEL: &str = "Thinking...";
/// 当前 assistant 有已进入调度/执行中的工具时的阶段文案。
pub const RUNNING_TOOLS_LABEL: &str = "Running tools...";
/// `/compact` 正在执行时的阶段文案（摘要请求没有流式输出，需要自带进度反馈）。
pub const COMPACTING_LABEL: &str = "Compacting context...";

/// 计划步骤原文 → 状态栏**单行**文案。
///
/// 计划原文由模型给出，可能含换行、Tab、CR、ESC 或其他控制字符：这里把控制字符与
/// 各类空白一律当作空格，连续空白折叠成一个、去首尾空白。**只净化展示副本**，
/// 不改计划、工具结果或数据库。零宽字符本身不占列；净化后没有任何可见列时回退
/// [`LOADING_LABEL`]，避免整行只剩看不见的动画。
pub fn single_line_label(raw: &str) -> Cow<'_, str> {
    if needs_single_line(raw) {
        let mut clean = String::with_capacity(raw.len());
        for chunk in raw.split(|ch: char| ch.is_control() || ch.is_whitespace()) {
            if chunk.is_empty() {
                continue;
            }
            if !clean.is_empty() {
                clean.push(' ');
            }
            clean.push_str(chunk);
        }
        return if clean.width() > 0 {
            Cow::Owned(clean)
        } else {
            Cow::Borrowed(LOADING_LABEL)
        };
    }
    if raw.width() > 0 {
        Cow::Borrowed(raw)
    } else {
        Cow::Borrowed(LOADING_LABEL)
    }
}

/// 原文是否需要单行净化（否则可直接借用，省掉每帧一次分配）。
fn needs_single_line(raw: &str) -> bool {
    raw.contains("  ")
        || raw.starts_with(' ')
        || raw.ends_with(' ')
        || raw
            .chars()
            .any(|ch| ch.is_control() || (ch.is_whitespace() && ch != ' '))
}
/// loading 文案右侧的括号统计：`(2s · ↓320 tok)`。
///
/// 用户要求：
/// - **只显示 agent 输出**（`↓`），不再显示输入方向的 `↑`；
/// - 耗时**最小单位为秒**，不出现毫秒（`0s` / `2s` / `1m 12s`）。
///
/// 两部分都可缺失：没有耗时也没有输出时返回 None，整段不画。
pub fn stream_stats_label(elapsed_ms: Option<i64>, output: Option<u64>) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(elapsed) = elapsed_ms.filter(|value| *value > 0) {
        parts.push(format_duration_seconds(elapsed));
    }
    if let Some(output) = output {
        parts.push(format!(
            "↓{} tok",
            crate::ui::text::token_count_precise(output)
        ));
    }
    (!parts.is_empty()).then(|| format!("({})", parts.join(" · ")))
}

/// loading 行的耗时格式：**最小单位是秒**（`0s` / `2s` / `1m 12s`）。
///
/// 与统计行的 [`crate::ui::text::format_duration`] 分开：那个要精确到毫秒（本轮统计
/// 用），这里只回答「这轮跑了多久」，毫秒级的抖动没有信息量。
fn format_duration_seconds(ms: i64) -> String {
    let seconds = (ms.max(0) + 500) / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    format!("{}m {}s", seconds / 60, seconds % 60)
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

    /// 只显示输出方向；` tok` 只挂一次。
    #[test]
    fn stats_label_shows_time_and_downward_tokens_only() {
        assert_eq!(
            stream_stats_label(Some(2_700), Some(320)).as_deref(),
            Some("(3s · ↓320 tok)")
        );
        assert_eq!(
            stream_stats_label(None, Some(1_200)).as_deref(),
            Some("(↓1.2k tok)")
        );
        assert_eq!(
            stream_stats_label(Some(2_000), None).as_deref(),
            Some("(2s)")
        );
        assert_eq!(stream_stats_label(None, None), None, "全空整段不画");
        assert_eq!(stream_stats_label(Some(0), None), None, "0ms 不算耗时");
    }

    /// 耗时最小单位是秒：永远不会出现 `ms`。
    #[test]
    fn stats_label_never_shows_milliseconds() {
        for (ms, expected) in [
            (120i64, "0s"),
            (999, "1s"),
            (1_400, "1s"),
            (4_200, "4s"),
            (38_400, "38s"),
            (72_000, "1m 12s"),
        ] {
            let label = stream_stats_label(Some(ms), None).expect("有耗时");
            assert_eq!(label, format!("({expected})"), "{ms}ms");
            assert!(!label.contains("ms"), "不得出现毫秒：{label}");
        }
    }
}

#[cfg(test)]
mod activity_tests;

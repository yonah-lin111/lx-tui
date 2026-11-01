//! 本地命令（`!cmd`）记录到模型上下文 `<user_shell_command>` 的格式化。

use crate::db::chat::UserShellRecord;

use super::truncate_result_for_replay;

/// 用户命令记录 → 稳定的 user 上下文文本。
///
/// 结构字段由代码拼接，输出正文先净化控制字符并转义结构符号，绝不改变记录边界。
/// 回放只截断一次（64 KiB），并显式标注丢弃字节。
pub fn format_user_shell(record: &UserShellRecord) -> String {
    let command = escape_record_text(&sanitize_control(&record.command));
    let output = truncate_result_for_replay(&escape_record_text(&sanitize_control(&record.output)));
    let mut result = String::new();
    result.push_str("State: ");
    result.push_str(&record.state);
    result.push('\n');
    result.push_str("Exit code: ");
    result.push_str(
        &record
            .exit_code
            .map_or_else(|| "unknown".to_string(), |code| code.to_string()),
    );
    result.push('\n');
    if let Some(duration_ms) = record.duration_ms {
        result.push_str(&format!(
            "Duration: {:.1} seconds\n",
            duration_ms as f64 / 1_000.0
        ));
    }
    if record.output_truncated() {
        result.push_str(&format!(
            "Output truncated: {} earlier bytes dropped\n",
            record.dropped_bytes
        ));
    }
    result.push_str("Output:\n");
    result.push_str(if output.is_empty() {
        "(no output)"
    } else {
        output.as_str()
    });
    format!(
        "<user_shell_command>\n<command>{command}</command>\n<result>\n{result}\n</result>\n</user_shell_command>"
    )
}

/// 结构文本转义：输出里的尖括号/& 不得伪造记录字段。
fn escape_record_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 剥离 ANSI/控制字符（保留换行与制表符）。
fn sanitize_control(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            // CSI / OSC 等转义序列：吞掉引导符与后续参数字节，直到终止字符。
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else {
                let _ = chars.next();
            }
            continue;
        }
        if ch == '\n' || ch == '\t' || !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

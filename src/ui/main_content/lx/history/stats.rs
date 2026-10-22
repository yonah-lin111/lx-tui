//! 本轮统计行：模型名、输入/输出 token、缓存命中率、tok/s 与耗时。
//!
//! 纯格式化：只读消息上的用量字段，不做任何 IO 或状态变更。

use crate::app::state::ChatMessageView;
use crate::ui::text;

/// 耗时短格式：统一在 [`text::format_duration`]（loading 行与思考块共用）。
pub(crate) use crate::ui::text::format_duration;

/// 本轮统计行（assistant 消息末尾）：`模型 · 2.9k in · 307 out · 70% cached ·
/// 33.4 tok/s · 2.7s`；仍在流中或无任何可用数据时返回 None。
///
/// tok/s 对齐 opencode：输出 token ÷ 本轮耗时（秒）。
pub(crate) fn stats_line(message: &ChatMessageView) -> Option<String> {
    if message.role != "assistant" || message.status == "streaming" {
        return None;
    }
    let mut fields: Vec<String> = Vec::new();
    if let Some(input) = message.input_tokens.filter(|tokens| *tokens > 0) {
        fields.push(format!(
            "{}{}",
            text::token_count(u64::try_from(input).unwrap_or(0)),
            text::LX_CHAT_STATS_INPUT_SUFFIX
        ));
    }
    if let Some(output) = message.output_tokens.filter(|tokens| *tokens > 0) {
        fields.push(format!(
            "{}{}",
            text::token_count(u64::try_from(output).unwrap_or(0)),
            text::LX_CHAT_STATS_OUTPUT_SUFFIX
        ));
    }
    // 缓存命中率：cached / prompt（OpenAI 口径 prompt_tokens 已含 cached）。
    if let (Some(cache), Some(input)) = (
        message.cache_read_tokens.filter(|tokens| *tokens > 0),
        message.input_tokens.filter(|tokens| *tokens > 0),
    ) && let Some(percent) = cache.saturating_mul(100).checked_div(input)
    {
        fields.push(format!(
            "{}%{}",
            percent.clamp(0, 100),
            text::LX_CHAT_STATS_CACHE_SUFFIX
        ));
    }
    if let (Some(output), Some(duration)) = (
        message.output_tokens.filter(|tokens| *tokens > 0),
        message.duration_ms.filter(|duration| *duration > 0),
    ) {
        let speed = output as f64 / (duration as f64 / 1000.0);
        if speed.is_finite() && speed > 0.0 {
            fields.push(format!("{speed:.1}{}", text::LX_CHAT_STATS_SPEED_SUFFIX));
        }
    }
    if let Some(duration) = message.duration_ms.filter(|duration| *duration > 0) {
        fields.push(format_duration(duration));
    }
    // 仅「模型名」不算统计：没有任何用量/耗时数据时不出行（失败且无 usage 的轮次）。
    if fields.is_empty() {
        return None;
    }
    if let Some(model) = message
        .model_label
        .as_deref()
        .filter(|model| !model.is_empty())
    {
        fields.insert(0, model.to_string());
    }
    Some(fields.join(text::LX_CHAT_STATS_SEP))
}

//! 本轮统计行：模型名、输入/输出 token、tok/s 与耗时。
//!
//! 纯格式化：只读消息上的用量字段，不做任何 IO 或状态变更。
//! 统计行**强制单行**：宽度不足时依次隐藏 `in+out` → 模型名 → tok/s，
//! 最后才对时长做尾部省略。

use unicode_width::UnicodeWidthStr;

use crate::app::state::ChatMessageView;
use crate::ui::text;

/// 耗时短格式：统一在 [`text::format_duration`]（loading 行与思考块共用）。
pub(crate) use crate::ui::text::format_duration;

/// 本轮统计行（assistant 消息末尾）：`模型 · 2.9k in · 307 out · 33.4 tok/s · 2.7s`。
///
/// 缓存命中率已移到输入框底边状态行（整个对话累计值），这里不再出现。
/// 仍在流中或无任何可用数据时返回 None。
pub(crate) fn stats_line(message: &ChatMessageView, width: u16) -> Option<String> {
    if message.role != "assistant" || message.status == "streaming" || width == 0 {
        return None;
    }
    let model = message
        .model_label
        .as_deref()
        .filter(|model| !model.is_empty());
    let input = message.input_tokens.filter(|tokens| *tokens > 0);
    let output = message.output_tokens.filter(|tokens| *tokens > 0);
    let duration = message.duration_ms.filter(|duration| *duration > 0);
    let speed = output.zip(duration).and_then(|(output, duration)| {
        let speed = output as f64 / (duration as f64 / 1000.0);
        (speed.is_finite() && speed > 0.0)
            .then(|| format!("{speed:.1}{}", text::LX_CHAT_STATS_SPEED_SUFFIX))
    });
    let duration_label = duration.map(format_duration);
    let input_label = input.map(|tokens| {
        format!(
            "{}{}",
            text::token_count(u64::try_from(tokens).unwrap_or(0)),
            text::LX_CHAT_STATS_INPUT_SUFFIX
        )
    });
    let output_label = output.map(|tokens| {
        format!(
            "{}{}",
            text::token_count(u64::try_from(tokens).unwrap_or(0)),
            text::LX_CHAT_STATS_OUTPUT_SUFFIX
        )
    });

    // 仅「模型名」不算统计：没有任何用量/耗时数据时不出行。
    if input_label.is_none()
        && output_label.is_none()
        && speed.is_none()
        && duration_label.is_none()
    {
        return None;
    }

    let mut candidates: Vec<Vec<&str>> = Vec::new();
    let mut full: Vec<&str> = Vec::new();
    if let Some(model) = model {
        full.push(model);
    }
    if let Some(input) = input_label.as_deref() {
        full.push(input);
    }
    if let Some(output) = output_label.as_deref() {
        full.push(output);
    }
    if let Some(speed) = speed.as_deref() {
        full.push(speed);
    }
    if let Some(duration) = duration_label.as_deref() {
        full.push(duration);
    }
    candidates.push(full);
    // 丢 in/out，保留模型 + tok/s + 时长。
    let mut without_tokens: Vec<&str> = Vec::new();
    if let Some(model) = model {
        without_tokens.push(model);
    }
    if let Some(speed) = speed.as_deref() {
        without_tokens.push(speed);
    }
    if let Some(duration) = duration_label.as_deref() {
        without_tokens.push(duration);
    }
    candidates.push(without_tokens);
    // 丢模型名，保留 tok/s + 时长。
    let mut without_model: Vec<&str> = Vec::new();
    if let Some(speed) = speed.as_deref() {
        without_model.push(speed);
    }
    if let Some(duration) = duration_label.as_deref() {
        without_model.push(duration);
    }
    candidates.push(without_model);
    // 只保时长。
    if let Some(duration) = duration_label.as_deref() {
        candidates.push(vec![duration]);
    }

    let limit = usize::from(width);
    for fields in candidates {
        let fields: Vec<&str> = fields
            .into_iter()
            .filter(|field| !field.is_empty())
            .collect();
        if fields.is_empty() {
            continue;
        }
        let line = fields.join(text::LX_CHAT_STATS_SEP);
        if line.width() <= limit {
            return Some(line);
        }
    }
    // 极窄：时长仍放不下时尾部省略；没有时长则退回第一个可用字段。
    let fallback = duration_label
        .as_deref()
        .or(speed.as_deref())
        .or(input_label.as_deref())
        .or(output_label.as_deref())?;
    Some(text::truncate_to_width(fallback, limit))
}

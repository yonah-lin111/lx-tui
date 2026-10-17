//! Google Gemini 适配。
//!
//! `models/{model}:streamGenerateContent?alt=sse`，`contents` 数组 + `generationConfig`；
//! Key 只按该协议放置在查询参数（`?key=`）；`thought=true` 的 part 独立为
//! reasoning；usage 来自 `usageMetadata`。

use serde_json::{Value, json};

use super::{
    StreamDecoder, TransportRequest, json_i64, json_str, reasoning_delta, segments_to_deltas,
};
use crate::chat::events::{ChatDelta, ChatRequest, StopReason};
use crate::chat::thinking::TagSplitter;

/// Gemini API 基础 URL。
const GOOGLE_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

/// 构造 streamGenerateContent 请求。
pub fn build(
    base_url: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
) -> Result<TransportRequest, String> {
    let key = api_key.ok_or("missing api key")?;
    let base = if base_url.is_empty() {
        GOOGLE_BASE
    } else {
        base_url
    };
    let model = request.model.model.trim_start_matches("models/");
    let url = format!(
        "{}/models/{}:streamGenerateContent?alt=sse&key={}",
        base.trim_end_matches('/'),
        model,
        key
    );
    // contents：role user/model + parts[{text}]；assistant 历史 role 为 model。
    let contents: Vec<Value> = request
        .turns
        .iter()
        .map(|turn| {
            let role = if turn.role == "assistant" {
                "model"
            } else {
                "user"
            };
            json!({"role": role, "parts": [{"text": turn.text}]})
        })
        .collect();
    let mut body = json!({"contents": contents});
    let mut generation_config = serde_json::Map::new();
    if let Some(params) = &request.variant_params {
        if let Some(budget) = params.get("thinkingBudget").and_then(Value::as_i64) {
            generation_config.insert("thinkingConfig".into(), json!({"thinkingBudget": budget}));
        }
        if let Some(include) = params.get("includeThoughts").and_then(Value::as_bool) {
            generation_config.insert("thinkingConfig".into(), json!({"includeThoughts": include}));
        }
    }
    if !generation_config.is_empty() {
        body["generationConfig"] = Value::Object(generation_config);
    }
    Ok(TransportRequest {
        url,
        headers: Vec::new(),
        body: serde_json::to_string(&body).map_err(|error| format!("encode body: {error}"))?,
    })
}

/// Gemini SSE 解码器：每帧是一个完整 JSON（非事件流），含 candidates parts。
#[derive(Debug, Default)]
pub struct Decoder {
    splitter: TagSplitter,
    finished: bool,
}

impl StreamDecoder for Decoder {
    fn decode_frame(&mut self, frame: &str) -> Result<Vec<ChatDelta>, String> {
        let value: Value =
            serde_json::from_str(frame).map_err(|error| format!("invalid json: {error}"))?;
        let mut out = Vec::new();
        // 错误响应。
        if let Some(message) = value
            .get("error")
            .and_then(|error| json_str(error, "message"))
        {
            return Err(format!("api error: {}", truncate(message)));
        }
        // usageMetadata（每帧可重复，取最后一次为准的事件层合并）。
        if let Some(usage) = value.get("usageMetadata") {
            let input = json_i64(usage, "promptTokenCount");
            let output = json_i64(usage, "candidatesTokenCount");
            let cache_read = usage
                .get("promptTokensDetails")
                .and_then(|details| json_i64(details, "cachedTokens"));
            if input.is_some() || output.is_some() {
                out.push(ChatDelta::Usage(crate::db::chat::Usage {
                    input,
                    output,
                    cache_read,
                    cache_write: None,
                }));
            }
        }
        // candidates：parts 数组逐项保持顺序（同帧可多 part）。
        if let Some(candidates) = value.get("candidates").and_then(Value::as_array) {
            for candidate in candidates {
                let parts = candidate
                    .get("content")
                    .and_then(|content| content.get("parts"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for part in &parts {
                    let thought = part
                        .get("thought")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let Some(text) = json_str(part, "text") else {
                        continue;
                    };
                    if thought {
                        // thought part：独立 reasoning（不进标签状态机）。
                        out.push(reasoning_delta(text));
                    } else {
                        out.extend(segments_to_deltas(self.splitter.feed(text)));
                    }
                }
                // finishReason：STOP 为正常完成。
                if let Some(reason) = json_str(candidate, "finishReason")
                    && matches!(reason, "STOP" | "SAFETY" | "RECITATION")
                {
                    self.finished = true;
                    out.push(ChatDelta::Finished {
                        reason: StopReason::Complete,
                    });
                    // MAX_TOKENS 等其余原因：等待服务端关闭流（finish 处理）。
                }
            }
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<ChatDelta> {
        segments_to_deltas(self.splitter.finish())
    }

    fn has_completion(&self) -> bool {
        self.finished
    }
}

/// 截断稳定错误码。
fn truncate(message: &str) -> String {
    let single: String = message
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    single.chars().take(120).collect()
}

#[cfg(test)]
mod tests;

//! OpenAI Responses API 适配（模型级 `openai-responses` transport）。
//!
//! `POST /responses`，`input` 数组承载消息；reasoning 经
//! `response.reasoning_summary_text.delta` 与 raw reasoning text 事件分流，
//! summary/raw 用元数据区分；完成判定 `response.completed` / `response.failed`。

use serde_json::{Value, json};

use super::{StreamDecoder, TransportRequest, join_url, json_i64, json_str, segments_to_deltas};
use crate::chat::events::{ChatDelta, ChatRequest, PartDelta, PartKind, StopReason};
use crate::chat::thinking::TagSplitter;

/// 官方 Responses API 基础 URL。
const OFFICIAL_BASE: &str = "https://api.openai.com/v1";

/// 构造 Responses 请求：`input` 数组（不沿用 Chat 的 messages 形状）。
pub fn build(
    base_url: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
) -> Result<TransportRequest, String> {
    let key = api_key.ok_or("missing api key")?;
    let url = if base_url.is_empty() {
        join_url(OFFICIAL_BASE, "responses")
    } else {
        join_url(base_url, "responses")
    };
    let input: Vec<Value> = request
        .turns
        .iter()
        .map(|turn| json!({"role": turn.role, "content": turn.text}))
        .collect();
    let mut body = json!({
        "model": request.model.model,
        "input": input,
        "stream": true,
    });
    if let Some(params) = &request.variant_params {
        let mut reasoning = serde_json::Map::new();
        if let Some(effort) = params.get("reasoningEffort").and_then(Value::as_str) {
            reasoning.insert("effort".into(), json!(effort));
        }
        if let Some(summary) = params.get("reasoningSummary").and_then(Value::as_str) {
            reasoning.insert("summary".into(), json!(summary));
        }
        if !reasoning.is_empty() {
            body["reasoning"] = Value::Object(reasoning);
        }
        if let Some(include) = params.get("include").and_then(Value::as_array) {
            body["include"] = json!(include);
        }
    }
    Ok(TransportRequest {
        url,
        headers: vec![("Authorization".into(), format!("Bearer {key}"))],
        body: serde_json::to_string(&body).map_err(|error| format!("encode body: {error}"))?,
    })
}

/// Responses SSE 解码器。
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
        let event = value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match event {
            "response.output_text.delta" => {
                if let Some(text) = json_str(&value, "delta") {
                    out.extend(segments_to_deltas(self.splitter.feed(text)));
                }
            }
            // summary reasoning：独立 reasoning part，带 summary 来源元数据。
            "response.reasoning_summary_text.delta" => {
                if let Some(text) = json_str(&value, "delta") {
                    out.push(ChatDelta::Part(PartDelta {
                        kind: PartKind::Reasoning,
                        text: text.into(),
                        metadata: Some(json!({"source": "summary"})),
                    }));
                }
            }
            // raw reasoning text：独立 reasoning part，带 raw 来源元数据。
            "response.reasoning_text.delta" => {
                if let Some(text) = json_str(&value, "delta") {
                    out.push(ChatDelta::Part(PartDelta {
                        kind: PartKind::Reasoning,
                        text: text.into(),
                        metadata: Some(json!({"source": "raw"})),
                    }));
                }
            }
            "response.completed" => {
                self.finished = true;
                if let Some(usage) = value
                    .get("response")
                    .and_then(|response| response.get("usage"))
                {
                    let input = json_i64(usage, "input_tokens");
                    let output = json_i64(usage, "output_tokens");
                    if input.is_some() || output.is_some() {
                        out.push(ChatDelta::Usage(crate::db::chat::Usage {
                            input,
                            output,
                            cache_read: usage
                                .get("input_tokens_details")
                                .and_then(|details| json_i64(details, "cached_tokens")),
                            cache_write: None,
                        }));
                    }
                }
                out.push(ChatDelta::Finished {
                    reason: StopReason::Complete,
                });
            }
            "response.failed" => {
                self.finished = true;
                let message = value
                    .get("response")
                    .and_then(|response| response.get("error"))
                    .and_then(|error| json_str(error, "message"))
                    .unwrap_or("response failed");
                out.push(ChatDelta::Finished {
                    reason: StopReason::Failed {
                        code: truncate(message),
                    },
                });
            }
            "response.incomplete" => {
                self.finished = true;
                out.push(ChatDelta::Finished {
                    reason: StopReason::Complete,
                });
            }
            "error" => {
                self.finished = true;
                let message = json_str(&value, "message").unwrap_or("stream error");
                return Err(format!("api error: {}", truncate(message)));
            }
            // 其余事件（created/in_progress/output_item.added 等）忽略。
            _ => {}
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

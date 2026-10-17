//! Anthropic Messages API 适配。
//!
//! `POST /v1/messages`，协议 header（`x-api-key` + `anthropic-version`），
//! `max_tokens` 必填；thinking 经 `content_block_delta.thinking_delta` 独立 part，
//! `signature_delta` 以 `content_block_index` 绑定对应 thinking part 元数据
//! （即使该帧无文本也必须更新）；完成判定 `message_stop`。

use serde_json::{Value, json};

use super::{
    StreamDecoder, TransportRequest, join_url, json_i64, json_str, reasoning_delta,
    segments_to_deltas,
};
use crate::chat::events::{ChatDelta, ChatRequest, PartDelta, PartKind, StopReason};
use crate::chat::thinking::TagSplitter;

/// Anthropic API 基础 URL。
const ANTHROPIC_BASE: &str = "https://api.anthropic.com";
/// 协议版本 header。
const API_VERSION: &str = "2023-06-01";
/// 默认输出上限（协议必填）。
const DEFAULT_MAX_TOKENS: i64 = 8192;

/// 构造 Messages 请求。
pub fn build(
    base_url: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
) -> Result<TransportRequest, String> {
    let key = api_key.ok_or("missing api key")?;
    let base = if base_url.is_empty() {
        ANTHROPIC_BASE
    } else {
        base_url
    };
    let url = join_url(base, "v1/messages");
    let messages: Vec<Value> = request
        .turns
        .iter()
        .map(|turn| {
            // 跨 provider 的 reasoning 元数据不回传；同 provider 续聊时由
            // 调用方把保存的 thinking block 原样并入（见 request.turns 扩展）。
            json!({"role": turn.role, "content": turn.text})
        })
        .collect();
    let mut body = json!({
        "model": request.model.model,
        "messages": messages,
        "max_tokens": DEFAULT_MAX_TOKENS,
        "stream": true,
    });
    if let Some(params) = &request.variant_params {
        // thinking 预算：thinkingBudget（token 数）映射为协议 thinking 结构。
        if let Some(budget) = params.get("thinkingBudget").and_then(Value::as_i64) {
            body["thinking"] = json!({"type": "enabled", "budget_tokens": budget});
            // 有预算时 max_tokens 必须大于预算（协议约束）。
            if budget >= DEFAULT_MAX_TOKENS {
                body["max_tokens"] = json!(budget.saturating_add(1024));
            }
        }
        if let Some(effort) = params.get("effort").and_then(Value::as_str) {
            body["reasoning_effort"] = json!(effort);
        }
    }
    Ok(TransportRequest {
        url,
        headers: vec![
            ("x-api-key".into(), key.to_string()),
            ("anthropic-version".into(), API_VERSION.into()),
        ],
        body: serde_json::to_string(&body).map_err(|error| format!("encode body: {error}"))?,
    })
}

/// Messages SSE 解码器。
#[derive(Debug, Default)]
pub struct Decoder {
    splitter: TagSplitter,
    finished: bool,
    /// content_block_index -> part 顺序号（signature 绑定用）。
    block_positions: Vec<Option<usize>>,
}

impl Decoder {
    /// 记录一个 content block 的 part 位置。
    fn bind_block(&mut self, index: usize, position: usize) {
        while self.block_positions.len() <= index {
            self.block_positions.push(None);
        }
        self.block_positions[index] = Some(position);
    }
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
            "message_start" => {
                // usage（input 侧）可能随 message_start 到达。
                if let Some(usage) = value
                    .get("message")
                    .and_then(|message| message.get("usage"))
                {
                    let input = json_i64(usage, "input_tokens");
                    let cache_read = json_i64(usage, "cache_read_input_tokens");
                    let cache_write = json_i64(usage, "cache_creation_input_tokens");
                    if input.is_some() || cache_read.is_some() || cache_write.is_some() {
                        out.push(ChatDelta::Usage(crate::db::chat::Usage {
                            input,
                            output: None,
                            cache_read,
                            cache_write,
                        }));
                    }
                }
            }
            "content_block_start" => {
                let index = json_i64(&value, "index").unwrap_or(0).max(0) as usize;
                let _block = value.get("content_block");
                // 记录 block 位置（text/thinking 都可能接 delta）。
                self.bind_block(index, index);
            }
            "content_block_delta" => {
                let index = json_i64(&value, "index").unwrap_or(0).max(0) as usize;
                if let Some(delta) = value.get("delta") {
                    let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");
                    match delta_type {
                        "text_delta" => {
                            if let Some(text) = json_str(delta, "text") {
                                out.extend(segments_to_deltas(self.splitter.feed(text)));
                            }
                        }
                        "thinking_delta" => {
                            if let Some(thinking) = json_str(delta, "thinking") {
                                out.push(reasoning_delta(thinking));
                            }
                        }
                        "signature_delta" => {
                            // 签名绑定到对应 thinking part 元数据；无文本帧也生效。
                            if let Some(signature) = json_str(delta, "signature") {
                                out.push(ChatDelta::Part(PartDelta {
                                    kind: PartKind::Reasoning,
                                    text: String::new(),
                                    metadata: Some(json!({
                                        "signature": signature,
                                        "blockIndex": index,
                                    })),
                                }));
                            }
                        }
                        "input_json_delta" => {
                            // 工具输入：不支持工具。
                            return Err("tool calls are not supported".into());
                        }
                        _ => {}
                    }
                }
            }
            "message_delta" => {
                // 结束原因与 output usage。
                if let Some(usage) = value.get("usage") {
                    let output = json_i64(usage, "output_tokens");
                    if output.is_some() {
                        out.push(ChatDelta::Usage(crate::db::chat::Usage {
                            input: None,
                            output,
                            cache_read: None,
                            cache_write: None,
                        }));
                    }
                }
            }
            "message_stop" => {
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
            "ping" | "content_block_stop" | "message_start_ignored" => {}
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

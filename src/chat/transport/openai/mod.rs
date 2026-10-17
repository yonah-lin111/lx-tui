//! OpenAI Chat Completions 适配：`openai` 与 `openai-compatible` 共用。
//!
//! 官方 `openai` 走固定 URL；兼容端点用配置 baseURL 安全拼接 `/chat/completions`。
//! thinking：`delta.reasoning_content` / `delta.reasoning` 字段独立 part；
//! 正文内嵌 `<think>` / `<thinking>` 标签由 TagSplitter 增量拆块。

use serde_json::{Value, json};

use super::{
    StreamDecoder, TransportRequest, join_url, json_str, openai_usage, reasoning_delta,
    segments_to_deltas,
};
use crate::chat::events::{ChatDelta, ChatRequest, StopReason};
use crate::chat::thinking::TagSplitter;

/// 官方 OpenAI API 基础 URL。
const OFFICIAL_BASE: &str = "https://api.openai.com/v1";

/// 构造 Chat Completions 请求。
///
/// `compatible` 为真时用配置 baseURL（典型已含 `/v1`）；官方端点固定 URL。
/// 不发送 tools/function；无 variant 时不发送推理等级字段。
pub fn build(
    base_url: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
    compatible: bool,
) -> Result<TransportRequest, String> {
    let key = api_key.ok_or("missing api key")?;
    let url = if compatible {
        join_url(base_url, "chat/completions")
    } else {
        join_url(OFFICIAL_BASE, "chat/completions")
    };
    let messages: Vec<Value> = request
        .turns
        .iter()
        .map(|turn| json!({"role": turn.role, "content": turn.text}))
        .collect();
    let mut body = json!({
        "model": request.model.model,
        "messages": messages,
        "stream": true,
        // 请求末帧携带 usage。
        "stream_options": {"include_usage": true},
    });
    if let Some(params) = &request.variant_params {
        // 已知 variant 字段（reasoningEffort 等）映射为协议字段；不伪造等级。
        if let Some(effort) = params.get("reasoningEffort").and_then(Value::as_str) {
            body["reasoning_effort"] = json!(effort);
        }
        // 供应商明确配置的附加参数原样并入顶层（仅对象字段）。
        if let Some(extra) = params.get("requestExtra").and_then(Value::as_object) {
            for (key, value) in extra {
                body[key.as_str()] = value.clone();
            }
        }
    }
    Ok(TransportRequest {
        url,
        headers: vec![("Authorization".into(), format!("Bearer {key}"))],
        body: serde_json::to_string(&body).map_err(|error| format!("encode body: {error}"))?,
    })
}

/// Chat Completions SSE 解码器。
#[derive(Debug, Default)]
pub struct Decoder {
    /// 正文内嵌标签拆分器（reasoning_content 直接进 reasoning，不二次扫描）。
    splitter: TagSplitter,
    /// 已发出的终态（防重复）。
    finished: bool,
    /// 已收到 usage。
    usage: Option<crate::db::chat::Usage>,
}

impl StreamDecoder for Decoder {
    fn decode_frame(&mut self, frame: &str) -> Result<Vec<ChatDelta>, String> {
        let value: Value =
            serde_json::from_str(frame).map_err(|error| format!("invalid json: {error}"))?;
        let mut out = Vec::new();
        // 错误响应（HTTP 200 但 body 是错误 JSON）。
        if let Some(message) = value
            .get("error")
            .and_then(|error| json_str(error, "message"))
        {
            return Err(format!("api error: {}", truncate_code(message)));
        }
        // usage：可选字段，可能与 finish_reason 同帧（部分 OpenAI 兼容端点如此），
        // 因此必须先记录、但不提前返回，继续解析 choices 与 finish_reason。
        if let Some(usage) = openai_usage(&value) {
            if let ChatDelta::Usage(usage_value) = &usage {
                self.usage = Some(*usage_value);
            }
            out.push(usage);
        }
        let Some(choices) = value.get("choices").and_then(Value::as_array) else {
            return Ok(out);
        };
        for choice in choices {
            let delta = choice.get("delta");
            if let Some(delta) = delta {
                // 原生推理字段：独立 reasoning part（不进标签状态机）。
                // 空串（端点常在每帧回 `reasoning_content:""`）不产出空 part。
                for key in ["reasoning_content", "reasoning"] {
                    if let Some(text) = json_str(delta, key).filter(|text| !text.is_empty()) {
                        out.push(reasoning_delta(text));
                    }
                }
                if let Some(text) = json_str(delta, "content") {
                    // 正文可能内嵌 <think> 标签：经状态机拆分。
                    out.extend(segments_to_deltas(self.splitter.feed(text)));
                }
            }
            // finish_reason 可能为 null（进行中）或空串；仅非空值视为完成信号。
            if let Some(reason) = choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
            {
                // tool_calls 之类的结束原因本期不支持，避免误判为完成。
                if reason != "stop" && reason != "length" {
                    return Err(format!("unsupported finish_reason: {reason}"));
                }
                self.finished = true;
                out.push(ChatDelta::Finished {
                    reason: StopReason::Complete,
                });
            }
            // 意外 tool call：不支持工具，本轮以不支持错误结束。
            if let Some(calls) = delta
                .and_then(|delta| delta.get("tool_calls"))
                .and_then(Value::as_array)
                && !calls.is_empty()
            {
                return Err("tool calls are not supported".into());
            }
        }
        Ok(out)
    }

    fn finish(&mut self) -> Vec<ChatDelta> {
        // 冲刷半截标签残余；终态由 client 统一追加。
        segments_to_deltas(self.splitter.finish())
    }

    fn has_completion(&self) -> bool {
        // Chat Completions：finish_reason 或 [DONE] 后正常 EOF 均视为完成；
        // 无完成信号且流截断由 client 判定。
        self.finished
    }

    fn mark_done(&mut self) {
        // `[DONE]` 是 OpenAI 兼容流的正常结束标记。
        self.finished = true;
    }
}

/// 截断错误码（不含完整响应体）。
fn truncate_code(message: &str) -> String {
    let single: String = message
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    single.chars().take(120).collect()
}

#[cfg(test)]
mod tests;

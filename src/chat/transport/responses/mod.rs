//! OpenAI Responses API 适配（模型级 `openai-responses` transport）。
//!
//! `POST /responses`，`input` 数组承载消息；reasoning 经
//! `response.reasoning_summary_text.delta` 与 raw reasoning text 事件分流，
//! summary/raw 用元数据区分；完成判定 `response.completed` / `response.failed`。
//!
//! 工具化字段（来源：OpenAI Responses 流式事件文档）：
//! - 请求 `tools: [{"type":"function","name","description","parameters"}]`（**扁平**，
//!   与 Chat Completions 的 `function` 包装不同），系统指令走顶层 `instructions`；
//! - `response.output_item.added` 且 `item.type=="function_call"` →
//!   `ToolInputStarted{call_id: item.call_id, name: item.name, ordinal: output_index}`；
//! - `response.function_call_arguments.delta` → `ToolInputDelta`；
//! - `response.function_call_arguments.done` → `ToolInputEnded`；
//! - `response.completed` → 先闭合所有未闭合调用，再发 `ResponseFinished{stop}`
//!   （有调用则 `ToolCalls`，否则 `Final`）与 `Finished{Complete}`；
//! - 续传：`{"type":"function_call","call_id","name","arguments"}` +
//!   `{"type":"function_call_output","call_id","output"}`；
//! - hosted tool 项（`item.type` 不是 `function_call` 的调用类输出项）不得当成本地执行，
//!   按协议错误结束。

use serde_json::{Value, json};

use super::super::transport::tools::{
    HistoryBlock, ToolCallBuffer, arguments_string, history_blocks, openai_responses_tools,
    protocol_error,
};
use super::{StreamDecoder, TransportRequest, join_url, json_i64, json_str, segments_to_deltas};
use crate::chat::events::{
    ChatDelta, ChatRequest, ModelStepStop, PartDelta, PartKind, StopReason,
    TOOL_ARGUMENTS_INVALID_CODE,
};
use crate::chat::thinking::TagSplitter;

/// 官方 Responses API 基础 URL。
const OFFICIAL_BASE: &str = "https://api.openai.com/v1";

/// Responses 输出项里属于**供应商托管**（非本地四工具）的类型：不得当本地调用执行。
const HOSTED_TOOL_ITEM_TYPES: [&str; 9] = [
    "web_search_call",
    "file_search_call",
    "code_interpreter_call",
    "image_generation_call",
    "mcp_call",
    "mcp_list_tools",
    "mcp_approval_request",
    "computer_call",
    "local_shell_call",
];

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
    let input = input_of(request)?;
    // 系统指令：`instructions` 只放首段，其余段改为前置 user 项。
    //
    // 形状见 [`super::SystemPromptParts`]：多个 system input 项会被代理拼回一条再按
    // 2000 判长度，超限整条替换（实测）；合并成一条前置 user 项后三种 transport 一致。
    let parts = super::SystemPromptParts::split(&request.resolve_system_prompt());
    let mut input = input;
    if let Some(tail) = parts.tail_message() {
        input.insert(0, json!({"role": "user", "content": tail}));
    }
    let mut body = json!({
        "model": request.model.model,
        "instructions": parts.head,
        "input": input,
        "stream": true,
    });
    // 摘要请求不声明工具（无工具就无从发起调用）。
    if let Some(surface) = request.tools_surface() {
        body["tools"] = openai_responses_tools(surface);
    }
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
        tools_requested: request.tools_surface().is_some(),
    })
}

/// 编码 `input`：`items` 非空走结构化历史（function_call / function_call_output），
/// 为空时沿用纯文本 turns（与旧行为逐字等价）。
fn input_of(request: &ChatRequest) -> Result<Vec<Value>, String> {
    if request.items.is_empty() {
        return Ok(request
            .turns
            .iter()
            .map(|turn| json!({"role": turn.role, "content": turn.text}))
            .collect());
    }
    let mut input: Vec<Value> = Vec::with_capacity(request.items.len());
    for block in history_blocks(&request.items)? {
        match block {
            HistoryBlock::Text { role, text } => {
                input.push(json!({"role": role.as_str(), "content": text}));
            }
            HistoryBlock::ToolCall {
                call_id,
                name,
                input: arguments,
            } => {
                // 函数调用输出项：arguments 仍是 JSON 字符串（与 Chat 一致）。
                input.push(json!({
                    "type": "function_call",
                    "call_id": call_id,
                    "name": name,
                    "arguments": arguments_string(&arguments)?,
                }));
            }
            HistoryBlock::ToolResult {
                call_id, content, ..
            } => {
                // 函数调用结果输入项：只用 call_id 关联，不重复 name。
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": content,
                }));
            }
        }
    }
    Ok(input)
}

/// Responses SSE 解码器。
#[derive(Debug, Default)]
pub struct Decoder {
    splitter: TagSplitter,
    finished: bool,
    /// 跨帧工具参数缓冲（按 `output_index` 聚合；每响应独立实例）。
    calls: ToolCallBuffer,
    /// 本响应是否已发出 `ResponseFinished`（防重复）。
    response_finished: bool,
    /// `item_id -> output_index`：arguments 事件只带 `item_id` 时用它定位调用。
    item_indexes: Vec<(String, usize)>,
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
            // output item 开始：只有 `function_call` 才是本地工具调用。
            "response.output_item.added" => {
                let Some(item) = value.get("item") else {
                    return Ok(out);
                };
                let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
                if HOSTED_TOOL_ITEM_TYPES.contains(&item_type) {
                    // 供应商托管的工具（web_search/file_search/...）不能当成本地执行。
                    return Err(protocol_error(format!(
                        "hosted tool item is not supported locally: {item_type}"
                    )));
                }
                if item_type != "function_call" {
                    // reasoning / message 等普通输出项：忽略。
                    return Ok(out);
                }
                let ordinal = value
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.calls.len() as u64) as usize;
                let call_id = item
                    .get("call_id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| protocol_error("function_call item without call_id"))?;
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| protocol_error("function_call item without name"))?;
                self.calls.identify(ordinal, call_id, name)?;
                if let Some(item_id) = json_str(item, "id") {
                    self.item_indexes.push((item_id.to_string(), ordinal));
                }
                if let Some((call_id, name)) = self.calls.take_start(ordinal) {
                    out.push(ChatDelta::ToolInputStarted {
                        call_id,
                        name,
                        ordinal,
                    });
                }
                // 少数实现会在 added 里直接给完整 arguments：一并累计。
                if let Some(arguments) = item
                    .get("arguments")
                    .and_then(Value::as_str)
                    .filter(|arguments| !arguments.is_empty())
                {
                    self.calls.push_delta(ordinal, arguments)?;
                    self.flush_delta(ordinal, &mut out);
                }
            }
            "response.function_call_arguments.delta" => {
                let ordinal = self.ordinal_of(&value)?;
                if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                    self.calls.push_delta(ordinal, delta)?;
                }
                self.flush_delta(ordinal, &mut out);
            }
            "response.function_call_arguments.done" => {
                let ordinal = self.ordinal_of(&value)?;
                // done 携带权威完整参数：以它为准（delta 可能被合并或缺失）。
                if let Some(arguments) = value.get("arguments").and_then(Value::as_str) {
                    self.calls.replace_arguments(ordinal, arguments)?;
                }
                self.flush_delta(ordinal, &mut out);
                if let Some((call_id, arguments_json)) = self.calls.end(ordinal) {
                    validate_arguments(&arguments_json)?;
                    out.push(ChatDelta::ToolInputEnded {
                        call_id,
                        arguments_json,
                    });
                }
            }
            "response.completed" => {
                // usage 必须先于终态发出：`Finished` 之后不再允许任何增量。
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
                out.extend(self.complete()?);
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
                // 未闭合工具参数在 incomplete 时不执行；无工具时按完成处理（旧行为）。
                if self.calls.has_open() {
                    return Err(protocol_error(
                        "response.incomplete with unclosed tool call",
                    ));
                }
                out.push(ChatDelta::ResponseFinished {
                    stop: ModelStepStop::Final,
                });
                out.push(ChatDelta::Finished {
                    reason: StopReason::Complete,
                });
            }
            "error" => {
                self.finished = true;
                let message = json_str(&value, "message").unwrap_or("stream error");
                return Err(format!("api error: {}", truncate(message)));
            }
            // 其余事件（created/in_progress/output_item.done 等）忽略。
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

    fn has_open_tool_input(&self) -> bool {
        self.calls.has_open()
    }
}

impl Decoder {
    /// 事件里定位 ordinal：优先 `output_index`，退化到 `item_id` 映射。
    fn ordinal_of(&mut self, value: &Value) -> Result<usize, String> {
        if let Some(ordinal) = value.get("output_index").and_then(Value::as_u64) {
            return Ok(ordinal as usize);
        }
        let item_id = json_str(value, "item_id")
            .ok_or_else(|| protocol_error("function call arguments event without output_index"))?;
        self.item_indexes
            .iter()
            .find(|(id, _)| id == item_id)
            .map(|(_, ordinal)| *ordinal)
            .ok_or_else(|| protocol_error(format!("unknown output item: {item_id}")))
    }

    /// 把缓冲里尚未发出的参数片段作为 `ToolInputDelta` 推出。
    fn flush_delta(&mut self, ordinal: usize, out: &mut Vec<ChatDelta>) {
        let chunk = self.calls.take_emittable(ordinal);
        if chunk.is_empty() {
            return;
        }
        if let Some(call_id) = self.calls.call_id_at(ordinal) {
            out.push(ChatDelta::ToolInputDelta {
                call_id: call_id.to_string(),
                chunk,
            });
        }
    }

    /// `response.completed`：闭合所有调用，发 `ResponseFinished` 与 `Finished`。
    fn complete(&mut self) -> Result<Vec<ChatDelta>, String> {
        let mut out = Vec::new();
        if self.response_finished {
            return Ok(out);
        }
        // 先闭合所有未闭合调用，再宣告本次响应结束（顺序与设计一致）。
        for (call_id, arguments_json) in self.calls.end_all_open()? {
            validate_arguments(&arguments_json)?;
            out.push(ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            });
        }
        let stop = if self.calls.has_calls() {
            ModelStepStop::ToolCalls
        } else {
            ModelStepStop::Final
        };
        out.push(ChatDelta::ResponseFinished { stop });
        self.response_finished = true;
        self.finished = true;
        out.push(ChatDelta::Finished {
            reason: StopReason::Complete,
        });
        Ok(out)
    }
}

/// 校验完整参数是合法 JSON。
fn validate_arguments(arguments_json: &str) -> Result<(), String> {
    serde_json::from_str::<Value>(arguments_json)
        .map(|_| ())
        .map_err(|_| format!("{TOOL_ARGUMENTS_INVALID_CODE}: malformed tool arguments"))
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

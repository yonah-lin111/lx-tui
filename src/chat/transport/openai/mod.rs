//! OpenAI Chat Completions 适配：`openai` 与 `openai-compatible` 共用。
//!
//! 官方 `openai` 走固定 URL；兼容端点用配置 baseURL 安全拼接 `/chat/completions`。
//! thinking：`delta.reasoning_content` / `delta.reasoning` 字段独立 part；
//! 正文内嵌 `<think>` / `<thinking>` 标签由 TagSplitter 增量拆块。
//!
//! 工具化字段全部来自 OpenAI Chat Completions 的流式事件 `chat.completion.chunk`
//! （官方文档字段：`choices[].delta.tool_calls[].{index,id,function.name,
//! function.arguments}`、`choices[].finish_reason ∈ {stop,length,tool_calls,
//! content_filter,function_call}`、末帧 `usage`、流末尾 `data: [DONE]`）：
//! - 请求：`tools: [{"type":"function","function":{name,description,parameters}}]`；
//! - 解码：按 `tool_calls[].index` 合并 id/name/arguments，首片发 `ToolInputStarted`，
//!   每片发 `ToolInputDelta`，`finish_reason="tool_calls"` 时闭合调用并发
//!   `ResponseFinished{ToolCalls}`；
//! - `stop`/`length` 且无工具 → `ResponseFinished{Final}` + `Finished{Complete}`；
//! - `[DONE]` 只代表本次响应流结束，不代表用户轮次结束。

use serde_json::{Value, json};

use super::super::prompt::system_prompt;
use super::super::transport::tools::{
    HistoryBlock, ToolCallBuffer, arguments_string, history_blocks, openai_chat_tools,
    protocol_error,
};
use super::{
    StreamDecoder, TransportRequest, join_url, json_str, openai_usage, reasoning_delta,
    segments_to_deltas,
};
use crate::chat::events::{
    ChatDelta, ChatRequest, ModelStepStop, StopReason, TOOL_ARGUMENTS_INVALID_CODE,
};
use crate::chat::thinking::TagSplitter;

/// 官方 OpenAI API 基础 URL。
const OFFICIAL_BASE: &str = "https://api.openai.com/v1";

/// 构造 Chat Completions 请求。
///
/// `compatible` 为真时用配置 baseURL（典型已含 `/v1`）；官方端点固定 URL。
/// 四工具定义恒发（`tool_choice` 保持默认 auto）；`items` 非空时按 call/result
/// 成对编码历史，为空时沿用纯文本 `turns`（与旧行为逐字等价）。
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
    let messages = messages_of(request)?;
    let mut body = json!({
        "model": request.model.model,
        "messages": messages,
        // 四条稳定工具定义（系统指令在首条 system 消息里）。
        "tools": openai_chat_tools(),
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
        tools_requested: true,
    })
}

/// 编码 `messages`：首条 system 为 `lx` 指令；`items` 非空走结构化历史。
fn messages_of(request: &ChatRequest) -> Result<Vec<Value>, String> {
    let mut messages: Vec<Value> = vec![json!({
        "role": "system",
        "content": system_prompt(),
    })];
    if request.items.is_empty() {
        messages.extend(
            request
                .turns
                .iter()
                .map(|turn| json!({"role": turn.role, "content": turn.text})),
        );
        return Ok(messages);
    }
    for block in history_blocks(&request.items)? {
        match block {
            HistoryBlock::Text { role, text } => {
                messages.push(json!({"role": role.as_str(), "content": text}));
            }
            HistoryBlock::ToolCall {
                call_id,
                name,
                input,
            } => {
                // assistant 的 tool_calls 消息：`arguments` 是 **JSON 字符串**（只序列化一次）。
                messages.push(json!({
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments_string(&input)?,
                        }
                    }]
                }));
            }
            HistoryBlock::ToolResult {
                call_id, content, ..
            } => {
                // role=tool + tool_call_id；失败结果同样作为 content 回传（wire 无 is_error 字段）。
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": content,
                }));
            }
        }
    }
    Ok(messages)
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
    /// 跨帧工具参数缓冲（按 `tool_calls[].index` 聚合；每响应独立实例）。
    calls: ToolCallBuffer,
    /// 本响应是否已发出 `ResponseFinished`（防重复）。
    response_finished: bool,
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
                    if let Some(text) = delta
                        .get(key)
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                    {
                        out.push(reasoning_delta(text));
                    }
                }
                if let Some(text) = delta
                    .get("content")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    // 正文可能内嵌 <think> 标签：经状态机拆分。
                    out.extend(segments_to_deltas(self.splitter.feed(text)));
                }
                // 工具调用分片：按 index 合并，跨帧拼接 id/name/arguments。
                if let Some(calls) = delta
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .filter(|calls| !calls.is_empty())
                {
                    for call in calls {
                        self.absorb_tool_call(call, &mut out)?;
                    }
                } else if has_deprecated_function_call(delta) {
                    // 已废弃的 `delta.function_call` 没有 index/ID，无法聚合与续传。
                    return Err(protocol_error(
                        "deprecated function_call delta without tool_calls index",
                    ));
                }
            }
            // finish_reason 可能为 null（进行中）或空串；仅非空值视为本次响应完成信号。
            if let Some(reason) = choice
                .get("finish_reason")
                .and_then(Value::as_str)
                .filter(|reason| !reason.is_empty())
            {
                out.extend(self.finish_response(reason)?);
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
        // `[DONE]` 是 OpenAI 兼容流的正常结束标记：只标本次响应流完成。
        self.finished = true;
    }

    fn has_open_tool_input(&self) -> bool {
        self.calls.has_open()
    }
}

impl Decoder {
    /// 合并一个 `delta.tool_calls[index]` 分片。
    fn absorb_tool_call(&mut self, call: &Value, out: &mut Vec<ChatDelta>) -> Result<(), String> {
        // 纯占位条目（`{}` 或全空字段）没有任何可聚合信息，直接忽略；
        // 兼容网关会在无调用时回这类空条目，不能据此报协议错误。
        if is_placeholder_tool_call(call) {
            return Ok(());
        }
        let ordinal = call
            .get("index")
            .and_then(Value::as_u64)
            .ok_or_else(|| protocol_error("tool_calls entry without index"))?
            as usize;
        // 旧协议把函数调用放在 `delta.function_call`（无 index/ID）；本期只支持
        // `tool_calls`，遇到无 index 的分片按协议错误失败，不猜调用。
        if let Some(id) = call
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            self.calls.bind_call_id(ordinal, id)?;
        }
        let function = call.get("function");
        if let Some(name) = function
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            self.calls.bind_name(ordinal, name)?;
        }
        if let Some(arguments) = function
            .and_then(|function| function.get("arguments"))
            .and_then(Value::as_str)
            .filter(|arguments| !arguments.is_empty())
        {
            self.calls.push_delta(ordinal, arguments)?;
        }
        // call ID 与 name 齐备后才能发 Started；此前累计的参数在 Started 后补发。
        if let Some((call_id, name)) = self.calls.take_start(ordinal) {
            out.push(ChatDelta::ToolInputStarted {
                call_id: call_id.clone(),
                name,
                ordinal,
            });
        }
        let chunk = self.calls.take_emittable(ordinal);
        if !chunk.is_empty()
            && let Some(call_id) = self.calls.call_id_at(ordinal)
        {
            out.push(ChatDelta::ToolInputDelta {
                call_id: call_id.to_string(),
                chunk,
            });
        }
        Ok(())
    }

    /// 收到 `finish_reason`：闭合工具并发出 `ResponseFinished`。
    fn finish_response(&mut self, reason: &str) -> Result<Vec<ChatDelta>, String> {
        let mut out = Vec::new();
        if self.response_finished {
            // 重复 finish_reason 帧（部分兼容端点会重发）：幂等忽略。
            return Ok(out);
        }
        match reason {
            "tool_calls" | "function_call" => {
                // 先闭合所有调用：参数不完整/非法属于协议错误，绝不执行半截参数。
                let ended = self.calls.end_all_open()?;
                if ended.is_empty() {
                    return Err(protocol_error(format!(
                        "finish_reason={reason} without any tool call"
                    )));
                }
                for (call_id, arguments_json) in ended {
                    validate_arguments(&arguments_json)?;
                    out.push(ChatDelta::ToolInputEnded {
                        call_id,
                        arguments_json,
                    });
                }
                out.push(ChatDelta::ResponseFinished {
                    stop: ModelStepStop::ToolCalls,
                });
                // `finish_reason` 已声明本次响应流结束：即使端点没补 `[DONE]`，
                // 也不应把整轮判成 "stream ended without completion"。
                self.finished = true;
            }
            "stop" | "length" | "content_filter" => {
                if self.calls.has_calls() {
                    // 端点先发过工具分片却又以 stop 结束：协议不一致，不猜执行。
                    return Err(protocol_error(format!(
                        "finish_reason={reason} after tool call fragments"
                    )));
                }
                out.push(ChatDelta::ResponseFinished {
                    stop: ModelStepStop::Final,
                });
                self.finished = true;
                out.push(ChatDelta::Finished {
                    reason: StopReason::Complete,
                });
            }
            other => {
                return Err(protocol_error(format!("unknown finish_reason: {other}")));
            }
        }
        self.response_finished = true;
        Ok(out)
    }
}

/// 是否真的带了**已废弃的** `delta.function_call` 调用体。
///
/// 兼容端点的占位形态比想象的多：`null`、`{}`、`{"name":"","arguments":""}`
/// （空字符串占位）、`"tool_calls": []` 等。它们都表示“本帧没有函数调用”。
/// `Value::is_some()` 对 `Value::Null` 为真，`as_str()` 对空字符串为 `Some("")`，
/// 只按“字段存在”判断会把普通对话帧误判成废弃协议，整轮失败
/// （用户实测：`你是谁` 这类纯聊天也失败）。
///
/// 只有 `name` 或 `arguments` 是**非空字符串**时，才算旧式函数调用体。
fn has_deprecated_function_call(delta: &Value) -> bool {
    let Some(call) = delta.get("function_call").and_then(Value::as_object) else {
        return false;
    };
    ["name", "arguments"].iter().any(|key| {
        call.get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    })
}

/// 是否为纯占位的 `tool_calls` 条目（无 index/id/name/arguments 任何有效值）。
///
/// 部分兼容网关在没有工具调用时也会回 `"tool_calls":[{}]` 或
/// `{"id":"","function":{"name":"","arguments":""}}` 之类的空壳，不能据此报协议错误。
fn is_placeholder_tool_call(call: &Value) -> bool {
    let non_empty_string = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    };
    if call.get("index").and_then(Value::as_u64).is_some() {
        return false;
    }
    let function = call.get("function");
    !non_empty_string(call.get("id"))
        && !non_empty_string(function.and_then(|function| function.get("name")))
        && !non_empty_string(function.and_then(|function| function.get("arguments")))
}

/// 校验完整参数是合法 JSON（`ToolInputEnded` 之前必须成立）。
fn validate_arguments(arguments_json: &str) -> Result<(), String> {
    serde_json::from_str::<Value>(arguments_json)
        .map(|_| ())
        .map_err(|_| format!("{TOOL_ARGUMENTS_INVALID_CODE}: malformed tool arguments"))
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

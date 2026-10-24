//! Anthropic Messages API 适配。
//!
//! `POST /v1/messages`，协议 header（`x-api-key` + `anthropic-version`），
//! `max_tokens` 必填；thinking 经 `content_block_delta.thinking_delta` 独立 part，
//! `signature_delta` 以 `content_block_index` 绑定对应 thinking part 元数据
//! （即使该帧无文本也必须更新）；完成判定 `message_stop`。
//!
//! 工具化字段（来源：Claude Messages API 流式事件与 tool use 文档）：
//! - 请求 `tools: [{"name","description","input_schema"}]`，系统指令走顶层 `system`；
//! - `content_block_start` 且 `content_block.type=="tool_use"` →
//!   `ToolInputStarted{call_id: block.id, name: block.name, ordinal: index}`；
//! - `content_block_delta` 且 `delta.type=="input_json_delta"` →
//!   `ToolInputDelta{chunk: delta.partial_json}`（按 `index` 绑定，不与其他 block 混淆）；
//! - `content_block_stop` → `ToolInputEnded`；
//! - `message_delta.delta.stop_reason=="tool_use"` → `ResponseFinished{ToolCalls}`，
//!   `"end_turn"` → `ResponseFinished{Final}`；
//! - `message_stop` → `Finished{Complete}`；
//! - thinking block 结束时发 `ReasoningEnded{item_id: index}`；
//! - 续传：assistant `tool_use` block 与 user `tool_result` block 相邻，`is_error` 透传。

use serde_json::{Value, json};

use super::super::transport::tools::{
    HistoryBlock, ToolCallBuffer, anthropic_tools, arguments_string, history_blocks, protocol_error,
};
use super::{
    StreamDecoder, TransportRequest, join_url, json_i64, json_str, reasoning_delta,
    segments_to_deltas,
};
use crate::chat::events::{
    ChatDelta, ChatRequest, ModelStepStop, PartDelta, PartKind, StopReason,
    TOOL_ARGUMENTS_INVALID_CODE,
};
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
    let messages = messages_of(request)?;
    let mut body = json!({
        "model": request.model.model,
        // 系统指令：Anthropic 用顶层 system，不进 messages。
        "system": request.resolve_system_prompt(),
        "messages": messages,
        "max_tokens": DEFAULT_MAX_TOKENS,
        "stream": true,
    });
    // 摘要请求不声明工具（无工具就无从发起调用）。
    if request.includes_tools() {
        body["tools"] = anthropic_tools();
    }
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
        tools_requested: request.includes_tools(),
    })
}

/// 编码 `messages`：`items` 非空时把同一 assistant 的文本与 tool_use 合进一条消息的
/// content block 数组，工具结果作为紧随其后的 user 消息（tool_result block）。
fn messages_of(request: &ChatRequest) -> Result<Vec<Value>, String> {
    if request.items.is_empty() {
        return Ok(request
            .turns
            .iter()
            .map(|turn| json!({"role": turn.role, "content": turn.text}))
            .collect());
    }
    let mut messages: Vec<Value> = Vec::new();
    for block in history_blocks(&request.items)? {
        match block {
            HistoryBlock::Text { role, text } => {
                push_block(
                    &mut messages,
                    role.as_str(),
                    json!({"type": "text", "text": text}),
                );
            }
            HistoryBlock::ToolCall {
                call_id,
                name,
                input,
            } => {
                push_block(
                    &mut messages,
                    "assistant",
                    json!({
                        "type": "tool_use",
                        "id": call_id,
                        "name": name,
                        // tool_use.input 是 JSON 对象（与 Chat 的 arguments 字符串不同）。
                        "input": input,
                    }),
                );
            }
            HistoryBlock::ToolResult {
                call_id,
                content,
                is_error,
                ..
            } => {
                // 工具结果进 user 消息（Anthropic 只有 user/assistant 两种角色）。
                push_block(
                    &mut messages,
                    "user",
                    json!({
                        "type": "tool_result",
                        "tool_use_id": call_id,
                        "content": content,
                        "is_error": is_error,
                    }),
                );
            }
        }
    }
    Ok(messages)
}

/// 追加一个 content block：相邻同 role 时并入同一条消息的 content 数组。
fn push_block(messages: &mut Vec<Value>, role: &str, block: Value) {
    let same_role = messages
        .last()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        == Some(role);
    if same_role
        && let Some(content) = messages
            .last_mut()
            .and_then(|message| message.get_mut("content"))
            .and_then(Value::as_array_mut)
    {
        content.push(block);
        return;
    }
    messages.push(json!({"role": role, "content": [block]}));
}

/// Messages SSE 解码器。
#[derive(Debug, Default)]
pub struct Decoder {
    splitter: TagSplitter,
    finished: bool,
    /// content_block_index -> part 顺序号（signature 绑定用）。
    block_positions: Vec<Option<usize>>,
    /// 跨帧工具参数缓冲（按 content block index 聚合；每响应独立实例）。
    calls: ToolCallBuffer,
    /// 仍在接收内容的 block index -> block 类型（`tool_use` / `thinking` / `text`）。
    blocks: Vec<Option<BlockKind>>,
    /// 本响应是否已发出 `ResponseFinished`（防重复）。
    response_finished: bool,
}

/// content block 的类型（只区分本解码器需要特殊处理的三种）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
    ToolUse,
}

impl Decoder {
    /// 记录一个 content block 的 part 位置。
    fn bind_block(&mut self, index: usize, position: usize) {
        while self.block_positions.len() <= index {
            self.block_positions.push(None);
        }
        self.block_positions[index] = Some(position);
    }

    /// 记录 block 类型。
    fn bind_kind(&mut self, index: usize, kind: BlockKind) {
        while self.blocks.len() <= index {
            self.blocks.push(None);
        }
        self.blocks[index] = Some(kind);
    }

    /// 取 block 类型（未知时为 None）。
    fn kind_at(&self, index: usize) -> Option<BlockKind> {
        self.blocks.get(index).copied().flatten()
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
                let block = value.get("content_block");
                let block_type = block
                    .and_then(|block| block.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match block_type {
                    "tool_use" => {
                        let call_id = block
                            .and_then(|block| block.get("id"))
                            .and_then(Value::as_str)
                            .filter(|id| !id.is_empty())
                            .ok_or_else(|| protocol_error("tool_use block without id"))?;
                        let name = block
                            .and_then(|block| block.get("name"))
                            .and_then(Value::as_str)
                            .filter(|name| !name.is_empty())
                            .ok_or_else(|| protocol_error("tool_use block without name"))?;
                        self.bind_kind(index, BlockKind::ToolUse);
                        self.bind_block(index, index);
                        self.calls.identify(index, call_id, name)?;
                        if let Some((call_id, name)) = self.calls.take_start(index) {
                            out.push(ChatDelta::ToolInputStarted {
                                call_id,
                                name,
                                ordinal: index,
                            });
                        }
                        // 部分 block 直接在 start 里给完整 input（非流式参数）。
                        if let Some(input) = block.and_then(|block| block.get("input"))
                            && input.as_object().is_some_and(|object| !object.is_empty())
                        {
                            let serialized = arguments_string(input)?;
                            self.calls.push_delta(index, &serialized)?;
                            self.flush_delta(index, &mut out);
                        }
                    }
                    "thinking" => {
                        self.bind_kind(index, BlockKind::Thinking);
                        self.bind_block(index, index);
                    }
                    // 供应商托管工具（如服务端 web 搜索）不得当成本地文件工具执行。
                    "server_tool_use" | "web_search_tool_result" | "code_execution_tool_result" => {
                        return Err(protocol_error(format!(
                            "hosted tool block is not supported locally: {block_type}"
                        )));
                    }
                    _ => {
                        self.bind_kind(index, BlockKind::Text);
                        self.bind_block(index, index);
                    }
                }
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
                            // 工具输入增量：按 block index 绑定，绝不与其他 block 交错混淆。
                            if self.kind_at(index) != Some(BlockKind::ToolUse) {
                                return Err(protocol_error(format!(
                                    "input_json_delta on non tool_use block {index}"
                                )));
                            }
                            let partial = delta
                                .get("partial_json")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            self.calls.push_delta(index, partial)?;
                            self.flush_delta(index, &mut out);
                        }
                        _ => {}
                    }
                }
            }
            "content_block_stop" => {
                let index = json_i64(&value, "index").unwrap_or(0).max(0) as usize;
                match self.kind_at(index) {
                    Some(BlockKind::ToolUse) => {
                        if let Some((call_id, arguments_json)) = self.calls.end(index) {
                            validate_arguments(&arguments_json)?;
                            out.push(ChatDelta::ToolInputEnded {
                                call_id,
                                arguments_json,
                            });
                        }
                    }
                    Some(BlockKind::Thinking) => {
                        // thinking block 结束：显式关闭对应思考块（item_id = block index）。
                        out.push(ChatDelta::ReasoningEnded {
                            item_id: Some(index.to_string()),
                        });
                    }
                    _ => {}
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
                if let Some(reason) = value
                    .get("delta")
                    .and_then(|delta| delta.get("stop_reason"))
                    .and_then(Value::as_str)
                    .filter(|reason| !reason.is_empty())
                {
                    tracing::trace!(target: "lx::chat", stop_reason = reason, "tool response stop");
                    out.extend(self.finish_response(reason)?);
                }
            }
            "message_stop" => {
                if self.calls.has_open() {
                    return Err(protocol_error("message_stop with unclosed tool_use block"));
                }
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
            "ping" | "message_start_ignored" => {}
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
    /// 把缓冲里尚未发出的参数片段作为 `ToolInputDelta` 推出。
    fn flush_delta(&mut self, index: usize, out: &mut Vec<ChatDelta>) {
        let chunk = self.calls.take_emittable(index);
        if chunk.is_empty() {
            return;
        }
        if let Some(call_id) = self.calls.call_id_at(index) {
            out.push(ChatDelta::ToolInputDelta {
                call_id: call_id.to_string(),
                chunk,
            });
        }
    }

    /// `message_delta.stop_reason`：工具阶段和正常完成分开。
    fn finish_response(&mut self, reason: &str) -> Result<Vec<ChatDelta>, String> {
        let mut out = Vec::new();
        if self.response_finished {
            return Ok(out);
        }
        let stop = match reason {
            "tool_use" => ModelStepStop::ToolCalls,
            "end_turn" | "stop_sequence" | "max_tokens" => ModelStepStop::Final,
            other => {
                return Err(protocol_error(format!("unknown stop_reason: {other}")));
            }
        };
        if stop == ModelStepStop::ToolCalls && self.calls.has_open() {
            // 工具阶段仍有未闭合 block：参数不完整，不得执行。
            return Err(protocol_error(
                "stop_reason=tool_use with unclosed tool_use block",
            ));
        }
        out.push(ChatDelta::ResponseFinished { stop });
        self.response_finished = true;
        Ok(out)
    }
}

/// 校验完整参数是合法 JSON 对象。
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

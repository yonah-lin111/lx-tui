//! Google Gemini 适配。
//!
//! `models/{model}:streamGenerateContent?alt=sse`，`contents` 数组 + `generationConfig`；
//! Key 只按该协议放置在查询参数（`?key=`）；`thought=true` 的 part 独立为
//! reasoning；usage 来自 `usageMetadata`。
//!
//! 工具化字段（来源：Gemini API function calling / streamGenerateContent 文档）：
//! - 请求 `tools: [{"functionDeclarations":[{name,description,parameters}]}]`，
//!   系统指令走 `systemInstruction.parts[0].text`；
//! - 解码：candidate `parts[].functionCall{name,args}` → `ToolInputStarted` +
//!   `ToolInputEnded`（args 已是**完整对象**，序列化为 JSON 字符串）；
//! - 续传：model `functionCall` part 与 user `functionResponse` part 相邻；
//! - `finishReason=="STOP"` 时若本轮有 functionCall → `ResponseFinished{ToolCalls}`，
//!   否则 `Final`；随后 `Finished{Complete}`。
//!
//! 两个 Gemini 特有约束：
//! 1. 流帧可能给**累计候选快照**：同一 `(candidate index, part index, name, args)`
//!    重复出现只执行一次（[`ToolCallBuffer`] 按 ordinal 去重）。
//! 2. 协议没有原生 call ID：内部合成
//!    `gemini-{response seq}-{candidate}-{part}-{name}`，只用在本进程的去重与 DB
//!    主键上，**绝不**写进发送给供应商的字段（`functionResponse` 只带 `name`，
//!    不带伪造 ID）。`response seq` 来自 [`super::next_response_seq`]：解码器实例
//!    按**一次模型响应**创建，同一用户轮次的第 2 次及之后的采样必须换一个序号，
//!    否则同名调用会与上一步的 call ID 相同，撞上 `UNIQUE(session_id, call_id)`。

use serde_json::{Value, json};

use super::super::transport::tools::{
    HistoryBlock, Role, ToolCallBuffer, arguments_string, google_tools, history_blocks,
    protocol_error,
};
use super::{
    StreamDecoder, TransportRequest, json_i64, json_str, reasoning_delta, segments_to_deltas,
};
use crate::chat::events::{
    ChatDelta, ChatRequest, ModelStepStop, StopReason, TOOL_ARGUMENTS_INVALID_CODE,
};
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
    let contents = contents_of(request)?;
    // 系统指令：只有首段进 `systemInstruction`；其余段改为前置 user content
    // （形状见 [`super::system_messages`]：代理会把多 part 拼回一条再按 2000 判长度）。
    let (head, rest) = super::system_messages(&request.resolve_system_prompt());
    let mut contents = contents;
    if !rest.is_empty() {
        let mut prefixed: Vec<Value> = rest
            .into_iter()
            .map(|chunk| {
                json!({
                    "role": "user",
                    "parts": [{"text": super::wrap_system_instructions(&chunk)}],
                })
            })
            .collect();
        prefixed.extend(contents);
        contents = prefixed;
    }
    let mut body = json!({
        "contents": contents,
        "systemInstruction": {"parts": [{"text": head}]},
    });
    // 摘要请求不声明工具（无工具就无从发起调用）。
    if request.includes_tools() {
        body["tools"] = google_tools();
    }
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
        tools_requested: request.includes_tools(),
    })
}

/// 编码 `contents`：`items` 非空时按 call/result 成对编码，否则沿用纯文本 `turns`。
fn contents_of(request: &ChatRequest) -> Result<Vec<Value>, String> {
    if request.items.is_empty() {
        // contents：role user/model + parts[{text}]；assistant 历史 role 为 model。
        return Ok(request
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
            .collect());
    }
    let mut contents: Vec<Value> = Vec::new();
    for block in history_blocks(&request.items)? {
        match block {
            HistoryBlock::Text { role, text } => {
                push_parts(&mut contents, google_role(role), json!({"text": text}));
            }
            HistoryBlock::ToolCall {
                call_id: _,
                name,
                input,
            } => {
                // model functionCall part：args 是 JSON 对象。
                // 内部合成的 call ID 不写进供应商字段（Gemini 无原生 ID）。
                push_parts(
                    &mut contents,
                    "model",
                    json!({"functionCall": {"name": name, "args": input}}),
                );
            }
            HistoryBlock::ToolResult {
                call_id: _,
                name,
                content,
                is_error,
            } => {
                // user functionResponse part：`response` 包装携带结构化结果。
                // 失败结果用 `error` 键显式标注（不与成功结果混淆）。
                let response = if is_error {
                    json!({"error": content})
                } else {
                    json!({"result": content})
                };
                push_parts(
                    &mut contents,
                    "user",
                    json!({"functionResponse": {"name": name, "response": response}}),
                );
            }
        }
    }
    Ok(contents)
}

/// 历史角色映射：Gemini 用 `model` 表示 assistant。
fn google_role(role: Role) -> &'static str {
    match role {
        Role::Assistant => "model",
        Role::User => "user",
    }
}

/// 追加 parts：相邻同 role 的 content 合并为一帧的 parts 数组。
fn push_parts(contents: &mut Vec<Value>, role: &str, part: Value) {
    let same_role = contents
        .last()
        .and_then(|content| content.get("role"))
        .and_then(Value::as_str)
        == Some(role);
    if same_role
        && let Some(parts) = contents
            .last_mut()
            .and_then(|content| content.get_mut("parts"))
            .and_then(Value::as_array_mut)
    {
        parts.push(part);
        return;
    }
    contents.push(json!({"role": role, "parts": [part]}));
}

/// Gemini SSE 解码器：每帧是一个完整 JSON（非事件流），含 candidates parts。
#[derive(Debug, Default)]
pub struct Decoder {
    splitter: TagSplitter,
    finished: bool,
    /// 本次模型响应的进程内唯一序号（并进合成 call ID，保证跨响应唯一）。
    response_seq: u64,
    /// 跨帧工具缓冲（按内部 ordinal 去重；每响应独立实例）。
    calls: ToolCallBuffer,
    /// `(candidate index, part index) -> 内部 ordinal`，保证同一 part 只有一条调用。
    part_slots: Vec<(usize, usize, usize)>,
    /// 已去重登记的 `(candidate, part, name, args)` 快照键。
    seen: Vec<(usize, usize, String, String)>,
    /// 本响应是否已发出 `ResponseFinished`（防重复）。
    response_finished: bool,
}

impl Decoder {
    /// 按响应序号构造解码器（ID 只在本次响应内稳定，跨响应不同）。
    pub fn new(response_seq: u64) -> Self {
        Self {
            response_seq,
            ..Self::default()
        }
    }

    /// 取或创建 `(candidate, part)` 对应的内部 ordinal。
    fn slot_of(&mut self, candidate: usize, part: usize) -> usize {
        if let Some((_, _, ordinal)) = self
            .part_slots
            .iter()
            .find(|(c, p, _)| *c == candidate && *p == part)
        {
            return *ordinal;
        }
        let ordinal = self.part_slots.len();
        self.part_slots.push((candidate, part, ordinal));
        ordinal
    }

    /// 处理一个 `functionCall` part：去重后一次性发出 Started + Ended。
    fn absorb_function_call(
        &mut self,
        candidate: usize,
        part: usize,
        call: &Value,
        out: &mut Vec<ChatDelta>,
    ) -> Result<(), String> {
        let name = call
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| protocol_error("functionCall without name"))?;
        // Gemini 的 `functionCall.args` 是 JSON 对象；缺省按空对象处理，非对象即协议错误。
        let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
        if !args.is_object() {
            return Err(format!(
                "{TOOL_ARGUMENTS_INVALID_CODE}: functionCall args of {name} are not a JSON object"
            ));
        }
        let arguments_json = arguments_string(&args)?;
        let key = (candidate, part, name.to_string(), arguments_json.clone());
        // 累计快照重复给同一 functionCall：只执行一次。
        if self.seen.contains(&key) {
            return Ok(());
        }
        self.seen.push(key);
        // 合成稳定内部 call ID：(response seq, candidate, part, name)，不含时间戳。
        // 响应序号保证同一轮第 2 次采样不会与第 1 次同名调用撞 ID。
        let ordinal = self.slot_of(candidate, part);
        let synthesized_id = format!("gemini-{}-{candidate}-{part}-{name}", self.response_seq);
        let Some((call_id, arguments_json)) =
            self.calls
                .start_complete(ordinal, &synthesized_id, name, &arguments_json)?
        else {
            return Ok(());
        };
        validate_arguments(&arguments_json)?;
        out.push(ChatDelta::ToolInputStarted {
            call_id: call_id.clone(),
            name: name.to_string(),
            ordinal,
        });
        out.push(ChatDelta::ToolInputEnded {
            call_id,
            arguments_json,
        });
        Ok(())
    }

    /// `finishReason` 到达：有 functionCall 则工具阶段，否则正常完成。
    fn finish_response(&mut self) -> Vec<ChatDelta> {
        let mut out = Vec::new();
        if self.response_finished {
            return out;
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
        out
    }
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
            for (candidate_index, candidate) in candidates.iter().enumerate() {
                let parts = candidate
                    .get("content")
                    .and_then(|content| content.get("parts"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for (part_index, part) in parts.iter().enumerate() {
                    // 只有**非 null 对象**才是 functionCall 调用体；部分端点/网关会在
                    // 纯文本 part 里带 `"functionCall": null` 占位，不能据此报协议错误。
                    if let Some(call) = part.get("functionCall").filter(|call| call.is_object()) {
                        self.absorb_function_call(candidate_index, part_index, call, &mut out)?;
                        continue;
                    }
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
                // finishReason：STOP/SAFETY/RECITATION 视为本次响应结束。
                if let Some(reason) = json_str(candidate, "finishReason")
                    && matches!(reason, "STOP" | "SAFETY" | "RECITATION" | "MAX_TOKENS")
                {
                    out.extend(self.finish_response());
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

    fn has_open_tool_input(&self) -> bool {
        self.calls.has_open()
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

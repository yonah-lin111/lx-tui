//! 四类协议共用的工具请求定义、历史归并与跨帧参数缓冲。
//!
//! 三个职责，都不依赖具体协议：
//!
//! 1. **请求侧**：把 [`crate::chat::tools::available_tools()`] 映射为四种 wire 形状
//!    （Chat Completions 的 `function` 包装、Responses 的扁平函数项、Anthropic 的
//!    `input_schema`、Gemini 的 `functionDeclarations`）。
//! 2. **历史归并**：把 [`ModelItem`] 序列归并成 [`HistoryBlock`]，四类协议再各自编码，
//!    避免四份几乎相同的分组逻辑漂移。
//! 3. **解码侧**：每个解码器各持一个 [`ToolCallBuffer`]，按**响应内顺序号**（Chat 的
//!    `tool_calls[].index`、Responses 的 `output_index`、Anthropic 的 content block
//!    index、Gemini 的 part index）聚合跨帧参数。禁止用全局单一缓冲。
//!
//! 稳定错误码（写进 `StopReason::Failed{code}`，不含密钥与原始响应）：
//! [`TOOLS_UNSUPPORTED_CODE`]、[`TOOL_PROTOCOL_ERROR_CODE`]、[`TOOL_INPUT_TRUNCATED_CODE`]；
//! 参数超限复用 [`crate::chat::events::TOOL_ARGUMENTS_TOO_LARGE_CODE`]。

use serde_json::{Value, json};

use crate::chat::agent::ToolSurface;
use crate::chat::events::{
    MAX_TOOL_ARGUMENTS_BYTES, ModelItem, TOOL_ARGUMENTS_INVALID_CODE, TOOL_ARGUMENTS_TOO_LARGE_CODE,
};
use crate::chat::tools::tools_for;

/// 供应商/兼容端点拒绝 `tools` 字段时的稳定错误码前缀。
///
/// 只用于明确的“能力不支持”判定：认证失败、限流、超时仍按原 HTTP 错误码处理，
/// 绝不因为收到 4xx 就降级为纯文本请求重试。
pub const TOOLS_UNSUPPORTED_CODE: &str = "tool calls not supported by endpoint";

/// 工具调用本身违反协议（缺 call ID、同 ID 冲突、hosted 工具、参数指向未知调用等）。
pub const TOOL_PROTOCOL_ERROR_CODE: &str = "tool call protocol error";

/// 流结束（EOF/中断）时工具参数仍未闭合：半截 JSON 不得执行。
pub const TOOL_INPUT_TRUNCATED_CODE: &str = "tool input truncated";

/// 构造一条带稳定错误码前缀的协议错误消息。
pub(crate) fn protocol_error(detail: impl std::fmt::Display) -> String {
    format!("{TOOL_PROTOCOL_ERROR_CODE}: {detail}")
}

/// OpenAI Chat Completions / compatible 的 `tools` 字段。
///
/// wire 形状（Chat Completions 请求体的 `tools`）：
/// `[{"type":"function","function":{"name","description","parameters"}}]`。
pub(crate) fn openai_chat_tools(surface: ToolSurface) -> Value {
    Value::Array(
        tools_for(surface)
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters(),
                    }
                })
            })
            .collect(),
    )
}

/// OpenAI Responses 的 `tools` 字段（函数项是**扁平**的，不套 `function`）。
///
/// wire 形状：`[{"type":"function","name","description","parameters"}]`。
pub(crate) fn openai_responses_tools(surface: ToolSurface) -> Value {
    Value::Array(
        tools_for(surface)
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters(),
                })
            })
            .collect(),
    )
}

/// Anthropic Messages 的 `tools` 字段：`[{"name","description","input_schema"}]`。
pub(crate) fn anthropic_tools(surface: ToolSurface) -> Value {
    Value::Array(
        tools_for(surface)
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters(),
                })
            })
            .collect(),
    )
}

/// Gemini 的 `tools` 字段：`[{"functionDeclarations":[{name,description,parameters}]}]`。
pub(crate) fn google_tools(surface: ToolSurface) -> Value {
    let declarations: Vec<Value> = tools_for(surface)
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters(),
            })
        })
        .collect();
    json!([{ "functionDeclarations": declarations }])
}

/// HTTP 状态码是否属于“该端点可能不支持工具调用”的能力类失败。
///
/// 401/403（认证）、408（超时）、429（限流）、5xx（服务端故障）不在此列，
/// 它们必须保持原有错误语义与重试规则。
pub fn is_tools_unsupported_status(status: u16) -> bool {
    matches!(status, 400 | 404 | 405 | 409 | 415 | 422 | 501)
}

/// 历史消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    User,
    Assistant,
}

impl Role {
    /// wire 通用角色名（Google 的 model 映射由该协议自行处理）。
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// 一条待编码的历史块（协议无关）：文本或一次工具调用/结果。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HistoryBlock {
    /// 纯文本（user 或 assistant 可见正文）。
    Text { role: Role, text: String },
    /// 用户多模态内容（文本 + 图片）。
    UserContent {
        text: String,
        images: Vec<crate::chat::events::ModelImage>,
    },
    /// assistant 发起的一次工具调用（`input` 已解析为 JSON 值）。
    ToolCall {
        call_id: String,
        name: String,
        input: Value,
    },
    /// 对应 `call_id` 的工具结果；`name` 由前面同 `call_id` 的调用解析得到
    /// （Gemini 的 `functionResponse` 必须带工具名，其他协议忽略此字段）。
    ToolResult {
        call_id: String,
        name: String,
        content: String,
        is_error: bool,
    },
}

/// 把结构化历史 `items` 归并成 [`HistoryBlock`] 序列（保持原始顺序）。
///
/// 校验：`input` 必须是 JSON 对象（四个工具的参数都是 object）；工具结果必须能对应到
/// 之前出现过的同 `call_id` 调用（禁止 orphan result 进请求）。
pub(crate) fn history_blocks(items: &[ModelItem]) -> Result<Vec<HistoryBlock>, String> {
    let mut blocks: Vec<HistoryBlock> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            ModelItem::UserText(text) => blocks.push(HistoryBlock::Text {
                role: Role::User,
                text: text.clone(),
            }),
            ModelItem::UserContent { text, images } => blocks.push(HistoryBlock::UserContent {
                text: text.clone(),
                images: images.clone(),
            }),
            ModelItem::AssistantText(text) => blocks.push(HistoryBlock::Text {
                role: Role::Assistant,
                text: text.clone(),
            }),
            ModelItem::AssistantToolCall {
                call_id,
                name,
                input,
            } => {
                if call_id.is_empty() || name.is_empty() {
                    return Err(protocol_error("tool call without call id or name"));
                }
                if !input.is_object() {
                    return Err(format!(
                        "{TOOL_ARGUMENTS_INVALID_CODE}: arguments of {name} are not a JSON object"
                    ));
                }
                blocks.push(HistoryBlock::ToolCall {
                    call_id: call_id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }
            ModelItem::ToolResult {
                call_id,
                content,
                is_error,
            } => {
                // 结果必须能对应到之前出现过的同 call_id 调用（禁止 orphan result），
                // 并解析出工具名供 Gemini 的 functionResponse 使用。
                let name = blocks.iter().rev().find_map(|block| match block {
                    HistoryBlock::ToolCall {
                        call_id: id, name, ..
                    } if id == call_id => Some(name.clone()),
                    _ => None,
                });
                let Some(name) = name else {
                    return Err(protocol_error(format!(
                        "tool result without a preceding call: {call_id}"
                    )));
                };
                blocks.push(HistoryBlock::ToolResult {
                    call_id: call_id.clone(),
                    name,
                    content: content.clone(),
                    is_error: *is_error,
                });
            }
        }
    }
    Ok(blocks)
}

/// 把已解析的 JSON 值序列化为 wire 用的 JSON 字符串（**只序列化一次**）。
pub(crate) fn arguments_string(input: &Value) -> Result<String, String> {
    serde_json::to_string(input).map_err(|error| format!("encode tool arguments: {error}"))
}

/// 一个正在接收参数的工具调用（跨帧状态，随解码器实例存活）。
#[derive(Debug, Clone, PartialEq)]
struct ToolCallEntry {
    /// 响应内顺序号（供应商原生位置）。
    ordinal: usize,
    /// 供应商原生 call ID；Chat 的首片之前可能尚未到达。
    call_id: Option<String>,
    /// 工具名；同样可能晚于首个参数片到达（部分兼容端点如此）。
    name: Option<String>,
    /// 已累计的参数片段（原始字符串，未解析）。
    arguments: String,
    /// 已作为 `ToolInputDelta` 发出的参数字节数（Started 之前到达的片段延后发出）。
    emitted: usize,
    /// 是否已发出 `ToolInputStarted`。
    started: bool,
    /// 是否已发出 `ToolInputEnded`。
    ended: bool,
}

/// 解码器的工具参数缓冲：按 ordinal 聚合，限制参数总字节数，检测同 ID 冲突。
#[derive(Debug, Default)]
pub(crate) struct ToolCallBuffer {
    entries: Vec<ToolCallEntry>,
}

impl ToolCallBuffer {
    /// 按顺序号取（或创建）一个调用槽位。
    fn slot(&mut self, ordinal: usize) -> &mut ToolCallEntry {
        if let Some(position) = self
            .entries
            .iter()
            .position(|entry| entry.ordinal == ordinal)
        {
            return &mut self.entries[position];
        }
        self.entries.push(ToolCallEntry {
            ordinal,
            call_id: None,
            name: None,
            arguments: String::new(),
            emitted: 0,
            started: false,
            ended: false,
        });
        let last = self.entries.len() - 1;
        &mut self.entries[last]
    }

    /// 绑定 ordinal 的 call ID（同 ID 不得出现在另一个 ordinal）。
    pub(crate) fn bind_call_id(&mut self, ordinal: usize, call_id: &str) -> Result<(), String> {
        if let Some(other) = self
            .entries
            .iter()
            .find(|entry| entry.ordinal != ordinal && entry.call_id.as_deref() == Some(call_id))
        {
            return Err(protocol_error(format!(
                "call id {call_id} appears at ordinal {} and {ordinal}",
                other.ordinal
            )));
        }
        let entry = self.slot(ordinal);
        if let Some(existing) = entry.call_id.as_deref()
            && existing != call_id
        {
            return Err(protocol_error(format!(
                "ordinal {ordinal} changed call id {existing} -> {call_id}"
            )));
        }
        entry.call_id = Some(call_id.to_string());
        Ok(())
    }

    /// 绑定 ordinal 的 name（同 ordinal 不得换名）。
    pub(crate) fn bind_name(&mut self, ordinal: usize, name: &str) -> Result<(), String> {
        let entry = self.slot(ordinal);
        if let Some(existing) = entry.name.as_deref()
            && existing != name
        {
            return Err(protocol_error(format!(
                "ordinal {ordinal} changed name {existing} -> {name}"
            )));
        }
        entry.name = Some(name.to_string());
        Ok(())
    }

    /// 一次性绑定 call ID 与 name（供应商在一帧内给齐的常规路径）。
    pub(crate) fn identify(
        &mut self,
        ordinal: usize,
        call_id: &str,
        name: &str,
    ) -> Result<(), String> {
        self.bind_call_id(ordinal, call_id)?;
        self.bind_name(ordinal, name)
    }

    /// call ID 与 name 齐备且尚未发过 Started 时返回 `(call_id, name)` 并把该调用标为已开始。
    ///
    /// 返回 `None` 表示还缺信息（或已开始过）：调用方不得发出 `ToolInputStarted`。
    pub(crate) fn take_start(&mut self, ordinal: usize) -> Option<(String, String)> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.ordinal == ordinal)?;
        if entry.started {
            return None;
        }
        let call_id = entry.call_id.clone()?;
        let name = entry.name.clone()?;
        entry.started = true;
        Some((call_id, name))
    }

    /// 取走尚未发出的参数片段（Started 之前累计的部分在此一次性补发）。
    ///
    /// 返回空串表示没有新增可发内容。
    pub(crate) fn take_emittable(&mut self, ordinal: usize) -> String {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.ordinal == ordinal)
        else {
            return String::new();
        };
        if !entry.started || entry.emitted >= entry.arguments.len() {
            return String::new();
        }
        let chunk = entry.arguments[entry.emitted..].to_string();
        entry.emitted = entry.arguments.len();
        chunk
    }

    /// 解析 ordinal 已绑定的 call ID（Anthropic 的 `input_json_delta` 走这条路径）。
    pub(crate) fn call_id_at(&self, ordinal: usize) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.ordinal == ordinal)
            .and_then(|entry| entry.call_id.as_deref())
    }

    /// 追加参数片段；累计超过 [`MAX_TOOL_ARGUMENTS_BYTES`] 立即失败。
    ///
    /// 参数可能早于 call ID/name 到达（部分兼容端点如此），因此这里按 ordinal 建槽：
    /// 真正可否执行由“call ID 齐备 + 参数闭合”共同判定，缺 ID 的槽位在收尾时是协议错误。
    pub(crate) fn push_delta(&mut self, ordinal: usize, chunk: &str) -> Result<(), String> {
        let entry = self.slot(ordinal);
        if entry.ended {
            // 参数已闭合后的迟到片段：不重复累计、不报错（幂等丢弃）。
            return Ok(());
        }
        let projected = entry.arguments.len().saturating_add(chunk.len());
        if projected > MAX_TOOL_ARGUMENTS_BYTES {
            return Err(TOOL_ARGUMENTS_TOO_LARGE_CODE.to_string());
        }
        entry.arguments.push_str(chunk);
        Ok(())
    }

    /// 用协议给出的权威完整参数替换已累计内容（Responses 的 `arguments.done`）。
    pub(crate) fn replace_arguments(
        &mut self,
        ordinal: usize,
        arguments: &str,
    ) -> Result<(), String> {
        if arguments.len() > MAX_TOOL_ARGUMENTS_BYTES {
            return Err(TOOL_ARGUMENTS_TOO_LARGE_CODE.to_string());
        }
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.ordinal == ordinal)
        else {
            return Err(protocol_error(format!(
                "arguments for unknown ordinal {ordinal}"
            )));
        };
        entry.arguments = arguments.to_string();
        entry.emitted = arguments.len();
        Ok(())
    }

    /// 结束一个调用：返回 `(call_id, arguments_json)`；已结束时返回 `None`。
    ///
    /// 参数为空串时补成 `{}`（无参调用的合法空对象），避免把“没有参数”误判成半截 JSON。
    pub(crate) fn end(&mut self, ordinal: usize) -> Option<(String, String)> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.ordinal == ordinal)?;
        if entry.ended {
            return None;
        }
        let call_id = entry.call_id.clone()?;
        entry.ended = true;
        let arguments = if entry.arguments.is_empty() {
            "{}".to_string()
        } else {
            std::mem::take(&mut entry.arguments)
        };
        Some((call_id, arguments))
    }

    /// 以完整参数一次性登记并结束（Gemini 的 `functionCall.args` 已是完整对象）。
    ///
    /// 已存在同 ordinal 的调用时返回 `None`（重复快照去重，由调用方负责）。
    pub(crate) fn start_complete(
        &mut self,
        ordinal: usize,
        call_id: &str,
        name: &str,
        arguments_json: &str,
    ) -> Result<Option<(String, String)>, String> {
        if arguments_json.len() > MAX_TOOL_ARGUMENTS_BYTES {
            return Err(TOOL_ARGUMENTS_TOO_LARGE_CODE.to_string());
        }
        if self.entries.iter().any(|entry| entry.ordinal == ordinal) {
            return Ok(None);
        }
        self.identify(ordinal, call_id, name)?;
        self.replace_arguments(ordinal, arguments_json)?;
        Ok(self.end(ordinal))
    }

    /// 按 ordinal 顺序结束所有未闭合调用（协议完成事件到达时使用）。
    ///
    /// 缺 call ID 的未闭合调用无法构造结果与续传键，按协议错误返回。
    pub(crate) fn end_all_open(&mut self) -> Result<Vec<(String, String)>, String> {
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| !entry.ended && entry.call_id.is_none())
        {
            return Err(protocol_error(format!(
                "ordinal {} has arguments but no call id",
                entry.ordinal
            )));
        }
        let mut ordinals: Vec<usize> = self
            .entries
            .iter()
            .filter(|entry| !entry.ended)
            .map(|entry| entry.ordinal)
            .collect();
        ordinals.sort_unstable();
        Ok(ordinals
            .into_iter()
            .filter_map(|ordinal| self.end(ordinal))
            .collect())
    }

    /// 是否仍有未闭合的调用（EOF 校验：半截参数不得执行）。
    pub(crate) fn has_open(&self) -> bool {
        self.entries.iter().any(|entry| !entry.ended)
    }

    /// 本轮响应是否出现过任何工具调用（含已结束）。
    pub(crate) fn has_calls(&self) -> bool {
        !self.entries.is_empty()
    }

    /// 已登记的调用槽位数（`output_index` 缺失时的兜底序号）。
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests;

//! 流式聊天协议适配：五种 transport 的请求编码与事件解码。
//!
//! 公共入口只接收 `ChatRequest + cancellation`，输出 `Vec<ChatDelta>`；
//! 每个 transport 分别封装请求 JSON、URL/header 与事件解码，SSE 帧层共用。
//! 模型级 `transport` 覆盖 provider `type`。

pub mod anthropic;
pub mod google;
pub mod openai;
pub mod responses;

use serde_json::Value;

use super::events::{ChatDelta, ChatRequest, PartDelta, PartKind};

/// 一次 transport 请求的完整描述（URL、header、body）；由 client 执行。
#[derive(Debug, Clone)]
pub struct TransportRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

/// 适配器解码出的流结果：完整 delta 列表 + 结束原因。
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedStream {
    pub deltas: Vec<ChatDelta>,
    pub done: bool,
}

/// 按 transport 构造请求（URL、认证 header、JSON body）。
///
/// 返回错误为稳定错误码（不含密钥与原始响应）。
pub fn build_request(
    transport: &str,
    base_url: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
) -> Result<TransportRequest, String> {
    match transport {
        "openai" => openai::build(base_url, api_key, request, false),
        "openai-compatible" => openai::build(base_url, api_key, request, true),
        "openai-responses" => responses::build(base_url, api_key, request),
        "anthropic" => anthropic::build(base_url, api_key, request),
        "google" => google::build(base_url, api_key, request),
        other => Err(format!("unsupported transport: {other}")),
    }
}

/// 按 transport 解码一帧 SSE data（已去掉 `data:` 前缀）。
///
/// 适配器内部维护跨帧状态（如 Anthropic content_block_index）；
/// 返回错误为稳定错误码。`finish` 只冲刷残余内容（如半截标签），
/// **不产生终态**——终态由 client 统一追加，保证恰好一个。
pub trait StreamDecoder {
    /// 喂入一帧；返回新产生的 delta。
    fn decode_frame(&mut self, frame: &str) -> Result<Vec<ChatDelta>, String>;
    /// 流 EOF 时冲刷残余内容（不追加 Finished；完成性由 client 判定）。
    fn finish(&mut self) -> Vec<ChatDelta>;
    /// 协议是否已显式收到完成信号（client 在 EOF 无信号时报中断错误）。
    fn has_completion(&self) -> bool;
    /// 收到协议级结束标记（OpenAI 兼容的 `[DONE]`）。
    ///
    /// 默认忽略：仅对 `[DONE]` 有语义的解码器（Chat Completions）覆写为置完成位；
    /// 其余协议（Anthropic/Google/Responses）由本协议事件判定完成。
    fn mark_done(&mut self) {}
}

/// 按 transport 构造解码器。
pub fn decoder_for(transport: &str) -> Box<dyn StreamDecoder + Send> {
    match transport {
        "anthropic" => Box::new(anthropic::Decoder::default()),
        "google" => Box::new(google::Decoder::default()),
        "openai-responses" => Box::new(responses::Decoder::default()),
        // openai 与 openai-compatible 共用 Chat Completions 事件形状。
        _ => Box::new(openai::Decoder::default()),
    }
}

/// 把标签拆分器的段列表转换为 delta 列表。
pub(crate) fn segments_to_deltas(segments: Vec<crate::chat::thinking::Segment>) -> Vec<ChatDelta> {
    segments
        .into_iter()
        .map(|segment| {
            ChatDelta::Part(PartDelta {
                kind: segment.kind,
                text: segment.text,
                metadata: None,
            })
        })
        .collect()
}

/// 追加一个 reasoning part delta。
pub(crate) fn reasoning_delta(text: impl Into<String>) -> ChatDelta {
    ChatDelta::Part(PartDelta {
        kind: PartKind::Reasoning,
        text: text.into(),
        metadata: None,
    })
}

/// 安全合并 base URL 与路径：避免重复 `/v1`、保留用户路径前缀。
pub(crate) fn join_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    // base 已以 /v1 结尾且 path 以 v1/ 开头时去重。
    if let Some(stripped) = path.strip_prefix("v1/")
        && base.ends_with("/v1")
    {
        return format!("{base}/{stripped}");
    }
    format!("{base}/{path}")
}

/// 从 JSON 对象取字符串字段。
pub(crate) fn json_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// 从 JSON 对象取数字字段（i64）。
pub(crate) fn json_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

/// 构造 OpenAI 风格 usage 增量（prompt/completion/total_tokens）。
pub(crate) fn openai_usage(value: &Value) -> Option<ChatDelta> {
    let usage = value.get("usage")?;
    let input = json_i64(usage, "prompt_tokens");
    let output = json_i64(usage, "completion_tokens");
    let cache_read = usage
        .get("prompt_tokens_details")
        .and_then(|details| json_i64(details, "cached_tokens"));
    if input.is_none() && output.is_none() && cache_read.is_none() {
        return None;
    }
    Some(ChatDelta::Usage(crate::db::chat::Usage {
        input,
        output,
        cache_read,
        cache_write: None,
    }))
}

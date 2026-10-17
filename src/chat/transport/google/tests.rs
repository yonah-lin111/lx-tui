//! Google Gemini 适配测试：请求形状（contents/tools/systemInstruction/Key 位置）、
//! thought part、functionCall 去重与合成 call ID、usage。

use serde_json::{Value, json};

use super::*;
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, ModelItem, ModelStepStop, PartKind, StopReason,
    TOOL_ARGUMENTS_INVALID_CODE,
};
use crate::chat::transport::tools::TOOL_PROTOCOL_ERROR_CODE;

fn request() -> ChatRequest {
    ChatRequest {
        turns: vec![
            ChatTurn {
                role: "user".into(),
                text: "问".into(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "assistant".into(),
                text: "答".into(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "user".into(),
                text: "续问".into(),
                reasoning_metadata: None,
            },
        ],
        items: Vec::new(),
        model: crate::db::chat::ModelSnapshot {
            provider: "gemini".into(),
            model: "gemini-flash".into(),
            variant: None,
        },
        variant_params: None,
    }
}

/// 带工具调用的结构化历史（成功 + 失败各一次）。
fn tool_request() -> ChatRequest {
    let mut req = request();
    req.turns = Vec::new();
    req.items = vec![
        ModelItem::UserText("读 a.rs".into()),
        ModelItem::AssistantToolCall {
            call_id: "gemini-0-0-read".into(),
            name: "read".into(),
            input: json!({"path": "a.rs"}),
        },
        ModelItem::ToolResult {
            call_id: "gemini-0-0-read".into(),
            content: "1: fn main() {}".into(),
            is_error: false,
        },
        ModelItem::AssistantToolCall {
            call_id: "gemini-0-1-write".into(),
            name: "write".into(),
            input: json!({"path": "b.rs", "content": ""}),
        },
        ModelItem::ToolResult {
            call_id: "gemini-0-1-write".into(),
            content: "permission_denied".into(),
            is_error: true,
        },
    ];
    req
}

fn body_of(req: &ChatRequest) -> Value {
    let built = super::build("https://example.invalid", Some("g-key"), req).expect("build ok");
    serde_json::from_str(&built.body).expect("json")
}

fn decode_frames(decoder: &mut dyn super::super::StreamDecoder, frames: &[&str]) -> Vec<ChatDelta> {
    let mut out = Vec::new();
    for frame in frames {
        out.extend(decoder.decode_frame(frame).expect("frame decodes"));
    }
    out.extend(decoder.finish());
    out
}

fn decode_all(decoder: &mut dyn super::super::StreamDecoder, frames: &[&str]) -> Vec<ChatDelta> {
    let mut out = decode_frames(decoder, frames);
    if !out
        .iter()
        .any(|delta| matches!(delta, ChatDelta::Finished { .. }))
    {
        out.push(ChatDelta::Finished {
            reason: if decoder.has_completion() {
                StopReason::Complete
            } else {
                StopReason::Failed {
                    code: "stream ended without completion".into(),
                }
            },
        });
    }
    out
}

fn trace(deltas: &[ChatDelta]) -> Vec<String> {
    deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::ToolInputStarted {
                call_id,
                name,
                ordinal,
            } => Some(format!("started:{call_id}:{name}:{ordinal}")),
            ChatDelta::ToolInputDelta { call_id, chunk } => {
                Some(format!("delta:{call_id}:{chunk}"))
            }
            ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            } => Some(format!("ended:{call_id}:{arguments_json}")),
            ChatDelta::ResponseFinished { stop } => Some(format!("response:{stop:?}")),
            ChatDelta::Finished { reason } => Some(format!("finished:{}", reason.as_str())),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------- 请求编码

#[test]
fn url_places_key_in_query_and_body_uses_contents() {
    let built =
        super::build("https://example.invalid", Some("g-key"), &request()).expect("build ok");
    assert!(
        built
            .url
            .ends_with("/models/gemini-flash:streamGenerateContent?alt=sse&key=g-key"),
        "Key 只按该协议放置在查询参数：{}",
        built.url
    );
    // 模型名重复 models/ 前缀去重。
    let built = super::build("https://example.invalid", Some("g-key"), &{
        let mut req = request();
        req.model.model = "models/gemini-flash".into();
        req
    })
    .expect("build ok");
    assert!(built.url.contains("/models/gemini-flash:"));
    assert!(!built.url.contains("models/models/"));
    let body: Value = serde_json::from_str(&built.body).expect("json");
    let contents = body
        .get("contents")
        .and_then(Value::as_array)
        .expect("contents");
    assert_eq!(contents.len(), 3);
    // assistant 历史 role 映射为 model。
    assert_eq!(contents[0]["role"], "user");
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(contents[2]["role"], "user");
    // Key 不进 body。
    assert!(!built.body.contains("g-key"));
}

/// 系统指令走 `systemInstruction.parts[0].text`。
#[test]
fn system_instruction_carries_lx_prompt() {
    let body = body_of(&request());
    assert_eq!(
        body["systemInstruction"]["parts"][0]["text"],
        crate::chat::prompt::LX_SYSTEM_PROMPT
    );
}

/// `tools[0].functionDeclarations` 与 `available_tools()` 一一对应。
#[test]
fn tools_use_function_declarations() {
    let body = body_of(&request());
    let declarations = body["tools"][0]["functionDeclarations"]
        .as_array()
        .expect("declarations");
    let specs = crate::chat::tools::available_tools();
    assert_eq!(declarations.len(), specs.len());
    for (wire, spec) in declarations.iter().zip(specs) {
        assert_eq!(wire["name"], spec.name);
        assert_eq!(wire["description"], spec.description);
        assert_eq!(wire["parameters"], spec.parameters());
    }
}

/// 结构化历史：model functionCall part 与 user functionResponse part 相邻成对。
#[test]
fn tool_history_encodes_function_call_and_response_parts() {
    let body = body_of(&tool_request());
    let contents = body["contents"].as_array().expect("contents");
    assert_eq!(contents.len(), 5, "user → model → user → model → user");
    assert_eq!(contents[0]["parts"][0]["text"], "读 a.rs");
    // model functionCall：args 是对象，不写入合成 call ID。
    let call = &contents[1]["parts"][0]["functionCall"];
    assert_eq!(contents[1]["role"], "model");
    assert_eq!(call["name"], "read");
    assert_eq!(call["args"], json!({"path": "a.rs"}));
    assert!(call.get("id").is_none(), "不得把合成 ID 写进供应商字段");
    // user functionResponse：name 由配对调用解析，response 包装结构化结果。
    assert_eq!(contents[2]["role"], "user");
    let response = &contents[2]["parts"][0]["functionResponse"];
    assert_eq!(response["name"], "read");
    assert_eq!(response["response"]["result"], "1: fn main() {}");
    // 失败结果用 error 键标注，不与成功结果混淆。
    let failed = &contents[4]["parts"][0]["functionResponse"];
    assert_eq!(failed["name"], "write");
    assert_eq!(failed["response"]["error"], "permission_denied");
}

#[test]
fn thinking_budget_maps_to_generation_config() {
    let mut req = request();
    req.variant_params = Some(json!({"thinkingBudget": 1024}));
    let body = body_of(&req);
    assert_eq!(
        body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        1024
    );
}

// ---------------------------------------------------------------- 解码

#[test]
fn thought_part_is_reasoning_and_order_preserved() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"思考内容","thought":true},{"text":"正文"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":3}}"#,
        ],
    );
    let parts: Vec<(PartKind, String)> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some((part.kind, part.text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        parts,
        vec![
            (PartKind::Reasoning, "思考内容".into()),
            (PartKind::Text, "正文".into()),
        ]
    );
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(12) && usage.output == Some(3)
    )));
    // 无 functionCall：ResponseFinished{Final} + Finished。
    assert!(deltas.contains(&ChatDelta::ResponseFinished {
        stop: ModelStepStop::Final
    }));
    assert_eq!(
        deltas.last(),
        Some(&ChatDelta::Finished {
            reason: StopReason::Complete
        })
    );
}

/// 单 functionCall：Started + Ended（args 已是完整对象）+ ResponseFinished{ToolCalls}。
#[test]
fn single_function_call_emits_started_and_ended() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a.rs"}}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            // 合成 ID 只用 (candidate, part, name)，不含时间戳。
            "started:gemini-0-0-read:read:0".to_string(),
            "ended:gemini-0-0-read:{\"path\":\"a.rs\"}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// 累计快照重复给同一 functionCall：只执行一次。
#[test]
fn repeated_cumulative_snapshot_is_deduped() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            // 第一帧只有 thought + functionCall。
            r#"{"candidates":[{"content":{"parts":[{"text":"想","thought":true},{"functionCall":{"name":"read","args":{"path":"a.rs"}}}],"role":"model"},"index":0}]}"#,
            // 第二帧是累计快照：thought 全文 + 同一个 functionCall（重复）。
            r#"{"candidates":[{"content":{"parts":[{"text":"想要读文件","thought":true},{"functionCall":{"name":"read","args":{"path":"a.rs"}}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    let lines = trace(&deltas);
    let started: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("started:"))
        .collect();
    assert_eq!(started.len(), 1, "同一 functionCall 只执行一次：{deltas:?}");
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("ended:"))
            .count(),
        1
    );
}

/// 同一帧两个 functionCall：按 part index 顺序，ID 各自稳定。
#[test]
fn two_function_calls_in_one_frame_keep_part_order() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a"}}},{"functionCall":{"name":"write","args":{"path":"b","content":""}}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:gemini-0-0-read:read:0".to_string(),
            "ended:gemini-0-0-read:{\"path\":\"a\"}".to_string(),
            "started:gemini-0-1-write:write:1".to_string(),
            "ended:gemini-0-1-write:{\"content\":\"\",\"path\":\"b\"}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// reasoning → tool → reasoning → text 交错：thought part 与 functionCall 顺序保持。
#[test]
fn reasoning_tool_reasoning_text_interleaving() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"先想","thought":true}],"role":"model"},"index":0}]}"#,
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":{"path":"a"}}}],"role":"model"},"index":0}]}"#,
            r#"{"candidates":[{"content":{"parts":[{"text":"再想","thought":true},{"text":"正文"}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    let trace: Vec<String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some(format!("part:{:?}:{}", part.kind, part.text)),
            ChatDelta::ToolInputStarted { call_id, .. } => Some(format!("started:{call_id}")),
            ChatDelta::ToolInputEnded { call_id, .. } => Some(format!("ended:{call_id}")),
            ChatDelta::ResponseFinished { stop } => Some(format!("response:{stop:?}")),
            ChatDelta::Finished { reason } => Some(format!("finished:{}", reason.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        trace,
        vec![
            "part:Reasoning:先想",
            "started:gemini-0-0-read",
            "ended:gemini-0-0-read",
            "part:Reasoning:再想",
            "part:Text:正文",
            "response:ToolCalls",
            "finished:complete",
        ]
    );
}

/// 缺 args 的 functionCall：按空对象处理（协议允许无参调用）。
#[test]
fn function_call_without_args_uses_empty_object() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"search"}}],"role":"model"},"finishReason":"STOP","index":0}]}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:gemini-0-0-search:search:0".to_string(),
            "ended:gemini-0-0-search:{}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// 缺 name 的 functionCall：协议错误，不执行。
#[test]
fn function_call_without_name_is_protocol_error() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"args":{"path":"a"}}}],"role":"model"},"index":0}]}"#,
        )
        .expect_err("missing name rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// 非对象类型的 args：序列化后校验失败（参数必须可被工具层解析）。
#[test]
fn function_call_with_non_object_args_is_invalid() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"read","args":"not-an-object"}}],"role":"model"},"index":0}]}"#,
        )
        .expect_err("non-object args rejected");
    assert!(error.contains(TOOL_ARGUMENTS_INVALID_CODE), "{error}");
}

/// 未知 finishReason（如 MAX_TOKENS 之外的值）不产生终态，EOF 由 client 判失败。
#[test]
fn unknown_finish_reason_waits_for_client() {
    let mut decoder = Decoder::default();
    let deltas = decode_frames(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"x"}],"role":"model"},"finishReason":"OTHER"}]}"#,
        ],
    );
    assert!(!decoder.has_completion());
    assert!(
        !deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::Finished { .. }))
    );
}

/// `MAX_TOKENS` 也视为本次响应结束（内容已截断但响应本身完成）。
#[test]
fn max_tokens_finish_reason_completes_response() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"x"}],"role":"model"},"finishReason":"MAX_TOKENS"}]}"#,
        ],
    );
    assert!(deltas.contains(&ChatDelta::ResponseFinished {
        stop: ModelStepStop::Final
    }));
    assert!(deltas.contains(&ChatDelta::Finished {
        reason: StopReason::Complete
    }));
}

#[test]
fn error_response_is_rejected() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(r#"{"error":{"message":"quota exceeded"}}"#)
        .expect_err("error rejected");
    assert!(error.contains("quota exceeded"));
}

#[test]
fn eof_without_stop_finish_reason_is_protocol_error() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[r#"{"candidates":[{"content":{"parts":[{"text":"x"}],"role":"model"}}]}"#],
    );
    assert!(matches!(
        deltas.last(),
        Some(ChatDelta::Finished {
            reason: StopReason::Failed { .. }
        })
    ));
}

#[test]
fn empty_base_url_uses_official_endpoint() {
    let built = super::build("", Some("g-key"), &request()).expect("build ok");
    assert!(
        built
            .url
            .starts_with("https://generativelanguage.googleapis.com/v1beta/")
    );
}

/// 用量只报**本响应**的实际值：两次采样各自解码，由上层累加。
#[test]
fn usage_is_reported_per_response_only() {
    let mut first = Decoder::default();
    let first_deltas = decode_all(
        &mut first,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"a"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":2}}"#,
        ],
    );
    let mut second = Decoder::default();
    let second_deltas = decode_all(
        &mut second,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"b"}],"role":"model"},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":40,"candidatesTokenCount":5}}"#,
        ],
    );
    let usages = |deltas: &[ChatDelta]| {
        deltas
            .iter()
            .filter_map(|delta| match delta {
                ChatDelta::Usage(usage) => Some((usage.input, usage.output)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(usages(&first_deltas), vec![(Some(10), Some(2))]);
    assert_eq!(usages(&second_deltas), vec![(Some(40), Some(5))]);
}

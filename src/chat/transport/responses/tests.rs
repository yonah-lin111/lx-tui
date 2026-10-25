//! Responses API 适配测试：请求形状（tools/instructions/历史）、reasoning、工具事件、hosted 拒绝。

use serde_json::{Value, json};

use super::*;
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, MAX_TOOL_ARGUMENTS_BYTES, ModelItem, ModelStepStop, PartKind,
    StopReason, TOOL_ARGUMENTS_INVALID_CODE, TOOL_ARGUMENTS_TOO_LARGE_CODE,
};
use crate::chat::transport::tools::{TOOL_INPUT_TRUNCATED_CODE, TOOL_PROTOCOL_ERROR_CODE};

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
        ],
        items: Vec::new(),
        model: crate::db::chat::ModelSnapshot {
            provider: "example".into(),
            model: "grok".into(),
            variant: Some("high".into()),
        },
        variant_params: None,
        purpose: crate::chat::events::RequestPurpose::Chat,
        system_prompt: None,
    }
}

/// 带工具调用的结构化历史。
fn tool_request() -> ChatRequest {
    let mut req = request();
    req.turns = Vec::new();
    req.items = vec![
        ModelItem::UserText("读 src/main.rs".into()),
        ModelItem::AssistantToolCall {
            call_id: "call_a".into(),
            name: "read".into(),
            input: json!({"path": "src/main.rs"}),
        },
        ModelItem::ToolResult {
            call_id: "call_a".into(),
            content: "1: fn main() {}".into(),
            is_error: false,
        },
    ];
    req
}

fn body_of(req: &ChatRequest) -> Value {
    let built = super::build("https://api.example.com/v1", Some("key"), req).expect("build ok");
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

/// 把 delta 收敛成紧凑形式（工具事件 + 响应终态）。
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
fn body_uses_input_not_messages() {
    let built =
        super::build("https://api.example.com/v1", Some("key"), &request()).expect("build ok");
    assert_eq!(built.url, "https://api.example.com/v1/responses");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert!(body.get("input").is_some(), "Responses 用 input");
    assert!(body.get("messages").is_none(), "不沿用 messages 形状");
    assert_eq!(body["stream"], true);
    assert_eq!(body["model"], "grok");
    // 系统指令：第一段进顶层 instructions，其余作为 system input 项（见 split_system_prompt）。
    let expected = crate::chat::prompt::system_prompt();
    let chunks = crate::chat::transport::split_system_prompt(&expected);
    assert_eq!(body["instructions"], chunks[0]);
    let joined: String = chunks.iter().cloned().collect();
    assert_eq!(joined, expected, "分段拼接必须逐字还原");
}

/// `tools` 是扁平函数项（不套 `function`），schema 与 `available_tools()` 一致。
#[test]
fn tools_are_flat_function_items() {
    let body = body_of(&request());
    let tools = body["tools"].as_array().expect("tools array");
    let specs = crate::chat::tools::available_tools();
    assert_eq!(tools.len(), specs.len());
    for (wire, spec) in tools.iter().zip(specs) {
        assert_eq!(wire["type"], "function");
        assert!(
            wire.get("function").is_none(),
            "Responses 不套 function 包装"
        );
        assert_eq!(wire["name"], spec.name);
        assert_eq!(wire["description"], spec.description);
        assert_eq!(wire["parameters"], spec.parameters());
    }
}

/// 纯文本回退：input 与旧行为逐字一致。
#[test]
fn plain_text_input_is_unchanged() {
    let body = body_of(&request());
    // system 分段（除第一段进 instructions）会作为 system input 项前置；
    // 去掉它们之后，user/assistant 的编码与旧行为逐字一致。
    let input = body["input"].as_array().expect("input");
    let tail: Vec<&Value> = input
        .iter()
        .skip_while(|item| item["role"] == "system")
        .collect();
    assert_eq!(
        tail,
        vec![
            &json!({"role": "user", "content": "问"}),
            &json!({"role": "assistant", "content": "答"}),
        ]
    );
}

/// 结构化历史：function_call 输出项 + function_call_output 输入项成对编码。
#[test]
fn tool_history_encodes_function_call_items() {
    let body = body_of(&tool_request());
    let input = body["input"].as_array().expect("input");
    // system 分段前置（第一段已进 instructions），其后才是历史项。
    let offset = input
        .iter()
        .take_while(|item| item["role"] == "system")
        .count();
    assert_eq!(input.len(), offset + 3);
    assert_eq!(input[offset]["role"], "user");
    // function_call：call_id/name/arguments（JSON 字符串，只序列化一次）。
    assert_eq!(input[offset + 1]["type"], "function_call");
    assert_eq!(input[offset + 1]["call_id"], "call_a");
    assert_eq!(input[offset + 1]["name"], "read");
    let arguments = input[offset + 1]["arguments"]
        .as_str()
        .expect("arguments string");
    assert_eq!(
        serde_json::from_str::<Value>(arguments).expect("parse"),
        json!({"path": "src/main.rs"})
    );
    // function_call_output：只用 call_id 关联。
    assert_eq!(input[offset + 2]["type"], "function_call_output");
    assert_eq!(input[offset + 2]["call_id"], "call_a");
    assert_eq!(input[offset + 2]["output"], "1: fn main() {}");
    assert!(input[offset + 2].get("name").is_none());
}

#[test]
fn variant_maps_reasoning_structure() {
    let mut req = request();
    req.variant_params = Some(json!({
        "reasoningEffort": "high",
        "reasoningSummary": "auto",
    }));
    let body = body_of(&req);
    assert_eq!(body["reasoning"]["effort"], "high");
    assert_eq!(body["reasoning"]["summary"], "auto");
}

#[test]
fn no_variant_sends_no_reasoning() {
    assert!(body_of(&request()).get("reasoning").is_none());
}

// ---------------------------------------------------------------- 解码

#[test]
fn summary_and_raw_reasoning_kept_separate_with_metadata() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"response.output_text.delta","delta":"正文"}"#,
            r#"{"type":"response.reasoning_summary_text.delta","delta":"摘要"}"#,
            r#"{"type":"response.reasoning_text.delta","delta":"原始"}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":2}}}"#,
        ],
    );
    // 正文不混入 reasoning；summary/raw 各自独立 part 且带来源。
    let parts: Vec<(PartKind, String, Option<String>)> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some((
                part.kind,
                part.text.clone(),
                part.metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("source"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        parts,
        vec![
            (PartKind::Text, "正文".into(), None),
            (PartKind::Reasoning, "摘要".into(), Some("summary".into())),
            (PartKind::Reasoning, "原始".into(), Some("raw".into())),
        ]
    );
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(5) && usage.output == Some(2)
    )));
    // 无工具：ResponseFinished{Final} + Finished。
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

/// 单工具调用：output_item.added → arguments delta(s) → done → completed。
#[test]
fn single_function_call_full_sequence() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"read","arguments":""}}"#,
            r#"{"type":"response.function_call_arguments.delta","item_id":"item-1","output_index":0,"delta":"{\"path\":"}"#,
            r#"{"type":"response.function_call_arguments.delta","item_id":"item-1","output_index":0,"delta":"\"a.rs\"}"}"#,
            r#"{"type":"response.function_call_arguments.done","item_id":"item-1","output_index":0,"arguments":"{\"path\":\"a.rs\"}"}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":7,"output_tokens":3}}}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:call_a:read:0".to_string(),
            "delta:call_a:{\"path\":".to_string(),
            "delta:call_a:\"a.rs\"}".to_string(),
            "ended:call_a:{\"path\":\"a.rs\"}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// 同响应两个工具：ordinal = output_index，收尾按 ordinal 顺序。
#[test]
fn two_function_calls_keep_output_index_order() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"item-2","type":"function_call","call_id":"call_b","name":"write","arguments":""}}"#,
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"read","arguments":""}}"#,
            r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a\"}"}"#,
            r#"{"type":"response.function_call_arguments.done","output_index":1,"arguments":"{\"path\":\"b\",\"content\":\"\"}"}"#,
            r#"{"type":"response.completed","response":{}}"#,
        ],
    );
    // 两个 done 都到达后 completed 不再重复发 Ended。
    let ended: Vec<(String, String)> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            } => Some((call_id.clone(), arguments_json.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        ended,
        vec![
            ("call_a".to_string(), "{\"path\":\"a\"}".to_string()),
            (
                "call_b".to_string(),
                "{\"path\":\"b\",\"content\":\"\"}".to_string()
            ),
        ]
    );
    assert!(deltas.contains(&ChatDelta::ResponseFinished {
        stop: ModelStepStop::ToolCalls
    }));
}

/// reasoning → tool → reasoning → text 交错（summary/raw 两种 reasoning）。
#[test]
fn reasoning_tool_reasoning_text_interleaving() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"response.reasoning_summary_text.delta","delta":"先想"}"#,
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"read","arguments":""}}"#,
            r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a\"}"}"#,
            r#"{"type":"response.reasoning_text.delta","delta":"再想"}"#,
            r#"{"type":"response.output_text.delta","delta":"正文"}"#,
            r#"{"type":"response.completed","response":{}}"#,
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
            "started:call_a",
            "ended:call_a",
            "part:Reasoning:再想",
            "part:Text:正文",
            "response:ToolCalls",
            "finished:complete",
        ]
    );
}

/// `arguments.done` 缺 delta 也能闭合；EOF 前未闭合属于截断。
#[test]
fn done_without_deltas_closes_call() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"read","arguments":""}}"#,
            r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{}"}"#,
            r#"{"type":"response.completed","response":{}}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:call_a:read:0".to_string(),
            "ended:call_a:{}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// `response.completed` 时仍有未闭合调用：先补 Ended 再 ResponseFinished（设计 §17）。
#[test]
fn completed_closes_open_calls_then_finishes() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"read","arguments":""}}"#,
            r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"{}"}"#,
            r#"{"type":"response.completed","response":{}}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:call_a:read:0".to_string(),
            "delta:call_a:{}".to_string(),
            "ended:call_a:{}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// hosted tool（web_search_call 等）不得当成本地调用执行。
#[test]
fn hosted_tool_item_is_rejected() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"ws-1","type":"web_search_call","status":"completed"}}"#,
        )
        .expect_err("hosted tool rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
    assert!(error.contains("web_search_call"), "{error}");
}

/// 普通输出项（message / reasoning）被忽略，不影响工具流程。
#[test]
fn non_call_output_items_are_ignored() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg-1","type":"message","role":"assistant","content":[]}}"#,
            r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"rs-1","type":"reasoning","summary":[]}}"#,
            r#"{"type":"response.output_text.delta","delta":"ok"}"#,
            r#"{"type":"response.completed","response":{}}"#,
        ],
    );
    assert_eq!(
        deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::ToolInputStarted { .. }))
            .count(),
        0
    );
    assert!(deltas.contains(&ChatDelta::ResponseFinished {
        stop: ModelStepStop::Final
    }));
}

/// 非法 JSON 参数：done 时校验失败（不发 Ended）。
#[test]
fn malformed_arguments_fail_with_invalid_code() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"read","arguments":""}}"#,
        )
        .expect("added frame");
    let error = decoder
        .decode_frame(
            r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":"}"#,
        )
        .expect_err("half json rejected");
    assert!(error.contains(TOOL_ARGUMENTS_INVALID_CODE), "{error}");
}

/// 参数超限：delta 累计超限立即失败。
#[test]
fn oversized_arguments_fail_with_too_large_code() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"write","arguments":""}}"#,
        )
        .expect("added frame");
    let big = "x".repeat(MAX_TOOL_ARGUMENTS_BYTES + 1);
    let frame = format!(
        r#"{{"type":"response.function_call_arguments.delta","output_index":0,"delta":"{big}"}}"#
    );
    let error = decoder
        .decode_frame(&frame)
        .expect_err("too large rejected");
    assert_eq!(error, TOOL_ARGUMENTS_TOO_LARGE_CODE);
}

/// function_call 项缺 call_id：协议错误（不合成假 ID）。
#[test]
fn function_call_without_call_id_is_protocol_error() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","name":"read","arguments":""}}"#,
        )
        .expect_err("missing call_id rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// EOF 时未闭合：`has_open_tool_input` 为真（client 报截断）。
#[test]
fn eof_with_open_tool_input_is_reported() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"read","arguments":""}}"#,
        )
        .expect("added frame");
    assert!(decoder.has_open_tool_input());
    assert_eq!(TOOL_INPUT_TRUNCATED_CODE, "tool input truncated");
}

#[test]
fn response_failed_maps_to_failed_stop() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[r#"{"type":"response.failed","response":{"error":{"message":"server busy"}}}"#],
    );
    assert!(matches!(
        deltas.last(),
        Some(ChatDelta::Finished {
            reason: StopReason::Failed { code }
        }) if code.contains("server busy")
    ));
}

#[test]
fn eof_without_completed_is_protocol_error() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[r#"{"type":"response.output_text.delta","delta":"x"}"#],
    );
    assert_eq!(
        deltas.last(),
        Some(&ChatDelta::Finished {
            reason: StopReason::Failed {
                code: "stream ended without completion".into()
            }
        })
    );
}

/// `response.incomplete` 且工具参数未闭合：协议错误（不执行）。
#[test]
fn incomplete_with_open_call_is_protocol_error() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_a","name":"read","arguments":""}}"#,
        )
        .expect("added frame");
    let error = decoder
        .decode_frame(r#"{"type":"response.incomplete","response":{}}"#)
        .expect_err("incomplete with open call rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

#[test]
fn api_error_event_is_rejected() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(r#"{"type":"error","message":"bad request"}"#)
        .expect_err("error event rejected");
    assert!(error.contains("bad request"));
}

#[test]
fn model_level_transport_overrides_provider_type() {
    // 请求构造入口由 transport 字符串决定（模型级 openai-responses 覆盖 provider type）。
    let request = super::super::build_request(
        "openai-responses",
        "https://api.example.com/v1",
        Some("key"),
        &request(),
    );
    assert!(request.is_ok());
    assert!(request.expect("ok").url.ends_with("/responses"));
}

/// 用量只报**本响应**的实际值：两次采样各自解码，由上层累加。
#[test]
fn usage_is_reported_per_response_only() {
    let mut first = Decoder::default();
    let first_deltas = decode_all(
        &mut first,
        &[
            r#"{"type":"response.output_text.delta","delta":"a"}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":2}}}"#,
        ],
    );
    let mut second = Decoder::default();
    let second_deltas = decode_all(
        &mut second,
        &[
            r#"{"type":"response.output_text.delta","delta":"b"}"#,
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":40,"output_tokens":5}}}"#,
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

// ------------------------------------------------- G8 核查：原生 ID 跨响应唯一

/// **G8 核查**：Responses 的 call ID 来自 `item.call_id`，解码器不自造 ID；
/// 两次响应（两个解码器）都直接用供应商给的 ID。
#[test]
fn call_ids_come_from_item_call_id_not_from_the_decoder() {
    let frames = &[
        r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"item-1","type":"function_call","call_id":"call_srv_1","name":"read","arguments":""}}"#,
        r#"{"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"path\":\"a\"}"}"#,
        r#"{"type":"response.completed","response":{}}"#,
    ];
    let ids_of = |frames: &[&str]| {
        decode_frames(&mut Decoder::default(), frames)
            .into_iter()
            .filter_map(|delta| match delta {
                ChatDelta::ToolInputStarted { call_id, .. } => Some(call_id),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(ids_of(frames), vec!["call_srv_1".to_string()]);
    // 同一份 wire 帧进两个解码器 → 同一个供应商 ID（解码器不参与生成）。
    assert_eq!(ids_of(frames), ids_of(frames));
}

/// 摘要请求：不声明工具，系统提示使用请求覆写值（四协议同口径）。
#[test]
fn compact_request_omits_tools_and_overrides_the_system_prompt() {
    let request = crate::chat::events::ChatRequest {
        turns: vec![crate::chat::events::ChatTurn {
            role: "user".into(),
            text: "compress the transcript".into(),
            reasoning_metadata: None,
        }],
        items: vec![crate::chat::events::ModelItem::UserText(
            "compress the transcript".into(),
        )],
        model: crate::db::chat::ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
        variant_params: None,
        purpose: crate::chat::events::RequestPurpose::CompactSummary,
        system_prompt: Some("COMPACT INSTRUCTIONS".into()),
    };
    let built = super::build("https://api.example.invalid", Some("key"), &request).expect("build");
    let body: serde_json::Value = serde_json::from_str(&built.body).expect("json");
    assert!(!built.tools_requested, "摘要请求不得声明工具能力");
    assert!(
        !built.body.contains("\"tools\""),
        "body 不得含 tools：{body}"
    );
    let prompt = serde_json::to_string(&body).expect("json");
    assert!(prompt.contains("COMPACT INSTRUCTIONS"), "系统提示被覆写");
}

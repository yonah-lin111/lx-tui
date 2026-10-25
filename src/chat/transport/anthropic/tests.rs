//! Anthropic Messages 适配测试：请求形状（tools/system/历史）、签名绑定、工具事件序列。

use serde_json::{Value, json};

use super::*;
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, MAX_TOOL_ARGUMENTS_BYTES, ModelItem, ModelStepStop, PartKind,
    StopReason, TOOL_ARGUMENTS_INVALID_CODE, TOOL_ARGUMENTS_TOO_LARGE_CODE,
};
use crate::chat::transport::tools::{TOOL_INPUT_TRUNCATED_CODE, TOOL_PROTOCOL_ERROR_CODE};

fn request() -> ChatRequest {
    ChatRequest {
        turns: vec![ChatTurn {
            role: "user".into(),
            text: "你好".into(),
            reasoning_metadata: None,
        }],
        items: Vec::new(),
        model: crate::db::chat::ModelSnapshot {
            provider: "anthropic".into(),
            model: "claude".into(),
            variant: None,
        },
        variant_params: None,
        purpose: crate::chat::events::RequestPurpose::Chat,
        system_prompt: None,
    }
}

/// 前置系统规则段的数量（`role=user` 且内容是 `<system-instructions>` 包裹）。
fn system_rule_prefix(body: &Value) -> usize {
    body["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .take_while(|message| {
                    message["role"] == "user"
                        && message["content"]
                            .as_str()
                            .or_else(|| message["content"][0]["text"].as_str())
                            .is_some_and(|text| text.starts_with("<system-instructions>"))
                })
                .count()
        })
        .unwrap_or(0)
}

/// 带工具调用的结构化历史（一次成功、一次失败结果）。
fn tool_request() -> ChatRequest {
    let mut req = request();
    req.turns = Vec::new();
    req.items = vec![
        ModelItem::UserText("读 src/main.rs".into()),
        ModelItem::AssistantText("先读一下".into()),
        ModelItem::AssistantToolCall {
            call_id: "toolu_a".into(),
            name: "read".into(),
            input: json!({"path": "src/main.rs"}),
        },
        ModelItem::ToolResult {
            call_id: "toolu_a".into(),
            content: "1: fn main() {}".into(),
            is_error: false,
        },
        ModelItem::AssistantToolCall {
            call_id: "toolu_b".into(),
            name: "write".into(),
            input: json!({"path": "out.rs", "content": ""}),
        },
        ModelItem::ToolResult {
            call_id: "toolu_b".into(),
            content: "permission_denied".into(),
            is_error: true,
        },
    ];
    req
}

fn body_of(req: &ChatRequest) -> Value {
    let built = super::build("https://api.example.com", Some("key"), req).expect("build ok");
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

/// 把 delta 收敛成便于断言的紧凑形式（只保留工具/思考结束/响应终态）。
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
            ChatDelta::ReasoningEnded { item_id } => Some(format!("reasoning-ended:{item_id:?}")),
            ChatDelta::ResponseFinished { stop } => Some(format!("response:{stop:?}")),
            ChatDelta::Finished { reason } => Some(format!("finished:{}", reason.as_str())),
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------- 请求编码

#[test]
fn body_has_max_tokens_messages_and_protocol_headers() {
    let built = super::build("https://api.example.com", Some("key"), &request()).expect("build ok");
    assert_eq!(built.url, "https://api.example.com/v1/messages");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert!(body.get("max_tokens").is_some(), "max_tokens 必填");
    assert!(body.get("messages").is_some());
    assert_eq!(body["stream"], true);
    // 协议 header：x-api-key（不是 Bearer）+ anthropic-version。
    let headers: Vec<String> = built
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect();
    assert!(
        headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case("x-api-key: key"))
    );
    assert!(
        headers
            .iter()
            .any(|header| header.eq_ignore_ascii_case("anthropic-version: 2023-06-01"))
    );
    // Key 不进 URL/body。
    assert!(!built.url.contains("key"));
    assert!(!built.body.contains("key"));
}

/// 纯文本回退：去掉前置的规则段后，messages 与旧行为逐字一致。
#[test]
fn plain_text_history_is_unchanged() {
    let body = body_of(&request());
    let messages = body["messages"].as_array().expect("messages");
    // 首段之外的系统规则合并成一条 `user` 消息前置（见 `SystemPromptParts`），先跳过它。
    let prefix = system_rule_prefix(&body);
    assert_eq!(
        &messages[prefix..],
        &[json!({"role": "user", "content": "你好"})],
        "真实历史必须与旧行为逐字一致"
    );
    // 系统指令只有首段：顶层 system 是字符串；其余规则合并成一条前置 user 消息。
    let parts =
        crate::chat::transport::SystemPromptParts::split(&crate::chat::prompt::system_prompt());
    assert_eq!(body["system"], json!(parts.head));
    assert_eq!(prefix, 1, "其余规则只发一条前置消息");
    assert!(parts.tail_message().is_some());
}

/// `tools` 用 `input_schema` 承载四工具 schema。
#[test]
fn tools_use_input_schema() {
    let body = body_of(&request());
    let tools = body["tools"].as_array().expect("tools array");
    let specs = crate::chat::tools::available_tools();
    assert_eq!(tools.len(), specs.len());
    for (wire, spec) in tools.iter().zip(specs) {
        assert_eq!(wire["name"], spec.name);
        assert_eq!(wire["description"], spec.description);
        assert_eq!(wire["input_schema"], spec.parameters());
    }
}

/// 结构化历史：assistant 的文本与 tool_use 合进同一条消息；tool_result 进下一条 user。
#[test]
fn tool_history_encodes_tool_use_and_tool_result_blocks() {
    let body = body_of(&tool_request());
    let messages = body["messages"].as_array().expect("messages");
    // 跳过前置的系统规则段：真实历史从第一条用户消息开始。
    let base = system_rule_prefix(&body);
    // user → assistant(text+tool_use_a) → user(tool_result_a) → assistant(tool_use_b) → user(tool_result_b)
    assert_eq!(messages.len() - base, 5, "call 与 result 必须成对相邻");
    // 第 1 条：user 文本。
    assert_eq!(messages[base]["role"], "user");
    assert_eq!(messages[base]["content"][0]["text"], "读 src/main.rs");
    // 第 2 条：assistant 的 text 与 tool_use 合进同一条消息的 content 数组。
    assert_eq!(messages[base + 1]["role"], "assistant");
    let blocks = messages[base + 1]["content"].as_array().expect("blocks");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "先读一下");
    assert_eq!(blocks[1]["type"], "tool_use");
    assert_eq!(blocks[1]["id"], "toolu_a");
    assert_eq!(blocks[1]["name"], "read");
    // tool_use.input 是 JSON 对象（不是字符串）。
    assert_eq!(blocks[1]["input"], json!({"path": "src/main.rs"}));
    // 第 3 条：紧邻的 user tool_result（成功）。
    assert_eq!(messages[base + 2]["role"], "user");
    let results = messages[base + 2]["content"].as_array().expect("results");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["type"], "tool_result");
    assert_eq!(results[0]["tool_use_id"], "toolu_a");
    assert_eq!(results[0]["content"], "1: fn main() {}");
    assert_eq!(results[0]["is_error"], json!(false));
    // 第 4/5 条：第二次调用与失败结果（is_error 透传）。
    assert_eq!(messages[base + 3]["role"], "assistant");
    assert_eq!(messages[base + 3]["content"][0]["type"], "tool_use");
    assert_eq!(
        messages[base + 3]["content"][0]["input"],
        json!({"path": "out.rs", "content": ""})
    );
    assert_eq!(messages[base + 4]["role"], "user");
    assert_eq!(messages[base + 4]["content"][0]["tool_use_id"], "toolu_b");
    assert_eq!(messages[base + 4]["content"][0]["is_error"], json!(true));
    assert_eq!(
        messages[base + 4]["content"][0]["content"],
        "permission_denied"
    );
}

#[test]
fn thinking_budget_maps_to_thinking_structure() {
    let mut req = request();
    req.variant_params = Some(json!({"thinkingBudget": 4096}));
    let body = body_of(&req);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 4096);
    assert!(body["max_tokens"].as_i64().unwrap_or(0) > 4096);
}

// ---------------------------------------------------------------- 解码：thinking

#[test]
fn thinking_and_signature_deltas_bound_to_part_metadata() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":50}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"想法"}}"#,
            // 独立帧的 signature_delta：无文本也必须绑定到对应 thinking part。
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"sig-9"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"回复"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":9}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
    // 签名 delta：kind=reasoning、text 空、metadata 带 signature 与 blockIndex。
    let signature = deltas
        .iter()
        .find_map(|delta| match delta {
            ChatDelta::Part(part) if part.metadata.is_some() => Some(part),
            _ => None,
        })
        .expect("signature delta exists");
    assert_eq!(signature.kind, PartKind::Reasoning);
    assert_eq!(signature.text, "", "空文本帧不生成正文");
    assert_eq!(
        signature
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("signature"))
            .and_then(Value::as_str),
        Some("sig-9")
    );
    // thinking 文本与正文分离。
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Part(part) if part.kind == PartKind::Reasoning && part.text == "想法"
    )));
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Part(part) if part.kind == PartKind::Text && part.text == "回复"
    )));
    // thinking block 结束时明确发出 ReasoningEnded（item_id = block index）。
    assert!(deltas.contains(&ChatDelta::ReasoningEnded {
        item_id: Some("0".into())
    }));
    // 正常完成：ResponseFinished{Final} + 一个 Finished。
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

#[test]
fn ping_and_block_stop_are_ignored() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
    let text: String = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) if part.kind == PartKind::Text => Some(part.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "ok");
}

#[test]
fn usage_from_message_start_and_delta() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":50,"cache_read_input_tokens":7}}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":9}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(50) && usage.cache_read == Some(7)
    )));
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.output == Some(9)
    )));
}

// ---------------------------------------------------------------- 解码：工具

/// 单工具调用：Started → Delta(s) → Ended → ResponseFinished{ToolCalls} → Finished。
#[test]
fn single_tool_use_full_sequence() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":20}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"src/main.rs\"}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":12}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:toolu_a:read:0".to_string(),
            "delta:toolu_a:{\"path\":".to_string(),
            "delta:toolu_a:\"src/main.rs\"}".to_string(),
            "ended:toolu_a:{\"path\":\"src/main.rs\"}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// 同响应两个 tool_use block：按 block index 顺序、互不混淆。
#[test]
fn two_tool_use_blocks_keep_block_index_order() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a\"}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_b","name":"write","input":{}}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"b\",\"content\":\"x\"}"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:toolu_a:read:0".to_string(),
            "delta:toolu_a:{\"path\":\"a\"}".to_string(),
            "ended:toolu_a:{\"path\":\"a\"}".to_string(),
            "started:toolu_b:write:1".to_string(),
            "delta:toolu_b:{\"path\":\"b\",\"content\":\"x\"}".to_string(),
            "ended:toolu_b:{\"path\":\"b\",\"content\":\"x\"}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// 交错 block：两个 tool_use 的 `input_json_delta` 交替到达，参数不得串味。
#[test]
fn interleaved_tool_use_deltas_do_not_mix() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_b","name":"search","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"pattern\":"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"a.rs\"}"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"fn main\"}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
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
            ("toolu_a".to_string(), "{\"path\":\"a.rs\"}".to_string()),
            (
                "toolu_b".to_string(),
                "{\"pattern\":\"fn main\"}".to_string()
            ),
        ]
    );
}

/// reasoning → tool → reasoning → text 交错：思考结束按 block index 绑定。
#[test]
fn reasoning_tool_reasoning_text_interleaving() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"先想"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a\"}"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"content_block_start","index":2,"content_block":{"type":"thinking"}}"#,
            r#"{"type":"content_block_delta","index":2,"delta":{"type":"thinking_delta","thinking":"再想"}}"#,
            r#"{"type":"content_block_stop","index":2}"#,
            r#"{"type":"content_block_start","index":3,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":3,"delta":{"type":"text_delta","text":"正文"}}"#,
            r#"{"type":"content_block_stop","index":3}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
    let trace: Vec<String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some(format!("part:{:?}:{}", part.kind, part.text)),
            ChatDelta::ToolInputStarted { call_id, .. } => Some(format!("started:{call_id}")),
            ChatDelta::ToolInputEnded { call_id, .. } => Some(format!("ended:{call_id}")),
            ChatDelta::ReasoningEnded { item_id } => Some(format!("reasoning-ended:{item_id:?}")),
            ChatDelta::ResponseFinished { stop } => Some(format!("response:{stop:?}")),
            ChatDelta::Finished { reason } => Some(format!("finished:{}", reason.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        trace,
        vec![
            "part:Reasoning:先想",
            "reasoning-ended:Some(\"0\")",
            "started:toolu_a",
            "ended:toolu_a",
            "part:Reasoning:再想",
            "reasoning-ended:Some(\"2\")",
            "part:Text:正文",
            "response:ToolCalls",
            "finished:complete",
        ]
    );
}

/// `content_block_start` 直接给完整 input（非流式参数）也能形成可执行调用。
#[test]
fn tool_use_with_complete_input_in_start_frame() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"write","input":{"path":"a","content":"x"}}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
    assert_eq!(
        trace(&deltas),
        vec![
            "started:toolu_a:write:0".to_string(),
            "delta:toolu_a:{\"content\":\"x\",\"path\":\"a\"}".to_string(),
            "ended:toolu_a:{\"content\":\"x\",\"path\":\"a\"}".to_string(),
            "response:ToolCalls".to_string(),
            "finished:complete".to_string(),
        ]
    );
}

/// `input_json_delta` 出现在非 tool_use block 上：协议错误，不执行。
#[test]
fn input_json_delta_on_non_tool_block_is_protocol_error() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#)
        .expect("start frame");
    let error = decoder
        .decode_frame(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
        )
        .expect_err("misplaced tool delta rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// 半截 JSON：content_block_stop 时校验失败（不发 Ended）。
#[test]
fn malformed_tool_arguments_fail_with_invalid_code() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
        )
        .expect("start frame");
    decoder
        .decode_frame(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
        )
        .expect("delta frame");
    let error = decoder
        .decode_frame(r#"{"type":"content_block_stop","index":0}"#)
        .expect_err("half json rejected");
    assert!(error.contains(TOOL_ARGUMENTS_INVALID_CODE), "{error}");
}

/// 参数超限：立即失败，不进入结束阶段。
#[test]
fn oversized_tool_arguments_fail_with_too_large_code() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"write","input":{}}}"#,
        )
        .expect("start frame");
    let big = "x".repeat(MAX_TOOL_ARGUMENTS_BYTES + 1);
    let frame = format!(
        r#"{{"type":"content_block_delta","index":0,"delta":{{"type":"input_json_delta","partial_json":"{big}"}}}}"#
    );
    let error = decoder
        .decode_frame(&frame)
        .expect_err("too large rejected");
    assert_eq!(error, TOOL_ARGUMENTS_TOO_LARGE_CODE);
}

/// `stop_reason=tool_use` 但 block 未闭合：协议错误，不执行。
#[test]
fn tool_use_stop_with_open_block_is_protocol_error() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
        )
        .expect("start frame");
    decoder
        .decode_frame(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
        )
        .expect("delta frame");
    let error = decoder
        .decode_frame(r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#)
        .expect_err("unclosed block rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// 同 call ID 出现在另一个 block index：协议错误。
#[test]
fn duplicate_call_id_across_blocks_is_protocol_error() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
        )
        .expect("first block");
    let error = decoder
        .decode_frame(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
        )
        .expect_err("duplicate id rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// `message_stop` 时仍有未闭合工具：协议错误（client 也会用 `has_open_tool_input` 兜底）。
#[test]
fn message_stop_with_open_tool_use_is_protocol_error() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_a","name":"read","input":{}}}"#,
        )
        .expect("start frame");
    assert!(decoder.has_open_tool_input());
    let error = decoder
        .decode_frame(r#"{"type":"message_stop"}"#)
        .expect_err("open block rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
    assert_eq!(TOOL_INPUT_TRUNCATED_CODE, "tool input truncated");
}

/// 未知 stop_reason：明确协议错误。
#[test]
fn unknown_stop_reason_is_protocol_error() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#)
        .expect_err("unknown reason rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

#[test]
fn eof_without_message_stop_is_protocol_error() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(&mut decoder, &[r#"{"type":"ping"}"#]);
    assert!(matches!(
        deltas.last(),
        Some(ChatDelta::Finished {
            reason: StopReason::Failed { .. }
        })
    ));
}

/// 供应商托管工具 block（服务端 web 搜索等）不得当成本地执行。
#[test]
fn hosted_tool_blocks_are_rejected() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search"}}"#,
        )
        .expect_err("hosted tool rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
    assert!(error.contains("server_tool_use"), "{error}");
}

/// 用量只报**本响应**的实际值：两次采样各自解码，由上层累加。
#[test]
fn usage_is_reported_per_response_only() {
    let mut first = Decoder::default();
    let first_deltas = decode_all(
        &mut first,
        &[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10}}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":2}}"#,
            r#"{"type":"message_stop"}"#,
        ],
    );
    let mut second = Decoder::default();
    let second_deltas = decode_all(
        &mut second,
        &[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":40}}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
            r#"{"type":"message_stop"}"#,
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
    assert_eq!(
        usages(&first_deltas),
        vec![(Some(10), None), (None, Some(2))]
    );
    assert_eq!(
        usages(&second_deltas),
        vec![(Some(40), None), (None, Some(5))]
    );
}

// ------------------------------------------------- G8 核查：原生 ID 跨响应唯一

/// **G8 核查**：Anthropic 的 call ID 来自 `content_block_start(tool_use).id`，
/// 解码器不自造 ID；两次响应（两个解码器）都直接用供应商给的 ID。
#[test]
fn call_ids_come_from_block_id_not_from_the_decoder() {
    let frames = &[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"read","input":{}}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a\"}"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        r#"{"type":"message_stop"}"#,
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
    assert_eq!(ids_of(frames), vec!["toolu_1".to_string()]);
    // 不同采样拿到不同供应商 ID 时不撞库。
    let second = &[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_2","name":"read","input":{}}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a\"}"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        r#"{"type":"message_stop"}"#,
    ];
    assert_ne!(ids_of(frames)[0], ids_of(second)[0]);
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

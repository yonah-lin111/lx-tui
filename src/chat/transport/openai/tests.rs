//! OpenAI Chat 适配测试：请求构造（tools/历史编码）、thinking、usage、工具事件序列。

use serde_json::{Value, json};

use super::*;
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, MAX_TOOL_ARGUMENTS_BYTES, ModelItem, ModelStepStop, PartKind,
    StopReason, TOOL_ARGUMENTS_INVALID_CODE, TOOL_ARGUMENTS_TOO_LARGE_CODE,
};
use crate::chat::transport::tools::{TOOL_INPUT_TRUNCATED_CODE, TOOL_PROTOCOL_ERROR_CODE};

/// 构造测试请求（纯文本历史）。
fn request() -> ChatRequest {
    ChatRequest {
        turns: vec![
            ChatTurn {
                role: "user".into(),
                text: "第一问".into(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "assistant".into(),
                text: "第一答".into(),
                reasoning_metadata: None,
            },
            ChatTurn {
                role: "user".into(),
                text: "第二问".into(),
                reasoning_metadata: None,
            },
        ],
        items: Vec::new(),
        model: crate::db::chat::ModelSnapshot {
            provider: "example".into(),
            model: "example-chat".into(),
            variant: None,
        },
        variant_params: None,
        purpose: crate::chat::events::RequestPurpose::Chat,
        system_prompt: None,
    }
}

/// 带工具调用的结构化历史（一次调用 + 结果）。
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
    let built =
        super::build("https://api.example.com/v1", Some("key"), req, true).expect("build ok");
    serde_json::from_str(&built.body).expect("json body")
}

/// 只解码协议帧（不补 EOF 终态），用于断言工具阶段的事件顺序。
fn decode_frames(decoder: &mut dyn super::super::StreamDecoder, frames: &[&str]) -> Vec<ChatDelta> {
    let mut out = Vec::new();
    for frame in frames {
        out.extend(decoder.decode_frame(frame).expect("frame decodes"));
    }
    out.extend(decoder.finish());
    out
}

fn decode_all(decoder: &mut dyn super::super::StreamDecoder, frames: &[&str]) -> Vec<ChatDelta> {
    let mut out = Vec::new();
    for frame in frames {
        out.extend(decoder.decode_frame(frame).expect("frame decodes"));
    }
    out.extend(decoder.finish());
    // client 的 EOF 终态补充逻辑：无协议信号时报中断错误。
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

/// 把 delta 序列收敛成便于断言的紧凑形式（只保留工具事件与响应终态）。
fn tool_trace(deltas: &[ChatDelta]) -> Vec<String> {
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
fn compatible_url_join_avoids_duplicate_v1() {
    let built = super::build("https://api.example.com/v1", Some("key"), &request(), true)
        .expect("build ok");
    assert_eq!(built.url, "https://api.example.com/v1/chat/completions");

    let built = super::build("https://api.example.com/v1/", Some("key"), &request(), true)
        .expect("build ok");
    assert_eq!(built.url, "https://api.example.com/v1/chat/completions");

    // 用户路径前缀保留：不以 /v1 结尾时不重复。
    let built = super::build(
        "https://api.example.com/paas/v4",
        Some("key"),
        &request(),
        true,
    )
    .expect("build ok");
    assert_eq!(
        built.url,
        "https://api.example.com/paas/v4/chat/completions"
    );
}

#[test]
fn official_uses_fixed_url() {
    let built = super::build("", Some("key"), &request(), false).expect("build ok");
    assert_eq!(built.url, "https://api.openai.com/v1/chat/completions");
}

/// 前置系统规则消息的条数（`role=user` 且以 `<system_instructions>` 开头）。
fn system_rule_prefix(body: &Value) -> usize {
    body["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .skip_while(|message| message["role"] == "system")
                .take_while(|message| {
                    message["role"] == "user"
                        && message["content"]
                            .as_str()
                            .is_some_and(|text| text.starts_with("<system_instructions>"))
                })
                .count()
        })
        .unwrap_or(0)
}

/// 纯文本 `items` 为空时：首条是 system（lx 指令），其后逐条等价于旧 `turns` 编码。
#[test]
fn body_uses_messages_and_stream() {
    let built = super::build("https://api.example.com/v1", Some("key"), &request(), true)
        .expect("build ok");
    let body: Value = serde_json::from_str(&built.body).expect("json body");
    assert_eq!(body["model"], "example-chat");
    assert_eq!(body["stream"], true);
    let messages = body["messages"].as_array().expect("messages array");
    // 首条 system 是提示首段；其余段以 user 规则前置（见 `system_messages`）。
    let (head, rest) =
        crate::chat::transport::system_messages(&crate::chat::prompt::system_prompt());
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], json!(head));
    let base = system_rule_prefix(&body) + 1;
    assert_eq!(base - 1, rest.len(), "前置规则段数必须与切分一致");
    // 真实历史与 turns 一一对应（旧行为等价）。
    assert_eq!(messages.len(), base + 3);
    assert_eq!(messages[base]["role"], "user");
    assert_eq!(messages[base]["content"], "第一问");
    assert_eq!(messages[base + 1]["role"], "assistant");
    assert_eq!(messages[base + 1]["content"], "第一答");
    assert_eq!(messages[base + 2]["role"], "user");
    assert_eq!(messages[base + 2]["content"], "第二问");
    // 认证只放 Authorization header，不进 URL/body。
    assert!(!built.url.contains("key"));
    assert!(!built.body.contains("key"));
    let auth = built
        .headers
        .iter()
        .find(|(name, _)| name == "Authorization")
        .expect("auth header");
    assert_eq!(auth.1, "Bearer key");
    assert!(built.tools_requested);
}

/// 四工具定义在 `tools` 字段里，schema 与 `available_tools()` 逐字一致。
#[test]
fn tools_field_matches_available_tools() {
    let body = body_of(&request());
    let tools = body["tools"].as_array().expect("tools array");
    let specs = crate::chat::tools::available_tools();
    assert_eq!(tools.len(), specs.len());
    for (wire, spec) in tools.iter().zip(specs) {
        assert_eq!(wire["type"], "function");
        assert_eq!(wire["function"]["name"], spec.name);
        assert_eq!(wire["function"]["description"], spec.description);
        assert_eq!(wire["function"]["parameters"], spec.parameters());
    }
    // `tool_choice` 不强制设置（保持端点默认 auto）。
    assert!(body.get("tool_choice").is_none());
}

/// 结构化历史：assistant 的 tool_calls + role=tool 成对编码；arguments 只序列化一次。
#[test]
fn tool_history_is_encoded_as_call_and_tool_message() {
    let body = body_of(&tool_request());
    let messages = body["messages"].as_array().expect("messages");
    // 首条 system + 前置规则段，其后是 user / assistant / tool。
    assert_eq!(messages[0]["role"], "system");
    let base = system_rule_prefix(&body) + 1;
    assert_eq!(messages.len(), base + 3);
    let user = &messages[base];
    assert_eq!(user["role"], "user");
    assert_eq!(user["content"], "读 src/main.rs");
    // assistant 的 tool_calls：arguments 是 JSON 字符串（不是嵌套对象、不被二次编码）。
    let assistant = &messages[base + 1];
    assert_eq!(assistant["role"], "assistant");
    assert!(assistant["content"].is_null());
    let calls = assistant["tool_calls"].as_array().expect("tool_calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["id"], "call_a");
    assert_eq!(calls[0]["type"], "function");
    assert_eq!(calls[0]["function"]["name"], "read");
    let arguments = calls[0]["function"]["arguments"]
        .as_str()
        .expect("arguments is a JSON string");
    let parsed: Value = serde_json::from_str(arguments).expect("arguments parse");
    assert_eq!(parsed, json!({"path": "src/main.rs"}));
    // 结果消息：role=tool + tool_call_id。
    let result = &messages[base + 2];
    assert_eq!(result["role"], "tool");
    assert_eq!(result["tool_call_id"], "call_a");
    assert_eq!(result["content"], "1: fn main() {}");
}

#[test]
fn no_variant_sends_no_reasoning_field() {
    assert!(body_of(&request()).get("reasoning_effort").is_none());
}

#[test]
fn variant_reasoning_effort_mapped() {
    let mut req = request();
    req.variant_params = Some(json!({"reasoningEffort": "high"}));
    assert_eq!(body_of(&req)["reasoning_effort"], "high");
}

#[test]
fn missing_key_is_rejected_before_request() {
    let error = super::build("https://api.example.com/v1", None, &request(), true)
        .expect_err("missing key rejected");
    assert_eq!(error, "missing api key");
}

/// 带 tools 的能力类 4xx 归类为“该端点不支持工具调用”，认证/限流保持原语义。
#[test]
fn capability_status_maps_to_tools_unsupported() {
    use crate::chat::transport::tools::is_tools_unsupported_status;
    assert!(is_tools_unsupported_status(400));
    assert!(is_tools_unsupported_status(422));
    assert!(!is_tools_unsupported_status(401));
    assert!(!is_tools_unsupported_status(429));
    // client 侧的归类函数：带 tools + 400 → 能力错误码前缀。
    assert_eq!(
        super::super::super::client::http_error_code_for_tests(400, None, true),
        crate::chat::transport::tools::TOOLS_UNSUPPORTED_CODE.to_string() + " · http 400"
    );
    assert_eq!(
        super::super::super::client::http_error_code_for_tests(401, None, true),
        "http 401"
    );
}

// ---------------------------------------------------------------- 解码：文本

#[test]
fn decodes_text_and_usage_and_completion() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"content":"你好"}}]}"#,
            r#"{"choices":[{"delta":{"content":"世界"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":42,"completion_tokens":7}}"#,
            // [DONE] 由 client 层跳过 JSON 解析，此处只验证 JSON 帧。
        ],
    );
    let text: String = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) if part.kind == PartKind::Text => Some(part.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "你好世界");
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(42) && usage.output == Some(7)
    )));
    // `finish_reason=stop` 且无工具：先 ResponseFinished{Final}，再恰好一个 Finished。
    assert!(deltas.contains(&ChatDelta::ResponseFinished {
        stop: ModelStepStop::Final
    }));
    assert_eq!(
        deltas
            .iter()
            .filter(|delta| matches!(delta, ChatDelta::Finished { .. }))
            .count(),
        1
    );
    assert!(deltas.contains(&ChatDelta::Finished {
        reason: StopReason::Complete
    }));
}

#[test]
fn decodes_native_reasoning_field_as_reasoning_part() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"reasoning_content":"思考中"}}]}"#,
            r#"{"choices":[{"delta":{"content":"答案"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ],
    );
    let kinds: Vec<(PartKind, String)> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some((part.kind, part.text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            (PartKind::Reasoning, "思考中".into()),
            (PartKind::Text, "答案".into()),
        ]
    );
}

#[test]
fn embedded_think_tags_are_split() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"content":"前 <th"}}]}"#,
            r#"{"choices":[{"delta":{"content":"ink>隐秘</think> 后"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ],
    );
    let kinds: Vec<(PartKind, String)> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some((part.kind, part.text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            (PartKind::Text, "前 ".into()),
            (PartKind::Reasoning, "隐秘".into()),
            (PartKind::Text, " 后".into()),
        ]
    );
}

#[test]
fn api_error_body_reports_code() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(r#"{"error":{"message":"rate limited","code":429}}"#)
        .expect_err("error body rejected");
    assert!(error.contains("api error"));
}

#[test]
fn eof_without_completion_signal_is_protocol_error() {
    // 设计要求：EOF 缺协议完成信号视为中断，不擅自标记完成。
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[r#"{"choices":[{"delta":{"content":"x"}}]}"#],
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

#[test]
fn usage_missing_stays_none() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"content":"x"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ],
    );
    assert!(
        !deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::Usage(_)))
    );
}

#[test]
fn completion_and_usage_in_same_frame_still_marks_finished() {
    // 回归：部分 OpenAI 兼容端点把 finish_reason 与 usage 放在同一帧。
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"content":"你好"}}]}"#,
            r#"{"choices":[{"delta":{"role":"assistant","content":""},"finish_reason":"stop"}],
                "usage":{"prompt_tokens":2005,"completion_tokens":174}}"#,
        ],
    );
    assert!(
        deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::Finished { .. })),
        "同帧的 finish_reason 必须仍产出完成信号"
    );
    assert!(
        deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::Usage(u) if u.input == Some(2005))),
        "usage 仍被提取"
    );
    assert!(decoder.has_completion(), "同帧 completion+usage 置完成位");
}

#[test]
fn empty_reasoning_content_produces_no_part() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"content":"hi","reasoning_content":""}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ],
    );
    assert!(
        !deltas.iter().any(|delta| matches!(
            delta,
            ChatDelta::Part(part) if part.kind == PartKind::Reasoning
        )),
        "空 reasoning_content 不产生 reasoning part"
    );
}

#[test]
fn done_marker_marks_completion() {
    let mut decoder = Decoder::default();
    // `[DONE]` 由 client 层调用 mark_done（此处直接验证解码器语义）。
    assert!(!decoder.has_completion());
    decoder.mark_done();
    assert!(decoder.has_completion(), "[DONE] 视为完成");
}

// ---------------------------------------------------------------- 解码：工具

/// 单工具调用完整序列：Started → Delta(s) → Ended → ResponseFinished{ToolCalls}。
#[test]
fn single_tool_call_emits_full_sequence() {
    let mut decoder = Decoder::default();
    let deltas = decode_frames(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"read","arguments":""}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"src/main.rs\"}"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":11,"completion_tokens":9}}"#,
            // `[DONE]` 由 client 层处理：只标本次响应流完成。
        ],
    );
    assert_eq!(
        tool_trace(&deltas),
        vec![
            "started:call_a:read:0".to_string(),
            "delta:call_a:{\"path\":".to_string(),
            "delta:call_a:\"src/main.rs\"}".to_string(),
            "ended:call_a:{\"path\":\"src/main.rs\"}".to_string(),
            "response:ToolCalls".to_string(),
        ]
    );
    // 工具阶段不发整轮终态；用户轮次结束由上层在工具执行后决定。
    assert!(
        !deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::Finished { .. })),
        "tool_calls 不是整轮终态"
    );
    assert!(matches!(
        deltas
            .iter()
            .find(|delta| matches!(delta, ChatDelta::Usage(_))),
        Some(ChatDelta::Usage(usage)) if usage.input == Some(11) && usage.output == Some(9)
    ));
}

/// 同响应两个工具：ordinal/index 与到达顺序一致，结束按 index 升序。
#[test]
fn two_tool_calls_keep_index_order() {
    let mut decoder = Decoder::default();
    // 第二个调用先到，但 index=1；收尾必须按 index 顺序。
    let deltas = decode_frames(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"write","arguments":"{\"path\":\"b\"}"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ],
    );
    assert_eq!(
        tool_trace(&deltas),
        vec![
            "started:call_b:write:1".to_string(),
            "delta:call_b:{\"path\":\"b\"}".to_string(),
            "started:call_a:read:0".to_string(),
            "delta:call_a:{\"path\":\"a\"}".to_string(),
            "ended:call_a:{\"path\":\"a\"}".to_string(),
            "ended:call_b:{\"path\":\"b\"}".to_string(),
            "response:ToolCalls".to_string(),
        ]
    );
}

/// reasoning → tool → reasoning → text 交错：part 顺序严格按到达顺序。
#[test]
fn reasoning_tool_reasoning_text_interleaving() {
    let mut decoder = Decoder::default();
    let deltas = decode_frames(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"reasoning_content":"先想"}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"正文"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ],
    );
    let trace: Vec<String> = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) => Some(format!("part:{:?}:{}", part.kind, part.text)),
            ChatDelta::ToolInputStarted { call_id, .. } => Some(format!("started:{call_id}")),
            ChatDelta::ToolInputEnded { call_id, .. } => Some(format!("ended:{call_id}")),
            ChatDelta::ResponseFinished { stop } => Some(format!("response:{stop:?}")),
            _ => None,
        })
        .collect();
    assert_eq!(
        trace,
        vec![
            "part:Reasoning:先想",
            "started:call_a",
            "part:Text:正文",
            "ended:call_a",
            "response:ToolCalls",
        ]
    );
}

/// id 与 name 晚于参数片到达时：Started 在可识别的那一帧发出，并补发此前参数。
#[test]
fn late_id_still_emits_started_once_with_buffered_arguments() {
    let mut decoder = Decoder::default();
    let deltas = decode_frames(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"\"a.rs\"}"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ],
    );
    // Started 在可识别的一帧才发出，此前累计的参数片段一次性补发（不丢片段、不重复）。
    assert_eq!(
        tool_trace(&deltas),
        vec![
            "started:call_a:read:0".to_string(),
            "delta:call_a:{\"path\":\"a.rs\"}".to_string(),
            "ended:call_a:{\"path\":\"a.rs\"}".to_string(),
            "response:ToolCalls".to_string(),
        ]
    );
}

/// 非法 JSON 参数：不发 Ended、不产生可执行调用，按参数错误失败。
#[test]
fn malformed_arguments_fail_with_invalid_code() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{\"path\":"}}]},"finish_reason":"tool_calls"}]}"#,
        )
        .expect_err("half json rejected");
    assert!(error.contains(TOOL_ARGUMENTS_INVALID_CODE), "{error}");
}

/// 半截参数继续累计超过上限：立即报超限错误码。
#[test]
fn oversized_arguments_fail_with_too_large_code() {
    let mut decoder = Decoder::default();
    // 恰好等于上限仍然合法，超出 1 字节即失败。
    let big = "x".repeat(MAX_TOOL_ARGUMENTS_BYTES + 1);
    let frame = format!(
        r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"id":"call_a","function":{{"name":"write","arguments":"{big}"}}}}]}}}}]}}"#
    );
    let error = decoder
        .decode_frame(&frame)
        .expect_err("too large rejected");
    assert_eq!(error, TOOL_ARGUMENTS_TOO_LARGE_CODE);
}

/// 同 index 换 name → 协议错误（绝不静默换工具执行）。
#[test]
fn conflicting_name_on_same_index_is_protocol_error() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":""}}]}}]}"#,
        )
        .expect("first frame");
    let error = decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"write","arguments":""}}]}}]}"#,
        )
        .expect_err("name change rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// 同 call ID 出现在另一个 index → 协议错误。
#[test]
fn conflicting_call_id_on_another_index_is_protocol_error() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{}"}}]}}]}"#,
        )
        .expect("first frame");
    let error = decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"call_a","function":{"name":"read","arguments":"{}"}}]}}]}"#,
        )
        .expect_err("duplicate id rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// `finish_reason=tool_calls` 但没有任何调用：协议错误，不执行。
#[test]
fn tool_calls_finish_without_calls_is_protocol_error() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#)
        .expect_err("missing calls rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
    // 诊断文案必须回显真实结束原因，便于区分 `function_call` 与 `tool_calls`。
    assert!(error.contains("finish_reason=tool_calls"), "{error}");

    let mut legacy = Decoder::default();
    let error = legacy
        .decode_frame(r#"{"choices":[{"delta":{},"finish_reason":"function_call"}]}"#)
        .expect_err("missing legacy call rejected");
    assert!(error.contains("finish_reason=function_call"), "{error}");
}

/// 先给过工具分片却以 stop 结束：不一致，不猜执行。
#[test]
fn stop_after_tool_fragments_is_protocol_error() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{}"}}]}}]}"#,
        )
        .expect("fragment frame");
    let error = decoder
        .decode_frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#)
        .expect_err("inconsistent finish rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// 未知 finish_reason：明确协议错误（不再有“unsupported finish_reason”含糊分支）。
#[test]
fn unknown_finish_reason_is_protocol_error() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(r#"{"choices":[{"delta":{},"finish_reason":"eos"}]}"#)
        .expect_err("unknown reason rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// EOF 时工具参数未闭合：`has_open_tool_input` 为真（client 判定截断失败）。
#[test]
fn eof_with_open_tool_input_is_reported() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{\"path\":"}}]}}]}"#,
        )
        .expect("fragment frame");
    assert!(decoder.has_open_tool_input());
    // 截断错误码稳定，供 client/UI 归类。
    assert_eq!(TOOL_INPUT_TRUNCATED_CODE, "tool input truncated");
}

/// 重复 finish_reason 帧幂等：不重复发 ResponseFinished。
#[test]
fn duplicate_finish_reason_is_idempotent() {
    let mut decoder = Decoder::default();
    let first = decoder
        .decode_frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#)
        .expect("first finish");
    let second = decoder
        .decode_frame(r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#)
        .expect("second finish");
    assert!(
        first
            .iter()
            .any(|delta| matches!(delta, ChatDelta::ResponseFinished { .. }))
    );
    assert!(second.is_empty(), "重复 finish 不再产生事件");
}

/// `[DONE]` 后仍有未闭合工具（异常端点）：client 依据 `has_open_tool_input` 判失败。
#[test]
fn done_marker_does_not_close_open_tool_input() {
    let mut decoder = Decoder::default();
    decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{\"path\":"}}]}}]}"#,
        )
        .expect("fragment frame");
    decoder.mark_done();
    assert!(decoder.has_completion());
    assert!(decoder.has_open_tool_input(), "[DONE] 不等于工具参数闭合");
}

/// `finish_reason=tool_calls` 之后即使没有 `[DONE]`，EOF 也应判为完成（不是中断）。
#[test]
fn tool_calls_finish_marks_completion_without_done_marker() {
    let mut decoder = Decoder::default();
    let deltas = decode_frames(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read","arguments":"{}"}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ],
    );
    assert!(
        decoder.has_completion(),
        "finish_reason 已是本次响应的完成信号"
    );
    assert!(!decoder.has_open_tool_input());
    assert!(deltas.contains(&ChatDelta::ResponseFinished {
        stop: ModelStepStop::ToolCalls
    }));
}

/// 用量只报**本响应**的实际值：两次采样各自解码，由上层累加。
#[test]
fn usage_is_reported_per_response_only() {
    let mut first = Decoder::default();
    let first_deltas = decode_all(
        &mut first,
        &[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2}}"#,
        ],
    );
    let mut second = Decoder::default();
    let second_deltas = decode_all(
        &mut second,
        &[
            r#"{"choices":[{"delta":{"content":"b"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":40,"completion_tokens":5}}"#,
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
    // 每个解码器只报自己那一次的用量（不累计、不重复）。
    assert_eq!(usages(&first_deltas), vec![(Some(10), Some(2))]);
    assert_eq!(usages(&second_deltas), vec![(Some(40), Some(5))]);
}

/// 旧协议 `delta.function_call`（无 index）：明确协议错误，不猜调用。
#[test]
fn deprecated_function_call_delta_is_protocol_error() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"function_call":{"name":"read","arguments":"{}"}}}]}"#,
        )
        .expect_err("deprecated function_call rejected");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
    assert!(error.contains("function_call"), "{error}");
}

/// 回归：兼容端点在**每一帧**都回 `"function_call": null` / `"tool_calls": null`
/// 占位键（DeepSeek 等网关常见）。这属于普通对话帧，不能被当成“废弃协议”失败，
/// 否则像 `你是谁` 这样的纯聊天请求整轮报
/// `tool call protocol error: deprecated function_call delta without tool_calls index`。
///
/// 用户实测的第二种形态是**空字符串占位**：
/// `"function_call":{"name":"","arguments":""}`（正文完整后于流末出现）。
#[test]
fn null_function_call_placeholders_are_not_protocol_errors() {
    let mut decoder = Decoder::default();
    let deltas = decode_frames(
        &mut decoder,
        &[
            // 首帧：role/空 content + 两个 null 占位键。
            r#"{"choices":[{"delta":{"role":"assistant","content":"","function_call":null,"tool_calls":null},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"content":"你","function_call":null},"finish_reason":null}]}"#,
            // 空 tool_calls 数组同样是占位，不应触发 `without any tool call`。
            r#"{"choices":[{"delta":{"content":"好","tool_calls":[]},"finish_reason":null}]}"#,
            // 空对象占位。
            r#"{"choices":[{"delta":{"function_call":{}},"finish_reason":null}]}"#,
            // 用户实测形态：空字符串字段占位（流末出现）。
            r#"{"choices":[{"delta":{"function_call":{"name":"","arguments":""}},"finish_reason":null}]}"#,
            // 字段为 null 的占位。
            r#"{"choices":[{"delta":{"function_call":{"name":null,"arguments":null}},"finish_reason":null}]}"#,
            // 仅空 arguments 的占位。
            r#"{"choices":[{"delta":{"function_call":{"arguments":""}},"finish_reason":"stop"}]}"#,
        ],
    );
    let text: String = deltas
        .iter()
        .filter_map(|delta| match delta {
            ChatDelta::Part(part) if part.kind == PartKind::Text => Some(part.text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "你好");
    assert!(
        !deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::ToolInputStarted { .. })),
        "空占位不得凭空造出工具调用"
    );
    assert!(
        matches!(deltas.last(), Some(ChatDelta::Finished { .. })),
        "纯聊天帧仍以恰好一个终态结束: {deltas:?}"
    );
}

/// 回归：没有工具调用时，兼容网关回的空壳 `tool_calls` 条目不是协议错误。
#[test]
fn placeholder_tool_call_entries_are_ignored() {
    let mut decoder = Decoder::default();
    let deltas = decode_frames(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"content":"x","tool_calls":[{}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"","function":{"name":"","arguments":""}}]},"finish_reason":null}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ],
    );
    assert!(
        !deltas
            .iter()
            .any(|delta| matches!(delta, ChatDelta::ToolInputStarted { .. })),
        "空壳条目不得造出调用: {deltas:?}"
    );
    assert!(matches!(deltas.last(), Some(ChatDelta::Finished { .. })));
}

/// 真正的旧式 `function_call` 调用体（带 name）仍必须报协议错误，不静默忽略。
#[test]
fn deprecated_function_call_with_body_still_fails_after_null_guard() {
    let mut decoder = Decoder::default();
    for frame in [
        r#"{"choices":[{"delta":{"function_call":{"name":"read","arguments":"{}"}}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":null,"function_call":{"arguments":"{}"}}}]}"#,
    ] {
        let error = decoder
            .decode_frame(frame)
            .expect_err("real deprecated function_call rejected");
        assert!(error.contains("deprecated function_call"), "{error}");
    }
}

// ------------------------------------------------- G8 核查：原生 ID 跨响应唯一

/// **G8 核查**：Chat Completions 的 call ID 来自供应商原生 `tool_calls[].id`，
/// 解码器不自造 ID，因此同一会话的多次采样不会因解码器复用而撞库。
///
/// 这里用**两个独立解码器**（等价于两次模型响应）喂同一份 wire 帧：
/// 两次拿到的都是供应商给的原 ID，而不是解码器合成的位置 ID。
#[test]
fn call_ids_come_from_the_wire_not_from_the_decoder() {
    let frames = &[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_server_1","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":null}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
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
    // 同一份 wire 帧进两个解码器 → 同一个供应商 ID（解码器不参与生成）。
    assert_eq!(ids_of(frames), vec!["call_server_1".to_string()]);
    assert_eq!(ids_of(frames), ids_of(frames));
    // 供应商给不同 ID（同一轮的下一次采样）时不会撞库。
    let second = &[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_server_2","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":null}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
    ];
    assert_ne!(ids_of(frames)[0], ids_of(second)[0]);
}

/// **G8 核查**：缺 `id` 的 Chat 分片不会被合成假 ID（供应商 ID 是唯一来源）。
#[test]
fn missing_wire_id_is_a_protocol_error_not_a_synthesized_id() {
    let mut decoder = Decoder::default();
    let first = decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":null}]}"#,
        )
        .expect("first frame decodes");
    // 只有参数、没有 ID：不得发出 Started（不猜调用）。
    assert!(
        !first
            .iter()
            .any(|delta| matches!(delta, ChatDelta::ToolInputStarted { .. })),
        "缺 ID 不得发 Started：{first:?}"
    );
    // 闭合时按协议错误失败（不会补一个合成 ID 出来）。
    let error = decoder
        .decode_frame(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#)
        .expect_err("closing without id is a protocol error");
    assert!(error.contains(TOOL_PROTOCOL_ERROR_CODE), "{error}");
}

/// 摘要请求（`CompactSummary`）：不声明工具、系统提示用请求覆写值。
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
    let built =
        super::build("https://api.example.invalid", Some("key"), &request, true).expect("build");
    let body: serde_json::Value = serde_json::from_str(&built.body).expect("json");
    assert!(
        body.get("tools").is_none(),
        "摘要请求不得携带工具定义：{body}"
    );
    assert_eq!(body["messages"][0]["content"], "COMPACT INSTRUCTIONS");
    assert!(!built.tools_requested);
}

//! OpenAI Chat 适配测试：请求构造、thinking、usage、tool call 拒绝与标签拆分。

use serde_json::Value;

use super::*;
use crate::chat::events::{ChatDelta, PartKind, StopReason};

/// 构造测试请求。
fn request() -> ChatRequest {
    ChatRequest {
        turns: vec![
            crate::chat::events::ChatTurn {
                role: "user".into(),
                text: "第一问".into(),
                reasoning_metadata: None,
            },
            crate::chat::events::ChatTurn {
                role: "assistant".into(),
                text: "第一答".into(),
                reasoning_metadata: None,
            },
            crate::chat::events::ChatTurn {
                role: "user".into(),
                text: "第二问".into(),
                reasoning_metadata: None,
            },
        ],
        model: crate::db::chat::ModelSnapshot {
            provider: "example".into(),
            model: "example-chat".into(),
            variant: None,
        },
        variant_params: None,
    }
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

#[test]
fn body_uses_messages_and_stream() {
    let built = super::build("https://api.example.com/v1", Some("key"), &request(), true)
        .expect("build ok");
    let body: Value = serde_json::from_str(&built.body).expect("json body");
    assert_eq!(body["model"], "example-chat");
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["messages"].as_array().expect("messages array").len(),
        3
    );
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][1]["role"], "assistant");
    // 认证只放 Authorization header，不进 URL/body。
    assert!(!built.url.contains("key"));
    assert!(!built.body.contains("key"));
    let auth = built
        .headers
        .iter()
        .find(|(name, _)| name == "Authorization")
        .expect("auth header");
    assert_eq!(auth.1, "Bearer key");
}

#[test]
fn no_variant_sends_no_reasoning_field() {
    let built = super::build("https://api.example.com/v1", Some("key"), &request(), true)
        .expect("build ok");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert!(body.get("reasoning_effort").is_none());
}

#[test]
fn variant_reasoning_effort_mapped() {
    let mut req = request();
    req.variant_params = Some(serde_json::json!({"reasoningEffort": "high"}));
    let built =
        super::build("https://api.example.com/v1", Some("key"), &req, true).expect("build ok");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert_eq!(body["reasoning_effort"], "high");
}

#[test]
fn missing_key_is_rejected_before_request() {
    let error = super::build("https://api.example.com/v1", None, &request(), true)
        .expect_err("missing key rejected");
    assert_eq!(error, "missing api key");
}

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
    // 终态恰好一个；usage 帧在 finish_reason 之后到达，不改变结束原因。
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
fn tool_calls_are_rejected() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"choices":[{"delta":{"tool_calls":[{"id":"c1","function":{"name":"f"}}]}}]}"#,
        )
        .expect_err("tool call rejected");
    assert!(error.contains("tool calls are not supported"));
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
    // 此前 usage 命中即提前返回，finish_reason 被跳过 → 误报 “stream ended
    // without completion”，真实内容被标成 failed。
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
    // 端点常在每帧回 `reasoning_content:""`：不得产出空 reasoning part。
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"choices":[{"delta":{"content":"hi","reasoning_content":""}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[{"delta":{},"reasoning_content":"","finish_reason":"stop"}]}"#,
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

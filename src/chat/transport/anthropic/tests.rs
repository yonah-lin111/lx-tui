//! Anthropic Messages 适配测试：请求形状、签名绑定、usage 与 message_stop。

use serde_json::Value;

use super::*;
use crate::chat::events::{ChatDelta, PartKind, StopReason};

fn request() -> ChatRequest {
    ChatRequest {
        turns: vec![crate::chat::events::ChatTurn {
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
    }
}

fn decode_all(decoder: &mut dyn super::super::StreamDecoder, frames: &[&str]) -> Vec<ChatDelta> {
    let mut out = Vec::new();
    for frame in frames {
        out.extend(decoder.decode_frame(frame).expect("frame decodes"));
    }
    out.extend(decoder.finish());
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

#[test]
fn thinking_budget_maps_to_thinking_structure() {
    let mut req = request();
    req.variant_params = Some(serde_json::json!({"thinkingBudget": 4096}));
    let built = super::build("https://api.example.com", Some("key"), &req).expect("build ok");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 4096);
    // max_tokens 必须大于预算。
    assert!(body["max_tokens"].as_i64().unwrap_or(0) > 4096);
}

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
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"回复"}}"#,
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
    // usage：input 来自 message_start，output 来自 message_delta。
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.input == Some(50)
    )));
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Usage(usage) if usage.output == Some(9)
    )));
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
fn tool_input_delta_is_rejected() {
    let mut decoder = Decoder::default();
    let error = decoder
        .decode_frame(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{"}}"#,
        )
        .expect_err("tool input rejected");
    assert!(error.contains("tool calls are not supported"));
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

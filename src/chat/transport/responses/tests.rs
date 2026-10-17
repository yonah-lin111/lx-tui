//! Responses API 适配测试：请求形状、summary/raw reasoning、usage 与失败。

use serde_json::Value;

use super::*;
use crate::chat::events::{ChatDelta, PartKind, StopReason};

fn request() -> ChatRequest {
    ChatRequest {
        turns: vec![
            crate::chat::events::ChatTurn {
                role: "user".into(),
                text: "问".into(),
                reasoning_metadata: None,
            },
            crate::chat::events::ChatTurn {
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
fn body_uses_input_not_messages() {
    let built =
        super::build("https://api.example.com/v1", Some("key"), &request()).expect("build ok");
    assert_eq!(built.url, "https://api.example.com/v1/responses");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert!(body.get("input").is_some(), "Responses 用 input");
    assert!(body.get("messages").is_none(), "不沿用 messages 形状");
    assert_eq!(body["stream"], true);
    assert_eq!(body["model"], "grok");
}

#[test]
fn variant_maps_reasoning_structure() {
    let mut req = request();
    req.variant_params = Some(serde_json::json!({
        "reasoningEffort": "high",
        "reasoningSummary": "auto",
    }));
    let built = super::build("https://api.example.com/v1", Some("key"), &req).expect("build ok");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert_eq!(body["reasoning"]["effort"], "high");
    assert_eq!(body["reasoning"]["summary"], "auto");
}

#[test]
fn no_variant_sends_no_reasoning() {
    let built =
        super::build("https://api.example.com/v1", Some("key"), &request()).expect("build ok");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert!(body.get("reasoning").is_none());
}

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

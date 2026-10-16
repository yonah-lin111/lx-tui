//! Google Gemini 适配测试：请求形状（contents/role/Key 位置）、thought part 与 usage。

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
            crate::chat::events::ChatTurn {
                role: "user".into(),
                text: "续问".into(),
                reasoning_metadata: None,
            },
        ],
        model: crate::db::chat::ModelSnapshot {
            provider: "gemini".into(),
            model: "gemini-flash".into(),
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

#[test]
fn thinking_budget_maps_to_generation_config() {
    let mut req = request();
    req.variant_params = Some(serde_json::json!({"thinkingBudget": 1024}));
    let built = super::build("https://example.invalid", Some("g-key"), &req).expect("build ok");
    let body: Value = serde_json::from_str(&built.body).expect("json");
    assert_eq!(
        body["generationConfig"]["thinkingConfig"]["thinkingBudget"],
        1024
    );
}

#[test]
fn thought_part_is_reasoning_and_order_preserved() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            // 同帧多 part：thought 在前、正文在后，顺序保持。
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
    assert_eq!(
        deltas.last(),
        Some(&ChatDelta::Finished {
            reason: StopReason::Complete
        })
    );
}

#[test]
fn thought_false_part_is_text() {
    let mut decoder = Decoder::default();
    let deltas = decode_all(
        &mut decoder,
        &[
            r#"{"candidates":[{"content":{"parts":[{"text":"普通"}],"role":"model"},"finishReason":"STOP"}]}"#,
        ],
    );
    assert!(deltas.iter().any(|delta| matches!(
        delta,
        ChatDelta::Part(part) if part.kind == PartKind::Text && part.text == "普通"
    )));
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

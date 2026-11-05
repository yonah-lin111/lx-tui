//! API Key 校验与模型发现测试：离线短路、目录合并与本地 mock 端点。

use std::collections::BTreeMap;

use super::*;

#[tokio::test]
async fn empty_api_key_is_rejected_without_network() {
    let error = verify_provider_api_key(
        ProviderTransport::Openai,
        "https://api.example.com/v1",
        "   ",
        None,
    )
    .await
    .expect_err("empty key");
    assert_eq!(error, "API key cannot be empty");

    let error = fetch_provider_models(
        ProviderTransport::Google,
        "https://generativelanguage.googleapis.com/v1beta",
        "",
    )
    .await
    .expect_err("empty key");
    assert_eq!(error, "API key cannot be empty");
}

#[tokio::test]
async fn invalid_base_url_is_rejected() {
    let error = verify_provider_api_key(ProviderTransport::Openai, "ftp://bad", "key", None)
        .await
        .expect_err("bad url");
    assert!(error.contains("scheme"), "{error}");
}

#[tokio::test]
async fn offline_dummy_hosts_short_circuit() {
    for base_url in [
        "https://api.example.invalid/v1",
        "http://localhost:20128/v1",
        "https://vendor.invalid",
    ] {
        let outcome =
            verify_provider_api_key(ProviderTransport::OpenaiCompatible, base_url, "k", None)
                .await
                .expect("offline dummy");
        assert!(outcome.discovered_models.is_empty(), "{base_url}");
    }
}

#[test]
fn parse_models_payload_supports_openai_google_and_strings() {
    let openai = serde_json::json!({
        "data": [
            {"id": "gpt-5", "display_name": "GPT-5"},
            {"id": "gpt-5"}
        ]
    });
    let models = parse_discovered_models(&openai).expect("openai payload");
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "gpt-5");
    assert_eq!(models[0].name.as_deref(), Some("GPT-5"));

    let google = serde_json::json!({
        "models": [
            {"name": "models/gemini-3-pro", "displayName": "Gemini 3 Pro"}
        ]
    });
    let models = parse_discovered_models(&google).expect("google payload");
    assert_eq!(models[0].id, "gemini-3-pro");
    assert_eq!(models[0].name.as_deref(), Some("Gemini 3 Pro"));

    let strings = serde_json::json!(["models/a", "b"]);
    let models = parse_discovered_models(&strings).expect("string payload");
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn merge_keeps_existing_and_uses_catalog_metadata() {
    let mut existing = BTreeMap::new();
    existing.insert(
        "custom".to_string(),
        ModelConfig {
            id: "custom".to_string(),
            name: "custom".to_string(),
            transport: None,
            limit: None,
            modalities: None,
            variants: BTreeMap::new(),
            variant: None,
            pricing: None,
            extra: serde_json::Map::new(),
        },
    );
    let catalog = default_models_for_preset("openai");
    let catalog_id = catalog.keys().next().expect("openai catalog model").clone();
    let discovered = vec![
        DiscoveredModel {
            id: "custom".to_string(),
            name: Some("Custom Display".to_string()),
        },
        DiscoveredModel {
            id: catalog_id.clone(),
            name: None,
        },
    ];
    let merged = merge_discovered_models_for_preset("openai", Some(&existing), &discovered);
    assert_eq!(merged["custom"].name, "Custom Display");
    assert_eq!(merged[&catalog_id], catalog[&catalog_id]);
}

#[test]
fn merge_infers_reasoning_variants_for_new_models() {
    let discovered = vec![
        DiscoveredModel {
            id: "o3-mini".to_string(),
            name: None,
        },
        DiscoveredModel {
            id: "claude-opus-4-1".to_string(),
            name: Some("Claude Opus 4.1".to_string()),
        },
        DiscoveredModel {
            id: "plain-chat".to_string(),
            name: None,
        },
    ];
    let merged = merge_discovered_models_for_preset("unknown-provider", None, &discovered);
    let o3 = &merged["o3-mini"];
    assert_eq!(o3.name, "o3-mini");
    assert_eq!(
        o3.variants.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["high", "low", "medium"]
    );
    assert_eq!(o3.variants["high"]["reasoningEffort"], "high");
    assert!(!merged["claude-opus-4-1"].variants.is_empty());
    assert!(merged["plain-chat"].variants.is_empty());
}

#[tokio::test]
async fn fetch_models_handles_google_models_payload_from_local_mock() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind mock");
    let port = listener.local_addr().expect("addr").port();
    let mock = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let read = stream.read(&mut chunk).await.expect("read");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let payload = r#"{"models":[
            {"name":"models/gemini-3-pro","displayName":"Gemini 3 Pro"},
            {"name":"models/gemini-2.5-flash","displayName":"Gemini 2.5 Flash"}
        ]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
        String::from_utf8_lossy(&buffer).into_owned()
    });

    let models = fetch_provider_models(
        ProviderTransport::Google,
        &format!("http://127.0.0.1:{port}/v1beta"),
        "test-key",
    )
    .await
    .expect("fetch google models");
    assert_eq!(
        models
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        vec!["gemini-2.5-flash", "gemini-3-pro"]
    );
    let request = mock.await.expect("mock join");
    assert!(request.contains("GET /v1beta/models"), "{request}");
    assert!(
        request.to_ascii_lowercase().contains("x-goog-api-key"),
        "{request}"
    );
}

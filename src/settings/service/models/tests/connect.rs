//! Quick Connect / 凭证校验相关 API 测试（从 `tests.rs` 按场景拆分）。

use super::{put_body, revision_of, start_models, state_user_root};
use crate::settings::service::SettingsEvent;
use crate::settings::service::tests_helpers::{body, get, request, status, write};

use std::time::Duration;

#[tokio::test]
async fn get_settings_separates_custom_providers_and_quick_connect_presets() {
    let (service, _events, _dir) = start_models("models-quick-connect-list").await;
    let port = service.port();
    let token = service.token().to_string();

    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &put_body(),
            None,
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{}", body(&saved));

    let response = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    assert_eq!(status(&response), 200);
    let parsed: serde_json::Value = serde_json::from_str(body(&response)).expect("json");

    // `providers` 只含 ai.json 的自定义供应商。
    assert_eq!(parsed["providers"].as_array().expect("providers").len(), 1);
    assert_eq!(parsed["providers"][0]["id"], serde_json::json!("p"));

    // `quickConnect` 返回全部预设，按名称 A-Z 排序。
    let presets = parsed["quickConnect"]
        .as_array()
        .expect("quickConnect array");
    assert!(presets.len() > 100, "preset catalog present");
    let mut names: Vec<String> = presets
        .iter()
        .map(|preset| {
            preset["name"]
                .as_str()
                .expect("preset name")
                .to_ascii_lowercase()
        })
        .collect();
    let sorted = {
        let mut copy = names.clone();
        copy.sort();
        copy
    };
    assert_eq!(names, sorted, "quickConnect sorted A-Z by name");
    names.clear();

    let openai = presets
        .iter()
        .find(|preset| preset["id"] == serde_json::json!("openai"))
        .expect("openai preset");
    for field in [
        "note",
        "defaultBaseUrl",
        "baseUrl",
        "connectionLabel",
        "formAnswers",
        "envVars",
        "methods",
        "models",
        "defaultModels",
    ] {
        assert!(openai.get(field).is_some(), "preset field: {field}");
    }
    assert_eq!(openai["type"], serde_json::json!("openai"));
    assert!(
        !openai["defaultModels"]
            .as_array()
            .expect("default models")
            .is_empty(),
        "preset default models"
    );
    let methods = openai["methods"].as_array().expect("methods");
    assert!(
        methods
            .iter()
            .any(|method| method["type"] == serde_json::json!("oauth")),
        "openai exposes OAuth methods"
    );
    assert!(
        methods
            .iter()
            .any(|method| method["type"] == serde_json::json!("apiKey")),
        "openai exposes an API key method"
    );
    // 未保存凭证时状态标签只能是未连接或环境变量命中。
    let label = openai["connectionLabel"].as_str().expect("label");
    assert!(
        label == "Not connected" || label.starts_with("Connected · $"),
        "unexpected label: {label}"
    );
    service.shutdown();
}
#[tokio::test]
async fn put_connect_saves_credentials_without_polluting_ai_json() {
    let (service, mut events, _dir) = start_models("models-connect-save").await;
    let port = service.port();
    let token = service.token().to_string();
    let root = state_user_root(&service);

    let payload = r#"{"providers":[{"id":"openai","apiKey":"sk-quick-connect-test","enabled":true,"baseUrl":"https://api.example.invalid/v1"}]}"#;
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/connect?token={token}"),
            payload,
            None,
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{}", body(&saved));
    assert!(revision_of(body(&saved)).is_some(), "revision returned");

    // 凭证落在 connect.json，ai.json 不被创建。
    let connect_path = root.join("config").join("connect.json");
    let connect_text = std::fs::read_to_string(&connect_path).expect("connect.json written");
    assert!(
        connect_text.contains("sk-quick-connect-test"),
        "{connect_text}"
    );
    assert!(
        !root.join("config").join("ai.json").exists(),
        "ai.json stays untouched"
    );

    // GET 反映已连接状态与明文 Key。
    let got = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let parsed: serde_json::Value = serde_json::from_str(body(&got)).expect("json");
    let openai = parsed["quickConnect"]
        .as_array()
        .expect("quickConnect")
        .iter()
        .find(|preset| preset["id"] == serde_json::json!("openai"))
        .expect("openai preset");
    assert_eq!(openai["connected"], serde_json::json!(true));
    assert_eq!(openai["enabled"], serde_json::json!(true));
    assert_eq!(openai["connectionType"], serde_json::json!("apiKey"));
    assert_eq!(
        openai["connectionLabel"],
        serde_json::json!("Connected · API key")
    );
    assert_eq!(openai["apiKey"], serde_json::json!("sk-quick-connect-test"));

    // 广播的运行时快照包含该快捷连接供应商。
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .expect("event within timeout")
        .expect("event present");
    match event {
        SettingsEvent::ModelsChanged(settings) => {
            assert!(settings.providers.contains_key("openai"));
            assert!(settings.enabled_providers.iter().any(|id| id == "openai"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    service.shutdown();
}
#[tokio::test]
async fn put_settings_accepts_default_model_from_connected_preset() {
    let (service, _events, _dir) = start_models("models-connect-default").await;
    let port = service.port();
    let token = service.token().to_string();

    let connected = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/connect?token={token}"),
            r#"{"providers":[{"id":"openai","apiKey":"sk-default-model-test","baseUrl":"https://api.example.invalid/v1"}]}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&connected), 200, "{}", body(&connected));

    // 取该预设的一个内置模型 id。
    let got = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let parsed: serde_json::Value = serde_json::from_str(body(&got)).expect("json");
    let openai = parsed["quickConnect"]
        .as_array()
        .expect("quickConnect")
        .iter()
        .find(|preset| preset["id"] == serde_json::json!("openai"))
        .expect("openai preset");
    let model_id = openai["models"][0]["id"]
        .as_str()
        .expect("preset model id")
        .to_string();

    let payload = format!(
        r#"{{
  "providers": [{{"key": "p", "id": "p", "type": "openai-compatible", "name": "P",
    "baseUrl": "https://api.example.invalid/v1", "apiKey": "custom-key",
    "models": [{{"id": "m", "name": "M"}}]}}],
  "enabledProviders": ["p"],
  "defaultModel": {{"provider": "openai", "model": "{model_id}"}},
  "revision": ""
}}"#
    );
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &payload,
            None,
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{}", body(&saved));

    let after = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let after_json: serde_json::Value = serde_json::from_str(body(&after)).expect("json");
    assert_eq!(
        after_json["defaultModel"],
        serde_json::json!({"provider": "openai", "model": model_id, "variant": null})
    );
    let ai_text = std::fs::read_to_string(state_user_root(&service).join("config").join("ai.json"))
        .expect("ai.json written");
    assert!(ai_text.contains("\"openai\""), "{ai_text}");
    service.shutdown();
}
#[tokio::test]
async fn disconnect_removes_preset_and_clears_dangling_default_model() {
    let (service, mut events, _dir) = start_models("models-connect-disconnect").await;
    let port = service.port();
    let token = service.token().to_string();
    let root = state_user_root(&service);

    let connected = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/connect?token={token}"),
            r#"{"providers":[{"id":"openai","apiKey":"sk-disconnect-test","baseUrl":"https://api.example.invalid/v1"}]}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&connected), 200, "{}", body(&connected));
    let _ = events.try_recv();

    let got = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let parsed: serde_json::Value = serde_json::from_str(body(&got)).expect("json");
    let openai = parsed["quickConnect"]
        .as_array()
        .expect("quickConnect")
        .iter()
        .find(|preset| preset["id"] == serde_json::json!("openai"))
        .expect("openai preset");
    let model_id = openai["models"][0]["id"]
        .as_str()
        .expect("preset model id")
        .to_string();
    let payload = format!(
        r#"{{"providers": [], "enabledProviders": [], "defaultModel": {{"provider": "openai", "model": "{model_id}"}}, "revision": ""}}"#
    );
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &payload,
            None,
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{}", body(&saved));
    let _ = events.try_recv();

    let disconnected = request(
        port,
        &write(
            "POST",
            &format!("/api/models/connect/disconnect?token={token}"),
            r#"{"providerId":"openai"}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&disconnected), 200, "{}", body(&disconnected));

    // connect.json 已无该供应商；ai.json 的悬空默认选择被清理。
    let connect_text = std::fs::read_to_string(root.join("config").join("connect.json"))
        .expect("connect.json readable");
    assert!(
        !connect_text.contains("sk-disconnect-test"),
        "{connect_text}"
    );
    let after = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let after_json: serde_json::Value = serde_json::from_str(body(&after)).expect("json");
    assert_eq!(after_json["defaultModel"], serde_json::Value::Null);
    let openai = after_json["quickConnect"]
        .as_array()
        .expect("quickConnect")
        .iter()
        .find(|preset| preset["id"] == serde_json::json!("openai"))
        .expect("openai preset");
    assert_eq!(openai["connected"], serde_json::json!(false));
    assert_eq!(
        openai["connectionLabel"],
        serde_json::json!("Not connected")
    );

    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .expect("event within timeout")
        .expect("event present");
    match event {
        SettingsEvent::ModelsChanged(settings) => {
            assert!(!settings.providers.contains_key("openai"));
            assert!(settings.default_model.is_none());
        }
        other => panic!("unexpected event: {other:?}"),
    }
    service.shutdown();
}
#[tokio::test]
async fn oauth_start_status_and_cancel_round_trip() {
    let (service, _events, _dir) = start_models("models-connect-oauth").await;
    let port = service.port();
    let token = service.token().to_string();

    let started = request(
        port,
        &write(
            "POST",
            &format!("/api/models/connect/oauth/start?token={token}"),
            r#"{"providerId":"openai","methodId":"chatgpt-browser","formAnswers":{}}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&started), 200, "{}", body(&started));
    let parsed: serde_json::Value = serde_json::from_str(body(&started)).expect("json");
    let attempt_id = parsed["attemptId"]
        .as_str()
        .expect("attempt id")
        .to_string();
    assert!(!parsed["url"].as_str().expect("url").is_empty());
    assert_eq!(parsed["mode"], serde_json::json!("auto"));

    let status_response = request(
        port,
        &get(&format!(
            "/api/models/connect/oauth/status?attemptId={attempt_id}&token={token}"
        )),
    )
    .await;
    assert_eq!(status(&status_response), 200);
    let status_json: serde_json::Value =
        serde_json::from_str(body(&status_response)).expect("json");
    assert_eq!(status_json["status"], serde_json::json!("pending"));

    let cancelled = request(
        port,
        &write(
            "DELETE",
            &format!("/api/models/connect/oauth?attemptId={attempt_id}&token={token}"),
            "",
            None,
        ),
    )
    .await;
    assert_eq!(status(&cancelled), 200, "{}", body(&cancelled));

    let missing = request(
        port,
        &get(&format!(
            "/api/models/connect/oauth/status?attemptId={attempt_id}&token={token}"
        )),
    )
    .await;
    assert_eq!(status(&missing), 404);
    service.shutdown();
}
#[tokio::test]
async fn fetch_models_works_for_quick_connect_preset() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind mock");
    let model_port = listener.local_addr().expect("addr").port();
    let mock = tokio::spawn(async move {
        let mut requests = Vec::new();
        // 第一次是 PUT /connect 的 Key 校验，第二次是 POST /fetch。
        for _ in 0..2 {
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
            let payload = r#"{"data":[{"id":"quick-m1","owned_by":"preset"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
            requests.push(String::from_utf8_lossy(&buffer).into_owned());
        }
        requests.join("\n")
    });

    let (service, _events, _dir) = start_models("models-connect-fetch").await;
    let port = service.port();
    let token = service.token().to_string();
    let payload = format!(
        r#"{{"providers":[{{"id":"openai","apiKey":"sk-local-fetch","baseUrl":"http://127.0.0.1:{model_port}/v1"}}]}}"#
    );
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/connect?token={token}"),
            &payload,
            None,
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{}", body(&saved));

    // 校验成功后的 `/models` 结果已合并进 connect.json 的预设模型表。
    let after_connect = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    assert_eq!(status(&after_connect), 200);
    let after_connect: serde_json::Value =
        serde_json::from_str(body(&after_connect)).expect("json");
    let openai = after_connect["quickConnect"]
        .as_array()
        .expect("quickConnect")
        .iter()
        .find(|preset| preset["id"] == serde_json::json!("openai"))
        .expect("openai preset");
    assert!(
        openai["models"]
            .as_array()
            .expect("models")
            .iter()
            .any(|model| model["id"] == serde_json::json!("quick-m1")),
        "discovered model merged into preset record: {openai}"
    );

    let response = request(
        port,
        &write(
            "POST",
            &format!("/api/models/fetch?token={token}"),
            r#"{"providerId":"openai"}"#,
            None,
        ),
    )
    .await;
    assert_eq!(status(&response), 200, "fetch 成功: {}", body(&response));
    let parsed: serde_json::Value = serde_json::from_str(body(&response)).expect("json");
    assert_eq!(parsed["models"][0]["id"], serde_json::json!("quick-m1"));
    let recorded = mock.await.expect("mock join");
    assert!(
        recorded.contains("GET /v1/models"),
        "命中预设地址的 /v1/models：{recorded}"
    );
    service.shutdown();
}
/// 启动一次性本地 mock：返回端口与记录到的请求文本。
async fn spawn_json_response(
    status_line: &'static str,
    payload: &'static str,
) -> (u16, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind mock");
    let port = listener.local_addr().expect("addr").port();
    let handle = tokio::spawn(async move {
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
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
        String::from_utf8_lossy(&buffer).into_owned()
    });
    (port, handle)
}

#[tokio::test]
async fn connect_rejects_unauthorized_key_without_writing() {
    let (model_port, mock) = spawn_json_response(
        "401 Unauthorized",
        r#"{"error":{"message":"invalid x-api-key"}}"#,
    )
    .await;
    let (service, _events, _dir) = start_models("models-connect-401").await;
    let port = service.port();
    let token = service.token().to_string();
    let root = state_user_root(&service);

    let payload = format!(
        r#"{{"providers":[{{"id":"openai","apiKey":"sk-invalid-401","baseUrl":"http://127.0.0.1:{model_port}/v1"}}]}}"#
    );
    let response = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/connect?token={token}"),
            &payload,
            None,
        ),
    )
    .await;
    assert_eq!(
        status(&response),
        422,
        "invalid key rejected: {}",
        body(&response)
    );
    assert!(
        body(&response).contains("Invalid API key"),
        "error mentions invalid key: {}",
        body(&response)
    );
    assert!(
        !root.join("config").join("connect.json").exists(),
        "invalid key must not be written"
    );
    let recorded = mock.await.expect("mock join");
    assert!(recorded.contains("GET /v1/models"), "{recorded}");
    service.shutdown();
}

#[tokio::test]
async fn settings_put_rejects_unauthorized_key_without_writing() {
    let (model_port, mock) = spawn_json_response(
        "401 Unauthorized",
        r#"{"error":{"message":"unauthorized"}}"#,
    )
    .await;
    let (service, _events, _dir) = start_models("models-settings-401").await;
    let port = service.port();
    let token = service.token().to_string();
    let root = state_user_root(&service);

    let payload = format!(
        r#"{{
  "providers": [{{"key": "p", "id": "p", "type": "openai-compatible", "name": "P",
    "baseUrl": "http://127.0.0.1:{model_port}/v1", "apiKey": "sk-invalid-settings", "models": []}}],
  "enabledProviders": [],
  "revision": ""
}}"#
    );
    let response = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &payload,
            None,
        ),
    )
    .await;
    assert_eq!(
        status(&response),
        422,
        "invalid key rejected: {}",
        body(&response)
    );
    assert!(
        body(&response).contains("Invalid API key"),
        "error mentions invalid key: {}",
        body(&response)
    );
    assert!(
        !root.join("config").join("ai.json").exists(),
        "invalid key must not be written"
    );
    let recorded = mock.await.expect("mock join");
    assert!(recorded.contains("GET /v1/models"), "{recorded}");
    service.shutdown();
}

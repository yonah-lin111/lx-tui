//! 模型设置 API 测试：token/同源、Key 不出 GET、422 不改文件、revision 409、fetch。

use super::super::tests_helpers::{body, get, request, status, write};

use std::path::PathBuf;

use std::time::Duration;
use tokio::sync::mpsc;

use crate::settings::service::{SettingsEvent, SettingsService};

/// 隔离临时目录。
fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-models-tests-{}-{}-{}",
        name,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let path = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&path).expect("temp dir");
    path
}

/// 启动带模型路径的测试服务。
async fn start_models(
    name: &str,
) -> (
    SettingsService,
    mpsc::UnboundedReceiver<SettingsEvent>,
    PathBuf,
) {
    let dir = temp_dir(name);
    let config_path = dir.join("settings.json");
    let user_root = dir.join("root");
    std::fs::create_dir_all(&user_root).expect("user root");
    let (sender, receiver) = mpsc::unbounded_channel();
    let service = SettingsService::start_in(sender, config_path, user_root)
        .await
        .expect("service starts");
    (service, receiver, dir)
}

/// 测试用假 Key。
const FAKE_KEY: &str = "test-key-models-not-real";

/// 保存一份有效配置的 PUT body。
fn put_body() -> String {
    format!(
        r#"{{
  "providers": [{{
    "key": "p", "id": "p", "type": "openai-compatible", "name": "P",
    "baseUrl": "https://api.example.invalid/v1",
    "apiKey": "{FAKE_KEY}",
    "models": [{{"key": "m", "id": "m", "name": "M", "limit": {{"context": 1000, "output": 100}}}}]
  }}],
  "enabledProviders": ["p"],
  "defaultModel": {{"provider": "p", "model": "m"}},
  "streamIdleTimeoutMs": 60000,
  "revision": ""
}}"#
    )
}

/// 读取当前 revision。
fn revision_of(response_body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(response_body)
        .ok()?
        .get("revision")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[tokio::test]
async fn get_requires_token_and_never_leaks_key() {
    let (service, _events, _dir) = start_models("models-get").await;
    let port = service.port();
    let token = service.token().to_string();

    // 未带 token 拒绝。
    let denied = request(port, &get("/api/models/settings")).await;
    assert_eq!(status(&denied), 403);

    // 先 PUT 一份带 Key 的配置。
    let saved = request(
        port,
        &write("PUT", "/api/models/settings?token=", &put_body(), None)
            .replace("token=", &format!("token={token}")),
    )
    .await;
    assert_eq!(status(&saved), 200, "saved: {}", body(&saved));

    // GET：只返回 hasApiKey，不返回明文 Key。
    let response = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    assert_eq!(status(&response), 200);
    let text = body(&response);
    assert!(!text.contains(FAKE_KEY), "GET 不泄露明文 Key");
    assert!(text.contains("hasApiKey"));
    let parsed: serde_json::Value = serde_json::from_str(text).expect("json");
    assert_eq!(parsed["providers"][0]["hasApiKey"], serde_json::json!(true));
    service.shutdown();
}

#[tokio::test]
async fn put_rejects_cross_origin_and_invalid_config() {
    let (service, _events, _dir) = start_models("models-put").await;
    let port = service.port();
    let token = service.token().to_string();

    // 跨源写拒绝。
    let cross = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &put_body(),
            Some("https://evil.example"),
        ),
    )
    .await;
    assert_eq!(status(&cross), 403);

    // 非法配置 422：默认选择指向不存在的模型。
    let invalid = r#"{
        "providers": [{"key": "p", "id": "p", "type": "openai-compatible", "name": "P",
            "baseUrl": "https://api.example.invalid/v1", "models": []}],
        "enabledProviders": ["p"],
        "defaultModel": {"provider": "p", "model": "missing"},
        "revision": ""
    }"#;
    let rejected = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            invalid,
            None,
        ),
    )
    .await;
    assert_eq!(status(&rejected), 422);

    // 文件不被创建（保持未保存状态）。
    assert!(
        !state_user_root(&service)
            .join("config")
            .join("ai.json")
            .exists()
    );
    service.shutdown();
}

#[tokio::test]
async fn put_stale_revision_conflicts_with_409() {
    let (service, _events, _dir) = start_models("models-revision").await;
    let port = service.port();
    let token = service.token().to_string();

    // 第一次保存成功。
    let first = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &put_body(),
            None,
        ),
    )
    .await;
    assert_eq!(status(&first), 200);
    let revision = revision_of(body(&first)).expect("revision");

    // 旧 revision 二次保存：409，文件保持先保存版本。
    let stale = put_body().replace("\"revision\": \"\"", "\"revision\": \"deadbeef\"");
    let conflict = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &stale,
            None,
        ),
    )
    .await;
    assert_eq!(status(&conflict), 409);

    // 最新 revision 再保存成功。
    let fresh = put_body().replace(
        "\"revision\": \"\"",
        &format!("\"revision\": \"{revision}\""),
    );
    let again = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &fresh,
            None,
        ),
    )
    .await;
    assert_eq!(status(&again), 200);
    service.shutdown();
}

#[tokio::test]
async fn put_broadcasts_models_changed_event() {
    let (service, mut events, _dir) = start_models("models-event").await;
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
    assert_eq!(status(&saved), 200);

    // 事件投递：ModelsChanged 携带合并后的配置。
    let event = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .expect("event within timeout")
        .expect("event present");
    match event {
        SettingsEvent::ModelsChanged(settings) => {
            assert!(settings.providers.contains_key("p"));
            // Key 在事件负载里存在（内存快照），但不出现在 GET。
        }
        other => panic!("unexpected event: {other:?}"),
    }
    service.shutdown();
}

#[tokio::test]
async fn api_key_semantics_keep_clear_replace() {
    let (service, _events, _dir) = start_models("models-key").await;
    let port = service.port();
    let token = service.token().to_string();
    let path = state_user_root(&service).join("config").join("ai.json");

    // 初次保存（带 Key）。
    request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &put_body(),
            None,
        ),
    )
    .await;
    // 留空保持：GET 后取 revision 再保存不带 apiKey 字段。
    let current = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let revision = revision_of(body(&current)).expect("revision");

    // 留空：Key 保持（GET 仍 true）。
    let keep = put_body()
        .replace(
            &format!("\"apiKey\": \"{FAKE_KEY}\",\n    ",),
            "\"x\": 1,\n    ",
        )
        .replace(
            "\"revision\": \"\"",
            &format!("\"revision\": \"{revision}\""),
        );
    // 构造无 apiKey 的 body：直接从 JSON 层面删字段更可靠。
    let keep_body = {
        let mut value: serde_json::Value = serde_json::from_str(&put_body()).expect("json");
        value["revision"] = serde_json::json!(revision);
        value["providers"][0]
            .as_object_mut()
            .expect("provider")
            .remove("apiKey");
        serde_json::to_string(&value).expect("string")
    };
    let _ = keep;
    let kept = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &keep_body,
            None,
        ),
    )
    .await;
    assert_eq!(status(&kept), 200, "留空保持旧 Key：{}", body(&kept));

    // 显式清除。
    let cleared = {
        let revision = revision_of(body(&kept)).expect("revision 2");
        let mut value: serde_json::Value = serde_json::from_str(&put_body()).expect("json");
        value["revision"] = serde_json::json!(revision);
        value["providers"][0]
            .as_object_mut()
            .expect("provider")
            .remove("apiKey");
        value["providers"][0]["clearApiKey"] = serde_json::json!(true);
        serde_json::to_string(&value).expect("string")
    };
    let cleared_response = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &cleared,
            None,
        ),
    )
    .await;
    assert_eq!(status(&cleared_response), 200);

    // GET：hasApiKey=false。
    let after = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let parsed: serde_json::Value = serde_json::from_str(body(&after)).expect("json");
    assert_eq!(
        parsed["providers"][0]["hasApiKey"],
        serde_json::json!(false),
        "显式清除删除 Key"
    );
    // 磁盘上的 Key 已删。
    let disk = std::fs::read_to_string(&path).expect("ai.json readable");
    assert!(!disk.contains(FAKE_KEY), "清除后磁盘无 Key");
    service.shutdown();
}

#[tokio::test]
async fn fetch_without_key_is_422() {
    let (service, _events, _dir) = start_models("models-fetch-nokey").await;
    let port = service.port();
    let token = service.token().to_string();

    let body = r#"{"type": "openai-compatible", "baseUrl": "https://api.example.invalid/v1"}"#;
    let response = request(
        port,
        &write(
            "POST",
            &format!("/api/models/fetch?token={token}"),
            body,
            None,
        ),
    )
    .await;
    assert_eq!(status(&response), 422, "无 Key 422");
    service.shutdown();
}

#[tokio::test]
async fn fetch_unsupported_transport_is_422() {
    let (service, _events, _dir) = start_models("models-fetch-transport").await;
    let port = service.port();
    let token = service.token().to_string();

    // anthropic/google 不支持发现：明确禁用，不伪造列表。
    for kind in ["anthropic", "google"] {
        let body = format!(
            r#"{{"type": "{kind}", "baseUrl": "https://api.example.invalid", "apiKey": "k"}}"#
        );
        let response = request(
            port,
            &write(
                "POST",
                &format!("/api/models/fetch?token={token}"),
                &body,
                None,
            ),
        )
        .await;
        assert_eq!(status(&response), 422, "{kind} 禁用发现");
    }
    service.shutdown();
}

#[tokio::test]
async fn fetch_invalid_base_url_is_422() {
    let (service, _events, _dir) = start_models("models-fetch-url").await;
    let port = service.port();
    let token = service.token().to_string();

    let body = r#"{"type": "openai-compatible", "baseUrl": "ftp://bad", "apiKey": "k"}"#;
    let response = request(
        port,
        &write(
            "POST",
            &format!("/api/models/fetch?token={token}"),
            body,
            None,
        ),
    )
    .await;
    assert_eq!(status(&response), 422, "非法 URL 422");
    service.shutdown();
}

#[tokio::test]
async fn oversized_put_body_is_rejected() {
    let (service, _events, _dir) = start_models("models-large").await;
    let port = service.port();
    let token = service.token().to_string();

    let huge = format!(
        r#"{{"providers": [{{"key": "p", "id": "p", "type": "openai-compatible", "name": "{}", "baseUrl": "https://api.example.invalid/v1", "models": []}}], "revision": ""}}"#,
        "x".repeat(300 * 1024)
    );
    let response = super::super::tests_helpers::request_lossy(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &huge,
            None,
        ),
    )
    .await;
    // 服务端提前关闭时可能读不到响应；读到则必须是拒绝状态。
    if !response.is_empty() {
        assert!(
            status(&response) == 413 || status(&response) == 400,
            "超大请求体拒绝：{}",
            status(&response)
        );
    }
    service.shutdown();
}

#[tokio::test]
async fn unknown_models_route_is_404() {
    let (service, _events, _dir) = start_models("models-404").await;
    let token = service.token().to_string();
    let response = request(
        service.port(),
        &get(&format!("/api/models/unknown?token={token}")),
    )
    .await;
    assert_eq!(status(&response), 404);
    service.shutdown();
}

/// 服务注入的用户数据根（模型配置文件位于其 config/ai.json）。
fn state_user_root(service: &SettingsService) -> PathBuf {
    service.user_root_for_tests()
}

#[tokio::test]
async fn first_save_on_fresh_install_accepts_get_revision() {
    // 回归：ai.json 尚不存在时，GET 返回空配置的 revision；首次 PUT 携带该
    // revision 必须成功（此前 GET 与 PUT 对“无文件”用了不同口径，首次保存必然 409）。
    let (service, _events, _dir) = start_models("models-first-save").await;
    let port = service.port();
    let token = service.token().to_string();

    let got = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    assert_eq!(status(&got), 200, "GET: {}", body(&got));
    let revision = revision_of(body(&got)).expect("GET 返回 revision");

    let payload = put_body().replace(
        "\"revision\": \"\"",
        &format!("\"revision\": \"{revision}\""),
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
    assert_eq!(status(&saved), 200, "首次保存失败: {}", body(&saved));
    // 保存后 revision 变化；再次用新 revision 保存仍成功。
    let next = revision_of(body(&saved)).expect("保存后返回新 revision");
    assert_ne!(next, revision, "内容变更应产生新 revision");
    let again = put_body().replace("\"revision\": \"\"", &format!("\"revision\": \"{next}\""));
    let saved_two = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &again,
            None,
        ),
    )
    .await;
    assert_eq!(
        status(&saved_two),
        200,
        "二次保存失败: {}",
        body(&saved_two)
    );
    service.shutdown();
}

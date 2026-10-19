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
async fn get_requires_token_and_returns_stored_key() {
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

    // GET：直接回传已存 Key，供设置页原地编辑。
    let response = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    assert_eq!(status(&response), 200);
    let parsed: serde_json::Value = serde_json::from_str(body(&response)).expect("json");
    assert_eq!(
        parsed["providers"][0]["apiKey"],
        serde_json::json!(FAKE_KEY),
        "GET 回传明文 Key"
    );
    assert!(parsed["providers"][0].get("hasApiKey").is_none());
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
async fn model_record_key_follows_model_id() {
    // 回归：前端可能为模型派生独立的显示键；后端必须按模型 id 存储记录键，
    // 不得因“键与 id 不一致”报 422（截图中的报错即此）。
    let (service, _events, _dir) = start_models("models-key-id").await;
    let port = service.port();
    let token = service.token().to_string();

    let payload = r#"{
        "providers": [{
            "key": "9router-lx", "id": "9router-lx", "type": "openai-compatible",
            "name": "9router", "baseUrl": "http://localhost:20128/v1", "apiKey": "test-key-not-real",
            "models": [{
                "key": "model-al4yqe", "id": "cbn/deepseek-v4.1-flash",
                "name": "deepseek-v4.1-flash",
                "limit": {"context": 1000000, "output": 65527}
            }]
        }],
        "enabledProviders": ["9router-lx"],
        "defaultModel": {"provider": "9router-lx", "model": "cbn/deepseek-v4.1-flash"},
        "revision": ""
    }"#;
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            payload,
            None,
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "按 id 存储: {}", body(&saved));

    // 落盘记录键 == 模型 id；GET 回传的 key/id 一致。
    let got = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let value: serde_json::Value = serde_json::from_str(body(&got)).expect("json");
    let model = &value["providers"][0]["models"][0];
    assert_eq!(model["key"], serde_json::json!("cbn/deepseek-v4.1-flash"));
    assert_eq!(model["id"], serde_json::json!("cbn/deepseek-v4.1-flash"));
    assert_eq!(
        value["defaultModel"]["model"],
        serde_json::json!("cbn/deepseek-v4.1-flash")
    );
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
            // Key 随事件负载进入内存快照（GET 同样回传，供页面编辑）。
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
    // 取当前 revision，供后续两次保存使用。
    let current = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let revision = revision_of(body(&current)).expect("revision");

    // 字段缺失：Key 保持。
    let keep_body = {
        let mut value: serde_json::Value = serde_json::from_str(&put_body()).expect("json");
        value["revision"] = serde_json::json!(revision);
        value["providers"][0]
            .as_object_mut()
            .expect("provider")
            .remove("apiKey");
        serde_json::to_string(&value).expect("string")
    };
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
    assert_eq!(status(&kept), 200, "缺失字段保持旧 Key：{}", body(&kept));

    // 空字符串：显式清除。
    let cleared = {
        let revision = revision_of(body(&kept)).expect("revision 2");
        let mut value: serde_json::Value = serde_json::from_str(&put_body()).expect("json");
        value["revision"] = serde_json::json!(revision);
        value["providers"][0]["apiKey"] = serde_json::json!("");
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

    // GET：key 已清空。
    let after = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let parsed: serde_json::Value = serde_json::from_str(body(&after)).expect("json");
    assert_eq!(
        parsed["providers"][0]["apiKey"],
        serde_json::json!(""),
        "空字符串清除 Key"
    );
    // 磁盘上的 Key 已删。
    let disk = std::fs::read_to_string(&path).expect("ai.json readable");
    assert!(!disk.contains(FAKE_KEY), "清除后磁盘无 Key");
    service.shutdown();
}

#[tokio::test]
async fn variant_params_and_modalities_survive_round_trip() {
    let (service, _events, _dir) = start_models("models-variants").await;
    let port = service.port();
    let token = service.token().to_string();
    let path = state_user_root(&service).join("config").join("ai.json");

    // 首轮保存：模型带思考等级参数对象、模态与未知字段。
    let mut value: serde_json::Value = serde_json::from_str(&put_body()).expect("json");
    value["providers"][0]["models"][0]["variants"] = serde_json::json!({
        "low": {"reasoningEffort": "low"},
        "high": {"reasoningEffort": "high"}
    });
    value["providers"][0]["models"][0]["variant"] = serde_json::json!("high");
    value["providers"][0]["models"][0]["modalities"] = serde_json::json!({"input": ["text"]});
    value["providers"][0]["models"][0]["x-custom"] = serde_json::json!(1);
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &serde_json::to_string(&value).expect("string"),
            None,
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "保存成功：{}", body(&saved));

    // GET：variants 带参数对象（不是键数组），供前端原样往返。
    let current = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let parsed: serde_json::Value = serde_json::from_str(body(&current)).expect("json");
    assert_eq!(
        parsed["providers"][0]["models"][0]["variants"]["high"]["reasoningEffort"],
        serde_json::json!("high"),
        "GET 携带思考等级参数"
    );

    // 再保存一次（模拟设置页编辑后保存）：参数与模态不丢。
    let revision = revision_of(body(&current)).expect("revision");
    let mut again: serde_json::Value = serde_json::from_str(body(&current)).expect("json");
    again["revision"] = serde_json::json!(revision);
    let resaved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &serde_json::to_string(&again).expect("string"),
            None,
        ),
    )
    .await;
    assert_eq!(status(&resaved), 200, "二次保存成功：{}", body(&resaved));

    let disk = std::fs::read_to_string(&path).expect("ai.json readable");
    let on_disk: serde_json::Value = serde_json::from_str(&disk).expect("disk json");
    let model = &on_disk["ai"]["providers"]["p"]["models"]["m"];
    assert_eq!(
        model["variants"]["high"]["reasoningEffort"],
        serde_json::json!("high"),
        "二次保存后思考等级参数仍在：{disk}"
    );
    assert_eq!(model["variant"], serde_json::json!("high"));
    assert_eq!(
        model["modalities"],
        serde_json::json!({"input": ["text"]}),
        "模态往返保留"
    );
    assert_eq!(model["x-custom"], serde_json::json!(1), "未知字段往返保留");
    service.shutdown();
}

#[tokio::test]
async fn modalities_returned_and_explicit_null_clears() {
    let (service, _events, _dir) = start_models("models-modalities").await;
    let port = service.port();
    let token = service.token().to_string();

    // 首轮保存：模型带输入/输出模态。
    let mut value: serde_json::Value = serde_json::from_str(&put_body()).expect("json");
    value["providers"][0]["models"][0]["modalities"] =
        serde_json::json!({"input": ["text", "image"], "output": ["text"]});
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &serde_json::to_string(&value).expect("string"),
            None,
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "保存成功：{}", body(&saved));

    // GET 返回 modalities：设置页据此渲染勾选框。
    let current = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let parsed: serde_json::Value = serde_json::from_str(body(&current)).expect("json");
    assert_eq!(
        parsed["providers"][0]["models"][0]["modalities"],
        serde_json::json!({"input": ["text", "image"], "output": ["text"]}),
        "GET 返回模态"
    );

    // 字段缺失 = 保留旧值（老客户端往返不丢模态）。
    let revision = revision_of(body(&current)).expect("revision");
    let mut keep: serde_json::Value = serde_json::from_str(body(&current)).expect("json");
    keep["revision"] = serde_json::json!(revision);
    keep["providers"][0]["models"][0]
        .as_object_mut()
        .expect("model")
        .remove("modalities");
    let kept = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &serde_json::to_string(&keep).expect("string"),
            None,
        ),
    )
    .await;
    assert_eq!(status(&kept), 200, "缺失保留：{}", body(&kept));
    let after_keep = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let after_keep_json: serde_json::Value = serde_json::from_str(body(&after_keep)).expect("json");
    assert!(
        !after_keep_json["providers"][0]["models"][0]["modalities"].is_null(),
        "缺失字段保留旧模态"
    );

    // 显式 null = 清除（设置页清空全部勾选）。
    let revision = revision_of(body(&after_keep)).expect("revision 2");
    let mut clear: serde_json::Value = after_keep_json.clone();
    clear["revision"] = serde_json::json!(revision);
    clear["providers"][0]["models"][0]["modalities"] = serde_json::Value::Null;
    let cleared = request(
        port,
        &write(
            "PUT",
            &format!("/api/models/settings?token={token}"),
            &serde_json::to_string(&clear).expect("string"),
            None,
        ),
    )
    .await;
    assert_eq!(status(&cleared), 200, "显式清除：{}", body(&cleared));
    let after_clear = request(port, &get(&format!("/api/models/settings?token={token}"))).await;
    let after_clear: serde_json::Value = serde_json::from_str(body(&after_clear)).expect("json");
    assert!(
        after_clear["providers"][0]["models"][0]["modalities"].is_null(),
        "显式 null 清除模态"
    );
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

/// 回归：fetch 端点在事件循环的 Tokio 运行时内执行，绝不能再建运行时
/// （`block_on` 会 panic "Cannot start a runtime from within a runtime"）。
/// 本测试在 `#[tokio::test]` 运行时里走 POST /api/models/fetch 到本地 mock，
/// 成功即证明路径不再嵌套运行时。
#[tokio::test]
async fn fetch_returns_models_from_local_mock_without_nested_runtime() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind mock");
    let model_port = listener.local_addr().expect("addr").port();
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
        let payload = r#"{"data":[{"id":"m2","owned_by":"b"},{"id":"m1","owned_by":"a"}]}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            payload.len(),
            payload
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
        // 记录请求行，断言命中的是 /v1/models。
        String::from_utf8_lossy(&buffer).into_owned()
    });

    let (service, _events, _dir) = start_models("models-fetch-ok").await;
    let port = service.port();
    let token = service.token().to_string();
    let payload = format!(
        r#"{{"type":"openai-compatible","baseUrl":"http://127.0.0.1:{model_port}/v1","apiKey":"k"}}"#
    );
    let response = request(
        port,
        &write(
            "POST",
            &format!("/api/models/fetch?token={token}"),
            &payload,
            None,
        ),
    )
    .await;
    assert_eq!(status(&response), 200, "fetch 成功: {}", body(&response));
    let parsed: serde_json::Value = serde_json::from_str(body(&response)).expect("json");
    let models = parsed["models"].as_array().expect("models array");
    assert_eq!(models.len(), 2, "去重并按 id 排序后两条");
    assert_eq!(models[0]["id"], serde_json::json!("m1"));
    assert_eq!(models[1]["id"], serde_json::json!("m2"));

    let recorded = mock.await.expect("mock join");
    assert!(
        recorded.contains("GET /v1/models"),
        "命中 /v1/models：{recorded}"
    );
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

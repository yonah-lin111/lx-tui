use super::super::SettingsEvent;
use super::super::tests::start;
use super::super::tests_helpers::{body, get, request, status, write};

#[tokio::test]
async fn state_returns_config_defaults_and_logging_info() {
    let (service, _events, _dir) = start("state").await;
    let token = service.token().to_string();
    let response = request(service.port(), &get(&format!("/api/state?token={token}"))).await;
    assert_eq!(status(&response), 200);
    let json = body(&response);
    assert!(json.contains("\"sidebar_width\":24"), "{json}");
    assert!(json.contains("\"log_level\":\"info\""), "{json}");
    assert!(json.contains("\"log_payload\":false"), "{json}");
    assert!(json.contains("\"logging\":{\"dir\":"), "{json}");
    assert!(json.contains("\"env_locked\":"), "{json}");
    assert!(json.contains("\"payload_env_locked\":"), "{json}");
    assert!(json.contains("\"commands\":[]"), "{json}");
    service.shutdown();
}

#[tokio::test]
async fn config_save_validates_origin_values_and_preserves_partial_fields() {
    let (service, mut events, dir) = start("config").await;
    let port = service.port();
    let token = service.token().to_string();
    let url = format!("/api/config?token={token}");
    let origin = format!("http://127.0.0.1:{port}");

    // 跨源写入被拒绝。
    let cross = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":30}}",
            Some("http://evil.example"),
        ),
    )
    .await;
    assert_eq!(status(&cross), 403);

    // 非法值拒绝且不落盘。
    let invalid = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":-3}}",
            Some(&origin),
        ),
    )
    .await;
    assert_eq!(status(&invalid), 422);
    assert!(
        !dir.join("settings.json").exists(),
        "invalid config not written"
    );

    // 非法日志级别同样拒绝。
    let invalid_level = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"log_level\":\"verbose\"}}",
            Some(&origin),
        ),
    )
    .await;
    assert_eq!(status(&invalid_level), 422);
    assert!(
        !dir.join("settings.json").exists(),
        "invalid log level not written"
    );

    // 合法值保存并投递事件。
    let valid = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":30,\"sidebar_min_width\":18,\"sidebar_max_width\":40,\"narrow_width\":80,\"min_width\":40,\"min_height\":10,\"min_pane_width\":10,\"min_pane_height\":3,\"log_level\":\"debug\",\"log_payload\":true}}",
            Some(&origin),
        ),
    )
    .await;
    assert_eq!(status(&valid), 200);
    let saved = std::fs::read_to_string(dir.join("settings.json")).expect("settings written");
    assert!(saved.contains("\"sidebar_width\": 30"), "{saved}");
    assert!(saved.contains("\"log_level\": \"debug\""), "{saved}");
    assert!(saved.contains("\"log_payload\": true"), "{saved}");
    let event = events.try_recv().expect("config event");
    match event {
        SettingsEvent::ConfigChanged(config) => {
            assert_eq!(config.sidebar_width, 30);
            assert_eq!(config.log_level, "debug");
            assert!(config.log_payload);
        }
        other => panic!("unexpected event: {other:?}"),
    }

    // 增量更新 log_level 时保留已落盘的 sidebar_width: 30。
    let partial = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"log_level\":\"warn\"}}",
            Some(&origin),
        ),
    )
    .await;
    assert_eq!(status(&partial), 200);
    match events.try_recv().expect("partial config event") {
        SettingsEvent::ConfigChanged(config) => {
            assert_eq!(config.sidebar_width, 30);
            assert_eq!(config.log_level, "warn");
            assert!(config.log_payload);
        }
        other => panic!("unexpected event: {other:?}"),
    }
    service.shutdown();
}

#[tokio::test]
async fn logging_page_uses_custom_select_and_open_endpoint_enforces_auth() {
    let (service, _events, _dir) = start("logging-ui").await;
    let port = service.port();
    let token = service.token().to_string();

    let html = body(&request(port, &get(&format!("/?token={token}"))).await).to_string();
    assert!(!html.contains("<select"), "no native select allowed");
    for hook in [
        "id=\"log-level-host\"",
        "id=\"log_payload\"",
        "id=\"log_path\"",
        "copyLogPath()",
        "openLogFolder()",
    ] {
        assert!(html.contains(hook), "missing logging html hook: {hook}");
    }

    let js = body(&request(port, &get("/settings.js")).await).to_string();
    for symbol in [
        "initLoggingSelect",
        "logLevelSelect",
        "copyLogPath",
        "openLogFolder",
        "/api/logs/open",
    ] {
        assert!(js.contains(symbol), "missing logging js symbol: {symbol}");
    }

    // 未携带 token 拒绝访问。
    let no_token = request(port, &write("POST", "/api/logs/open", "{}", None)).await;
    assert_eq!(status(&no_token), 403);

    // 跨源 POST 拒绝访问。
    let cross_origin = request(
        port,
        &write(
            "POST",
            &format!("/api/logs/open?token={token}"),
            "{}",
            Some("http://evil.example"),
        ),
    )
    .await;
    assert_eq!(status(&cross_origin), 403);
    service.shutdown();
}

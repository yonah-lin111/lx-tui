use super::*;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

/// 隔离临时目录。
fn temp_dir(name: &str) -> PathBuf {
    let unique = format!(
        "lx-tui-service-tests-{}-{}-{}",
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

/// 启动测试服务：注入隔离的 settings.json 路径。
async fn start(
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

/// 发送原始 HTTP 请求并读回完整响应文本。
async fn request(port: u16, raw: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    stream.write_all(raw.as_bytes()).await.expect("write");
    let mut out = String::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_string(&mut out))
        .await
        .expect("no timeout")
        .expect("read");
    out
}

/// 解析响应状态码。
fn status(response: &str) -> u16 {
    response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0)
}

/// 响应体（跳过 header）。
fn body(response: &str) -> &str {
    response.split("\r\n\r\n").nth(1).unwrap_or_default()
}

/// 便捷 GET。
fn get(path: &str) -> String {
    format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
}

/// 便捷带 JSON body 的写请求。
fn write(method: &str, path: &str, body: &str, origin: Option<&str>) -> String {
    let origin = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n{origin}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn page_requires_token_and_serves_self_contained_html() {
    let (service, _events, _dir) = start("page").await;
    let port = service.port();
    let token = service.token().to_string();

    let denied = request(port, &get("/")).await;
    assert_eq!(status(&denied), 403);

    let ok = request(port, &get(&format!("/?token={token}"))).await;
    assert_eq!(status(&ok), 200);
    let html = body(&ok);
    assert!(html.contains("General"), "tabs present");
    assert!(html.contains("Commands"), "commands tab present");
    assert!(!html.contains("https://"), "no external resources: {html}");
    assert!(!html.contains("http://"), "no external resources: {html}");
    service.shutdown();
}

#[tokio::test]
async fn state_returns_config_defaults() {
    let (service, _events, _dir) = start("state").await;
    let token = service.token().to_string();
    let response = request(service.port(), &get(&format!("/api/state?token={token}"))).await;
    assert_eq!(status(&response), 200);
    let json = body(&response);
    assert!(json.contains("\"sidebar_width\":24"), "{json}");
    assert!(json.contains("\"commands\":[]"), "{json}");
    service.shutdown();
}

#[tokio::test]
async fn config_save_validates_origin_and_values() {
    let (service, mut events, dir) = start("config").await;
    let port = service.port();
    let token = service.token().to_string();
    let url = format!("/api/config?token={token}");

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
            Some("http://127.0.0.1"),
        ),
    )
    .await;
    assert_eq!(status(&invalid), 422);
    assert!(
        !dir.join("settings.json").exists(),
        "invalid config not written"
    );

    // 合法值保存并投递事件。
    let valid = request(
        port,
        &write(
            "PUT",
            &url,
            "{\"config\":{\"sidebar_width\":30,\"sidebar_min_width\":18,\"sidebar_max_width\":40,\"narrow_width\":80,\"min_width\":40,\"min_height\":10,\"min_pane_width\":10,\"min_pane_height\":3}}",
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&valid), 200);
    let saved = std::fs::read_to_string(dir.join("settings.json")).expect("settings written");
    assert!(saved.contains("\"sidebar_width\": 30"), "{saved}");
    let event = events.try_recv().expect("config event");
    match event {
        SettingsEvent::ConfigChanged(config) => assert_eq!(config.sidebar_width, 30),
        other => panic!("unexpected event: {other:?}"),
    }
    service.shutdown();
}

#[tokio::test]
async fn command_crud_over_http_and_snapshot() {
    let (service, mut events, _dir) = start("commands").await;
    let port = service.port();
    let token = service.token().to_string();
    let save_url = format!("/api/commands/user/agentMD/review?token={token}");
    let payload = "{\"description\":\"review diff\",\"md_scope\":\"template\",\"body\":\"# Review\\ncheck it\"}";
    let saved = request(
        port,
        &write(
            "PUT",
            &save_url,
            payload,
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{saved}");
    match events.try_recv() {
        Ok(SettingsEvent::CommandsChanged) => {}
        other => panic!("expected CommandsChanged, got {other:?}"),
    }

    service.set_commands_snapshot(&[CommandView {
        name: "review".into(),
        scope: "user".into(),
        kind: "agentMD".into(),
        description: "review diff".into(),
        md_scope: "template".into(),
        title: String::new(),
        body: "# Review\ncheck it".into(),
    }]);
    let state = request(service.port(), &get(&format!("/api/state?token={token}"))).await;
    assert!(
        body(&state).contains("\"name\":\"review\""),
        "{}",
        body(&state)
    );

    // 非法名与保留名被拒绝。
    let bad = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/..%2Fescape?token={token}"),
            payload,
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert!(status(&bad) == 400 || status(&bad) == 404, "{bad}");
    let reserved = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/add?token={token}"),
            payload,
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&reserved), 400);

    // 重命名与删除。
    let renamed = request(
        port,
        &write(
            "POST",
            &format!("/api/commands/user/agentMD/review/rename?token={token}"),
            "{\"to\":\"review2\"}",
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&renamed), 200, "{renamed}");
    let deleted = request(
        port,
        &write(
            "DELETE",
            &format!("/api/commands/user/agentMD/review2?token={token}"),
            "",
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&deleted), 200, "{deleted}");
    let missing = request(
        port,
        &write(
            "DELETE",
            &format!("/api/commands/user/agentMD/review2?token={token}"),
            "",
            Some(&format!("http://127.0.0.1:{port}")),
        ),
    )
    .await;
    assert_eq!(status(&missing), 404);
    service.shutdown();
}

#[tokio::test]
async fn oversized_body_is_rejected_and_service_survives() {
    let (service, _events, _dir) = start("oversize").await;
    let port = service.port();
    let token = service.token().to_string();
    let huge = "x".repeat(MAX_BODY_BYTES + 1);
    let raw = format!(
        "PUT /api/commands/user/agentMD/huge?token={token} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\n\r\n{huge}",
        huge.len()
    );
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let _ = stream.write_all(raw.as_bytes()).await;
    let mut out = String::new();
    let _ = tokio::time::timeout(Duration::from_secs(2), stream.read_to_string(&mut out)).await;
    if !out.is_empty() {
        assert_ne!(status(&out), 200);
    }
    // 服务仍可响应正常请求。
    let alive = request(port, &get(&format!("/?token={token}"))).await;
    assert_eq!(status(&alive), 200);
    service.shutdown();
}

#[tokio::test]
async fn shutdown_stops_listening() {
    let (service, _events, _dir) = start("shutdown").await;
    let port = service.port();
    service.shutdown();
    let mut closed = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            closed = true;
            break;
        }
    }
    assert!(closed, "listener stops after shutdown");
}

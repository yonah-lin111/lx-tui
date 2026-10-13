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
    assert!(html.contains("color-scheme: dark"), "dark theme");
    assert!(
        html.contains("commands-grid"),
        "commands list/editor layout"
    );
    assert!(!html.contains("cmd-preview"), "no preview pane");
    assert!(html.contains("editor-empty"), "empty placeholder container");
    service.shutdown();
}

#[tokio::test]
async fn page_locks_viewport_and_avoids_extra_scrollbars() {
    let (service, _events, _dir) = start("page-layout").await;
    let token = service.token().to_string();
    let response = request(service.port(), &get(&format!("/?token={token}"))).await;
    assert_eq!(status(&response), 200);
    let html = body(&response);

    // 根容器锁定视口高度，永不出现根滚动条。
    assert!(
        html.contains("html, body { height: 100%; }"),
        "viewport height locked"
    );
    assert!(
        html.contains("display: flex; overflow: hidden;"),
        "body overflow hidden"
    );
    // 宽屏时命令网格的行高被视口约束，滚动只发生在栏内。
    assert!(
        html.contains("grid-template-rows: minmax(0, 1fr)"),
        "wide grid rows bounded by viewport"
    );
    assert!(
        html.contains("grid-template-rows: max-content max-content"),
        "narrow stacked grid sizes rows to content and scrolls inside"
    );
    // 左栏列表独立滚动但隐藏滚动条；右栏卡片依赖细滚动条兜底。
    assert!(
        html.contains(".command-list::-webkit-scrollbar { display: none; }"),
        "left list scrollbar hidden"
    );
    assert!(
        html.contains(".editor-body::-webkit-scrollbar { width: 8px; }"),
        "editor fallback scrollbar styled"
    );
    assert!(
        html.contains("scrollbar-width: thin"),
        "editor fallback scrollbar thin"
    );
    service.shutdown();
}

#[tokio::test]
async fn page_editor_supports_rename_flow() {
    let (service, _events, _dir) = start("page-rename").await;
    let port = service.port();
    let token = service.token().to_string();

    // HTML/JS 层：Name 输入不再被禁用；保存时先调 rename 再 PUT。
    let page = request(port, &get(&format!("/?token={token}"))).await;
    assert_eq!(status(&page), 200);
    let html = body(&page);
    assert!(!html.contains(".disabled"), "name input never disabled");
    assert!(html.contains("'/rename'"), "rename endpoint wired");
    assert!(
        html.contains("JSON.stringify({to: name})"),
        "rename payload wired"
    );

    // 协议层：复刻前端保存序列（先 rename 后 PUT 新路径），内容跟随新名。
    let origin = Some(format!("http://127.0.0.1:{port}"));
    let payload = "{\"description\":\"d\",\"md_scope\":\"global\",\"body\":\"first\"}";
    let created = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/alpha?token={token}"),
            payload,
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&created), 200, "{created}");
    let renamed = request(
        port,
        &write(
            "POST",
            &format!("/api/commands/user/agentMD/alpha/rename?token={token}"),
            "{\"to\":\"beta\"}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&renamed), 200, "{renamed}");
    let saved = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/beta?token={token}"),
            "{\"description\":\"d\",\"md_scope\":\"global\",\"body\":\"second\"}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&saved), 200, "{saved}");
    let state = request(port, &get(&format!("/api/state?token={token}"))).await;
    let state_body = body(&state);
    assert!(state_body.contains("\"name\":\"beta\""), "{state_body}");
    assert!(!state_body.contains("\"name\":\"alpha\""), "{state_body}");
    assert!(state_body.contains("second"), "{state_body}");

    // 改到已存在的名字必须失败，主流程因此不会覆盖另一条命令。
    let created = request(
        port,
        &write(
            "PUT",
            &format!("/api/commands/user/agentMD/gamma?token={token}"),
            payload,
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&created), 200, "{created}");
    let conflict = request(
        port,
        &write(
            "POST",
            &format!("/api/commands/user/agentMD/beta/rename?token={token}"),
            "{\"to\":\"gamma\"}",
            origin.as_deref(),
        ),
    )
    .await;
    assert_eq!(status(&conflict), 422, "{conflict}");
    let state = request(port, &get(&format!("/api/state?token={token}"))).await;
    assert!(body(&state).contains("\"name\":\"beta\""), "beta kept");
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

    // 保存后无需 TUI 推送：服务端已按磁盘刷新快照。
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
    // 删除后无需 TUI 推送：快照同步移除。
    let after = request(service.port(), &get(&format!("/api/state?token={token}"))).await;
    assert!(
        !body(&after).contains("\"name\":\"review\""),
        "{}",
        body(&after)
    );
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

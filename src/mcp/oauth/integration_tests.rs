//! OAuth 登录流程集成测试：本地 mock 授权服务器完成发现、动态注册与换 token。

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::config::mcp::{McpServerConfig, McpTransportConfig};

async fn mock_authorization_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("bind");
    let address = listener.local_addr().expect("addr");
    let base = format!("http://{address}");
    let server_base = base.clone();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buffer = vec![0u8; 8192];
            let read = stream.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("");
            let (status, body) = if path == "/.well-known/oauth-authorization-server" {
                (
                    "200 OK",
                    json!({
                        "issuer": server_base,
                        "authorization_endpoint": format!("{server_base}/authorize"),
                        "token_endpoint": format!("{server_base}/token"),
                        "registration_endpoint": format!("{server_base}/register"),
                        "scopes_supported": ["read"],
                    })
                    .to_string(),
                )
            } else if path.starts_with("/.well-known/oauth-protected-resource") {
                (
                    "200 OK",
                    json!({"authorization_servers": [server_base]}).to_string(),
                )
            } else if path == "/register" {
                (
                    "200 OK",
                    json!({"client_id": "lx-client", "client_secret": "lx-secret"}).to_string(),
                )
            } else if path == "/token" {
                (
                    "200 OK",
                    json!({
                        "access_token": "access-token",
                        "refresh_token": "refresh-token",
                        "expires_in": 3600,
                        "scope": "read",
                    })
                    .to_string(),
                )
            } else {
                ("404 Not Found", "{}".to_string())
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    (base, handle)
}

fn oauth_server(name: &str, url: &str) -> EffectiveServer {
    EffectiveServer {
        name: name.into(),
        scope: crate::config::mcp::McpScope::User,
        config: McpServerConfig {
            enabled: true,
            transport: McpTransportConfig::Http {
                url: url.into(),
                auth: McpHttpAuth::Oauth,
                bearer_token: None,
                bearer_token_env_var: None,
                http_headers: BTreeMap::new(),
                env_http_headers: BTreeMap::new(),
                scopes: vec!["read".into()],
                client_id: None,
                client_secret: None,
            },
            startup_timeout_sec: Some(10),
            tool_timeout_sec: Some(30),
            enabled_tools: None,
            disabled_tools: Vec::new(),
        },
        shadowed_user: false,
    }
}

#[tokio::test]
async fn authorization_discovers_registers_and_stores_credentials() {
    let (base, handle) = mock_authorization_server().await;
    let server = oauth_server("mock", &format!("{base}/mcp"));
    let project = std::env::temp_dir().join(format!("lx-oauth-project-{}", std::process::id()));
    std::fs::create_dir_all(&project).expect("project");
    let pending = tokio::time::timeout(
        Duration::from_secs(5),
        begin_authorization(&server, &project, "http://127.0.0.1:9/mcp/oauth/callback"),
    )
    .await
    .expect("discovery timeout")
    .expect("begin");
    assert!(
        pending
            .authorize_url
            .starts_with(&format!("{base}/authorize?")),
        "{}",
        pending.authorize_url
    );
    assert!(pending.authorize_url.contains("code_challenge="));
    assert!(pending.authorize_url.contains("state="));
    let url = reqwest::Url::parse(&pending.authorize_url).expect("url");
    let state = url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .expect("state");
    assert_eq!(state, pending.state);
    assert!(!pending.client_id.is_empty());
    assert_eq!(pending.token_endpoint, format!("{base}/token"));

    let dir = std::env::temp_dir().join(format!("lx-oauth-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = dir.join("credentials.json");
    let store = store_at(path.clone());
    complete_authorization(&pending, "code-1", &store)
        .await
        .expect("complete");
    let credentials = store.get(&pending.key).expect("credentials");
    assert_eq!(credentials.access_token, "access-token");
    assert_eq!(credentials.refresh_token.as_deref(), Some("refresh-token"));
    assert!(credentials.expires_at_ms > now_ms());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&project);
    handle.abort();
}

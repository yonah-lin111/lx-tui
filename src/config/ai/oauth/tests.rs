//! OAuth 纯逻辑测试（不发起网络请求）。

use std::collections::BTreeMap;

use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;

use super::browser::{browser_authorize_url, parse_query, percent_decode, percent_encode};
use super::{
    credential_from_token, credential_needs_refresh, now_ms, pkce_challenge,
    refresh_oauth_credential,
};
use crate::config::ai::connect::OAuthCredential;

fn callback_params(url: &str) -> BTreeMap<String, String> {
    url.split_once('?')
        .map(|(_, query)| parse_query(query.split('#').next().unwrap_or(query)))
        .unwrap_or_default()
}

#[test]
fn pkce_challenge_matches_rfc_vector() {
    let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    assert_eq!(
        pkce_challenge(verifier),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn authorize_url_contains_pkce_and_state() {
    let url = browser_authorize_url(
        "openai",
        "https://auth.openai.com/oauth/authorize",
        "client",
        "openid profile",
        "http://127.0.0.1:1455/auth/callback",
        "state-value",
        "challenge-value",
    );
    let params = callback_params(&url);
    assert_eq!(
        params.get("response_type").map(String::as_str),
        Some("code")
    );
    assert_eq!(
        params.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(params.get("state").map(String::as_str), Some("state-value"));
    assert_eq!(
        params.get("redirect_uri").map(String::as_str),
        Some("http://127.0.0.1:1455/auth/callback")
    );
}

#[test]
fn percent_codec_round_trips() {
    let encoded = percent_encode("a b+c/d?e");
    assert_eq!(percent_decode(&encoded), "a b+c/d?e");
}

#[test]
fn credential_from_token_reads_api_key_and_expiry() {
    let value = json!({ "api_key": "sk-test", "api_key_expires_in": 3600 });
    let mut credential = credential_from_token("browser", &value).expect("credential");
    assert_eq!(credential.access_token, "sk-test");
    assert!(credential.expires_at_ms.is_some());
    assert!(
        !credential_needs_refresh(&credential, now_ms()),
        "no refresh token"
    );
    credential.refresh_token = Some("refresh".to_string());
    assert!(credential_needs_refresh(&credential, now_ms() + 3_600_000));
}

#[tokio::test]
async fn start_oauth_flow_rejects_unknown_provider_and_method() {
    let answers = BTreeMap::new();
    assert!(
        super::start_oauth_flow("not-a-provider", "device", &answers)
            .await
            .is_err()
    );
    assert!(
        super::start_oauth_flow("opencode", "api-key", &answers)
            .await
            .is_err()
    );
}

async fn serve_json_once(listener: TcpListener, body: &'static str) {
    let (mut stream, _) = listener.accept().await.expect("accept");
    let mut buffer = [0u8; 8192];
    let _ = stream.read(&mut buffer).await;
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .expect("write response");
}

async fn http_round_trip(port: u16, request: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect callback listener");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    String::from_utf8_lossy(&response).into_owned()
}

#[tokio::test]
async fn refresh_preserves_refresh_token_and_metadata_when_response_omits_them() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind token server");
    let port = listener.local_addr().expect("token server address").port();
    let server = tokio::spawn(serve_json_once(
        listener,
        r#"{"access_token":"new-access","expires_in":3600}"#,
    ));
    let instance = format!("http://127.0.0.1:{port}");
    let credential = OAuthCredential {
        method_id: "pkce".to_string(),
        access_token: "old-access".to_string(),
        refresh_token: Some("keep-refresh".to_string()),
        expires_at_ms: Some(0),
        account_label: Some("work".to_string()),
        metadata: BTreeMap::from([
            ("instanceUrl".to_string(), instance.clone()),
            ("custom".to_string(), "keep".to_string()),
        ]),
    };
    let answers = BTreeMap::from([("instanceUrl".to_string(), instance)]);
    let refreshed = refresh_oauth_credential("gitlab", &credential, &answers)
        .await
        .expect("refresh");
    server.await.expect("token server");
    assert_eq!(refreshed.access_token, "new-access");
    assert_eq!(refreshed.refresh_token.as_deref(), Some("keep-refresh"));
    assert_eq!(refreshed.account_label.as_deref(), Some("work"));
    assert_eq!(
        refreshed.metadata.get("custom").map(String::as_str),
        Some("keep")
    );
    assert!(refreshed.metadata.contains_key("instanceUrl"));
}

#[tokio::test]
async fn digitalocean_implicit_callback_without_query_state_succeeds() {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind callback listener");
    let port = listener
        .local_addr()
        .expect("callback listener address")
        .port();
    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let flow = super::browser::BrowserFlow::for_test(
        listener,
        "digitalocean",
        "https://cloud.digitalocean.com/v1/oauth/token",
        "expected-state",
        "/auth/callback",
        cancel_rx,
    );
    let task = tokio::spawn(super::browser::run_browser_flow(flow));

    let response = http_round_trip(
        port,
        "GET /auth/callback HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(response.contains("200 OK"), "{response}");
    assert!(response.contains("access_token"), "{response}");

    let body = r#"{"access_token":"do-token","state":"expected-state","expires_in":3600}"#;
    let request = format!(
        "POST /auth/token HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let response = http_round_trip(port, &request).await;
    assert!(response.contains("200 OK"), "{response}");
    let credential = task.await.expect("join callback task").expect("callback");
    assert_eq!(credential.access_token, "do-token");
}

#[test]
fn resolve_verification_url_expands_relative_console_paths() {
    let url = super::device::resolve_verification_url(
        "https://opencode.ai/console/auth/device/code",
        "/console/device?user_code=LCBQ-NTRM&client_id=opencode-cli",
    );
    assert_eq!(
        url,
        "https://opencode.ai/console/device?user_code=LCBQ-NTRM&client_id=opencode-cli"
    );

    let full = super::device::resolve_verification_url(
        "https://github.com/login/device/code",
        "https://github.com/login/device",
    );
    assert_eq!(full, "https://github.com/login/device");
}

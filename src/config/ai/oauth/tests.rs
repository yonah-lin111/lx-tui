//! OAuth 纯逻辑测试（不发起网络请求）。

use std::collections::BTreeMap;

use serde_json::json;

use super::browser::{browser_authorize_url, parse_query, percent_decode, percent_encode};
use super::{credential_from_token, credential_needs_refresh, now_ms, pkce_challenge};

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

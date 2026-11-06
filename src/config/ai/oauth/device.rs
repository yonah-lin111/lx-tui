//! Device Code 授权流：发起设备码请求并轮询 token 端点。

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{oneshot, watch};

use super::super::connect::OAuthCredential;
use super::super::presets::PresetProviderSpec;
use super::{
    OAuthPromptMode, OAuthStarted, credential_from_token, http_client, json_i64, json_string,
    wait_or_cancel,
};

const POLL_SAFETY_MARGIN: Duration = Duration::from_secs(3);
const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const OPENAI_DEVICE_REDIRECT: &str = "https://auth.openai.com/deviceauth/callback";
const OPENAI_DEVICE_URL: &str = "https://auth.openai.com/codex/device";
const OPENAI_USER_AGENT: &str = "lx-tui";

struct DeviceGrant {
    device_code: Option<String>,
    device_auth_id: Option<String>,
    user_code: String,
    url: String,
    interval: Duration,
    openai: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn start_device_flow(
    spec: &PresetProviderSpec,
    method_id: &str,
    device_code_url: &str,
    token_url: &str,
    client_id: &str,
    scope: &str,
    form_answers: &BTreeMap<String, String>,
    attempt_id: String,
    cancel_rx: watch::Receiver<bool>,
    result_tx: oneshot::Sender<Result<OAuthCredential, String>>,
) -> Result<OAuthStarted, String> {
    let client = http_client()?;
    let grant = request_device_grant(
        &client,
        spec.id,
        device_code_url,
        client_id,
        scope,
        form_answers,
    )
    .await?;
    let started = OAuthStarted {
        attempt_id,
        url: grant.url.clone(),
        user_code: Some(grant.user_code.clone()),
        instructions: format!("Enter code: {}", grant.user_code),
        mode: OAuthPromptMode::Auto,
    };
    let provider_id = spec.id.to_string();
    let method_id = method_id.to_string();
    let client_id = client_id.to_string();
    let token_url = token_url.to_string();
    let answers = form_answers.clone();
    tokio::spawn(async move {
        let result = poll_device_grant(
            &client,
            &provider_id,
            &method_id,
            &token_url,
            &client_id,
            &answers,
            &grant,
            cancel_rx,
        )
        .await;
        let _ = result_tx.send(result);
    });
    Ok(started)
}

async fn request_device_grant(
    client: &reqwest::Client,
    provider_id: &str,
    device_code_url: &str,
    client_id: &str,
    scope: &str,
    answers: &BTreeMap<String, String>,
) -> Result<DeviceGrant, String> {
    if provider_id == "openai" {
        return request_openai_device_grant(client, device_code_url, client_id).await;
    }
    let device_code_url = if provider_id == "github-copilot" {
        format!(
            "https://{}/login/device/code",
            github_copilot_domain(answers)
        )
    } else {
        device_code_url.to_string()
    };
    let request = client
        .post(&device_code_url)
        .header(reqwest::header::ACCEPT, "application/json");
    let request = if provider_id.starts_with("opencode") {
        request.json(&json!({
            "client_id": client_id,
            "supports_org_scope": true,
        }))
    } else if provider_id == "github-copilot" {
        request.json(&json!({
            "client_id": client_id,
            "scope": scope,
        }))
    } else {
        let mut form = vec![("client_id", client_id)];
        if !scope.is_empty() {
            form.push(("scope", scope));
        }
        if provider_id == "xai" {
            form.push(("referrer", "opencode"));
        }
        request.form(&form)
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("device authorization request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("device authorization response failed: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "device authorization failed: HTTP {}",
            status.as_u16()
        ));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| "device authorization returned invalid JSON".to_string())?;
    let user_code = json_string(&value, "user_code")
        .ok_or_else(|| "device authorization response has no user code".to_string())?;
    let raw_url = json_string(&value, "verification_uri_complete")
        .or_else(|| json_string(&value, "verification_uri"))
        .ok_or_else(|| "device authorization response has no verification URL".to_string())?;
    let url = resolve_verification_url(&device_code_url, &raw_url);
    let device_code = json_string(&value, "device_code")
        .ok_or_else(|| "device authorization response has no device code".to_string())?;
    Ok(DeviceGrant {
        device_code: Some(device_code),
        device_auth_id: None,
        user_code,
        url,
        interval: json_interval(&value),
        openai: false,
    })
}

async fn request_openai_device_grant(
    client: &reqwest::Client,
    device_code_url: &str,
    client_id: &str,
) -> Result<DeviceGrant, String> {
    let response = client
        .post(device_code_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, OPENAI_USER_AGENT)
        .json(&json!({ "client_id": client_id }))
        .send()
        .await
        .map_err(|error| format!("device authorization request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("device authorization response failed: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "device authorization failed: HTTP {}",
            status.as_u16()
        ));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| "device authorization returned invalid JSON".to_string())?;
    Ok(DeviceGrant {
        device_code: None,
        device_auth_id: Some(
            json_string(&value, "device_auth_id")
                .ok_or_else(|| "OpenAI device response has no device_auth_id".to_string())?,
        ),
        user_code: json_string(&value, "user_code")
            .ok_or_else(|| "OpenAI device response has no user code".to_string())?,
        url: OPENAI_DEVICE_URL.to_string(),
        interval: json_interval(&value),
        openai: true,
    })
}

#[allow(clippy::too_many_arguments)]
async fn poll_device_grant(
    client: &reqwest::Client,
    provider_id: &str,
    method_id: &str,
    token_url: &str,
    client_id: &str,
    form_answers: &BTreeMap<String, String>,
    grant: &DeviceGrant,
    mut cancel_rx: watch::Receiver<bool>,
) -> Result<OAuthCredential, String> {
    let token_url = if provider_id == "github-copilot" {
        format!(
            "https://{}/login/oauth/access_token",
            github_copilot_domain(form_answers)
        )
    } else {
        token_url.to_string()
    };
    let deadline = tokio::time::Instant::now() + super::OAUTH_TIMEOUT;
    let mut interval = grant.interval.max(Duration::from_secs(1));
    loop {
        if tokio::time::Instant::now() >= deadline {
            return Err("OAuth device authorization timed out".to_string());
        }
        if wait_or_cancel(interval + POLL_SAFETY_MARGIN, &mut cancel_rx).await {
            return Err("OAuth flow was cancelled".to_string());
        }
        let value = if grant.openai {
            match poll_openai_device(client, client_id, grant).await? {
                Some(tokens) => tokens,
                None => continue,
            }
        } else {
            match poll_standard_device(client, provider_id, &token_url, client_id, grant).await? {
                PollOutcome::Token(value) => value,
                PollOutcome::Pending => continue,
                PollOutcome::SlowDown => {
                    interval += Duration::from_secs(5);
                    continue;
                }
            }
        };
        let mut credential = credential_from_token(method_id, &value)?;
        if provider_id == "github-copilot" {
            credential = github_copilot_metadata(client, form_answers, credential).await;
        } else if provider_id.starts_with("opencode")
            && let Some(org_id) = json_string(&value, "org_id")
        {
            credential.metadata.insert("orgID".to_string(), org_id);
        }
        return Ok(credential);
    }
}

enum PollOutcome {
    Token(Value),
    Pending,
    SlowDown,
}

async fn poll_standard_device(
    client: &reqwest::Client,
    provider_id: &str,
    token_url: &str,
    client_id: &str,
    grant: &DeviceGrant,
) -> Result<PollOutcome, String> {
    let device_code = grant
        .device_code
        .as_deref()
        .ok_or_else(|| "device flow has no device code".to_string())?;
    let request = client
        .post(token_url)
        .header(reqwest::header::ACCEPT, "application/json");
    let request = if provider_id.starts_with("opencode") || provider_id == "github-copilot" {
        request.json(&json!({
            "client_id": client_id,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        }))
    } else {
        let form = [
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];
        request.form(&form)
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("device token request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("device token response failed: {error}"))?;
    let parsed: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
    if status.is_success() {
        if json_string(&parsed, "access_token").is_some() {
            return Ok(PollOutcome::Token(parsed));
        }
        if let Some(error) = json_string(&parsed, "error") {
            return classify_poll_error(&error, &parsed);
        }
        return Err("device token response has no access token".to_string());
    }
    if let Some(error) = json_string(&parsed, "error") {
        return classify_poll_error(&error, &parsed);
    }
    Err(format!(
        "device token request failed: HTTP {}",
        status.as_u16()
    ))
}

fn classify_poll_error(error: &str, parsed: &Value) -> Result<PollOutcome, String> {
    match error {
        "authorization_pending" => Ok(PollOutcome::Pending),
        "slow_down" => Ok(PollOutcome::SlowDown),
        other => {
            let detail = json_string(parsed, "error_description").unwrap_or_default();
            if detail.is_empty() {
                Err(format!("device authorization failed: {other}"))
            } else {
                Err(format!("device authorization failed: {other} ({detail})"))
            }
        }
    }
}

async fn poll_openai_device(
    client: &reqwest::Client,
    client_id: &str,
    grant: &DeviceGrant,
) -> Result<Option<Value>, String> {
    let device_auth_id = grant
        .device_auth_id
        .as_deref()
        .ok_or_else(|| "OpenAI device flow has no device_auth_id".to_string())?;
    let response = client
        .post(OPENAI_TOKEN_URL.replace("/oauth/token", "/api/accounts/deviceauth/token"))
        .header(reqwest::header::ACCEPT, "application/json")
        .json(&json!({
            "device_auth_id": device_auth_id,
            "user_code": grant.user_code,
        }))
        .send()
        .await
        .map_err(|error| format!("OpenAI device token request failed: {error}"))?;
    let status = response.status();
    if status.as_u16() == 403 || status.as_u16() == 404 {
        return Ok(None);
    }
    let body = response
        .text()
        .await
        .map_err(|error| format!("OpenAI device token response failed: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "OpenAI device authorization failed: HTTP {}",
            status.as_u16()
        ));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| "OpenAI device token returned invalid JSON".to_string())?;
    let code = json_string(&value, "authorization_code")
        .ok_or_else(|| "OpenAI device token has no authorization code".to_string())?;
    let verifier = json_string(&value, "code_verifier")
        .ok_or_else(|| "OpenAI device token has no code verifier".to_string())?;
    let tokens = exchange_openai_code(client, client_id, &code, &verifier).await?;
    Ok(Some(tokens))
}

async fn exchange_openai_code(
    client: &reqwest::Client,
    client_id: &str,
    code: &str,
    verifier: &str,
) -> Result<Value, String> {
    let response = client
        .post(OPENAI_TOKEN_URL)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, OPENAI_USER_AGENT)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", OPENAI_DEVICE_REDIRECT),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|error| format!("OpenAI token exchange failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("OpenAI token response failed: {error}"))?;
    if !status.is_success() {
        return Err(format!(
            "OpenAI token exchange failed: HTTP {}",
            status.as_u16()
        ));
    }
    serde_json::from_str(&body).map_err(|_| "OpenAI token response is invalid JSON".to_string())
}

async fn github_copilot_metadata(
    client: &reqwest::Client,
    answers: &BTreeMap<String, String>,
    mut credential: OAuthCredential,
) -> OAuthCredential {
    let domain = github_copilot_domain(answers);
    let endpoint = if domain == "github.com" {
        "https://api.github.com/copilot_internal/user".to_string()
    } else {
        format!("https://api.{domain}/copilot_internal/user")
    };
    if let Ok(response) = client
        .get(endpoint)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::USER_AGENT, OPENAI_USER_AGENT)
        .bearer_auth(&credential.access_token)
        .send()
        .await
        && response.status().is_success()
        && let Ok(value) = response.json::<Value>().await
    {
        if let Some(api) = value
            .get("endpoints")
            .and_then(|endpoints| endpoints.get("api"))
            .and_then(Value::as_str)
            .filter(|api| !api.trim().is_empty())
        {
            credential.metadata.insert(
                "apiEndpoint".to_string(),
                api.trim().trim_end_matches('/').to_string(),
            );
        }
        if let Some(login) = json_string(&value, "login") {
            credential.account_label = Some(login);
        }
    }
    if domain != "github.com" {
        credential
            .metadata
            .insert("enterpriseUrl".to_string(), domain);
    }
    credential
}

fn github_copilot_domain(answers: &BTreeMap<String, String>) -> String {
    if answers.get("deploymentType").map(String::as_str) != Some("enterprise") {
        return "github.com".to_string();
    }
    answers
        .get("enterpriseUrl")
        .map(|value| normalize_host(value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "github.com".to_string())
}

fn json_interval(value: &Value) -> Duration {
    let seconds = json_i64(value, "interval").unwrap_or(5).max(1) as u64;
    Duration::from_secs(seconds)
}

/// 将相对验证路径（如 OpenCode Console 返回的 `/console/device?user_code=...`）解析为完整 HTTP(S) URL。
pub(super) fn resolve_verification_url(endpoint_url: &str, verification_uri: &str) -> String {
    let trimmed = verification_uri.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }
    if let Some((scheme, rest)) = endpoint_url.split_once("://") {
        let host = rest.split('/').next().unwrap_or_default();
        if !host.is_empty() {
            if trimmed.starts_with('/') {
                return format!("{scheme}://{host}{trimmed}");
            }
            return format!("{scheme}://{host}/{trimmed}");
        }
    }
    trimmed.to_string()
}

fn normalize_host(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_string()
}

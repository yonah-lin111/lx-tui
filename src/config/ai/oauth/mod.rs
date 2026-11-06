//! OAuth 授权流：Device Code 轮询与本地回环 Browser PKCE 回调。
//!
//! 与 `opencode-dev` 的认证插件对齐：设备码流返回授权 URL + 用户码并在后台轮询
//! token 端点；浏览器流在 `127.0.0.1` 上监听回调、校验 `state`、用授权码换取
//! token。所有进行中的尝试由 [`OAuthTaskHandle`] 取消，默认 10 分钟超时。
//!
//! 凭证只通过 [`OAuthCredential`] 返回给调用方，不写入日志。

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot, watch};

use super::connect::OAuthCredential;
use super::presets::{OAuthFlowKind, PresetAuthMethod, find_preset};

pub(super) const OAUTH_TIMEOUT: Duration = Duration::from_secs(600);

mod browser;
mod device;

#[cfg(test)]
mod tests;

/// 授权提示模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthPromptMode {
    /// 自动完成（设备码轮询 / 浏览器回调）。
    Auto,
    /// 需要用户手动粘贴授权码。
    Code,
}

impl OAuthPromptMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Code => "code",
        }
    }
}

/// 启动后的授权提示（展示给 `/connect` 面板或设置页）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthStarted {
    pub attempt_id: String,
    pub url: String,
    pub user_code: Option<String>,
    pub instructions: String,
    pub mode: OAuthPromptMode,
}

/// 进行中的 OAuth 尝试：取消句柄 + 结果接收端。
pub struct OAuthTaskHandle {
    attempt_id: String,
    cancel: watch::Sender<bool>,
    result: oneshot::Receiver<Result<OAuthCredential, String>>,
    code: Option<mpsc::Sender<String>>,
}

impl OAuthTaskHandle {
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }

    /// 取消授权流，释放本地回调端口与轮询任务。
    pub fn cancel(&self) {
        let _ = self.cancel.send(true);
    }

    pub fn is_cancelled(&self) -> bool {
        *self.cancel.borrow()
    }

    /// 手动提交授权码（仅 [`OAuthFlowKind::AuthorizationCode`] 支持）。
    pub fn submit_code(&self, code: impl Into<String>) -> Result<(), String> {
        let sender = self
            .code
            .as_ref()
            .ok_or_else(|| "this OAuth flow does not accept a manual code".to_string())?;
        sender
            .try_send(code.into())
            .map_err(|_| "OAuth flow is no longer waiting for a code".to_string())
    }

    /// 等待授权完成；被取消或失败时返回错误。
    pub async fn wait(mut self) -> Result<OAuthCredential, String> {
        (&mut self.result)
            .await
            .map_err(|_| "OAuth flow ended without a result".to_string())?
    }

    /// 取出结果接收端（供设置页轮询任务使用）。
    pub fn take_result(&mut self) -> Option<oneshot::Receiver<Result<OAuthCredential, String>>> {
        let placeholder = oneshot::channel();
        let result = std::mem::replace(&mut self.result, placeholder.1);
        Some(result)
    }
}

/// 启动 OAuth 授权流。
pub async fn start_oauth_flow(
    provider_id: &str,
    method_id: &str,
    form_answers: &BTreeMap<String, String>,
) -> Result<(OAuthStarted, OAuthTaskHandle), String> {
    let spec = find_preset(provider_id)
        .ok_or_else(|| format!("unknown preset provider '{provider_id}'"))?;
    let flow = spec
        .method(method_id)
        .and_then(PresetAuthMethod::oauth_flow)
        .ok_or_else(|| format!("provider '{provider_id}' has no OAuth method '{method_id}'"))?;
    let attempt_id = random_token(16)?;
    let (cancel_tx, cancel_rx) = watch::channel(false);
    let (result_tx, result_rx) = oneshot::channel();
    let (started, code_tx) = match flow {
        OAuthFlowKind::DeviceCode {
            device_code_url,
            token_url,
            client_id,
            scope,
        } => {
            let started = device::start_device_flow(
                spec,
                method_id,
                device_code_url,
                token_url,
                client_id,
                scope,
                form_answers,
                attempt_id.clone(),
                cancel_rx,
                result_tx,
            )
            .await?;
            (started, None)
        }
        OAuthFlowKind::BrowserPkce {
            authorize_url,
            token_url,
            client_id,
            scope,
            fixed_port,
        } => {
            let started = browser::start_browser_flow(
                spec,
                method_id,
                authorize_url,
                token_url,
                client_id,
                scope,
                fixed_port,
                form_answers,
                attempt_id.clone(),
                cancel_rx,
                result_tx,
            )
            .await?;
            (started, None)
        }
        OAuthFlowKind::AuthorizationCode {
            authorize_url,
            token_url,
            client_id,
            scope,
        } => {
            let (code_tx, code_rx) = mpsc::channel(1);
            let started = browser::start_code_flow(
                provider_id,
                method_id,
                authorize_url,
                token_url,
                client_id,
                scope,
                form_answers,
                attempt_id.clone(),
                cancel_rx,
                code_rx,
                result_tx,
            )?;
            (started, Some(code_tx))
        }
    };
    Ok((
        started,
        OAuthTaskHandle {
            attempt_id,
            cancel: cancel_tx,
            result: result_rx,
            code: code_tx,
        },
    ))
}

/// 按供应商补齐凭证 metadata（OpenAI 需要 ChatGPT 后端端点与账户 ID）。
pub(super) fn credential_from_token_for(
    provider_id: &str,
    method_id: &str,
    value: &Value,
) -> Result<OAuthCredential, String> {
    let access_token = json_string(value, "access_token")
        .or_else(|| json_string(value, "api_key"))
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "token response has no access token".to_string())?;
    let refresh_token = json_string(value, "refresh_token").filter(|token| !token.is_empty());
    let expires_in =
        json_i64(value, "expires_in").or_else(|| json_i64(value, "api_key_expires_in"));
    let mut metadata = BTreeMap::new();
    if provider_id == "openai" {
        metadata.insert(
            "apiEndpoint".to_string(),
            "https://chatgpt.com/backend-api/codex".to_string(),
        );
        let id_token = json_string(value, "id_token");
        let claims = jwt_claims(id_token.as_deref().unwrap_or(access_token.as_str()));
        if let Some(account_id) = claims.as_ref().and_then(chatgpt_account_id) {
            metadata.insert("chatgptAccountId".to_string(), account_id);
        }
    }
    Ok(OAuthCredential {
        method_id: method_id.to_string(),
        access_token,
        refresh_token,
        expires_at_ms: expires_in.map(|seconds| now_ms() + seconds * 1000),
        account_label: None,
        metadata,
    })
}

/// JWT 第二段：base64url 无填充解码为 JSON claims。
fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&decoded).ok()
}

/// 从 JWT claims 中提取 ChatGPT 账户 ID。
fn chatgpt_account_id(claims: &Value) -> Option<String> {
    let auth = claims.get("https://api.openai.com/auth");
    let direct = claims
        .get("chatgpt_account_id")
        .or_else(|| auth.and_then(|auth| auth.get("chatgpt_account_id")))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty());
    if let Some(id) = direct {
        return Some(id.to_string());
    }
    [Some(claims), auth]
        .into_iter()
        .flatten()
        .find_map(|container| {
            container
                .get("organizations")
                .and_then(Value::as_array)
                .and_then(|organizations| organizations.first())
                .and_then(|organization| organization.get("id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        })
}

/// 刷新 OAuth 凭证（存在 `refresh_token` 时）。
pub async fn refresh_oauth_credential(
    provider_id: &str,
    credential: &OAuthCredential,
    form_answers: &BTreeMap<String, String>,
) -> Result<OAuthCredential, String> {
    let spec = find_preset(provider_id)
        .ok_or_else(|| format!("unknown preset provider '{provider_id}'"))?;
    let method_id = credential.method_id.as_str();
    let method = spec
        .method(method_id)
        .ok_or_else(|| format!("provider '{provider_id}' has no method '{method_id}'"))?;
    let flow = method
        .oauth_flow()
        .ok_or_else(|| "credential is not an OAuth method".to_string())?;
    if provider_id == "github-copilot" {
        return Ok(credential.clone());
    }
    let refresh_token = credential
        .refresh_token
        .as_deref()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| "OAuth credential has no refresh token".to_string())?;
    let client = http_client()?;
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", flow.client_id()),
    ];
    if !flow.scope().is_empty() {
        form.push(("scope", flow.scope()));
    }
    // 表单答案（如 GitLab `instanceUrl`）用于展开 token URL；凭证 metadata 优先。
    let mut url_answers = form_answers.clone();
    url_answers.extend(credential.metadata.clone());
    let token_url = expand_flow_url(flow.token_url(), &url_answers);
    let request = client
        .post(&token_url)
        .header(reqwest::header::ACCEPT, "application/json");
    let request = if provider_id.starts_with("opencode") {
        request.json(&serde_json::json!({
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
            "client_id": flow.client_id(),
        }))
    } else {
        request.form(&form)
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("token refresh failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("token refresh response failed: {error}"))?;
    if !status.is_success() {
        return Err(format!("token refresh failed: HTTP {}", status.as_u16()));
    }
    let value: Value = serde_json::from_str(&body)
        .map_err(|_| "token refresh returned invalid JSON".to_string())?;
    let mut refreshed = credential_from_token_for(provider_id, method_id, &value)?;
    refreshed.refresh_token = refreshed
        .refresh_token
        .or_else(|| credential.refresh_token.clone());
    refreshed.account_label = refreshed
        .account_label
        .or_else(|| credential.account_label.clone());
    for (key, value) in &credential.metadata {
        refreshed
            .metadata
            .entry(key.clone())
            .or_insert_with(|| value.clone());
    }
    Ok(refreshed)
}

/// 刷新 `connect.json` 中所有即将过期的 OAuth 凭证；有刷新发生时返回 `true`。
pub async fn ensure_connect_oauth_fresh(ai_path: &std::path::Path) -> bool {
    let connect_path = super::connect::connect_path_for_ai_path(ai_path);
    let connect = match super::connect::load_connect(&connect_path) {
        Ok(Some(connect)) => connect,
        _ => return false,
    };
    let now = now_ms();
    let pending: Vec<(String, OAuthCredential, BTreeMap<String, String>)> = connect
        .providers
        .iter()
        .filter(|(_, record)| record.enabled)
        .filter_map(|(provider_id, record)| {
            let oauth = record.oauth.as_ref()?;
            credential_needs_refresh(oauth, now).then(|| {
                (
                    provider_id.clone(),
                    oauth.clone(),
                    record.form_answers.clone(),
                )
            })
        })
        .collect();
    if pending.is_empty() {
        return false;
    }
    let mut refreshed = Vec::new();
    for (provider_id, oauth, form_answers) in pending {
        if let Ok(credential) = refresh_oauth_credential(&provider_id, &oauth, &form_answers).await
        {
            refreshed.push((provider_id, credential));
        }
    }
    if refreshed.is_empty() {
        return false;
    }
    let mut latest = match super::connect::load_connect(&connect_path) {
        Ok(Some(latest)) => latest,
        _ => return false,
    };
    for (provider_id, credential) in refreshed {
        if let Some(record) = latest.providers.get_mut(&provider_id) {
            record.oauth = Some(credential);
        }
    }
    super::connect::save_connect(&connect_path, &latest).is_ok()
}

/// 凭证是否需要在 5 分钟内刷新。
pub fn credential_needs_refresh(credential: &OAuthCredential, now_ms: i64) -> bool {
    credential
        .expires_at_ms
        .is_some_and(|expires| expires - now_ms < 5 * 60 * 1000)
        && credential
            .refresh_token
            .as_deref()
            .is_some_and(|token| !token.is_empty())
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

pub(super) fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(concat!("lx-tui/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("http client: {error}"))
}

pub(super) async fn wait_or_cancel(
    duration: Duration,
    cancel_rx: &mut watch::Receiver<bool>,
) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(duration) => false,
        _ = cancel_rx.changed() => true,
    }
}

pub(super) fn expand_flow_url(url: &str, answers: &BTreeMap<String, String>) -> String {
    let instance = answers
        .get("instanceUrl")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("https://gitlab.com")
        .trim_end_matches('/');
    url.replace("${instanceUrl}", instance)
}

pub(super) fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub(super) fn random_token(bytes: usize) -> Result<String, String> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut buffer = vec![0u8; bytes];
    let filled = crate::platform::fill_random_bytes(&mut buffer).is_ok();
    if !filled {
        let now = now_ms() as u64;
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut seed =
            now ^ (u64::from(std::process::id()) << 32) ^ seq.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        for chunk in buffer.chunks_mut(8) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let raw = seed.to_le_bytes();
            let len = chunk.len();
            chunk.copy_from_slice(&raw[..len]);
        }
    }
    Ok(URL_SAFE_NO_PAD.encode(buffer))
}

pub(super) fn json_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .map(|text| text.trim().to_string())
}

pub(super) fn json_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
    })
}

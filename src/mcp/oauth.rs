//! MCP OAuth：credentials.json 读写、PKCE 登录、动态注册与刷新。
//!
//! 凭据按 `revision` 原子落盘（目录 0700 / 文件 0600）；token 与 client_secret
//! 只出现在本模块与设置页回调里，不写日志、不进错误文本。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::mcp::{EffectiveServer, McpHttpAuth, McpScope, McpTransportConfig};

/// 启动/刷新时提前量：token 在 60 秒内过期就先刷新。
const REFRESH_MARGIN_MS: i64 = 60_000;
/// 一次 OAuth HTTP 请求的超时。
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// 凭据文件。
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CredentialsFile {
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub servers: BTreeMap<String, Credentials>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at_ms", &self.expires_at_ms)
            .field("scopes", &self.scopes)
            .field("issuer", &self.issuer)
            .field("token_endpoint", &self.token_endpoint)
            .finish()
    }
}

/// 单个 server 的 OAuth 凭据。
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Credentials {
    pub client_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix 毫秒；0 表示未知。
    #[serde(default)]
    pub expires_at_ms: i64,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// 刷新端点（登录时发现）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
}

/// 取 token 失败的两类语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// 没有凭据或刷新被拒：需要用户在设置页登录。
    NeedsLogin,
    /// 网络/协议临时失败。
    Failed(String),
}

/// 一个进行中的授权（设置服务内存表）。
#[derive(Clone)]
pub struct PendingAuthorization {
    pub authorize_url: String,
    pub state: String,
    pub key: String,
    pub verifier: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub token_endpoint: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub issuer: Option<String>,
    pub created_ms: i64,
}

/// 凭据存储（同一路径在进程内共享）。
pub struct CredentialStore {
    path: PathBuf,
    inner: Mutex<CredentialsFile>,
}

static STORES: OnceLock<Mutex<HashMap<PathBuf, Arc<CredentialStore>>>> = OnceLock::new();

fn stores() -> &'static Mutex<HashMap<PathBuf, Arc<CredentialStore>>> {
    STORES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 打开（或复用）某个路径的凭据存储；文件损坏时按空文件启动并保留原文件
/// （下一次写入前用户仍可手工修复；读取不静默覆盖）。
pub fn store_at(path: PathBuf) -> Arc<CredentialStore> {
    let mut registry = match stores().lock() {
        Ok(registry) => registry,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(store) = registry.get(&path) {
        return store.clone();
    }
    let file = read_file(&path).unwrap_or_else(|error| {
        if path.exists() {
            tracing::warn!(target: "lx::mcp", path = %path.display(), %error, "mcp credentials unreadable");
        }
        CredentialsFile::default()
    });
    let store = Arc::new(CredentialStore {
        path: path.clone(),
        inner: Mutex::new(file),
    });
    registry.insert(path, store.clone());
    store
}

/// 默认凭据路径：`<data>/mcp/credentials.json`。
pub fn default_path() -> Option<PathBuf> {
    crate::config::user_data_root()
        .ok()
        .map(|root| root.join("mcp").join("credentials.json"))
}

/// 进程级凭据存储。
pub fn global_store() -> Option<Arc<CredentialStore>> {
    default_path().map(store_at)
}

/// 当前凭据 revision（混入连接指纹；读取失败按 0）。
pub fn revision() -> u64 {
    global_store().map_or(0, |store| store.revision())
}

impl CredentialStore {
    /// 当前 revision。
    pub fn revision(&self) -> u64 {
        self.lock().revision
    }

    /// 读取一份凭据。
    pub fn get(&self, key: &str) -> Option<Credentials> {
        self.lock().servers.get(key).cloned()
    }

    /// 写入/替换一份凭据并原子落盘（revision +1）。
    pub fn set(&self, key: &str, credentials: Credentials) -> Result<(), String> {
        let mut file = self.lock();
        file.servers.insert(key.to_string(), credentials);
        file.revision = file.revision.wrapping_add(1);
        write_file(&self.path, &file)
    }

    /// 删除一份凭据（不存在也推进 revision，确保重连）。
    pub fn remove(&self, key: &str) -> Result<(), String> {
        let mut file = self.lock();
        file.servers.remove(key);
        file.revision = file.revision.wrapping_add(1);
        write_file(&self.path, &file)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, CredentialsFile> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

fn read_file(path: &Path) -> Result<CredentialsFile, String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&text).map_err(|error| error.to_string())
}

fn write_file(path: &Path, file: &CredentialsFile) -> Result<(), String> {
    let body = serde_json::to_string_pretty(file).map_err(|error| error.to_string())?;
    let body = format!("{body}\n");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        crate::platform::restrict_dir_permissions(parent).map_err(|error| error.to_string())?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, body.as_bytes()).map_err(|error| error.to_string())?;
    crate::platform::restrict_file_permissions(&temp).map_err(|error| error.to_string())?;
    std::fs::rename(&temp, path).map_err(|error| error.to_string())
}

/// 凭据键：scope + server 名 + url；同名 server 换 url 不复用旧 token。
pub fn credential_key(server: &EffectiveServer, project_root: &Path) -> String {
    let scope = match server.scope {
        McpScope::User => "user".to_string(),
        McpScope::Project => format!("project:{}", project_root.display()),
    };
    let url = match &server.config.transport {
        McpTransportConfig::Http { url, .. } => url.as_str(),
        McpTransportConfig::Stdio { .. } => "",
    };
    format!("{scope}:{}:{url}", server.name)
}

/// 取可用的 access token；临近过期时先刷新。
pub async fn access_token_for(
    key: &str,
    url: &str,
    scopes: &[String],
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Result<String, TokenError> {
    let store = global_store().ok_or(TokenError::NeedsLogin)?;
    let Some(credentials) = store.get(key) else {
        return Err(TokenError::NeedsLogin);
    };
    if credentials.expires_at_ms == 0 || credentials.expires_at_ms - now_ms() > REFRESH_MARGIN_MS {
        return Ok(credentials.access_token);
    }
    // 刷新失败按设计进入 NeedsLogin：用户需要重新登录，而不是每轮重复等超时。
    refresh_credentials(
        &store,
        key,
        &credentials,
        url,
        scopes,
        client_id,
        client_secret,
    )
    .await
    .map_err(|_| TokenError::NeedsLogin)?;
    store
        .get(key)
        .map(|credentials| credentials.access_token)
        .ok_or(TokenError::NeedsLogin)
}

/// 尝试刷新（调用失败恢复路径）；返回是否得到新 token。
pub async fn refresh(key: &str, url: &str) -> bool {
    let Some(store) = global_store() else {
        return false;
    };
    let Some(credentials) = store.get(key) else {
        return false;
    };
    refresh_credentials(&store, key, &credentials, url, &[], None, None)
        .await
        .is_ok()
}

async fn refresh_credentials(
    store: &CredentialStore,
    key: &str,
    credentials: &Credentials,
    url: &str,
    scopes: &[String],
    client_id: Option<&str>,
    client_secret: Option<&str>,
) -> Result<(), TokenError> {
    let Some(refresh_token) = credentials.refresh_token.as_deref() else {
        return Err(TokenError::NeedsLogin);
    };
    let token_endpoint = match credentials.token_endpoint.as_deref() {
        Some(endpoint) => endpoint.to_string(),
        None => {
            let metadata = discover(url).await.map_err(TokenError::Failed)?;
            metadata.token_endpoint
        }
    };
    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "refresh_token".into()),
        ("refresh_token", refresh_token.to_string()),
        (
            "client_id",
            client_id.unwrap_or(&credentials.client_id).to_string(),
        ),
    ];
    let secret = client_secret.or(credentials.client_secret.as_deref());
    if let Some(secret) = secret {
        form.push(("client_secret", secret.to_string()));
    }
    let response = reqwest::Client::new()
        .post(&token_endpoint)
        .timeout(HTTP_TIMEOUT)
        .form(&form)
        .send()
        .await
        .map_err(|error| TokenError::Failed(format!("oauth refresh failed: {error}")))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        // 400 invalid_grant = refresh token 被拒；其余按临时失败处理。
        return if status == reqwest::StatusCode::BAD_REQUEST
            || status == reqwest::StatusCode::UNAUTHORIZED
        {
            Err(TokenError::NeedsLogin)
        } else {
            Err(TokenError::Failed(format!(
                "oauth refresh failed: HTTP {}",
                status.as_u16()
            )))
        };
    }
    let token: TokenResponse = serde_json::from_str(&text)
        .map_err(|_| TokenError::Failed("oauth refresh returned invalid JSON".into()))?;
    let expires_at_ms = expires_at(&token);
    let updated = Credentials {
        access_token: token.access_token,
        refresh_token: token
            .refresh_token
            .or_else(|| credentials.refresh_token.clone()),
        expires_at_ms,
        scopes: token
            .scope
            .as_deref()
            .map(|scope| {
                scope
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|scopes| !scopes.is_empty())
            .unwrap_or_else(|| scopes.to_vec()),
        ..credentials.clone()
    };
    store
        .set(key, updated)
        .map_err(|error| TokenError::Failed(format!("oauth credential save failed: {error}")))
}

/// 开始授权：发现元数据 → 动态注册（或使用预注册 client）→ 生成 PKCE/state。
pub async fn begin_authorization(
    server: &EffectiveServer,
    project_root: &Path,
    redirect_uri: &str,
) -> Result<PendingAuthorization, String> {
    let McpTransportConfig::Http {
        url,
        auth,
        scopes,
        client_id,
        client_secret,
        ..
    } = &server.config.transport
    else {
        return Err("oauth is only supported for http servers".into());
    };
    if !matches!(auth, McpHttpAuth::Oauth) {
        return Err("server auth is not oauth".into());
    }
    let metadata = discover(url).await?;
    let (client_id, client_secret) = match (client_id.clone(), client_secret.clone()) {
        (Some(client_id), client_secret) => (client_id, client_secret),
        (None, _) => {
            let Some(endpoint) = metadata.registration_endpoint.as_deref() else {
                return Err(
                    "authorization server does not support dynamic registration; set client_id in the server config"
                        .into(),
                );
            };
            register_client(endpoint, redirect_uri).await?
        }
    };
    let verifier = random_token(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(32);
    let effective_scopes = if scopes.is_empty() {
        metadata.scopes_supported.clone()
    } else {
        scopes.clone()
    };
    let mut url_builder = reqwest::Url::parse(&metadata.authorization_endpoint)
        .map_err(|error| format!("invalid authorization endpoint: {error}"))?;
    {
        let mut query = url_builder.query_pairs_mut();
        query.append_pair("response_type", "code");
        query.append_pair("client_id", &client_id);
        query.append_pair("redirect_uri", redirect_uri);
        query.append_pair("state", &state);
        query.append_pair("code_challenge", &challenge);
        query.append_pair("code_challenge_method", "S256");
        if !effective_scopes.is_empty() {
            query.append_pair("scope", &effective_scopes.join(" "));
        }
        query.append_pair("resource", url);
    }
    Ok(PendingAuthorization {
        authorize_url: url_builder.to_string(),
        state,
        key: credential_key(server, project_root),
        verifier,
        client_id,
        client_secret,
        token_endpoint: metadata.token_endpoint,
        redirect_uri: redirect_uri.to_string(),
        scopes: effective_scopes,
        issuer: metadata.issuer,
        created_ms: now_ms(),
    })
}

/// 用回调 code 换 token 并写入凭据文件。
pub async fn complete_authorization(
    pending: &PendingAuthorization,
    code: &str,
    store: &CredentialStore,
) -> Result<(), String> {
    let mut form: Vec<(&str, String)> = vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.to_string()),
        ("redirect_uri", pending.redirect_uri.clone()),
        ("client_id", pending.client_id.clone()),
        ("code_verifier", pending.verifier.clone()),
    ];
    if let Some(secret) = &pending.client_secret {
        form.push(("client_secret", secret.clone()));
    }
    let response = reqwest::Client::new()
        .post(&pending.token_endpoint)
        .timeout(HTTP_TIMEOUT)
        .form(&form)
        .send()
        .await
        .map_err(|error| format!("token exchange failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("token exchange failed: HTTP {}", status.as_u16()));
    }
    let token: TokenResponse = response
        .json()
        .await
        .map_err(|error| format!("token exchange returned invalid JSON: {error}"))?;
    let expires_at_ms = expires_at(&token);
    let credentials = Credentials {
        client_id: pending.client_id.clone(),
        client_secret: pending.client_secret.clone(),
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at_ms,
        scopes: token
            .scope
            .as_deref()
            .map(|scope| {
                scope
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|scopes| !scopes.is_empty())
            .unwrap_or_else(|| pending.scopes.clone()),
        issuer: pending.issuer.clone(),
        token_endpoint: Some(pending.token_endpoint.clone()),
    };
    store.set(&pending.key, credentials)
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    /// RFC 6749：空格分隔的 scope 字符串。
    #[serde(default)]
    scope: Option<String>,
}

fn expires_at(token: &TokenResponse) -> i64 {
    token
        .expires_in
        .filter(|seconds| *seconds > 0)
        .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1000)))
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
struct Metadata {
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
    scopes_supported: Vec<String>,
    issuer: Option<String>,
}

/// 发现授权服务器元数据（RFC 9728 + RFC 8414/OpenID）。
async fn discover(resource_url: &str) -> Result<Metadata, String> {
    let client = reqwest::Client::new();
    let parsed =
        reqwest::Url::parse(resource_url).map_err(|error| format!("invalid mcp url: {error}"))?;
    let mut resource_metadata_urls: Vec<String> = Vec::new();
    let probe = client
        .get(resource_url)
        .timeout(HTTP_TIMEOUT)
        .header("Accept", "application/json")
        .send()
        .await;
    if let Ok(response) = probe
        && response.status() == reqwest::StatusCode::UNAUTHORIZED
        && let Some(header) = response
            .headers()
            .get(reqwest::header::WWW_AUTHENTICATE)
            .and_then(|value| value.to_str().ok())
        && let Some(url) = www_authenticate_resource_metadata(header, &parsed)
    {
        resource_metadata_urls.push(url);
    }
    let host = parsed.host_str().unwrap_or_default();
    let origin = match parsed.port() {
        Some(port) => format!("{}://{host}:{port}", parsed.scheme()),
        None => format!("{}://{host}", parsed.scheme()),
    };
    let path = parsed.path().trim_start_matches('/');
    if !path.is_empty() {
        resource_metadata_urls.push(format!(
            "{origin}/.well-known/oauth-protected-resource/{path}"
        ));
    }
    resource_metadata_urls.push(format!("{origin}/.well-known/oauth-protected-resource"));

    let mut issuer: Option<String> = None;
    let mut resource_scopes: Vec<String> = Vec::new();
    for url in resource_metadata_urls {
        let Ok(response) = client
            .get(&url)
            .timeout(HTTP_TIMEOUT)
            .header("Accept", "application/json")
            .send()
            .await
        else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(value) = response.json::<serde_json::Value>().await else {
            continue;
        };
        issuer = value
            .get("authorization_servers")
            .and_then(serde_json::Value::as_array)
            .and_then(|servers| servers.first())
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        resource_scopes = value
            .get("scopes_supported")
            .and_then(serde_json::Value::as_array)
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if issuer.is_some() {
            break;
        }
    }
    let issuer = issuer.unwrap_or_else(|| origin.clone());
    let mut metadata_urls = vec![
        format!(
            "{}/.well-known/oauth-authorization-server",
            issuer.trim_end_matches('/')
        ),
        format!(
            "{}/.well-known/openid-configuration",
            issuer.trim_end_matches('/')
        ),
    ];
    if issuer != origin {
        metadata_urls.push(format!("{origin}/.well-known/oauth-authorization-server"));
    }
    for url in metadata_urls {
        let Ok(response) = client
            .get(&url)
            .timeout(HTTP_TIMEOUT)
            .header("Accept", "application/json")
            .send()
            .await
        else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let Ok(value) = response.json::<serde_json::Value>().await else {
            continue;
        };
        let Some(authorization_endpoint) = value
            .get("authorization_endpoint")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(token_endpoint) = value
            .get("token_endpoint")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let registration_endpoint = value
            .get("registration_endpoint")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        let mut scopes_supported: Vec<String> = value
            .get("scopes_supported")
            .and_then(serde_json::Value::as_array)
            .map(|scopes| {
                scopes
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if scopes_supported.is_empty() {
            scopes_supported = resource_scopes;
        }
        return Ok(Metadata {
            authorization_endpoint: authorization_endpoint.to_string(),
            token_endpoint: token_endpoint.to_string(),
            registration_endpoint,
            scopes_supported,
            issuer: value
                .get("issuer")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or(Some(issuer)),
        });
    }
    Err("oauth metadata discovery failed".into())
}

fn www_authenticate_resource_metadata(header: &str, base: &reqwest::Url) -> Option<String> {
    let marker = "resource_metadata=";
    let start = header.find(marker)? + marker.len();
    let rest = &header[start..];
    let value = if let Some(stripped) = rest.strip_prefix('"') {
        stripped.split('"').next().unwrap_or_default()
    } else {
        rest.split([',', ' ']).next().unwrap_or_default()
    };
    base.join(value).ok().map(|url| url.to_string())
}

async fn register_client(
    endpoint: &str,
    redirect_uri: &str,
) -> Result<(String, Option<String>), String> {
    let body = serde_json::json!({
        "client_name": "lx",
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let response = reqwest::Client::new()
        .post(endpoint)
        .timeout(HTTP_TIMEOUT)
        .json(&body)
        .send()
        .await
        .map_err(|error| format!("dynamic client registration failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "dynamic client registration failed: HTTP {}",
            response.status().as_u16()
        ));
    }
    let value: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("dynamic client registration returned invalid JSON: {error}"))?;
    let client_id = value
        .get("client_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("dynamic client registration returned no client_id")?
        .to_string();
    let client_secret = value
        .get("client_secret")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    Ok((client_id, client_secret))
}

/// 32 字节随机数 → base64url（无填充）或 hex（用于 state）。
fn random_token(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    let filled = std::fs::File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut buffer))
        .is_ok();
    if !filled {
        let now = now_ms() as u64;
        let mut seed = now ^ u64::from(std::process::id()) << 32;
        for chunk in buffer.chunks_mut(8) {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let bytes = seed.to_le_bytes();
            let len = chunk.len();
            chunk.copy_from_slice(&bytes[..len]);
        }
    }
    if bytes == 32 {
        URL_SAFE_NO_PAD.encode(&buffer)
    } else {
        buffer.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod tests;

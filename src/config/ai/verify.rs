//! API Key 校验与远端模型发现（OpenAI / OpenAI 兼容 / Anthropic / Google）。
//!
//! 校验分两级：优先探测 `<baseURL>/models`，端点缺失（404/405/501）或返回非 JSON
//! 时回退到一次极简对话请求。任何一级命中鉴权错误都视为 Key 无效；供应商服务端
//! 错误单独报告。`fetch_provider_models` 复用同一套端点规则，供设置页拉取模型列表。

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::{Value, json};

use super::presets::default_models_for_preset;
use super::{ModelConfig, ProviderTransport, validate_base_url};

/// 远端发现到的模型条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredModel {
    pub id: String,
    pub name: Option<String>,
}

/// API Key 校验结果：校验通过时附带 `/models` 返回的模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyOutcome {
    pub discovered_models: Vec<DiscoveredModel>,
}

/// Anthropic 版本头（`/models` 与 `/messages` 均要求）。
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// 模型探测整体超时。
const MODELS_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// 对话探测整体超时（生成请求通常比列表慢）。
const CHAT_PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// 响应体上限，避免异常供应商拖垮内存。
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// 鉴权错误关键词（忽略大小写）。
const AUTH_ERROR_KEYWORDS: [&str; 7] = [
    "invalid_api_key",
    "authentication_error",
    "api_key_invalid",
    "api key not valid",
    "invalid x-api-key",
    "unauthorized",
    "unauthenticated",
];

/// 校验 API Key：先探测 `/models`，必要时回退到极简对话请求。
pub async fn verify_provider_api_key(
    transport: ProviderTransport,
    base_url: &str,
    api_key: &str,
    probe_model: Option<&str>,
) -> Result<VerifyOutcome, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("API key cannot be empty".into());
    }
    validate_base_url(base_url).map_err(|error| error.to_string())?;
    // 离线单元测试使用的保留域名/本地固定端口：不做真实 DNS 与网络请求。
    if is_offline_dummy(base_url) {
        return Ok(VerifyOutcome {
            discovered_models: Vec::new(),
        });
    }

    let client = build_client(MODELS_PROBE_TIMEOUT)?;
    match probe_models(&client, transport, base_url, api_key).await {
        ModelsProbe::Models(models) => Ok(VerifyOutcome {
            discovered_models: models,
        }),
        ModelsProbe::Auth(message)
        | ModelsProbe::Server(message)
        | ModelsProbe::Rejected(message) => Err(message),
        ModelsProbe::Fallback => {
            let client = build_client(CHAT_PROBE_TIMEOUT)?;
            chat_probe(&client, transport, base_url, api_key, probe_model).await
        }
    }
}

/// 拉取供应商远端模型列表（四种协议均支持），按模型 id 排序。
pub async fn fetch_provider_models(
    transport: ProviderTransport,
    base_url: &str,
    api_key: &str,
) -> Result<Vec<DiscoveredModel>, String> {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return Err("API key cannot be empty".into());
    }
    validate_base_url(base_url).map_err(|error| error.to_string())?;
    let client = build_client(MODELS_PROBE_TIMEOUT)?;
    let mut models = match probe_models(&client, transport, base_url, api_key).await {
        ModelsProbe::Models(models) => models,
        ModelsProbe::Auth(message)
        | ModelsProbe::Server(message)
        | ModelsProbe::Rejected(message) => return Err(message),
        ModelsProbe::Fallback => {
            return Err("provider does not expose a models endpoint".to_string());
        }
    };
    models.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(models)
}

/// 把远端发现的模型合并进预设模型表。
///
/// 已存在的模型保持原配置（仅在名称为空/等于 id 时用发现到的显示名补全）；
/// 目录里已有的模型直接复用目录元数据；其余按发现到的 id 构造，并为推理模型
/// 补上 low/medium/high 思考等级。
pub fn merge_discovered_models_for_preset(
    provider_id: &str,
    existing: Option<&BTreeMap<String, ModelConfig>>,
    discovered: &[DiscoveredModel],
) -> BTreeMap<String, ModelConfig> {
    let catalog = default_models_for_preset(provider_id);
    let mut out = existing.cloned().unwrap_or_else(|| catalog.clone());
    for item in discovered {
        if let Some(current) = out.get_mut(&item.id) {
            if current.name == item.id
                && let Some(name) = item
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
            {
                current.name = name.to_string();
            }
            continue;
        }
        if let Some(model) = catalog.get(&item.id) {
            out.insert(item.id.clone(), model.clone());
            continue;
        }
        let name = item
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&item.id)
            .to_string();
        out.insert(
            item.id.clone(),
            ModelConfig {
                id: item.id.clone(),
                name,
                transport: None,
                limit: None,
                modalities: None,
                variants: reasoning_variants(&item.id),
                variant: None,
                pricing: None,
                extra: serde_json::Map::new(),
            },
        );
    }
    out
}

/// 构建探测用 HTTP 客户端（禁止重定向，避免 Key 被转发到第三方）。
fn build_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("http client init failed: {error}"))
}

enum ModelsProbe {
    Models(Vec<DiscoveredModel>),
    Auth(String),
    Server(String),
    Rejected(String),
    Fallback,
}

async fn probe_models(
    client: &reqwest::Client,
    transport: ProviderTransport,
    base_url: &str,
    api_key: &str,
) -> ModelsProbe {
    let urls = models_urls(transport, base_url);
    let mut needs_fallback = false;
    for url in urls {
        let request = apply_auth(
            client.get(&url).header("Accept", "application/json"),
            transport,
            api_key,
        );
        let (status, text) = match send(request).await {
            Ok(pair) => pair,
            Err(message) => return ModelsProbe::Rejected(message),
        };
        let code = status.as_u16();
        if status.is_success() {
            let Ok(value) = serde_json::from_str::<Value>(&text) else {
                needs_fallback = true;
                continue;
            };
            if let Some(error) = payload_error_without_models(&value) {
                return ModelsProbe::Auth(format!("Invalid API key: {error}"));
            }
            match parse_discovered_models(&value) {
                Some(models) => return ModelsProbe::Models(models),
                None => {
                    needs_fallback = true;
                    continue;
                }
            }
        }
        if code == 401 || code == 403 {
            let message = error_message(&text).unwrap_or_else(|| "unauthorized".to_string());
            return ModelsProbe::Auth(format!("Invalid API key (HTTP {code}): {message}"));
        }
        if code == 400 && contains_auth_error(&text) {
            let message = error_message(&text).unwrap_or_else(|| "invalid API key".to_string());
            return ModelsProbe::Auth(format!("Invalid API key: {message}"));
        }
        if status.is_server_error() {
            return ModelsProbe::Server(format!("Provider server error (HTTP {code})"));
        }
        if code == 404 || code == 405 || code == 501 {
            needs_fallback = true;
            continue;
        }
        // 非 JSON 的 4xx 同样当作端点不可用，交给对话探测判定鉴权。
        if serde_json::from_str::<Value>(&text).is_err() {
            needs_fallback = true;
            continue;
        }
        return ModelsProbe::Rejected(format!(
            "Provider rejected the models request (HTTP {code})"
        ));
    }
    if needs_fallback {
        ModelsProbe::Fallback
    } else {
        ModelsProbe::Rejected("provider returned no usable models response".to_string())
    }
}

async fn chat_probe(
    client: &reqwest::Client,
    transport: ProviderTransport,
    base_url: &str,
    api_key: &str,
    probe_model: Option<&str>,
) -> Result<VerifyOutcome, String> {
    let model = probe_model
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .unwrap_or("gpt-4o-mini");
    let requests = chat_probe_requests(transport, base_url, model);
    let mut last_unavailable = None;
    for (index, probe) in requests.iter().enumerate() {
        let last = index + 1 == requests.len();
        let request = apply_auth(
            client.post(&probe.url).header("Accept", "application/json"),
            transport,
            api_key,
        )
        .json(&probe.body);
        let (status, text) = send(request).await?;
        let code = status.as_u16();
        if code == 401 || code == 403 || contains_auth_error(&text) {
            return Err(format!("Invalid API key (HTTP {code})"));
        }
        if status.is_success() {
            return Ok(VerifyOutcome {
                discovered_models: Vec::new(),
            });
        }
        let json_ok = serde_json::from_str::<Value>(&text).is_ok();
        // 400/404/422 且返回合法 JSON：说明鉴权已通过，只是模型或参数被拒绝。
        // 还有备用端点时先尝试（OpenAI Responses / Anthropic 无 `/v1` 前缀），
        // 全部端点都 404 后再按同一规则收敛。
        if matches!(code, 400 | 404 | 422) && json_ok {
            if !last && matches!(code, 404 | 405) {
                last_unavailable = Some(format!("probe endpoint unavailable (HTTP {code})"));
                continue;
            }
            return Ok(VerifyOutcome {
                discovered_models: Vec::new(),
            });
        }
        if code == 404 || code == 405 {
            last_unavailable = Some(format!("probe endpoint unavailable (HTTP {code})"));
            continue;
        }
        return Err(format!(
            "Provider rejected the verification request (HTTP {code})"
        ));
    }
    Err(last_unavailable.unwrap_or_else(|| "Provider verification failed".to_string()))
}

struct ChatProbeRequest {
    url: String,
    body: Value,
}

fn chat_probe_requests(
    transport: ProviderTransport,
    base_url: &str,
    model: &str,
) -> Vec<ChatProbeRequest> {
    let chat_body = || {
        json!({
            "model": model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1,
        })
    };
    match transport {
        ProviderTransport::Openai => vec![
            ChatProbeRequest {
                url: join_url(base_url, "responses"),
                body: json!({
                    "model": model,
                    "input": "hi",
                    "max_output_tokens": 16,
                }),
            },
            ChatProbeRequest {
                url: join_url(base_url, "chat/completions"),
                body: chat_body(),
            },
        ],
        ProviderTransport::OpenaiCompatible => vec![ChatProbeRequest {
            url: join_url(base_url, "chat/completions"),
            body: chat_body(),
        }],
        ProviderTransport::Anthropic => anthropic_message_urls(base_url)
            .into_iter()
            .map(|url| ChatProbeRequest {
                url,
                body: chat_body(),
            })
            .collect(),
        ProviderTransport::Google => {
            let model = model.trim_start_matches("models/");
            vec![ChatProbeRequest {
                url: join_url(base_url, &format!("models/{model}:generateContent")),
                body: json!({"contents": [{"parts": [{"text": "hi"}]}]}),
            }]
        }
    }
}

fn models_urls(transport: ProviderTransport, base_url: &str) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    match transport {
        ProviderTransport::Anthropic => {
            let mut urls = vec![join_url(base, "models")];
            if !base.ends_with("/v1") {
                urls.push(join_url(base, "v1/models"));
            }
            urls
        }
        ProviderTransport::Google => {
            let mut urls = vec![join_url(base, "models")];
            if !base.ends_with("/v1beta") && !base.ends_with("/v1") {
                urls.push(join_url(base, "v1beta/models"));
            }
            urls
        }
        ProviderTransport::Openai | ProviderTransport::OpenaiCompatible => {
            vec![join_url(base, "models")]
        }
    }
}

fn anthropic_message_urls(base_url: &str) -> Vec<String> {
    let base = base_url.trim_end_matches('/');
    let mut urls = vec![join_url(base, "messages")];
    if !base.ends_with("/v1") {
        urls.push(join_url(base, "v1/messages"));
    }
    urls
}

fn join_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    format!("{base}/{}", path.trim_start_matches('/'))
}

fn apply_auth(
    request: reqwest::RequestBuilder,
    transport: ProviderTransport,
    api_key: &str,
) -> reqwest::RequestBuilder {
    match transport {
        ProviderTransport::Openai | ProviderTransport::OpenaiCompatible => {
            request.header("Authorization", format!("Bearer {api_key}"))
        }
        ProviderTransport::Anthropic => request
            .header("x-api-key", api_key)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("anthropic-version", ANTHROPIC_VERSION),
        ProviderTransport::Google => request.header("x-goog-api-key", api_key),
    }
}

async fn send(request: reqwest::RequestBuilder) -> Result<(reqwest::StatusCode, String), String> {
    let response = request.send().await.map_err(request_error)?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|_| "provider response could not be read".to_string())?;
    if text.len() > MAX_BODY_BYTES {
        return Err("provider response too large".to_string());
    }
    Ok((status, text))
}

fn request_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        "provider request timed out".to_string()
    } else {
        "provider request failed".to_string()
    }
}

/// 离线测试域名：`.invalid` 保留域名与本地 `20128` 开发端口。
fn is_offline_dummy(base_url: &str) -> bool {
    let host = host_of(base_url);
    let hostname = host.split(':').next().unwrap_or_default();
    hostname.ends_with(".invalid") || host == "localhost:20128"
}

fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or_default()
}

/// 顶层错误对象且没有模型数组：语义上等于鉴权失败。
fn payload_error_without_models(value: &Value) -> Option<String> {
    let object = value.as_object()?;
    let has_models = object.get("data").is_some_and(Value::is_array)
        || object.get("models").is_some_and(Value::is_array);
    if has_models {
        return None;
    }
    object.get("error").map(error_text)
}

fn error_message(text: &str) -> Option<String> {
    let value: Value = serde_json::from_str(text).ok()?;
    if let Some(error) = value.get("error") {
        return Some(error_text(error));
    }
    for key in ["message", "detail"] {
        if let Some(message) = value
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|message| !message.is_empty())
        {
            return Some(message.to_string());
        }
    }
    None
}

fn error_text(value: &Value) -> String {
    if let Some(text) = value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return text.to_string();
    }
    if let Some(object) = value.as_object() {
        for key in ["message", "detail", "error"] {
            if let Some(text) = object
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                return text.to_string();
            }
        }
    }
    "provider rejected the API key".to_string()
}

fn contains_auth_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    AUTH_ERROR_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(keyword))
}

/// 解析 `data[].id` / Google `models[].name` / 纯字符串数组。
fn parse_discovered_models(value: &Value) -> Option<Vec<DiscoveredModel>> {
    let items: Vec<&Value> = if let Some(array) = value.as_array() {
        array.iter().collect()
    } else if let Some(object) = value.as_object() {
        if let Some(data) = object.get("data").and_then(Value::as_array) {
            data.iter().collect()
        } else if let Some(models) = object.get("models").and_then(Value::as_array) {
            models.iter().collect()
        } else {
            return None;
        }
    } else {
        return None;
    };
    let mut out = Vec::new();
    for item in items {
        match item {
            Value::String(id) => push_discovered(&mut out, id, None),
            Value::Object(object) => {
                if let Some(id) = object.get("id").and_then(Value::as_str) {
                    let name = object
                        .get("display_name")
                        .or_else(|| object.get("displayName"))
                        .or_else(|| object.get("name"))
                        .and_then(Value::as_str);
                    push_discovered(&mut out, id, name);
                } else if let Some(name) = object.get("name").and_then(Value::as_str) {
                    let display = object.get("displayName").and_then(Value::as_str);
                    push_discovered(&mut out, name, display);
                }
            }
            _ => {}
        }
    }
    Some(out)
}

fn push_discovered(out: &mut Vec<DiscoveredModel>, raw_id: &str, name: Option<&str>) {
    let id = raw_id.trim();
    let id = id.strip_prefix("models/").unwrap_or(id);
    if id.is_empty() || out.iter().any(|model| model.id == id) {
        return;
    }
    let name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.strip_prefix("models/").unwrap_or(name).to_string());
    out.push(DiscoveredModel {
        id: id.to_string(),
        name,
    });
}

/// 推理模型 id 判定：`o1/o3/o4`、`gpt-5`、Claude 4 / 3.7、Gemini 2.5/3、
/// DeepSeek R1/reasoner 与 QwQ。
fn is_reasoning_model(model_id: &str) -> bool {
    let lowered = model_id.to_ascii_lowercase();
    let segment = lowered.rsplit('/').next().unwrap_or(&lowered);
    segment.starts_with("o1")
        || segment.starts_with("o3")
        || segment.starts_with("o4")
        || lowered.contains("gpt-5")
        || lowered.contains("claude-opus-4")
        || lowered.contains("claude-sonnet-4")
        || lowered.contains("claude-3-7")
        || lowered.contains("gemini-2.5")
        || lowered.contains("gemini-3")
        || lowered.contains("deepseek-r1")
        || lowered.contains("deepseek-reasoner")
        || lowered.contains("qwq")
}

fn reasoning_variants(model_id: &str) -> BTreeMap<String, Value> {
    if !is_reasoning_model(model_id) {
        return BTreeMap::new();
    }
    ["low", "medium", "high"]
        .into_iter()
        .map(|level| (level.to_string(), json!({ "reasoningEffort": level })))
        .collect()
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod verify_tests;

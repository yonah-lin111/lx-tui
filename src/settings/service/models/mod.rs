//! 模型设置 API：GET/PUT `/api/models/settings` 与 POST `/api/models/fetch`。
//!
//! - GET 只返回 `hasApiKey`，永不回传明文 Key。
//! - PUT 的 `apiKey` 字段是本次动作（留空保持 / 显式清除 / 非空替换），
//!   携带 `revision` 防止两个设置页互相覆盖；校验失败 422 且文件不改。
//! - fetch 用当前编辑中的 base URL/key 做一次性远端发现，后台异步执行，
//!   不堵本地连接；响应不含 Key。

use std::time::Duration;

use serde_json::{Value, json};

use super::handlers::{Request, Response};
use super::{ServiceState, SettingsEvent};

/// 模型配置文件（`config/ai.json`）路径。
fn ai_path(state: &ServiceState) -> std::path::PathBuf {
    match state.user_root() {
        Some(root) => root.join("config").join("ai.json"),
        None => crate::config::ai::ai_config_path()
            .unwrap_or_else(|_| std::path::PathBuf::from("config/ai.json")),
    }
}

/// `/api/models` 系列入口。
pub async fn route(request: &Request, state: &ServiceState) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/models/settings") => get_settings(state),
        ("PUT", "/api/models/settings") => put_settings(request, state),
        ("POST", "/api/models/fetch") => fetch_models(request, state).await,
        _ => Response::error(404, "not found"),
    }
}

/// 计算当前配置的 revision（内容哈希前 16 hex）。
fn revision_of(settings: &crate::config::ai::AiSettings) -> String {
    let value = serde_json::to_string(settings).unwrap_or_default();
    revision_hash(&value)
}

/// 稳定内容哈希（FNV-1a 64 位，hex 16 位）。
fn revision_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// GET：脱敏 provider/model 列表、默认选择与 revision。
fn get_settings(state: &ServiceState) -> Response {
    let path = ai_path(state);
    let settings = match crate::config::ai::load_ai(&path) {
        Ok(Some(settings)) => settings,
        Ok(None) => crate::config::ai::AiSettings::empty(),
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let providers: Vec<Value> = settings
        .providers
        .iter()
        .map(|(key, provider)| {
            json!({
                "id": provider.id,
                "key": key,
                "type": provider.kind.as_str(),
                "name": provider.name,
                "baseUrl": provider.options.base_url,
                "hasApiKey": provider.options.api_key.as_deref().is_some_and(|key| !key.is_empty()),
                "enabled": settings.enabled_providers.contains(key),
                "models": provider.models.iter().map(|(model_key, model)| {
                    json!({
                        "id": model.id,
                        "key": model_key,
                        "name": model.name,
                        "transport": model.transport.map(|transport| transport.as_str()),
                        "limit": model.limit.as_ref().map(|limit| json!({
                            "context": limit.context,
                            "output": limit.output,
                        })),
                        "modalities": model.modalities,
                        "variants": model.variants,
                        "variant": model.variant,
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let default_model = settings
        .default_model
        .as_ref()
        .map(|selection| {
            json!({
                "provider": selection.provider,
                "model": selection.model,
                "variant": selection.variant,
            })
        })
        .unwrap_or(Value::Null);
    let body = json!({
        "providers": providers,
        "enabledProviders": settings.enabled_providers,
        "defaultModel": default_model,
        "streamIdleTimeoutMs": settings.idle_timeout_ms(),
        "revision": revision_of(&settings),
    });
    Response::json(200, serde_json::to_string(&body).unwrap_or_default())
}

/// PUT：完整验证后原子保存；成功投递 ModelsChanged。
fn put_settings(request: &Request, state: &ServiceState) -> Response {
    let body: Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    // revision 冲突检查：与磁盘当前内容比较。文件缺失时用空配置的同一口径
    // 计算 revision（与 GET 一致），否则首次保存必然误报 409。
    let path = ai_path(state);
    let current = match crate::config::ai::load_ai(&path) {
        Ok(Some(settings)) => settings,
        Ok(None) => crate::config::ai::AiSettings::empty(),
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let current_revision = revision_of(&current);
    let provided = body.get("revision").and_then(Value::as_str).unwrap_or("");
    if !provided.is_empty() && provided != current_revision {
        return Response::error(409, "configuration changed elsewhere; reload and retry");
    }
    let merged = match merge_settings(Some(current), &body) {
        Ok(settings) => settings,
        Err(message) => return Response::error(422, &message),
    };
    if let Err(error) = crate::config::ai::save_ai(&path, &merged) {
        return Response::error(500, &error.to_string());
    }
    let _ = state
        .sender()
        .send(SettingsEvent::ModelsChanged(Box::new(merged.clone())));
    let body = json!({"ok": true, "revision": revision_of(&merged)});
    Response::json(200, serde_json::to_string(&body).unwrap_or_default())
}

/// PUT body 到磁盘配置的合并：provider 键为稳定 id；
/// `apiKey` 三态（省略/空 = Keep、`clearApiKey`=true = Clear、非空 = Replace）。
fn merge_settings(
    current: Option<crate::config::ai::AiSettings>,
    body: &Value,
) -> Result<crate::config::ai::AiSettings, String> {
    use crate::config::ai::{
        ModelConfig, ModelLimit, ModelSelection, ModelTransport, ProviderConfig, ProviderOptions,
        ProviderTransport,
    };
    let mut settings = current.unwrap_or_default();
    let Some(providers_value) = body.get("providers").and_then(Value::as_array) else {
        return Err("body must contain a providers array".into());
    };
    let mut providers = std::collections::BTreeMap::new();
    for item in providers_value {
        let key = item
            .get("key")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .ok_or("provider key is required")?
            .to_string();
        if key.trim().is_empty() {
            return Err("provider key must not be empty".into());
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(&key)
            .to_string();
        let kind_text = item
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("openai-compatible");
        let Some(kind) = ProviderTransport::parse(kind_text) else {
            return Err(format!("invalid provider type: {kind_text}"));
        };
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(&id)
            .to_string();
        let base_url = item
            .get("baseUrl")
            .and_then(Value::as_str)
            .ok_or("provider baseUrl is required")?
            .to_string();
        // Key 三态合并。
        let existing = settings.providers.get(&key);
        let api_key = merge_api_key(existing, item)?;
        let mut models = std::collections::BTreeMap::new();
        if let Some(models_value) = item.get("models").and_then(Value::as_array) {
            for model_item in models_value {
                // 记录键以模型 id 为准（lx-agent 语义：`selection.model` 就是发给
                // 供应商的模型 id）。前端可能派生一个独立的显示键，这里忽略它，
                // 直接按 id 存储，避免“键与 id 不一致”这类无意义冲突。
                let model_id = model_item
                    .get("id")
                    .and_then(Value::as_str)
                    .or_else(|| model_item.get("key").and_then(Value::as_str))
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .ok_or("model id is required")?
                    .to_string();
                let model_name = model_item
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(&model_id)
                    .to_string();
                let transport = model_item
                    .get("transport")
                    .and_then(Value::as_str)
                    .and_then(ModelTransport::parse);
                let limit = model_item.get("limit").and_then(|limit| {
                    let context = limit.get("context").and_then(Value::as_u64)?;
                    let output = limit.get("output").and_then(Value::as_u64)?;
                    Some(ModelLimit { context, output })
                });
                let mut variants = std::collections::BTreeMap::new();
                if let Some(variants_value) = model_item.get("variants").and_then(Value::as_object)
                {
                    for (variant_key, variant_value) in variants_value {
                        variants.insert(variant_key.clone(), variant_value.clone());
                    }
                }
                let variant = model_item
                    .get("variant")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .filter(|variant| !variant.is_empty());
                if let Some(variant) = &variant
                    && !variants.contains_key(variant)
                {
                    return Err(format!(
                        "model '{model_id}' variant '{variant}' is not in variants"
                    ));
                }
                let previous_model = existing.and_then(|provider| provider.models.get(&model_id));
                // 模态三态：字段缺失 = 保留旧值，显式 null = 清除，对象 = 覆盖。
                let modalities = match model_item.get("modalities") {
                    Some(Value::Null) => None,
                    Some(value) => Some(value.clone()),
                    None => previous_model.and_then(|model| model.modalities.clone()),
                };
                // 未知字段（`extra`）同样往返保留：已知键之外的字段并入。
                const KNOWN_KEYS: [&str; 9] = [
                    "id",
                    "key",
                    "name",
                    "transport",
                    "limit",
                    "variants",
                    "variant",
                    "modalities",
                    "pricing",
                ];
                let mut extra = previous_model
                    .map(|model| model.extra.clone())
                    .unwrap_or_default();
                if let Some(item) = model_item.as_object() {
                    for (field, value) in item {
                        if KNOWN_KEYS.contains(&field.as_str()) {
                            continue;
                        }
                        extra.insert(field.clone(), value.clone());
                    }
                }
                models.insert(
                    model_id.clone(),
                    ModelConfig {
                        id: model_id.clone(),
                        name: model_name,
                        transport,
                        limit,
                        modalities,
                        variants,
                        variant,
                        pricing: previous_model.and_then(|model| model.pricing.clone()),
                        extra,
                    },
                );
            }
        }
        providers.insert(
            key.clone(),
            ProviderConfig {
                id,
                kind,
                name,
                options: ProviderOptions { api_key, base_url },
                models,
                extra: existing
                    .map(|provider| provider.extra.clone())
                    .unwrap_or_default(),
            },
        );
    }
    settings.providers = providers;
    // 启用列表：未知 id 剔除。
    let enabled: Vec<String> = body
        .get("enabledProviders")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    settings.enabled_providers = enabled
        .into_iter()
        .filter(|id| settings.providers.contains_key(id))
        .collect();
    // 默认选择。
    settings.default_model = match body.get("defaultModel") {
        Some(Value::Null) | None => None,
        Some(selection) => {
            let provider = selection
                .get("provider")
                .and_then(Value::as_str)
                .ok_or("defaultModel.provider is required")?
                .to_string();
            let model = selection
                .get("model")
                .and_then(Value::as_str)
                .ok_or("defaultModel.model is required")?
                .to_string();
            let variant = selection
                .get("variant")
                .and_then(Value::as_str)
                .filter(|variant| !variant.is_empty())
                .map(str::to_string);
            Some(ModelSelection {
                provider,
                model,
                variant,
            })
        }
    };
    if let Some(timeout) = body.get("streamIdleTimeoutMs").and_then(Value::as_u64) {
        settings.stream_idle_timeout_ms = Some(timeout);
    }
    settings.extra.clear();
    settings.validate().map_err(|error| error.to_string())?;
    Ok(settings)
}

/// Key 三态合并：省略/空字符串保持旧 Key，`clearApiKey: true` 清除，非空替换。
fn merge_api_key(
    existing: Option<&crate::config::ai::ProviderConfig>,
    item: &Value,
) -> Result<Option<String>, String> {
    if item
        .get("clearApiKey")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Ok(None);
    }
    match item.get("apiKey").and_then(Value::as_str) {
        Some(new_key) if !new_key.is_empty() => Ok(Some(new_key.to_string())),
        // 留空/省略：保持旧 Key。
        _ => Ok(existing.and_then(|provider| provider.options.api_key.clone())),
    }
}

/// POST fetch：远端模型发现（仅 openai-compatible 的 `/v1/models` 语义）。
///
/// 使用请求 body 里的 `{type, baseUrl, apiKey}`（页面当前编辑值）；未带 Key
/// 时回落服务端已保存 Key。在事件循环的 Tokio 运行时内直接 await（连接超时短），
/// 绝不在运行时内再建运行时。
async fn fetch_models(request: &Request, state: &ServiceState) -> Response {
    let body: Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    // providerId 或显式 {type, baseUrl, apiKey}。
    let (base_url, api_key, transport) = match body.get("providerId").and_then(Value::as_str) {
        Some(provider_id) => {
            let path = ai_path(state);
            let settings = match crate::config::ai::load_ai(&path) {
                Ok(Some(settings)) => settings,
                Ok(None) => return Response::error(422, "no saved configuration"),
                Err(error) => return Response::error(500, &error.to_string()),
            };
            let Some(provider) = settings.providers.get(provider_id) else {
                return Response::error(422, "provider not found");
            };
            let key = provider.options.api_key.clone().or_else(|| {
                body.get("apiKey")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
            (
                provider.options.base_url.clone(),
                key,
                provider.kind.as_str().to_string(),
            )
        }
        None => {
            let transport = body
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("openai-compatible")
                .to_string();
            let base_url = body
                .get("baseUrl")
                .and_then(Value::as_str)
                .ok_or("baseUrl is required")
                .map(str::to_string);
            let api_key = body
                .get("apiKey")
                .and_then(Value::as_str)
                .map(str::to_string);
            match (base_url, api_key) {
                (Ok(base_url), api_key) => (base_url, api_key, transport),
                (Err(message), _) => return Response::error(422, message),
            }
        }
    };
    if let Err(error) = crate::config::ai::validate_base_url(&base_url) {
        return Response::error(422, &error.to_string());
    }
    let Some(api_key) = api_key.filter(|key| !key.is_empty()) else {
        return Response::error(422, "api key is required to fetch models");
    };
    // 仅支持 OpenAI 兼容发现端点；其它协议明确禁用（不伪造列表）。
    if transport != "openai-compatible" && transport != "openai" {
        return Response::error(
            422,
            "model discovery is not supported for this provider type",
        );
    }
    // 直接 await 远端发现（连接/整体超时在 fetch 内设定）；事件循环是 Tokio 运行时，
    // 不能在运行时中再 `block_on`（会 panic）。
    match fetch_remote_models(&base_url, &api_key).await {
        Ok(models) => {
            let models_json: Vec<Value> = models
                .iter()
                .map(|(id, owned_by)| json!({"id": id, "ownedBy": owned_by}))
                .collect();
            Response::json(
                200,
                serde_json::to_string(&json!({"models": models_json})).unwrap_or_default(),
            )
        }
        Err(code) => Response::error(502, &code),
    }
}

/// 异步执行一次发现请求。
async fn fetch_remote_models(
    base_url: &str,
    api_key: &str,
) -> Result<Vec<(String, String)>, String> {
    let url = join_models_url(base_url);
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("client build failed: {error}"))?;
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "upstream timeout".to_string()
            } else {
                "upstream request failed".to_string()
            }
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("upstream http {}", status.as_u16()));
    }
    let text = response
        .text()
        .await
        .map_err(|_| "upstream read failed".to_string())?;
    if text.len() > MAX_FETCH_BODY_BYTES {
        return Err("upstream response too large".into());
    }
    let value: Value = serde_json::from_str(&text).map_err(|_| "upstream response is not json")?;
    let Some(data) = value.get("data").and_then(Value::as_array) else {
        return Err("upstream response missing data array".into());
    };
    // 去重 + 按 id 排序。
    let mut models: Vec<(String, String)> = data
        .iter()
        .filter_map(|item| {
            let id = item.get("id").and_then(Value::as_str)?;
            let owned_by = item
                .get("owned_by")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some((id.to_string(), owned_by.to_string()))
        })
        .collect();
    models.sort();
    models.dedup_by(|left, right| left.0 == right.0);
    Ok(models)
}

/// 发现响应体上限。
const MAX_FETCH_BODY_BYTES: usize = 512 * 1024;

/// baseURL + `/models`（兼容重复 `/v1`）。
fn join_models_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = "models";
    if let Some(stripped) = path.strip_prefix("v1/")
        && base.ends_with("/v1")
    {
        return format!("{base}/{stripped}");
    }
    format!("{base}/{path}")
}

#[cfg(test)]
mod tests;

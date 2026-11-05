//! 云端 `models.dev` 模型目录同步与已连接供应商的实时模型刷新。
//!
//! 对齐 `opencode-dev/packages/core/src/models-dev.ts`：优先拉取
//! `models.opencode.ai/experimental/v2.0/api.json`，失败时回退 `models.dev/api.json`；
//! 响应带 `ETag` 时缓存到 `config/models_catalog_cache.json`，同时把模型合入进程内
//! 预设目录（[`super::presets::update_catalog_models`]），使后续物化立即使用最新模型。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::connect::{
    connect_path_for_ai_path, load_connect, materialize_preset_provider, save_connect,
};
use super::presets::{find_preset, update_catalog_models};
use super::verify::{fetch_provider_models, merge_discovered_models_for_preset};
use super::{AiConfigError, ModelConfig, ModelLimit, ProviderTransport, user_data_root};

/// 主目录地址（experimental v2，仅请求聊天/决策/图像模型）。
pub const PRIMARY_CATALOG_URL: &str =
    "https://models.opencode.ai/experimental/v2.0/api.json?type=chat,decision,image";

/// 回退目录地址（models.dev v1 全量目录）。
pub const FALLBACK_CATALOG_URL: &str = "https://models.dev/api.json";

/// 目录响应体上限。
const MAX_CATALOG_BYTES: usize = 32 * 1024 * 1024;

/// 云端目录缓存路径：`<user_data_root>/config/models_catalog_cache.json`。
pub fn catalog_cache_path() -> Result<PathBuf, AiConfigError> {
    Ok(user_data_root()?
        .join("config")
        .join("models_catalog_cache.json"))
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogCache {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, BTreeMap<String, ModelConfig>>,
}

/// 启动时读取磁盘缓存（缺失/损坏返回空表）。
#[cfg(not(test))]
pub(crate) fn cached_catalog_models() -> BTreeMap<String, BTreeMap<String, ModelConfig>> {
    load_cache().map(|cache| cache.models).unwrap_or_default()
}

fn load_cache() -> Option<CatalogCache> {
    let path = match catalog_cache_path() {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(error = %error, "model catalog cache path is unavailable");
            return None;
        }
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
        Err(error) => {
            tracing::warn!(path = %path.display(), error = %error, "model catalog cache is unreadable");
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(cache) => Some(cache),
        Err(error) => {
            tracing::warn!(path = %path.display(), error = %error, "model catalog cache is malformed");
            None
        }
    }
}

fn save_cache(cache: &CatalogCache) -> Result<(), AiConfigError> {
    let body = serde_json::to_string_pretty(cache)
        .map_err(|error| AiConfigError::Invalid(format!("serialize failed: {error}")))?;
    super::save_private_json(&catalog_cache_path()?, &format!("{body}\n"))
}

/// 解析 `models.dev` v2 / v1 JSON 为 `provider_id -> model_id -> ModelConfig`。
///
/// 过滤 `status == "deprecated"` 的模型；提取 `limit.context` / `limit.output`、
/// `modalities`，并在 `capabilities.reasoning`（或 v1 `reasoning`）为 true/对象时
/// 生成 low/medium/high 思考等级。
pub fn parse_models_dev_catalog(
    json_text: &str,
) -> Result<BTreeMap<String, BTreeMap<String, ModelConfig>>, String> {
    let root: Value = serde_json::from_str(json_text)
        .map_err(|error| format!("invalid catalog json: {error}"))?;
    let providers = root
        .as_object()
        .ok_or_else(|| "catalog must be a JSON object".to_string())?;
    let mut out = BTreeMap::new();
    for (provider_key, provider_value) in providers {
        let Some(provider) = provider_value.as_object() else {
            continue;
        };
        let provider_id = provider
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .unwrap_or(provider_key);
        let Some(models_value) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        let mut models = BTreeMap::new();
        for (model_key, model_value) in models_value {
            let Some(model) = model_value.as_object() else {
                continue;
            };
            if model.get("status").and_then(Value::as_str) == Some("deprecated") {
                continue;
            }
            if model
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind != "chat")
            {
                continue;
            }
            let id = model
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .unwrap_or(model_key);
            let name = model
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .unwrap_or(id)
                .to_string();
            let transport = model.get("api").and_then(Value::as_object).and_then(|api| {
                api.contains_key("responses")
                    .then_some(super::ModelTransport::OpenaiResponses)
            });
            let variants = reasoning_variants(provider_id, id, transport, model);
            let default_variant = variants.contains_key("high").then(|| "high".to_string());
            models.insert(
                id.to_string(),
                ModelConfig {
                    id: id.to_string(),
                    name,
                    transport,
                    limit: parse_limit(model.get("limit")),
                    modalities: parse_modalities(model.get("modalities")),
                    variants,
                    variant: default_variant,
                    pricing: None,
                    extra: serde_json::Map::new(),
                },
            );
        }
        out.insert(provider_id.to_string(), models);
    }
    Ok(out)
}

fn parse_limit(value: Option<&Value>) -> Option<ModelLimit> {
    let limit = value?.as_object()?;
    let context = limit.get("context").and_then(Value::as_u64);
    let output = limit.get("output").and_then(Value::as_u64);
    if context.is_none() && output.is_none() {
        return None;
    }
    Some(ModelLimit {
        context: context.unwrap_or(0),
        output: output.unwrap_or(0),
    })
}

fn parse_modalities(value: Option<&Value>) -> Option<Value> {
    let modalities = value?.as_object()?;
    let input = modalities.get("input").and_then(Value::as_array);
    let output = modalities.get("output").and_then(Value::as_array);
    if input.is_none() && output.is_none() {
        return None;
    }
    Some(json_object(input, output))
}

fn json_object(input: Option<&Vec<Value>>, output: Option<&Vec<Value>>) -> Value {
    let mut object = serde_json::Map::new();
    if let Some(input) = input {
        object.insert("input".to_string(), Value::Array(input.clone()));
    }
    if let Some(output) = output {
        object.insert("output".to_string(), Value::Array(output.clone()));
    }
    Value::Object(object)
}

fn reasoning_variants(
    provider_id: &str,
    model_id: &str,
    transport: Option<super::ModelTransport>,
    model: &serde_json::Map<String, Value>,
) -> BTreeMap<String, Value> {
    let reasoning = model
        .get("capabilities")
        .and_then(Value::as_object)
        .and_then(|capabilities| capabilities.get("reasoning"))
        .or_else(|| model.get("reasoning"));
    if !reasoning_enabled(reasoning) {
        return BTreeMap::new();
    }
    super::verify::protocol_reasoning_variants(provider_id, model_id, transport)
}

fn reasoning_enabled(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(enabled)) => *enabled,
        Some(Value::Object(object)) => object
            .get("supported")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        _ => false,
    }
}

enum FetchedCatalog {
    NotModified,
    Fetched {
        models: BTreeMap<String, BTreeMap<String, ModelConfig>>,
        etag: Option<String>,
    },
}

/// 拉取云端目录并合入进程内目录；返回本次目录中的模型总数。
pub async fn refresh_models_catalog() -> Result<usize, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("http client init failed: {error}"))?;
    let cache = load_cache();
    let mut last_error = None;
    for url in [PRIMARY_CATALOG_URL, FALLBACK_CATALOG_URL] {
        match fetch_catalog(&client, url, cache.as_ref()).await {
            Ok(FetchedCatalog::NotModified) => {
                let Some(cache) = cache.as_ref() else {
                    last_error = Some("catalog returned 304 without a cache".to_string());
                    continue;
                };
                let count = count_models(&cache.models);
                if !cache.models.is_empty() {
                    update_catalog_models(cache.models.clone());
                }
                return Ok(count);
            }
            Ok(FetchedCatalog::Fetched { models, etag }) => {
                let count = count_models(&models);
                let cache = CatalogCache {
                    source: Some(url.to_string()),
                    etag,
                    models: models.clone(),
                };
                if let Err(error) = save_cache(&cache) {
                    tracing::warn!(error = %error, "failed to persist model catalog cache");
                }
                update_catalog_models(models);
                return Ok(count);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "model catalog refresh failed".to_string()))
}

async fn fetch_catalog(
    client: &reqwest::Client,
    url: &str,
    cache: Option<&CatalogCache>,
) -> Result<FetchedCatalog, String> {
    let mut request = client.get(url).header("Accept", "application/json");
    if let Some(cache) = cache
        && cache.source.as_deref() == Some(url)
        && let Some(etag) = cache.etag.as_deref()
    {
        request = request.header("If-None-Match", etag);
    }
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "catalog request timed out".to_string()
        } else {
            "catalog request failed".to_string()
        }
    })?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(FetchedCatalog::NotModified);
    }
    let status = response.status();
    if !status.is_success() {
        return Err(format!("catalog fetch failed (HTTP {})", status.as_u16()));
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let text = response
        .text()
        .await
        .map_err(|_| "catalog response could not be read".to_string())?;
    if text.len() > MAX_CATALOG_BYTES {
        return Err("catalog response too large".to_string());
    }
    let models = parse_models_dev_catalog(&text)?;
    if count_models(&models) == 0 {
        return Err("catalog response contained no models".to_string());
    }
    Ok(FetchedCatalog::Fetched { models, etag })
}

fn count_models(models: &BTreeMap<String, BTreeMap<String, ModelConfig>>) -> usize {
    models.values().map(BTreeMap::len).sum()
}

struct RefreshTarget {
    id: String,
    name: String,
    transport: ProviderTransport,
    base_url: String,
    api_key: String,
    custom: bool,
}

/// 刷新已连接供应商的模型列表：先尽力同步云端目录，再逐个调用 `/models`。
///
/// `provider_id` 为 `None` 时刷新 `connect.json` 中全部已连接预设；指定 id 时支持
/// 预设或 `ai.json` 自定义供应商。单个供应商失败不会阻断其它供应商；全部失败
/// 且没有任何模型更新时返回错误。
pub async fn refresh_connected_provider_models(
    ai_path: &Path,
    provider_id: Option<&str>,
) -> Result<usize, String> {
    #[cfg(not(test))]
    if let Err(error) = refresh_models_catalog().await {
        tracing::warn!(error = %error, "cloud model catalog refresh skipped");
    }
    let connect_file = connect_path_for_ai_path(ai_path);
    let connect = load_connect(&connect_file)
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let mut custom = super::load_ai(ai_path)
        .map_err(|error| error.to_string())?
        .unwrap_or_default();
    let targets = collect_refresh_targets(&connect, &custom, provider_id)?;
    let mut total = 0;
    let mut custom_changed = false;
    let mut refreshed_models: BTreeMap<String, BTreeMap<String, ModelConfig>> = BTreeMap::new();
    let mut last_error = None;
    for target in targets {
        match fetch_provider_models(target.transport, &target.base_url, &target.api_key).await {
            Ok(discovered) => {
                if target.custom {
                    let existing = custom
                        .providers
                        .get(&target.id)
                        .map(|provider| provider.models.clone())
                        .unwrap_or_default();
                    let merged = merge_discovered_models_for_preset(
                        &target.id,
                        Some(&existing),
                        &discovered,
                    );
                    total += merged.len();
                    if let Some(provider) = custom.providers.get_mut(&target.id)
                        && provider.models != merged
                    {
                        provider.models = merged;
                        custom_changed = true;
                    }
                } else {
                    let existing = connect
                        .providers
                        .get(&target.id)
                        .and_then(|record| record.models.clone());
                    let merged = merge_discovered_models_for_preset(
                        &target.id,
                        existing.as_ref(),
                        &discovered,
                    );
                    total += merged.len();
                    refreshed_models.insert(target.id.clone(), merged);
                }
            }
            Err(error) => {
                last_error = Some(format!("Provider '{}': {error}", target.name));
            }
        }
    }
    if custom_changed {
        super::save_ai(ai_path, &custom).map_err(|error| error.to_string())?;
    }
    // 网络探测期间 connect.json 可能已被其它入口改写：重新加载后再合并写入。
    if !refreshed_models.is_empty() {
        let mut latest = load_connect(&connect_file)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        let mut connect_changed = false;
        for (provider_id, models) in refreshed_models {
            if let Some(record) = latest.providers.get_mut(&provider_id)
                && record.models.as_ref() != Some(&models)
            {
                record.models = Some(models);
                connect_changed = true;
            }
        }
        if connect_changed {
            save_connect(&connect_file, &latest).map_err(|error| error.to_string())?;
        }
    }
    if total == 0
        && let Some(error) = last_error
    {
        return Err(error);
    }
    Ok(total)
}

fn collect_refresh_targets(
    connect: &super::connect::ConnectSettings,
    custom: &super::AiSettings,
    provider_id: Option<&str>,
) -> Result<Vec<RefreshTarget>, String> {
    match provider_id {
        Some(id) => {
            if let Some(spec) = find_preset(id) {
                let record = connect.providers.get(id);
                let Some((provider, _enabled)) = materialize_preset_provider(spec, record) else {
                    return Err(format!("provider '{id}' is not connected"));
                };
                let Some(api_key) = provider
                    .options
                    .api_key
                    .filter(|key| !key.trim().is_empty())
                else {
                    return Err(format!("provider '{id}' has no credentials"));
                };
                return Ok(vec![RefreshTarget {
                    id: id.to_string(),
                    name: spec.name.to_string(),
                    transport: spec.transport,
                    base_url: provider.options.base_url,
                    api_key,
                    custom: false,
                }]);
            }
            let Some(provider) = custom.providers.get(id) else {
                return Err(format!("unknown provider '{id}'"));
            };
            let Some(api_key) = provider
                .options
                .api_key
                .clone()
                .filter(|key| !key.trim().is_empty())
            else {
                return Err(format!("provider '{id}' has no credentials"));
            };
            Ok(vec![RefreshTarget {
                id: id.to_string(),
                name: provider.name.clone(),
                transport: provider.kind,
                base_url: provider.options.base_url.clone(),
                api_key,
                custom: true,
            }])
        }
        None => {
            let mut targets = Vec::new();
            for (id, record) in &connect.providers {
                let Some(spec) = find_preset(id) else {
                    continue;
                };
                let Some((provider, enabled)) = materialize_preset_provider(spec, Some(record))
                else {
                    continue;
                };
                if !enabled {
                    continue;
                }
                let Some(api_key) = provider
                    .options
                    .api_key
                    .filter(|key| !key.trim().is_empty())
                else {
                    continue;
                };
                targets.push(RefreshTarget {
                    id: id.clone(),
                    name: spec.name.to_string(),
                    transport: spec.transport,
                    base_url: provider.options.base_url,
                    api_key,
                    custom: false,
                });
            }
            Ok(targets)
        }
    }
}

#[cfg(test)]
#[path = "catalog_refresh_tests.rs"]
mod catalog_refresh_tests;

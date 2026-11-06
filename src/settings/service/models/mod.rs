//! 模型设置 API：GET/PUT `/api/models/settings` 与 POST `/api/models/fetch`。
//!
//! - GET 回传完整 provider/model 配置（含明文 `apiKey`），供设置页直接编辑；
//!   页面仍按同源 + capability token 访问。
//! - PUT 的 `apiKey` 三态：字段缺失保持、空字符串清除、非空替换；
//!   携带 `revision` 防止两个设置页互相覆盖；校验失败 422 且文件不改。
//! - fetch 用当前编辑中的 base URL/key 做一次性远端发现，后台异步执行，
//!   不堵本地连接；响应不含 Key。

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

use super::handlers::{Request, Response};
use super::{ServiceState, SettingsEvent, ai_path, connect_path};

pub(super) mod connect;

/// `POST /api/models/fetch` 请求体。
#[derive(Debug, Default, Deserialize)]
struct FetchBody {
    #[serde(default, rename = "providerId")]
    provider_id: Option<String>,
    #[serde(default, rename = "type")]
    provider_type: Option<String>,
    #[serde(default, rename = "baseUrl")]
    base_url: Option<String>,
    #[serde(default, rename = "apiKey")]
    api_key: Option<String>,
    #[serde(default, rename = "formAnswers")]
    form_answers: BTreeMap<String, String>,
}

/// `/api/models` 系列入口。
pub async fn route(request: &Request, state: &ServiceState) -> Response {
    if request.path.starts_with("/api/models/connect") {
        return connect::route(request, state).await;
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/models/settings") => get_settings(state),
        ("PUT", "/api/models/settings") => put_settings(request, state).await,
        ("POST", "/api/models/fetch") => fetch_models(request, state).await,
        _ => Response::error(404, "not found"),
    }
}

/// 计算当前配置的 revision（内容哈希前 16 hex）。
///
/// 只覆盖模型相关字段：`webSearch` 等非模型配置不参与乐观锁，否则在 General
/// 页保存 Key 后，已打开的 Models 页会拿着旧 revision 误报 409。
pub(super) fn revision_of(settings: &crate::config::ai::AiSettings) -> String {
    revision_hash(&revision_value(settings, None).to_string())
}

/// 同时纳入 `connect.json` 内容的 revision。
///
/// GET 使用该口径；PUT 同时接受本口径与 legacy `revision_of`，因为
/// `PUT /api/models/settings` 不写 `connect.json`，快捷连接先保存后不应让
/// 已打开的 Models 页误报 409。
pub(super) fn revision_of_with_connect(
    settings: &crate::config::ai::AiSettings,
    connect: Option<&crate::config::ai::connect::ConnectSettings>,
) -> String {
    revision_hash(&revision_value(settings, connect).to_string())
}

fn revision_value(
    settings: &crate::config::ai::AiSettings,
    connect: Option<&crate::config::ai::connect::ConnectSettings>,
) -> Value {
    let connect_text = connect
        .and_then(|connect| serde_json::to_string(connect).ok())
        .unwrap_or_default();
    let mut value = json!({
        "defaultModel": &settings.default_model,
        "titleSummary": &settings.title_summary,
        "titleLanguage": settings.effective_title_language().as_str(),
        "enabledProviders": &settings.enabled_providers,
        "providers": &settings.providers,
        "streamIdleTimeoutMs": settings.stream_idle_timeout_ms,
    });
    if connect.is_some() {
        value["connect"] = Value::String(connect_text);
    }
    value
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
    let connect = match crate::config::ai::connect::load_connect(&connect_path(state)) {
        Ok(Some(connect)) => connect,
        Ok(None) => crate::config::ai::connect::ConnectSettings::default(),
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
                "apiKey": provider.options.api_key.as_deref().unwrap_or(""),
                "enabled": settings.enabled_providers.contains(key),
                "models": provider.models.iter().map(|(model_key, model)| {
                    model_value(model_key, model)
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
    let title_summary = settings
        .title_summary
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
        "quickConnect": connect::quick_connect_json(&connect),
        "enabledProviders": settings.enabled_providers,
        "defaultModel": default_model,
        "titleSummary": title_summary,
        "titleLanguage": settings.effective_title_language().as_str(),
        "streamIdleTimeoutMs": settings.idle_timeout_ms(),
        "revision": revision_of_with_connect(&settings, Some(&connect)),
    });
    Response::json(200, serde_json::to_string(&body).unwrap_or_default())
}

/// 单个模型的 API 形状（自定义供应商与快捷连接共用）。
pub(super) fn model_value(model_key: &str, model: &crate::config::ai::ModelConfig) -> Value {
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
}

/// PUT：完整验证后原子保存；成功投递 ModelsChanged。
async fn put_settings(request: &Request, state: &ServiceState) -> Response {
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
    let connect = match crate::config::ai::connect::load_connect(&connect_path(state)) {
        Ok(Some(connect)) => connect,
        Ok(None) => crate::config::ai::connect::ConnectSettings::default(),
        Err(error) => return Response::error(500, &error.to_string()),
    };
    // 同时接受含 `connect.json` 的新口径与 legacy 模型字段口径：Models 页
    // 不写 connect.json，先保存快捷连接后不应让这里误报 409。
    let current_revision = revision_of_with_connect(&current, Some(&connect));
    let current_legacy_revision = revision_of(&current);
    let provided = body.get("revision").and_then(Value::as_str).unwrap_or("");
    if !provided.is_empty() && provided != current_revision && provided != current_legacy_revision {
        return Response::error(409, "configuration changed elsewhere; reload and retry");
    }
    let merged = match merge_settings(Some(current), &body, Some(&connect)) {
        Ok(settings) => settings,
        Err(message) => return Response::error(422, &message),
    };
    if let Err(error) = crate::config::ai::save_ai(&path, &merged) {
        return Response::error(500, &error.to_string());
    }
    // TUI 需要完整运行时快照：自定义供应商 + 已连接的快捷连接供应商。
    let runtime = crate::config::ai::connect::merge_runtime_ai_settings(&merged, &connect);
    let _ = state
        .sender()
        .send(SettingsEvent::ModelsChanged(Box::new(runtime)));
    let body = json!({
        "ok": true,
        "revision": revision_of_with_connect(&merged, Some(&connect)),
    });
    Response::json(200, serde_json::to_string(&body).unwrap_or_default())
}

/// PUT body 到磁盘配置的合并：provider 键为稳定 id；
/// `apiKey` 三态（字段缺失 = Keep、空字符串 = Clear、非空 = Replace）。
fn merge_settings(
    current: Option<crate::config::ai::AiSettings>,
    body: &Value,
    connect: Option<&crate::config::ai::connect::ConnectSettings>,
) -> Result<crate::config::ai::AiSettings, String> {
    use crate::config::ai::{ProviderConfig, ProviderOptions, ProviderTransport};
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
        let models = match item.get("models").and_then(Value::as_array) {
            Some(models_value) => {
                parse_models(models_value, existing.map(|provider| &provider.models))?
            }
            None => std::collections::BTreeMap::new(),
        };
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
    // 默认模型、标题模型与标题语言选择。
    settings.default_model = parse_model_selection_field(body, "defaultModel")?;
    settings.title_summary = parse_model_selection_field(body, "titleSummary")?;
    settings.title_language = match body.get("titleLanguage") {
        Some(Value::Null) | None => None,
        Some(Value::String(raw)) if raw.trim().is_empty() => None,
        Some(Value::String(raw)) => Some(
            crate::config::ai::TitleLanguage::parse(raw)
                .ok_or_else(|| format!("unsupported titleLanguage '{raw}'"))?,
        ),
        Some(_) => return Err("titleLanguage must be a string".into()),
    };
    if let Some(timeout) = body.get("streamIdleTimeoutMs").and_then(Value::as_u64) {
        settings.stream_idle_timeout_ms = Some(timeout);
    }
    // 校验必须使用同目录的 `connect.json`：默认模型/标题模型允许指向已连接的
    // 快捷连接供应商（`save_ai` 自身也会按磁盘上的 connect.json 再校验一次）。
    settings
        .validate_with_connect(connect)
        .map_err(|error| error.to_string())?;
    Ok(settings)
}

/// 解析请求里的模型数组为存储表。
///
/// 记录键以模型 id 为准（lx-agent 语义：`selection.model` 就是发给供应商的
/// 模型 id）；模态三态（缺失保留、显式 null 清除、对象覆盖）与未知字段往返
/// 保留都以 `existing` 为基准。
pub(super) fn parse_models(
    models_value: &[Value],
    existing: Option<&std::collections::BTreeMap<String, crate::config::ai::ModelConfig>>,
) -> Result<std::collections::BTreeMap<String, crate::config::ai::ModelConfig>, String> {
    use crate::config::ai::{ModelConfig, ModelLimit, ModelTransport};
    let mut models = std::collections::BTreeMap::new();
    for model_item in models_value {
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
        if let Some(variants_value) = model_item.get("variants").and_then(Value::as_object) {
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
        let previous_model = existing.and_then(|models| models.get(&model_id));
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
    Ok(models)
}

fn parse_model_selection_field(
    body: &Value,
    field_name: &str,
) -> Result<Option<crate::config::ai::ModelSelection>, String> {
    use crate::config::ai::ModelSelection;
    match body.get(field_name) {
        Some(Value::Null) | None => Ok(None),
        Some(selection) => {
            let provider = selection
                .get("provider")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{field_name}.provider is required"))?
                .to_string();
            let model = selection
                .get("model")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{field_name}.model is required"))?
                .to_string();
            let variant = selection
                .get("variant")
                .and_then(Value::as_str)
                .filter(|variant| !variant.is_empty())
                .map(str::to_string);
            Ok(Some(ModelSelection {
                provider,
                model,
                variant,
            }))
        }
    }
}

/// Key 三态合并：字段缺失保持旧 Key，空字符串清除，非空替换。
fn merge_api_key(
    existing: Option<&crate::config::ai::ProviderConfig>,
    item: &Value,
) -> Result<Option<String>, String> {
    match item.get("apiKey").and_then(Value::as_str) {
        // 空字符串：显式清除已存 Key。
        Some("") => Ok(None),
        Some(new_key) => Ok(Some(new_key.to_string())),
        // 字段缺失：保持旧 Key（兼容 curl 与旧调用方）。
        None => Ok(existing.and_then(|provider| provider.options.api_key.clone())),
    }
}

/// POST fetch：远端模型发现（仅 openai-compatible 的 `/v1/models` 语义）。
///
/// 使用请求 body 里的 `{type, baseUrl, apiKey, formAnswers}`（页面当前编辑值）；
/// 未带 Key 时回落服务端已保存 Key，未保存的预设按表单答案展开地址。在事件循环
/// 的 Tokio 运行时内直接 await（连接超时短），绝不在运行时内再建运行时。
async fn fetch_models(request: &Request, state: &ServiceState) -> Response {
    let body: FetchBody = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    // 短效 OAuth Token 在远端发现前先刷新，避免过期凭证直接 401。
    if crate::config::ai::oauth::ensure_connect_oauth_fresh(&ai_path(state)).await {
        match crate::config::ai::connect::load_runtime_ai(&ai_path(state)) {
            Ok(Some(runtime)) => {
                let _ = state
                    .sender()
                    .send(SettingsEvent::ModelsChanged(Box::new(runtime)));
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(error = %error, "failed to reload runtime settings after oauth refresh");
            }
        }
    }
    // providerId 或显式 {type, baseUrl, apiKey}。
    let (base_url, api_key, transport) = match body.provider_id.as_deref() {
        Some(provider_id) => {
            let path = ai_path(state);
            let settings = match crate::config::ai::load_ai(&path) {
                Ok(Some(settings)) => settings,
                Ok(None) => crate::config::ai::AiSettings::empty(),
                Err(error) => return Response::error(500, &error.to_string()),
            };
            let (saved_base_url, saved_key, transport) =
                if let Some(provider) = settings.providers.get(provider_id) {
                    (
                        provider.options.base_url.clone(),
                        provider.options.api_key.clone(),
                        provider.kind.as_str().to_string(),
                    )
                } else {
                    // 快捷连接供应商：已保存时按 `connect.json` 物化；尚未保存
                    // （Quick Connect 首次 Fetch）时按预设目录 + 表单答案展开，
                    // 不能因为 connect.json 里还没有记录就报 provider not found。
                    let Some(spec) = crate::config::ai::presets::find_preset(provider_id) else {
                        return Response::error(422, "provider not found");
                    };
                    let connect =
                        crate::config::ai::connect::load_connect_or_default(&connect_path(state))
                            .unwrap_or_default();
                    let record = connect.providers.get(provider_id);
                    match crate::config::ai::connect::materialize_preset_provider(spec, record) {
                        Some((provider, _enabled)) => (
                            provider.options.base_url,
                            provider.options.api_key,
                            provider.kind.as_str().to_string(),
                        ),
                        None => {
                            let mut answers = record
                                .map(|record| record.form_answers.clone())
                                .unwrap_or_default();
                            for (key, value) in &body.form_answers {
                                let value = value.trim();
                                if !value.is_empty() {
                                    answers.insert(key.clone(), value.to_string());
                                }
                            }
                            let base_url = crate::config::ai::presets::resolve_preset_base_url(
                                spec,
                                &answers,
                                body.base_url.as_deref(),
                            );
                            let env_key = spec.env_vars.iter().find_map(|name| {
                                std::env::var(name)
                                    .ok()
                                    .map(|value| value.trim().to_string())
                                    .filter(|value| !value.is_empty())
                            });
                            (base_url, env_key, spec.transport.as_str().to_string())
                        }
                    }
                };
            // 页面草稿值优先于已保存值（非空才覆盖；未展开模板交回物化结果）。
            let base_url = body
                .base_url
                .as_deref()
                .map(str::trim)
                .filter(|url| !url.is_empty() && !url.contains("${"))
                .map(str::to_string)
                .unwrap_or(saved_base_url);
            let key = body
                .api_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_string)
                .or_else(|| saved_key.filter(|key| !key.trim().is_empty()));
            (base_url, key, transport)
        }
        None => {
            let transport = body
                .provider_type
                .as_deref()
                .unwrap_or("openai-compatible")
                .to_string();
            let Some(base_url) = body.base_url.as_deref().map(str::to_string) else {
                return Response::error(422, "baseUrl is required");
            };
            let api_key = body.api_key;
            (base_url, api_key, transport)
        }
    };
    if let Err(error) = crate::config::ai::validate_base_url(&base_url) {
        return Response::error(422, &error.to_string());
    }
    let Some(api_key) = api_key.filter(|key| !key.is_empty()) else {
        return Response::error(422, "api key is required to fetch models");
    };
    // 四种协议均支持远端发现：openai / openai-compatible / anthropic / google。
    let Some(transport) = crate::config::ai::ProviderTransport::parse(&transport) else {
        return Response::error(
            422,
            "model discovery is not supported for this provider type",
        );
    };
    // 直接 await 远端发现（连接/整体超时在 fetch 内设定）；事件循环是 Tokio 运行时，
    // 不能在运行时中再 `block_on`（会 panic）。
    match crate::config::ai::verify::fetch_provider_models(transport, &base_url, &api_key).await {
        Ok(models) => {
            let models_json: Vec<Value> = models
                .iter()
                .map(|model| json!({"id": model.id, "name": model.name, "ownedBy": ""}))
                .collect();
            Response::json(
                200,
                serde_json::to_string(&json!({"models": models_json})).unwrap_or_default(),
            )
        }
        Err(message) => Response::error(502, &message),
    }
}

/// 异步执行一次发现请求。
pub(crate) async fn fetch_remote_models(
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

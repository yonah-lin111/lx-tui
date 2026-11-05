//! Quick Connect 设置 API：`connect.json` 保存/断开与 OAuth 授权会话。
//!
//! 快捷连接的凭证与覆盖项独立存 `connect.json`，`ai.json` 只保留自定义供应商。
//! 任何一次连接状态变化都会先落盘、再清理 `ai.json` 中悬空的
//! `defaultModel` / `titleSummary`，最后广播合并后的运行时快照给 TUI。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::super::handlers::{Request, Response};
use super::super::{ServiceState, SettingsEvent, ai_path, connect_path};
use super::{model_value, parse_models, revision_of_with_connect};
use crate::config::ai::AiSettings;
use crate::config::ai::connect::{
    ConnectSettings, ConnectedPresetRecord, ConnectionSource, OAuthCredential, load_connect,
    materialize_preset_provider, merge_runtime_ai_settings, resolve_preset_connection_source,
    save_connect,
};
use crate::config::ai::oauth::{OAuthTaskHandle, start_oauth_flow};
use crate::config::ai::presets::{
    PresetAuthMethod, PresetProviderSpec, all_presets, default_models_for_preset, find_preset,
    resolve_preset_base_url,
};

/// OAuth 尝试状态（设置页轮询）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectOAuthStatus {
    Pending,
    Complete,
    Failed,
}

impl ConnectOAuthStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Complete => "complete",
            Self::Failed => "failed",
        }
    }
}

/// 进行中的 Quick Connect OAuth 尝试：取消/提交授权码句柄 + 轮询状态。
pub(crate) struct PendingConnectOAuth {
    status: ConnectOAuthStatus,
    error: Option<String>,
    handle: OAuthTaskHandle,
}

/// 设置服务共享的进行中 OAuth 表（后台任务与请求处理器都能持有）。
pub(crate) type PendingConnectOAuthMap =
    Arc<tokio::sync::Mutex<std::collections::HashMap<String, PendingConnectOAuth>>>;

/// `/api/models/connect*` 入口。
pub(super) async fn route(request: &Request, state: &ServiceState) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("PUT", "/api/models/connect") => put_connect(request, state).await,
        ("POST", "/api/models/connect/disconnect") => disconnect(request, state),
        ("POST", "/api/models/connect/oauth/start") => oauth_start(request, state).await,
        ("GET", "/api/models/connect/oauth/status")
        | ("POST", "/api/models/connect/oauth/status") => oauth_status(request, state).await,
        ("POST", "/api/models/connect/oauth/complete") => oauth_complete(request, state).await,
        ("DELETE", "/api/models/connect/oauth") => oauth_cancel(request, state).await,
        _ => Response::error(404, "not found"),
    }
}

/// `GET /api/models/settings` 的 `quickConnect` 数组：全部预设 + 当前连接状态。
pub(super) fn quick_connect_json(connect: &ConnectSettings) -> Value {
    let items: Vec<Value> = all_presets()
        .iter()
        .map(|spec| preset_json(spec, connect))
        .collect();
    Value::Array(items)
}

fn preset_json(spec: &PresetProviderSpec, connect: &ConnectSettings) -> Value {
    let record = connect.providers.get(spec.id);
    let source = resolve_preset_connection_source(spec, record);
    let connected = source.is_some();
    let enabled = record.map(|record| record.enabled).unwrap_or(connected);
    let form_answers = record
        .map(|record| record.form_answers.clone())
        .unwrap_or_default();
    // 已连接的预设优先展示运行时物化后的 Base URL（含 OAuth 动态端点与
    // 环境变量展开）；未连接时按表单答案展开模板。
    let base_url = materialize_preset_provider(spec, record)
        .map(|(provider, _enabled)| provider.options.base_url)
        .or_else(|| record.and_then(|record| record.base_url.clone()))
        .unwrap_or_else(|| resolve_preset_base_url(spec, &form_answers, None));
    let api_key = record
        .and_then(|record| record.api_key.as_deref())
        .unwrap_or("");
    let models = record
        .and_then(|record| record.models.clone())
        .filter(|models| !models.is_empty())
        .unwrap_or_else(|| default_models_for_preset(spec.id));
    let default_models = default_models_for_preset(spec.id);
    json!({
        "id": spec.id,
        "name": spec.name,
        "type": spec.transport.as_str(),
        "note": spec.note,
        "defaultBaseUrl": spec.default_base_url,
        "baseUrl": base_url,
        "enabled": enabled,
        "connected": connected,
        "connectionType": connection_type(source.as_ref()),
        "connectionLabel": connection_label(source.as_ref()),
        "apiKey": api_key,
        "formAnswers": form_answers,
        "envVars": spec.env_vars,
        "methods": spec.methods.iter().map(method_json).collect::<Vec<_>>(),
        "models": models
            .iter()
            .map(|(key, model)| model_value(key, model))
            .collect::<Vec<_>>(),
        "defaultModels": default_models
            .iter()
            .map(|(key, model)| model_value(key, model))
            .collect::<Vec<_>>(),
    })
}

fn method_json(method: &PresetAuthMethod) -> Value {
    json!({
        "id": method.id(),
        "label": method.label(),
        "type": if method.oauth_flow().is_some() { "oauth" } else { "apiKey" },
        "form": method.form().iter().map(|field| json!({
            "key": field.key,
            "label": field.label,
            "placeholder": field.placeholder,
            "required": field.required,
            "whenEq": field.when_eq.map(|(key, value)| json!([key, value])),
            "options": field.options.iter().map(|option| json!({
                "label": option.label,
                "value": option.value,
                "description": option.description,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn connection_type(source: Option<&ConnectionSource>) -> Value {
    match source {
        Some(ConnectionSource::ApiKey) => json!("apiKey"),
        Some(ConnectionSource::OAuth { .. }) => json!("oauth"),
        Some(ConnectionSource::Env { .. }) => json!("env"),
        None => Value::Null,
    }
}

fn connection_label(source: Option<&ConnectionSource>) -> String {
    match source {
        Some(ConnectionSource::ApiKey) => "Connected · API key".to_string(),
        Some(ConnectionSource::OAuth { .. }) => "Connected · OAuth".to_string(),
        Some(ConnectionSource::Env { var_name }) => format!("Connected · ${var_name}"),
        None => "Not connected".to_string(),
    }
}

/// 计算预设最终生效的 Base URL（OAuth 动态端点 / 表单展开 / 覆盖值）。
///
/// `allow_override` 为 false 时忽略 `record.base_url`，用于判断页面回传的
/// 地址是否只是预设物化结果；任何未展开的 `${...}` 模板都视为无效。
fn preset_resolved_base_url(
    spec: &PresetProviderSpec,
    record: &ConnectedPresetRecord,
    allow_override: bool,
) -> Option<String> {
    let oauth_endpoint = record
        .oauth
        .as_ref()
        .and_then(|oauth| oauth.metadata.get("apiEndpoint"))
        .map(|endpoint| endpoint.trim().trim_end_matches('/').to_string())
        .filter(|endpoint| !endpoint.is_empty());
    let resolved = match oauth_endpoint {
        Some(endpoint) => endpoint,
        None => {
            let instance = record
                .oauth
                .as_ref()
                .and_then(|oauth| oauth.metadata.get("instanceUrl"))
                .map(String::as_str)
                .filter(|instance| spec.id == "gitlab" && !instance.trim().is_empty());
            let method_override = record
                .oauth
                .as_ref()
                .and_then(|oauth| spec.method(&oauth.method_id))
                .and_then(PresetAuthMethod::base_url_override);
            let override_url = if allow_override {
                record.base_url.as_deref().or(method_override)
            } else {
                method_override
            };
            match instance {
                Some(instance) => {
                    resolve_preset_base_url(spec, &record.form_answers, Some(instance))
                }
                None => resolve_preset_base_url(spec, &record.form_answers, override_url),
            }
        }
    };
    if resolved.contains("${") {
        return None;
    }
    Some(resolved)
}

/// `PUT /api/models/connect`：保存一个或多个快捷连接项（对齐 opencode-dev，仅校验结构与表单，不做阻塞探测）。
async fn put_connect(request: &Request, state: &ServiceState) -> Response {
    let body: Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    let items: Vec<&Value> = match (
        body.get("providers").and_then(Value::as_array),
        body.get("provider"),
    ) {
        (Some(list), _) => list.iter().collect(),
        (None, Some(single)) => vec![single],
        _ => return Response::error(422, "body must contain a providers array"),
    };
    if items.is_empty() {
        return Response::error(422, "at least one provider is required");
    }
    let path = connect_path(state);
    let mut settings = match load_connect(&path) {
        Ok(Some(settings)) => settings,
        Ok(None) => ConnectSettings::default(),
        Err(error) => return Response::error(500, &error.to_string()),
    };
    for item in items {
        if let Err(message) = apply_connect_item(&mut settings, item) {
            return Response::error(422, &message);
        }
    }
    match commit_connect_change(state, &settings) {
        Ok((custom, _runtime)) => {
            let body = json!({
                "ok": true,
                "revision": revision_of_with_connect(&custom, Some(&settings)),
            });
            Response::json(200, body.to_string())
        }
        Err(response) => response,
    }
}

fn apply_connect_item(settings: &mut ConnectSettings, item: &Value) -> Result<(), String> {
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or("provider id is required")?;
    let spec = find_preset(id).ok_or_else(|| format!("unknown preset provider '{id}'"))?;
    let mut record = settings
        .providers
        .get(id)
        .cloned()
        .unwrap_or_else(|| ConnectedPresetRecord::new(id));
    if let Some(enabled) = item.get("enabled").and_then(Value::as_bool) {
        record.enabled = enabled;
    }
    if let Some(api_key) = item.get("apiKey").and_then(Value::as_str) {
        let trimmed = api_key.trim();
        if trimmed.is_empty() {
            record.api_key = None;
        } else {
            // API Key 与 OAuth 互斥：新 Key 必须压过旧 OAuth 凭证。
            record.api_key = Some(trimmed.to_string());
            record.oauth = None;
        }
    }
    if let Some(answers) = item.get("formAnswers").and_then(Value::as_object) {
        record.form_answers = answers
            .iter()
            .filter_map(|(key, value)| {
                value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| (key.clone(), value.to_string()))
            })
            .collect();
    }
    if let Some(base_url) = item.get("baseUrl").and_then(Value::as_str) {
        let trimmed = base_url.trim();
        // 空值 / 未展开的模板 / 与预设默认值或物化结果相同的地址都不算覆盖：
        // 覆盖值会绕过模板与 OAuth 动态端点，存下来会让该预设被永久冻结。
        let default_resolved = preset_resolved_base_url(spec, &record, false);
        record.base_url = if trimmed.is_empty()
            || trimmed.contains("${")
            || trimmed == spec.default_base_url
            || default_resolved.as_deref() == Some(trimmed)
        {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(models) = item.get("models").and_then(Value::as_array) {
        let parsed = parse_models(models, record.models.as_ref())?;
        // 与内置默认模型表一致时不落覆盖值，继续跟随云端目录更新。
        record.models = if parsed == default_models_for_preset(spec.id) {
            None
        } else {
            Some(parsed)
        };
    }
    if record.enabled && resolve_preset_connection_source(spec, Some(&record)).is_some() {
        let valid = preset_resolved_base_url(spec, &record, true)
            .is_some_and(|url| crate::config::ai::validate_base_url(&url).is_ok());
        if !valid {
            return Err("Base URL is incomplete or invalid; fill required form fields".into());
        }
    }
    settings.providers.insert(id.to_string(), record);
    Ok(())
}

/// `POST /api/models/connect/disconnect`：清除凭证；仅命中环境变量时写
/// `enabled: false` 以阻断自动探测。
fn disconnect(request: &Request, state: &ServiceState) -> Response {
    let body: Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    let Some(provider_id) = body.get("providerId").and_then(Value::as_str) else {
        return Response::error(422, "providerId is required");
    };
    let Some(spec) = find_preset(provider_id) else {
        return Response::error(422, "provider not found");
    };
    let path = connect_path(state);
    let mut settings = match load_connect(&path) {
        Ok(Some(settings)) => settings,
        Ok(None) => ConnectSettings::default(),
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let record = settings.providers.get(provider_id).cloned();
    // 仅命中环境变量时也必须保留一条 `enabled: false` 记录阻断自动探测；
    // 否则删除记录后会立即回退命中 `$ENV`，断开操作失效。
    let env_connected = spec.env_vars.iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
    });
    if env_connected {
        let mut record = record.unwrap_or_else(|| ConnectedPresetRecord::new(provider_id));
        record.enabled = false;
        record.api_key = None;
        record.oauth = None;
        settings.providers.insert(provider_id.to_string(), record);
    } else {
        settings.providers.remove(provider_id);
    }
    match commit_connect_change(state, &settings) {
        Ok(_) => Response::json(200, "{\"ok\":true}"),
        Err(response) => response,
    }
}

/// 落盘 `connect.json` → 清理 `ai.json` 悬空选择 → 广播运行时快照。
///
/// 返回 `(自定义配置, 合并后运行时配置)`；`ai.json` 读取失败时只清理内存选择，
/// 不改写文件，避免把损坏配置覆盖成空表。
fn commit_connect_change(
    state: &ServiceState,
    connect: &ConnectSettings,
) -> Result<(AiSettings, AiSettings), Response> {
    let ai_file = ai_path(state);
    let connect_file = connect_path(state);
    let mut custom = match crate::config::ai::load_ai(&ai_file) {
        Ok(Some(settings)) => settings,
        Ok(None) => AiSettings::empty(),
        Err(error) => {
            tracing::warn!(error = %error, "ai.json is unreadable; keeping it untouched");
            AiSettings::empty()
        }
    };
    save_connect(&connect_file, connect)
        .map_err(|error| Response::error(500, &error.to_string()))?;
    let runtime = merge_runtime_ai_settings(&custom, connect);
    if custom.default_model != runtime.default_model
        || custom.title_summary != runtime.title_summary
    {
        custom.default_model = runtime.default_model.clone();
        custom.title_summary = runtime.title_summary.clone();
        crate::config::ai::save_ai(&ai_file, &custom)
            .map_err(|error| Response::error(500, &error.to_string()))?;
    }
    let _ = state
        .sender()
        .send(SettingsEvent::ModelsChanged(Box::new(runtime.clone())));
    Ok((custom, runtime))
}

/// `POST /api/models/connect/oauth/start`：启动 OAuth 并后台等待授权结果。
async fn oauth_start(request: &Request, state: &ServiceState) -> Response {
    let body: Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    let Some(provider_id) = body.get("providerId").and_then(Value::as_str) else {
        return Response::error(422, "providerId is required");
    };
    let Some(method_id) = body.get("methodId").and_then(Value::as_str) else {
        return Response::error(422, "methodId is required");
    };
    let form_answers = form_answers(body.get("formAnswers"));
    let (started, mut handle) = match start_oauth_flow(provider_id, method_id, &form_answers).await
    {
        Ok(pair) => pair,
        Err(error) => return Response::error(422, &error),
    };
    let Some(receiver) = handle.take_result() else {
        return Response::error(500, "oauth flow is unavailable");
    };
    let attempt_id = started.attempt_id.clone();
    let map = state.connect_oauth();
    map.lock().await.insert(
        attempt_id.clone(),
        PendingConnectOAuth {
            status: ConnectOAuthStatus::Pending,
            error: None,
            handle,
        },
    );
    spawn_oauth_watcher(
        state,
        map,
        attempt_id,
        provider_id.to_string(),
        form_answers,
        receiver,
    );
    let response = json!({
        "attemptId": started.attempt_id,
        "url": started.url,
        "userCode": started.user_code,
        "instructions": started.instructions,
        "mode": started.mode.as_str(),
    });
    Response::json(200, response.to_string())
}

fn form_answers(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .map(|answers| {
            answers
                .iter()
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| (key.clone(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 后台等待授权结果：成功后写 `connect.json` 并广播，随后更新轮询状态。
fn spawn_oauth_watcher(
    state: &ServiceState,
    map: PendingConnectOAuthMap,
    attempt_id: String,
    provider_id: String,
    form_answers: BTreeMap<String, String>,
    receiver: oneshot::Receiver<Result<OAuthCredential, String>>,
) {
    let ai_file = ai_path(state);
    let connect_file = connect_path(state);
    let sender = state.sender().clone();
    tokio::spawn(async move {
        let result = match receiver.await {
            Ok(result) => result,
            Err(_) => Err("OAuth flow ended without a result".to_string()),
        };
        let (status, error) = match result {
            Ok(credential) => {
                match persist_oauth_credential(
                    &connect_file,
                    &ai_file,
                    &provider_id,
                    &form_answers,
                    credential,
                    &sender,
                ) {
                    Ok(()) => (ConnectOAuthStatus::Complete, None),
                    Err(message) => (ConnectOAuthStatus::Failed, Some(message)),
                }
            }
            Err(error) => (ConnectOAuthStatus::Failed, Some(error)),
        };
        let mut guard = map.lock().await;
        if let Some(entry) = guard.get_mut(&attempt_id) {
            entry.status = status;
            entry.error = error;
        }
    });
}

fn persist_oauth_credential(
    connect_file: &Path,
    ai_file: &Path,
    provider_id: &str,
    form_answers: &BTreeMap<String, String>,
    credential: OAuthCredential,
    sender: &tokio::sync::mpsc::UnboundedSender<SettingsEvent>,
) -> Result<(), String> {
    let mut settings = match load_connect(connect_file) {
        Ok(Some(settings)) => settings,
        Ok(None) => ConnectSettings::default(),
        Err(error) => return Err(format!("failed to load connect.json: {error}")),
    };
    let mut record = settings
        .providers
        .get(provider_id)
        .cloned()
        .unwrap_or_else(|| ConnectedPresetRecord::new(provider_id));
    // OAuth 与 API Key 互斥：新 OAuth 凭证必须压过旧 API Key。
    record.oauth = Some(credential);
    record.api_key = None;
    record.form_answers = form_answers.clone();
    record.enabled = true;
    settings.providers.insert(provider_id.to_string(), record);
    save_connect(connect_file, &settings).map_err(|error| error.to_string())?;
    let custom = crate::config::ai::load_ai(ai_file)
        .ok()
        .flatten()
        .unwrap_or_else(AiSettings::empty);
    let runtime = merge_runtime_ai_settings(&custom, &settings);
    let _ = sender.send(SettingsEvent::ModelsChanged(Box::new(runtime)));
    Ok(())
}

/// `GET/POST /api/models/connect/oauth/status?attemptId=...`。
async fn oauth_status(request: &Request, state: &ServiceState) -> Response {
    let Some(attempt_id) = attempt_id_of(request) else {
        return Response::error(422, "attemptId is required");
    };
    let map = state.connect_oauth();
    let mut guard = map.lock().await;
    let Some(entry) = guard.get(&attempt_id) else {
        return Response::error(404, "unknown oauth attempt");
    };
    let status = entry.status;
    let error = entry.error.clone();
    let mut body = json!({"status": status.as_str()});
    if let Some(error) = &error {
        body["error"] = json!(error);
    }
    // 终态条目立即移除，避免长期运行累积内存。
    if matches!(
        status,
        ConnectOAuthStatus::Complete | ConnectOAuthStatus::Failed
    ) {
        guard.remove(&attempt_id);
    }
    Response::json(200, body.to_string())
}

/// `POST /api/models/connect/oauth/complete`：手动提交授权码。
async fn oauth_complete(request: &Request, state: &ServiceState) -> Response {
    let body: Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    let Some(attempt_id) = body.get("attemptId").and_then(Value::as_str) else {
        return Response::error(422, "attemptId is required");
    };
    let Some(code) = body
        .get("code")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|code| !code.is_empty())
    else {
        return Response::error(422, "code is required");
    };
    let map = state.connect_oauth();
    let guard = map.lock().await;
    let Some(entry) = guard.get(attempt_id) else {
        return Response::error(404, "unknown oauth attempt");
    };
    match entry.handle.submit_code(code) {
        Ok(()) => Response::json(200, "{\"ok\":true}"),
        Err(message) => Response::error(422, &message),
    }
}

/// `DELETE /api/models/connect/oauth`：取消进行中的授权。
async fn oauth_cancel(request: &Request, state: &ServiceState) -> Response {
    let Some(attempt_id) = attempt_id_of(request) else {
        return Response::error(422, "attemptId is required");
    };
    let map = state.connect_oauth();
    let entry = map.lock().await.remove(&attempt_id);
    match entry {
        Some(entry) => {
            entry.handle.cancel();
            Response::json(200, "{\"ok\":true}")
        }
        None => Response::error(404, "unknown oauth attempt"),
    }
}

fn attempt_id_of(request: &Request) -> Option<String> {
    if let Some(value) = request
        .query
        .get("attemptId")
        .filter(|value| !value.is_empty())
    {
        return Some(value.clone());
    }
    let body: Value = serde_json::from_str(&request.body).ok()?;
    body.get("attemptId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

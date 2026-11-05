//! 快捷连接配置（`config/connect.json`）：凭证、覆盖项与运行时合并。
//!
//! 快捷连接与自定义供应商物理隔离：自定义供应商继续存 `ai.json`，预设供应商的
//! API Key / OAuth Token / 表单答案 / Base URL 覆盖与模型增补独立存 `connect.json`
//! （0600 权限 + 同目录原子写）。未连接的预设只驻留二进制内存，不落盘。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::presets::{
    PresetAuthMethod, PresetProviderSpec, all_presets, default_models_for_preset, find_preset,
    resolve_preset_base_url,
};
use super::{
    AiConfigError, AiSettings, ModelConfig, ModelSelection, ProviderConfig, ProviderOptions,
    save_private_json,
};

/// OAuth 落盘凭证（保存在 0600 权限的 `connect.json` 中）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCredential {
    pub method_id: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// 单个快捷连接供应商在 `connect.json` 中的持久化记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectedPresetRecord {
    pub id: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthCredential>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub form_answers: BTreeMap<String, String>,
    /// 用户覆盖的 Base URL；`None` 时取预设模板展开值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// 用户自定义后的模型表；`None` 时使用预设内置模型。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<BTreeMap<String, ModelConfig>>,
}

fn default_enabled() -> bool {
    true
}

impl ConnectedPresetRecord {
    /// 新建一条启用中的连接记录。
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            enabled: true,
            api_key: None,
            oauth: None,
            form_answers: BTreeMap::new(),
            base_url: None,
            models: None,
        }
    }
}

/// `~/.lx-t[-dev]/config/connect.json` 顶层结构。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectSettings {
    #[serde(default)]
    pub providers: BTreeMap<String, ConnectedPresetRecord>,
}

/// 快捷连接状态来源（`/connect` 面板与设置页标签展示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionSource {
    ApiKey,
    OAuth { label: Option<String> },
    Env { var_name: String },
}

impl ConnectionSource {
    /// 展示用短标签（如 `"API key"`、`"OAuth · org"`、`"$OPENAI_API_KEY"`）。
    pub fn summary_label(&self) -> String {
        match self {
            Self::ApiKey => "API key".to_string(),
            Self::OAuth { label: Some(label) } if !label.trim().is_empty() => {
                format!("OAuth · {}", label.trim())
            }
            Self::OAuth { .. } => "OAuth".to_string(),
            Self::Env { var_name } => format!("${var_name}"),
        }
    }

    /// 是否来自进程环境变量。
    pub fn is_env(&self) -> bool {
        matches!(self, Self::Env { .. })
    }
}

/// `~/.lx-t[-dev]/config/connect.json` 路径。
pub fn connect_config_path() -> Result<PathBuf, AiConfigError> {
    Ok(super::user_data_root()?.join("config").join("connect.json"))
}

/// 与给定 `ai.json` 同目录的 `connect.json` 路径。
pub fn connect_path_for_ai_path(ai_path: &Path) -> PathBuf {
    ai_path.with_file_name("connect.json")
}

/// 读取快捷连接配置；文件不存在返回 `None`，损坏返回错误（不静默覆盖）。
pub fn load_connect(path: &Path) -> Result<Option<ConnectSettings>, AiConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AiConfigError::Io(error)),
    };
    let settings: ConnectSettings = serde_json::from_str(&text)
        .map_err(|error| AiConfigError::Invalid(format!("malformed connect json: {error}")))?;
    Ok(Some(settings))
}

/// 读取快捷连接配置；缺失或损坏时返回默认空配置。
pub fn load_connect_or_default(path: &Path) -> Result<ConnectSettings, AiConfigError> {
    match load_connect(path) {
        Ok(Some(settings)) => Ok(settings),
        Ok(None) => Ok(ConnectSettings::default()),
        Err(error) => {
            tracing::warn!(error = %error, "connect.json is unreadable; using defaults");
            Ok(ConnectSettings::default())
        }
    }
}

/// 原子保存快捷连接配置（同目录临时文件 + 0600 权限）。
pub fn save_connect(path: &Path, settings: &ConnectSettings) -> Result<(), AiConfigError> {
    for (id, record) in &settings.providers {
        if record.id != *id {
            return Err(AiConfigError::Invalid(format!(
                "connect provider key '{id}' does not match id '{}'",
                record.id
            )));
        }
        if let Some(base_url) = &record.base_url {
            super::validate_base_url(base_url)?;
        }
        if let Some(spec) = find_preset(id)
            && let Some(models) = &record.models
        {
            for (model_key, model) in models {
                if model.id != *model_key {
                    return Err(AiConfigError::Invalid(format!(
                        "connect model key '{model_key}' does not match id '{}'",
                        model.id
                    )));
                }
                if spec.transport == super::ProviderTransport::Openai
                    && model.transport == Some(super::ModelTransport::OpenaiCompatible)
                {
                    return Err(AiConfigError::Invalid(format!(
                        "connect model '{model_key}' cannot use openai-compatible transport under openai provider"
                    )));
                }
                if let Some(variant) = &model.variant
                    && !model.variants.contains_key(variant)
                {
                    return Err(AiConfigError::Invalid(format!(
                        "connect model '{model_key}' default variant '{variant}' is not in variants"
                    )));
                }
            }
        }
    }
    let body = serde_json::to_string_pretty(settings)
        .map_err(|error| AiConfigError::Invalid(format!("serialize failed: {error}")))?;
    save_private_json(path, &format!("{body}\n"))
}

/// 解析连接来源（进程环境变量版本）。
pub fn resolve_preset_connection_source(
    spec: &PresetProviderSpec,
    record: Option<&ConnectedPresetRecord>,
) -> Option<ConnectionSource> {
    resolve_preset_connection_source_with_env(spec, record, |name| std::env::var(name).ok())
}

/// 解析连接来源（可注入环境变量查询，便于确定性测试）。
///
/// 优先级：显式 OAuth > 显式 API Key > 首个非空环境变量；记录被禁用时不再回退环境变量。
pub fn resolve_preset_connection_source_with_env<F>(
    spec: &PresetProviderSpec,
    record: Option<&ConnectedPresetRecord>,
    env: F,
) -> Option<ConnectionSource>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(record) = record {
        if let Some(oauth) = &record.oauth
            && !oauth.access_token.trim().is_empty()
        {
            return Some(ConnectionSource::OAuth {
                label: oauth
                    .account_label
                    .as_ref()
                    .map(|label| label.trim().to_string())
                    .filter(|label| !label.is_empty()),
            });
        }
        if let Some(api_key) = &record.api_key
            && !api_key.trim().is_empty()
        {
            return Some(ConnectionSource::ApiKey);
        }
        if !record.enabled {
            return None;
        }
    }
    for name in spec.env_vars {
        if let Some(value) = env(name)
            && !value.trim().is_empty()
        {
            return Some(ConnectionSource::Env {
                var_name: (*name).to_string(),
            });
        }
    }
    None
}

/// 将已连接的预设供应商物化为运行时 `ProviderConfig`，返回 `(provider, enabled)`。
pub fn materialize_preset_provider(
    spec: &PresetProviderSpec,
    record: Option<&ConnectedPresetRecord>,
) -> Option<(ProviderConfig, bool)> {
    materialize_preset_provider_with_env(spec, record, |name| std::env::var(name).ok())
}

/// `materialize_preset_provider` 的可注入环境变量版本。
pub fn materialize_preset_provider_with_env<F>(
    spec: &PresetProviderSpec,
    record: Option<&ConnectedPresetRecord>,
    env: F,
) -> Option<(ProviderConfig, bool)>
where
    F: Fn(&str) -> Option<String>,
{
    let source = resolve_preset_connection_source_with_env(spec, record, &env)?;
    let api_key = match &source {
        ConnectionSource::OAuth { .. } => record
            .and_then(|record| record.oauth.as_ref())
            .map(|oauth| oauth.access_token.trim().to_string()),
        ConnectionSource::ApiKey => record
            .and_then(|record| record.api_key.as_ref())
            .map(|key| key.trim().to_string()),
        ConnectionSource::Env { var_name } => env(var_name).map(|value| value.trim().to_string()),
    }
    .filter(|key| !key.is_empty())?;

    let mut form_answers = record
        .map(|record| record.form_answers.clone())
        .unwrap_or_default();
    // 模板端点（Cloudflare / Azure / Infomaniak / Snowflake / Databricks）可从环境变量补齐。
    let template_env: [(&str, &str, &str); 6] = [
        (
            "accountId",
            "CLOUDFLARE_ACCOUNT_ID",
            "${CLOUDFLARE_ACCOUNT_ID}",
        ),
        (
            "gatewayId",
            "CLOUDFLARE_GATEWAY_ID",
            "${CLOUDFLARE_GATEWAY_ID}",
        ),
        (
            "resourceName",
            "AZURE_RESOURCE_NAME",
            "${AZURE_RESOURCE_NAME}",
        ),
        (
            "productId",
            "INFOMANIAK_PRODUCT_ID",
            "${INFOMANIAK_PRODUCT_ID}",
        ),
        ("account", "SNOWFLAKE_ACCOUNT", "${SNOWFLAKE_ACCOUNT}"),
        ("baseUrl", "DATABRICKS_HOST", "${DATABRICKS_HOST}"),
    ];
    for (key, name, placeholder) in template_env {
        if form_answers.contains_key(key) || !spec.default_base_url.contains(placeholder) {
            continue;
        }
        if let Some(value) = env(name)
            && !value.trim().is_empty()
        {
            form_answers.insert(key.to_string(), value.trim().to_string());
        }
    }
    let oauth_method = record
        .and_then(|record| record.oauth.as_ref())
        .and_then(|oauth| spec.method(&oauth.method_id));
    let oauth_metadata_base = record
        .and_then(|record| record.oauth.as_ref())
        .and_then(|oauth| oauth.metadata.get("apiEndpoint"))
        .map(|endpoint| endpoint.trim().trim_end_matches('/').to_string())
        .filter(|endpoint| !endpoint.is_empty());
    let method_override = oauth_method
        .and_then(PresetAuthMethod::base_url_override)
        .filter(|url| !url.trim().is_empty());
    let base_url = oauth_metadata_base.unwrap_or_else(|| {
        let metadata_instance = record
            .and_then(|record| record.oauth.as_ref())
            .and_then(|oauth| oauth.metadata.get("instanceUrl"))
            .filter(|instance| spec.id == "gitlab" && !instance.trim().is_empty());
        let override_url = record
            .and_then(|record| record.base_url.as_deref())
            .or(method_override);
        match metadata_instance {
            Some(instance) => resolve_preset_base_url(spec, &form_answers, Some(instance)),
            None => resolve_preset_base_url(spec, &form_answers, override_url),
        }
    });
    if base_url.contains("${") {
        tracing::warn!(
            provider = spec.id,
            "preset base URL still has unresolved template fields"
        );
        return None;
    }
    super::validate_base_url(&base_url).ok()?;

    let models = record
        .and_then(|record| record.models.clone())
        .filter(|models| !models.is_empty())
        .unwrap_or_else(|| default_models_for_preset(spec.id));
    let enabled = record.map(|record| record.enabled).unwrap_or(true);
    Some((
        ProviderConfig {
            id: spec.id.to_string(),
            kind: spec.transport,
            name: spec.name.to_string(),
            options: ProviderOptions {
                api_key: Some(api_key),
                base_url,
            },
            models,
            extra: serde_json::Map::new(),
        },
        enabled,
    ))
}

/// 合并自定义 `ai.json` 与快捷连接（含环境变量命中），返回运行时快照。
pub fn merge_runtime_ai_settings(custom: &AiSettings, connect: &ConnectSettings) -> AiSettings {
    merge_runtime_ai_settings_with_env(custom, connect, |name| std::env::var(name).ok())
}

/// `merge_runtime_ai_settings` 的可注入环境变量版本。
pub(crate) fn merge_runtime_ai_settings_with_env<F>(
    custom: &AiSettings,
    connect: &ConnectSettings,
    env: F,
) -> AiSettings
where
    F: Fn(&str) -> Option<String>,
{
    let mut merged = custom.clone();
    for spec in all_presets() {
        if merged.providers.contains_key(spec.id) {
            continue;
        }
        let Some((provider, enabled)) =
            materialize_preset_provider_with_env(spec, connect.providers.get(spec.id), &env)
        else {
            continue;
        };
        merged.providers.insert(spec.id.to_string(), provider);
        if enabled && !merged.enabled_providers.iter().any(|id| id == spec.id) {
            merged.enabled_providers.push(spec.id.to_string());
        }
    }
    sanitize_runtime_selections(&mut merged);
    merged
}

fn sanitize_runtime_selections(settings: &mut AiSettings) {
    let enabled: std::collections::HashSet<String> = settings
        .enabled_providers
        .iter()
        .filter(|id| settings.providers.contains_key(*id))
        .cloned()
        .collect();
    super::sanitize_selection_slot(
        "default model",
        &mut settings.default_model,
        &settings.providers,
        &enabled,
    );
    super::sanitize_selection_slot(
        "title model",
        &mut settings.title_summary,
        &settings.providers,
        &enabled,
    );
}

/// 采集所有预设环境变量（非空、去首尾空白）的快照，供 `app/` 层注入使用。
pub fn collect_preset_env_snapshot() -> BTreeMap<String, String> {
    let mut snapshot = BTreeMap::new();
    for spec in all_presets() {
        for name in spec.env_vars {
            if snapshot.contains_key(*name) {
                continue;
            }
            if let Ok(value) = std::env::var(name) {
                let value = value.trim();
                if !value.is_empty() {
                    snapshot.insert((*name).to_string(), value.to_string());
                }
            }
        }
    }
    snapshot
}

/// 断开预设连接并清理 `ai.json` 中悬空的默认模型选择（进程环境变量版本）。
pub fn disconnect_preset_on_disk(
    ai_path: &Path,
    provider_id: &str,
) -> Result<AiSettings, AiConfigError> {
    disconnect_preset_on_disk_with_env(ai_path, provider_id, |name| std::env::var(name).ok())
}

/// 断开预设连接并清理 `ai.json` 中悬空的默认模型选择。
///
/// 若该预设仍能通过环境变量连接，则保留记录并置为禁用；否则直接移除记录。
pub fn disconnect_preset_on_disk_with_env<F>(
    ai_path: &Path,
    provider_id: &str,
    env: F,
) -> Result<AiSettings, AiConfigError>
where
    F: Fn(&str) -> Option<String>,
{
    let connect_path = connect_path_for_ai_path(ai_path);
    let mut connect = load_connect(&connect_path)?.unwrap_or_default();
    if has_active_preset_env(provider_id, &env) {
        let record = connect
            .providers
            .entry(provider_id.to_string())
            .or_insert_with(|| ConnectedPresetRecord::new(provider_id));
        record.enabled = false;
        record.api_key = None;
        record.oauth = None;
    } else {
        connect.providers.remove(provider_id);
    }
    save_connect(&connect_path, &connect)?;

    if ai_path.exists() {
        let text = std::fs::read_to_string(ai_path)?;
        let mut ai_settings = super::parse_ai_root(&text)?;
        let before_default = ai_settings.default_model.clone();
        let before_title = ai_settings.title_summary.clone();
        ai_settings.sanitize_optional_selections_with_connect(Some(&connect));
        for slot in [
            &mut ai_settings.default_model,
            &mut ai_settings.title_summary,
        ] {
            let dangling = slot.as_ref().is_some_and(|selection| {
                find_preset(&selection.provider).is_some()
                    && !is_connected_preset_selection_with_env(&connect, selection, &env)
            });
            if dangling {
                *slot = None;
            }
        }
        if ai_settings.default_model != before_default || ai_settings.title_summary != before_title
        {
            super::save_ai_with_connect(ai_path, &ai_settings, Some(&connect))?;
        }
        return Ok(merge_runtime_ai_settings_with_env(
            &ai_settings,
            &connect,
            &env,
        ));
    }
    Ok(merge_runtime_ai_settings_with_env(
        &AiSettings::empty(),
        &connect,
        &env,
    ))
}

fn has_active_preset_env<F>(provider_id: &str, env: &F) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    find_preset(provider_id).is_some_and(|spec| {
        spec.env_vars
            .iter()
            .any(|name| env(name).is_some_and(|value| !value.trim().is_empty()))
    })
}

/// 读取运行时合并后的模型配置：`ai.json` + 同目录 `connect.json`。
pub fn load_runtime_ai(ai_path: &Path) -> Result<Option<AiSettings>, AiConfigError> {
    let custom = load_custom_for_runtime(ai_path)?;
    let connect_path = connect_path_for_ai_path(ai_path);
    let connect = match load_connect(&connect_path) {
        Ok(settings) => settings,
        Err(error) => {
            tracing::warn!(
                path = %connect_path.display(),
                error = %error,
                "ignoring unreadable connect.json for runtime settings"
            );
            None
        }
    };
    let both_missing = custom.is_none() && connect.is_none();
    let merged = merge_runtime_ai_settings(
        &custom.unwrap_or_else(AiSettings::empty),
        &connect.unwrap_or_default(),
    );
    if both_missing && merged.providers.is_empty() {
        Ok(None)
    } else {
        Ok(Some(merged))
    }
}

/// 运行时加载：`defaultModel` 悬空时清理而不是让整份运行时配置失效。
fn load_custom_for_runtime(ai_path: &Path) -> Result<Option<AiSettings>, AiConfigError> {
    let text = match std::fs::read_to_string(ai_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AiConfigError::Io(error)),
    };
    let connect = super::load_connect_for_ai(ai_path);
    let mut settings = super::parse_ai_root(&text)?;
    settings.sanitize_optional_selections_with_connect(connect.as_ref());
    if settings.validate_with_connect(connect.as_ref()).is_err() {
        settings.default_model = None;
        settings.validate_with_connect(connect.as_ref())?;
    }
    Ok(Some(settings))
}

/// 读取运行时合并后的模型配置；缺失时返回空配置。
pub fn load_runtime_ai_or_empty(ai_path: &Path) -> Result<AiSettings, AiConfigError> {
    Ok(load_runtime_ai(ai_path)?.unwrap_or_else(AiSettings::empty))
}

/// 判断 `defaultModel` / `titleSummary` 是否指向一个已连接的预设模型。
pub(crate) fn is_connected_preset_selection(
    connect: &ConnectSettings,
    selection: &ModelSelection,
) -> bool {
    is_connected_preset_selection_with_env(connect, selection, |name| std::env::var(name).ok())
}

/// 可注入环境变量版本的 [`is_connected_preset_selection`]。
pub(crate) fn is_connected_preset_selection_with_env<F>(
    connect: &ConnectSettings,
    selection: &ModelSelection,
    env: F,
) -> bool
where
    F: Fn(&str) -> Option<String>,
{
    let Some(spec) = find_preset(&selection.provider) else {
        return false;
    };
    let Some((provider, enabled)) =
        materialize_preset_provider_with_env(spec, connect.providers.get(spec.id), env)
    else {
        return false;
    };
    if !enabled {
        return false;
    }
    let Some(model) = provider.models.get(&selection.model) else {
        return false;
    };
    selection
        .variant
        .as_ref()
        .is_none_or(|variant| model.variants.contains_key(variant))
}

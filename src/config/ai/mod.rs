//! 模型配置（`config/ai.json`）：类型、校验、原子保存与 lx-agent 一次性导入。
//!
//! 与 `settings.json`（布局整数）完全分离；模型配置是 `ai` 节点下的
//! providers/enabled_providers/defaultModel/streamIdleTimeoutMs。
//! 解析用 `serde`/`serde_json`；未知字段往返保留，已知字段严格校验。
//! API Key 只在内存与权限受控文件中保存，永不进入日志与 Web 响应。

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{ConfigError, user_data_root};

/// 默认流式空闲超时（毫秒），对齐 lx-agent。
pub const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 60_000;

/// 供应商传输协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderTransport {
    Openai,
    Anthropic,
    Google,
    #[serde(rename = "openai-compatible")]
    OpenaiCompatible,
}

impl ProviderTransport {
    /// 稳定字符串值（存储与展示）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::OpenaiCompatible => "openai-compatible",
        }
    }

    /// 解析稳定字符串值。
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "openai" => Some(Self::Openai),
            "anthropic" => Some(Self::Anthropic),
            "google" => Some(Self::Google),
            "openai-compatible" => Some(Self::OpenaiCompatible),
            _ => None,
        }
    }
}

/// 模型级传输协议：缺省继承 provider type；`openai-responses` 走 Responses API。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelTransport {
    Openai,
    Anthropic,
    Google,
    #[serde(rename = "openai-compatible")]
    OpenaiCompatible,
    #[serde(rename = "openai-responses")]
    OpenaiResponses,
}

impl ModelTransport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::OpenaiCompatible => "openai-compatible",
            Self::OpenaiResponses => "openai-responses",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "openai" => Some(Self::Openai),
            "anthropic" => Some(Self::Anthropic),
            "google" => Some(Self::Google),
            "openai-compatible" => Some(Self::OpenaiCompatible),
            "openai-responses" => Some(Self::OpenaiResponses),
            _ => None,
        }
    }
}

/// 模型配置（lx-agent `ModelProviderModel` 同语义）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    /// 模型级传输协议覆盖；缺省继承 provider type。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<ModelTransport>,
    /// 上下文/输出上限；缺失表示未知。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<ModelLimit>,
    /// 模态；仅往返保留，本期运行不使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<serde_json::Value>,
    /// 思考等级预设（variant key -> 供应商参数对象）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variants: BTreeMap<String, serde_json::Value>,
    /// 默认思考等级（必须存在于 variants）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// 计价（USD / 百万 token）；仅往返保留。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<serde_json::Value>,
    /// 未知字段往返保留。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 模型上下文/输出上限。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelLimit {
    pub context: u64,
    pub output: u64,
}

/// 供应商连接选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOptions {
    /// API Key；仅存内存与 0600 权限文件。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// 基础 URL；必须是 http(s) 且带主机。lx-agent 写作 `baseURL`。
    #[serde(rename = "baseURL")]
    pub base_url: String,
}

/// 模型供应商（lx-agent `ModelProvider` 同语义）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: ProviderTransport,
    pub name: String,
    pub options: ProviderOptions,
    #[serde(default)]
    pub models: BTreeMap<String, ModelConfig>,
    /// 未知字段往返保留。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// 默认模型选择。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

/// `ai.json` 的 `ai` 节点。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiSettings {
    /// 启用的供应商 id 列表（lx-agent 写作 snake_case `enabled_providers`）。
    #[serde(default, rename = "enabled_providers")]
    pub enabled_providers: Vec<String>,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    /// 默认模型选择（lx-agent 写作 camelCase `defaultModel`）。
    #[serde(
        default,
        rename = "defaultModel",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_model: Option<ModelSelection>,
    /// 流式空闲超时毫秒（lx-agent 写作 camelCase `streamIdleTimeoutMs`）。
    #[serde(
        rename = "streamIdleTimeoutMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub stream_idle_timeout_ms: Option<u64>,
    /// 未知字段往返保留（如 lx-agent 的 titleSummary 等，导入时丢弃、
    /// 本项目保存时不回写这些字段）。
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl AiSettings {
    /// 空配置。
    pub fn empty() -> Self {
        Self {
            enabled_providers: Vec::new(),
            providers: BTreeMap::new(),
            default_model: None,
            stream_idle_timeout_ms: None,
            extra: serde_json::Map::new(),
        }
    }

    /// 有效空闲超时；未配置取默认值。
    pub fn idle_timeout_ms(&self) -> u64 {
        self.stream_idle_timeout_ms
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_STREAM_IDLE_TIMEOUT_MS)
    }

    /// 校验整体一致性：provider id 匹配、默认选择指向存在且启用的模型、
    /// variant 存在于该模型 variants、base URL 合法。
    pub fn validate(&self) -> Result<(), AiConfigError> {
        let mut enabled = std::collections::HashSet::new();
        for id in &self.enabled_providers {
            if !self.providers.contains_key(id) {
                return Err(AiConfigError::Invalid(format!(
                    "enabled provider '{id}' is not defined"
                )));
            }
            enabled.insert(id.clone());
        }
        for (key, provider) in &self.providers {
            if provider.id != *key {
                return Err(AiConfigError::Invalid(format!(
                    "provider key '{key}' does not match id '{}'",
                    provider.id
                )));
            }
            validate_base_url(&provider.options.base_url)?;
            for (model_key, model) in &provider.models {
                if model.id != *model_key {
                    return Err(AiConfigError::Invalid(format!(
                        "model key '{model_key}' does not match id '{}'",
                        model.id
                    )));
                }
                if let Some(transport) = model.transport
                    && provider.kind == ProviderTransport::Openai
                    && transport == ModelTransport::OpenaiCompatible
                {
                    return Err(AiConfigError::Invalid(format!(
                        "model '{model_key}' cannot use openai-compatible transport under openai provider"
                    )));
                }
                if let Some(variant) = &model.variant
                    && !model.variants.contains_key(variant)
                {
                    return Err(AiConfigError::Invalid(format!(
                        "model '{model_key}' default variant '{variant}' is not in variants"
                    )));
                }
            }
        }
        if let Some(selection) = &self.default_model {
            let Some(provider) = self.providers.get(&selection.provider) else {
                return Err(AiConfigError::Invalid(format!(
                    "default model provider '{}' is not defined",
                    selection.provider
                )));
            };
            if !enabled.contains(&selection.provider) {
                return Err(AiConfigError::Invalid(format!(
                    "default model provider '{}' is not enabled",
                    selection.provider
                )));
            }
            let Some(model) = provider.models.get(&selection.model) else {
                return Err(AiConfigError::Invalid(format!(
                    "default model '{}' is not defined on provider '{}'",
                    selection.model, selection.provider
                )));
            };
            if let Some(variant) = &selection.variant
                && !model.variants.contains_key(variant)
            {
                return Err(AiConfigError::Invalid(format!(
                    "default model variant '{variant}' is not in model variants"
                )));
            }
        }
        Ok(())
    }

    /// 首个已启用且有模型的供应商与模型（导入时缺省默认选择回落）。
    pub fn first_enabled_model(&self) -> Option<ModelSelection> {
        for id in &self.enabled_providers {
            if let Some(provider) = self.providers.get(id)
                && let Some((model_key, model)) = provider.models.iter().next()
            {
                return Some(ModelSelection {
                    provider: id.clone(),
                    model: model_key.clone(),
                    variant: model.variant.clone(),
                });
            }
        }
        None
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        Self::empty()
    }
}

/// 模型配置错误。
#[derive(Debug)]
pub enum AiConfigError {
    /// HOME 缺失。
    HomeMissing,
    /// IO 错误。
    Io(std::io::Error),
    /// JSON 结构或取值非法。
    Invalid(String),
}

impl fmt::Display for AiConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid ai config: {message}"),
        }
    }
}

impl std::error::Error for AiConfigError {}

impl From<std::io::Error> for AiConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ConfigError> for AiConfigError {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::HomeMissing => Self::HomeMissing,
            ConfigError::Io(io) => Self::Io(io),
            ConfigError::Invalid(message) => Self::Invalid(message),
        }
    }
}

/// `config/ai.json` 路径：开发 `~/.lx-t-dev/config/ai.json`，生产 `~/.lx-t/config/ai.json`。
pub fn ai_config_path() -> Result<PathBuf, AiConfigError> {
    Ok(user_data_root()?.join("config").join("ai.json"))
}

/// lx-agent 源配置路径：`~/.lx/config/ai.json`（只读导入，永不回写）。
fn lx_agent_ai_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".lx")
            .join("config")
            .join("ai.json"),
    )
}

/// lx-agent 内置供应商路径：`~/.lx/config/builtin-providers.json`。
fn lx_agent_builtin_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty())?;
    Some(
        PathBuf::from(home)
            .join(".lx")
            .join("config")
            .join("builtin-providers.json"),
    )
}

/// 读取并校验模型配置；文件不存在返回 None，损坏返回错误（不静默覆盖）。
pub fn load_ai(path: &Path) -> Result<Option<AiSettings>, AiConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(AiConfigError::Io(error)),
    };
    let settings = parse_ai(&text)?;
    Ok(Some(settings))
}

/// 解析并校验 `ai.json` 文本。
pub fn parse_ai(text: &str) -> Result<AiSettings, AiConfigError> {
    #[derive(Deserialize)]
    struct Root {
        ai: AiSettings,
    }
    let root: Root = serde_json::from_str(text)
        .map_err(|error| AiConfigError::Invalid(format!("malformed json: {error}")))?;
    root.ai.validate()?;
    Ok(root.ai)
}

/// 原子保存 `ai.json`：同目录临时文件 + flush + rename；Unix 权限 0600。
///
/// 目录权限交由 `ensure_config_dir`（0700）；保存失败不破坏旧文件。
pub fn save_ai(path: &Path, settings: &AiSettings) -> Result<(), AiConfigError> {
    settings.validate()?;
    let mut root = serde_json::Map::new();
    root.insert(
        "ai".to_string(),
        serde_json::to_value(settings)
            .map_err(|error| AiConfigError::Invalid(format!("serialize failed: {error}")))?,
    );
    let body = serde_json::to_string_pretty(&root)
        .map_err(|error| AiConfigError::Invalid(format!("serialize failed: {error}")))?;
    let body = format!("{body}\n");
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, body.as_bytes())?;
    crate::platform::restrict_file_permissions(&temp)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(AiConfigError::Io(error));
    }
    Ok(())
}

/// 创建（或收紧）私有配置目录：Unix 下 0700。
pub fn create_private_dir(dir: &Path) -> Result<(), AiConfigError> {
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }
    crate::platform::restrict_dir_permissions(dir)?;
    Ok(())
}

/// 一次性导入：目标文件不存在时读取 lx-agent `ai.json`（必要时合并
/// `builtin-providers.json` 的有效预置项）并写入本项目。
///
/// - 目标已存在：跳过导入（文件存在即幂等标记）。
/// - 源不存在：写入空配置（UI 指向添加供应商）。
/// - 源损坏：返回错误且不创建目标文件，可修复后重试。
/// - 只导入模型相关字段与 API Key；不导入 lx-agent 的其它设置。
pub fn import_from_lx_agent(target: &Path) -> Result<bool, AiConfigError> {
    if target.exists() {
        return Ok(false);
    }
    let source_text: Option<String> = match lx_agent_ai_path() {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(AiConfigError::Io(error)),
        },
        None => None,
    };
    let mut settings = match source_text {
        None => AiSettings::empty(),
        Some(text) => parse_ai(&text)?,
    };
    // builtin-providers.json 补充有效预置供应商（lx-agent 同语义）。
    if let Some(builtin_path) = lx_agent_builtin_path()
        && let Ok(builtin_text) = std::fs::read_to_string(&builtin_path)
    {
        merge_builtin_providers(&mut settings, &builtin_text);
    }
    normalize_import(&mut settings);
    settings.validate()?;
    save_ai(target, &settings)?;
    Ok(true)
}

/// 解析 builtin-providers.json 并把缺失的预置供应商并入配置。
fn merge_builtin_providers(settings: &mut AiSettings, text: &str) {
    #[derive(Deserialize)]
    struct BuiltinRoot {
        #[serde(default, rename = "builtinProviders")]
        builtin_providers: BTreeMap<String, ProviderConfig>,
    }
    let Ok(root) = serde_json::from_str::<BuiltinRoot>(text) else {
        return;
    };
    for (id, provider) in root.builtin_providers {
        settings.providers.entry(id).or_insert(provider);
    }
}

/// 归一化导入结果：剔除未定义的 enabled 项、修正悬空默认选择。
fn normalize_import(settings: &mut AiSettings) {
    settings
        .enabled_providers
        .retain(|id| settings.providers.contains_key(id));
    // 默认选择无效时回落到首个已启用且有模型的供应商；再无则清除。
    let selection_valid = settings
        .default_model
        .as_ref()
        .and_then(|selection| {
            let provider = settings.providers.get(&selection.provider)?;
            let model = provider.models.get(&selection.model)?;
            let variant_ok = selection
                .variant
                .as_ref()
                .is_none_or(|variant| model.variants.contains_key(variant));
            variant_ok.then_some(())
        })
        .is_some();
    if !selection_valid {
        settings.default_model = settings.first_enabled_model();
    } else if let Some(selection) = settings.default_model.as_mut()
        && let Some(provider) = settings.providers.get(&selection.provider)
        && let Some(model) = provider.models.get(&selection.model)
    {
        // 默认 variant 缺失时回落到模型默认 variant。
        if selection.variant.is_none() {
            selection.variant = model.variant.clone();
        }
    }
    // 丢弃 lx-agent 独立设置节点（titleSummary 等），只保留本项目管理的字段。
    settings.extra.clear();
}

/// 校验 base URL：http/https 方案、非空主机、无控制字符。
pub fn validate_base_url(url: &str) -> Result<(), AiConfigError> {
    if url.chars().any(|ch| (ch as u32) < 0x20 || ch == '\u{7f}') {
        return Err(AiConfigError::Invalid(
            "base url contains control characters".into(),
        ));
    }
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(AiConfigError::Invalid("base url is empty".into()));
    }
    let (scheme, rest) = trimmed.split_once("://").ok_or_else(|| {
        AiConfigError::Invalid("base url must include a scheme (http/https)".into())
    })?;
    if scheme != "http" && scheme != "https" {
        return Err(AiConfigError::Invalid(format!(
            "unsupported base url scheme '{scheme}'"
        )));
    }
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if host.trim().is_empty() {
        return Err(AiConfigError::Invalid("base url host is empty".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

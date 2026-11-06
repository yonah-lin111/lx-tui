//! 内置预设供应商目录（对齐 `opencode-dev` 的 models.dev 快照）。
//!
//! 静态目录来自编译期嵌入的 [`presets_catalog.json`]：每个供应商包含名称、协议、
//! 默认 Base URL、环境变量、认证方法与常用聊天模型；模型条目含上下文/输出上限、
//! 模态、思考等级 `variants` 与默认 `variant`。
//!
//! 供应商规格在首次访问时解析并泄漏为 `'static`，之后只读复用；模型表放在
//! `RwLock` 中，启动时叠加磁盘缓存、运行时可被云端目录热更新。供应商按名称
//! 首字母（A-Z，不区分大小写）排序，不设分组。

use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

use serde::Deserialize;
use serde_json::Value;

use super::{ModelConfig, ModelLimit, ModelTransport, ProviderTransport};

/// 编译期嵌入的精简预设目录（数据源：opencode-dev models.dev 快照）。
const CATALOG_JSON: &str = include_str!("presets_catalog.json");

/// 表单可选项 `(label, value, description)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetFormOption {
    pub label: &'static str,
    pub value: &'static str,
    pub description: &'static str,
}

/// 预设认证表单字段（对齐 opencode-dev 的 `Form.Field`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetFormField {
    pub key: &'static str,
    pub label: &'static str,
    pub placeholder: &'static str,
    pub required: bool,
    /// 可选枚举项；为空表示自由文本输入。
    pub options: &'static [PresetFormOption],
    /// 条件显隐：仅当前置字段等于该值时才询问。
    pub when_eq: Option<(&'static str, &'static str)>,
}

impl PresetFormField {
    /// 该字段在当前表单答案下是否应当显示/询问。
    pub fn is_active(&self, answers: &BTreeMap<String, String>) -> bool {
        match self.when_eq {
            Some((key, expected)) => answers.get(key).is_some_and(|value| value == expected),
            None => true,
        }
    }
}

/// OAuth 授权模式（对齐 opencode-dev）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthFlowKind {
    /// OAuth 2.0 Device Authorization Grant：展示 URL + 设备码，后台轮询 token。
    DeviceCode {
        device_code_url: &'static str,
        token_url: &'static str,
        client_id: &'static str,
        scope: &'static str,
    },
    /// 本地回环浏览器 PKCE 授权：`127.0.0.1` 临时端口接收回调。
    BrowserPkce {
        authorize_url: &'static str,
        token_url: &'static str,
        client_id: &'static str,
        scope: &'static str,
        fixed_port: Option<u16>,
    },
    /// 浏览器授权码手动粘贴模式。
    AuthorizationCode {
        authorize_url: &'static str,
        token_url: &'static str,
        client_id: &'static str,
        scope: &'static str,
    },
}

impl OAuthFlowKind {
    /// token 端点。
    pub fn token_url(self) -> &'static str {
        match self {
            Self::DeviceCode { token_url, .. }
            | Self::BrowserPkce { token_url, .. }
            | Self::AuthorizationCode { token_url, .. } => token_url,
        }
    }

    /// OAuth 客户端 ID。
    pub fn client_id(self) -> &'static str {
        match self {
            Self::DeviceCode { client_id, .. }
            | Self::BrowserPkce { client_id, .. }
            | Self::AuthorizationCode { client_id, .. } => client_id,
        }
    }

    /// 申请的 scope（可为空）。
    pub fn scope(self) -> &'static str {
        match self {
            Self::DeviceCode { scope, .. }
            | Self::BrowserPkce { scope, .. }
            | Self::AuthorizationCode { scope, .. } => scope,
        }
    }

    /// 是否为浏览器回环回调流。
    pub fn is_browser(self) -> bool {
        matches!(self, Self::BrowserPkce { .. })
    }
}

/// 预设供应商支持的交互式连接方式（OAuth 在前，ApiKey 在后）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PresetAuthMethod {
    OAuth {
        id: &'static str,
        label: &'static str,
        flow: OAuthFlowKind,
        form: &'static [PresetFormField],
        /// OAuth 成功后若需覆盖默认 Base URL。
        base_url_override: Option<&'static str>,
    },
    ApiKey {
        label: &'static str,
        form: &'static [PresetFormField],
    },
}

impl PresetAuthMethod {
    /// 稳定方法 ID：OAuth 使用预设 ID，ApiKey 固定为 `api-key`。
    pub fn id(&self) -> &'static str {
        match self {
            Self::OAuth { id, .. } => id,
            Self::ApiKey { .. } => "api-key",
        }
    }

    /// 展示标签。
    pub fn label(&self) -> &'static str {
        match self {
            Self::OAuth { label, .. } | Self::ApiKey { label, .. } => label,
        }
    }

    /// 表单字段（可能为空）。
    pub fn form(&self) -> &'static [PresetFormField] {
        match self {
            Self::OAuth { form, .. } | Self::ApiKey { form, .. } => form,
        }
    }

    /// OAuth 流程定义；ApiKey 返回 `None`。
    pub fn oauth_flow(&self) -> Option<OAuthFlowKind> {
        match self {
            Self::OAuth { flow, .. } => Some(*flow),
            Self::ApiKey { .. } => None,
        }
    }

    /// OAuth 成功后的 Base URL 覆盖。
    pub fn base_url_override(&self) -> Option<&'static str> {
        match self {
            Self::OAuth {
                base_url_override, ..
            } => *base_url_override,
            Self::ApiKey { .. } => None,
        }
    }
}

/// 预设供应商静态规格（按名称 A-Z 排序）。
#[derive(Debug, Clone)]
pub struct PresetProviderSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub transport: ProviderTransport,
    pub default_base_url: &'static str,
    pub note: &'static str,
    pub env_vars: &'static [&'static str],
    pub methods: &'static [PresetAuthMethod],
}

impl PresetProviderSpec {
    /// 按方法 ID 查找认证方式。
    pub fn method(&self, id: &str) -> Option<&PresetAuthMethod> {
        self.methods.iter().find(|method| method.id() == id)
    }

    /// 是否存在任一 OAuth 认证方式。
    pub fn supports_oauth(&self) -> bool {
        self.methods
            .iter()
            .any(|method| method.oauth_flow().is_some())
    }

    /// 所有 OAuth 认证方式（保持预设顺序）。
    pub fn oauth_methods(&self) -> impl Iterator<Item = &PresetAuthMethod> {
        self.methods
            .iter()
            .filter(|method| method.oauth_flow().is_some())
    }

    /// 首个 API Key 认证方式。
    pub fn api_key_method(&self) -> Option<&PresetAuthMethod> {
        self.methods
            .iter()
            .find(|method| matches!(method, PresetAuthMethod::ApiKey { .. }))
    }
}

/// 返回按名称首字母（A-Z）排序的全部预设供应商规格。
pub fn all_presets() -> &'static [PresetProviderSpec] {
    catalog().presets
}

/// 按 ID 查找预设供应商规格。
pub fn find_preset(id: &str) -> Option<&'static PresetProviderSpec> {
    all_presets().iter().find(|spec| spec.id == id)
}

/// 获取某预设供应商的内置默认模型表（按需从嵌入目录物化）。
pub fn default_models_for_preset(id: &str) -> BTreeMap<String, ModelConfig> {
    let models = catalog()
        .models
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    models.get(id).cloned().unwrap_or_default()
}

/// 用云端目录更新进程内模型表；同名模型保留嵌入目录已声明的 `transport` 与默认 `variant`。
pub fn update_catalog_models(updates: BTreeMap<String, BTreeMap<String, ModelConfig>>) {
    let mut models = catalog()
        .models
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for (provider_id, provider_models) in updates {
        let is_openai = provider_id == "openai";
        let target = models.entry(provider_id).or_default();
        merge_provider_models(target, provider_models, is_openai);
    }
}

pub(super) fn merge_provider_models(
    target: &mut BTreeMap<String, ModelConfig>,
    incoming: BTreeMap<String, ModelConfig>,
    is_openai_provider: bool,
) {
    for (model_id, mut model) in incoming {
        let valid_limit = model
            .limit
            .is_some_and(|limit| limit.context > 0 && limit.output > 0);
        if !valid_limit {
            continue;
        }
        if let Some(existing) = target.get(&model_id) {
            if model.transport.is_none() {
                model.transport = existing.transport;
            }
            if !existing.variants.is_empty() {
                model.variants = existing.variants.clone();
            }
            if model.variant.is_none()
                || model
                    .variant
                    .as_ref()
                    .is_some_and(|variant| !model.variants.contains_key(variant))
            {
                model.variant = existing.variant.clone();
            }
        }
        if is_openai_provider && model.transport.is_none() {
            model.transport = Some(ModelTransport::OpenaiResponses);
        }
        target.insert(model_id, model);
    }
}

/// 解析预设 Base URL：表单占位符展开、覆盖值与特殊域名规则。
///
/// 优先级：`override_url`（非空）> 表单 `baseUrl` > 预设模板 + 表单答案。
pub fn resolve_preset_base_url(
    spec: &PresetProviderSpec,
    form_answers: &BTreeMap<String, String>,
    override_url: Option<&str>,
) -> String {
    if let Some(url) = override_url.map(str::trim).filter(|url| !url.is_empty()) {
        return normalize_base_input(url);
    }
    if let Some(url) = form_answers
        .get("baseUrl")
        .map(String::as_str)
        .map(str::trim)
        .filter(|url| !url.is_empty())
        && !spec.default_base_url.contains("${DATABRICKS_HOST}")
    {
        return normalize_base_input(url);
    }

    let mut base = spec.default_base_url.to_string();

    if spec.id == "gitlab"
        && let Some(instance) = form_answers
            .get("instanceUrl")
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    {
        base = base.replacen("https://gitlab.com", &normalize_base_input(instance), 1);
    }

    if spec.id == "github-copilot"
        && form_answers.get("deploymentType").map(String::as_str) == Some("enterprise")
        && let Some(domain) = form_answers
            .get("enterpriseUrl")
            .map(|value| normalize_host(value))
            .filter(|value| !value.is_empty())
    {
        return format!("https://copilot-api.{domain}");
    }

    for (key, value) in form_answers {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let placeholder = format!("${{{key}}}");
        if base.contains(&placeholder) {
            base = base.replace(&placeholder, value);
        }
        let upper = screaming_snake_case(key);
        let placeholder = format!("${{{upper}}}");
        if base.contains(&placeholder) {
            let replacement = if upper == "DATABRICKS_HOST" {
                normalize_host(value)
            } else {
                value.to_string()
            };
            base = base.replace(&placeholder, &replacement);
        }
    }

    // 已知环境变量占位符与表单 key 不同名，显式展开。
    let aliases: [(&str, &[&str]); 4] = [
        ("accountId", &["CLOUDFLARE_ACCOUNT_ID"]),
        ("gatewayId", &["CLOUDFLARE_GATEWAY_ID"]),
        ("resourceName", &["AZURE_RESOURCE_NAME"]),
        ("productId", &["INFOMANIAK_PRODUCT_ID"]),
    ];
    for (key, names) in aliases {
        let Some(value) = form_answers
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        for name in names {
            base = base.replace(&format!("${{{name}}}"), value);
        }
    }
    if let Some(value) = form_answers
        .get("resourceName")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        base = base.replace("${AZURE_COGNITIVE_SERVICES_RESOURCE_NAME}", value);
    }
    if let Some(value) = form_answers
        .get("account")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        base = base.replace("${SNOWFLAKE_ACCOUNT}", value);
    }
    if let Some(value) = form_answers
        .get("baseUrl")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        base = base.replace("${DATABRICKS_HOST}", &normalize_host(value));
    }

    base.trim_end_matches('/').to_string()
}

/// 补全 scheme 并去掉尾部斜杠。
fn normalize_base_input(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    }
}

/// 去掉 scheme 与路径，仅保留域名/主机。
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

/// `accountId` -> `ACCOUNT_ID`。
fn screaming_snake_case(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for (index, ch) in key.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out
}

struct Catalog {
    presets: &'static [PresetProviderSpec],
    models: RwLock<BTreeMap<String, BTreeMap<String, ModelConfig>>>,
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(build_catalog)
}

fn build_catalog() -> Catalog {
    let raw: RawCatalog = match serde_json::from_str(CATALOG_JSON) {
        Ok(raw) => raw,
        Err(error) => {
            tracing::error!(error = %error, "embedded preset catalog is malformed");
            return Catalog {
                presets: &[],
                models: RwLock::new(BTreeMap::new()),
            };
        }
    };
    let mut presets = Vec::with_capacity(raw.providers.len());
    let mut models = BTreeMap::new();
    for provider in raw.providers {
        let mut provider_models = BTreeMap::new();
        for model in provider.models {
            let entry = ModelConfig {
                id: model.id,
                name: model.name,
                transport: model.transport.as_deref().and_then(ModelTransport::parse),
                limit: model
                    .limit
                    .map(|[context, output]| ModelLimit { context, output }),
                modalities: model
                    .modalities
                    .map(|[input, output]| serde_json::json!({ "input": input, "output": output })),
                variants: model.variants,
                variant: model.variant,
                pricing: None,
                extra: serde_json::Map::new(),
            };
            provider_models.insert(entry.id.clone(), entry);
        }
        models.insert(provider.id.clone(), provider_models);
        presets.push(PresetProviderSpec {
            id: leak(provider.id),
            name: leak(provider.name),
            transport: ProviderTransport::parse(&provider.transport)
                .unwrap_or(ProviderTransport::OpenaiCompatible),
            default_base_url: leak(provider.base_url),
            note: leak(provider.note),
            env_vars: leak_slice(
                provider
                    .env
                    .into_iter()
                    .map(leak)
                    .collect::<Vec<&'static str>>(),
            ),
            methods: leak_slice(
                provider
                    .methods
                    .into_iter()
                    .map(convert_method)
                    .collect::<Vec<PresetAuthMethod>>(),
            ),
        });
    }
    presets.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.id.cmp(right.id))
    });
    // 磁盘缓存（上次云端同步结果）叠加在嵌入目录之上；单元测试不得依赖开发机缓存。
    #[cfg(not(test))]
    for (provider_id, cached_models) in super::catalog_refresh::cached_catalog_models() {
        let is_openai = provider_id == "openai";
        merge_provider_models(
            models.entry(provider_id).or_default(),
            cached_models,
            is_openai,
        );
    }
    Catalog {
        presets: leak_slice(presets),
        models: RwLock::new(models),
    }
}

fn convert_method(raw: RawMethod) -> PresetAuthMethod {
    match raw {
        RawMethod::ApiKey { label, form } => PresetAuthMethod::ApiKey {
            label: leak(label),
            form: convert_form(form),
        },
        RawMethod::OAuth {
            id,
            label,
            flow,
            form,
            base_url_override,
        } => PresetAuthMethod::OAuth {
            id: leak(id),
            label: leak(label),
            flow: convert_flow(flow),
            form: convert_form(form),
            base_url_override: base_url_override.map(leak),
        },
    }
}

fn convert_flow(raw: RawFlow) -> OAuthFlowKind {
    match raw {
        RawFlow::DeviceCode {
            device_code_url,
            token_url,
            client_id,
            scope,
        } => OAuthFlowKind::DeviceCode {
            device_code_url: leak(device_code_url),
            token_url: leak(token_url),
            client_id: leak(client_id),
            scope: leak(scope),
        },
        RawFlow::BrowserPkce {
            authorize_url,
            token_url,
            client_id,
            scope,
            fixed_port,
        } => OAuthFlowKind::BrowserPkce {
            authorize_url: leak(authorize_url),
            token_url: leak(token_url),
            client_id: leak(client_id),
            scope: leak(scope),
            fixed_port,
        },
        RawFlow::AuthorizationCode {
            authorize_url,
            token_url,
            client_id,
            scope,
        } => OAuthFlowKind::AuthorizationCode {
            authorize_url: leak(authorize_url),
            token_url: leak(token_url),
            client_id: leak(client_id),
            scope: leak(scope),
        },
    }
}

fn convert_form(raw: Vec<RawField>) -> &'static [PresetFormField] {
    leak_slice(
        raw.into_iter()
            .map(|field| PresetFormField {
                key: leak(field.key),
                label: leak(field.label),
                placeholder: leak(field.placeholder),
                required: field.required,
                options: leak_slice(
                    field
                        .options
                        .into_iter()
                        .map(|option| PresetFormOption {
                            label: leak(option.label),
                            value: leak(option.value),
                            description: leak(option.description),
                        })
                        .collect::<Vec<PresetFormOption>>(),
                ),
                when_eq: field.when_eq.map(|[key, value]| (leak(key), leak(value))),
            })
            .collect::<Vec<PresetFormField>>(),
    )
}

fn leak(value: String) -> &'static str {
    Box::leak(value.into_boxed_str())
}

fn leak_slice<T>(values: Vec<T>) -> &'static [T] {
    Box::leak(values.into_boxed_slice())
}

#[derive(Deserialize)]
struct RawCatalog {
    providers: Vec<RawProvider>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProvider {
    id: String,
    name: String,
    transport: String,
    base_url: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    methods: Vec<RawMethod>,
    models: Vec<RawModel>,
}

/// 模型条目使用紧凑键压缩嵌入体积：
/// `i`=id、`n`=name、`t`=transport、`l`=`[context, output]`、
/// `m`=`[input, output]`、`v`=variants、`d`=default variant。
#[derive(Deserialize)]
struct RawModel {
    #[serde(rename = "i")]
    id: String,
    #[serde(rename = "n")]
    name: String,
    #[serde(rename = "t")]
    #[serde(default)]
    transport: Option<String>,
    #[serde(rename = "l")]
    #[serde(default)]
    limit: Option<[u64; 2]>,
    #[serde(rename = "m")]
    #[serde(default)]
    modalities: Option<[Vec<String>; 2]>,
    #[serde(rename = "v")]
    #[serde(default)]
    variants: BTreeMap<String, Value>,
    #[serde(rename = "d")]
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RawMethod {
    #[serde(rename = "apiKey")]
    ApiKey {
        label: String,
        #[serde(default)]
        form: Vec<RawField>,
    },
    #[serde(rename = "oauth")]
    OAuth {
        id: String,
        label: String,
        flow: RawFlow,
        #[serde(default)]
        form: Vec<RawField>,
        #[serde(default)]
        base_url_override: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind")]
enum RawFlow {
    #[serde(rename = "deviceCode", rename_all = "camelCase")]
    DeviceCode {
        device_code_url: String,
        token_url: String,
        client_id: String,
        #[serde(default)]
        scope: String,
    },
    #[serde(rename = "browserPkce", rename_all = "camelCase")]
    BrowserPkce {
        authorize_url: String,
        token_url: String,
        client_id: String,
        #[serde(default)]
        scope: String,
        #[serde(default)]
        fixed_port: Option<u16>,
    },
    #[serde(rename = "authorizationCode", rename_all = "camelCase")]
    AuthorizationCode {
        authorize_url: String,
        token_url: String,
        client_id: String,
        #[serde(default)]
        scope: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawField {
    key: String,
    label: String,
    #[serde(default)]
    placeholder: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    options: Vec<RawOption>,
    #[serde(default)]
    when_eq: Option<[String; 2]>,
}

#[derive(Deserialize)]
struct RawOption {
    label: String,
    value: String,
    #[serde(default)]
    description: String,
}

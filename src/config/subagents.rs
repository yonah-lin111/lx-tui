//! Subagent 角色与并发配置（`config/subagents.json`）。
//!
//! 与 `skills.json` / `mcp.json` 同一套约定：缺失文件返回默认配置，损坏文件报错，
//! 保存走临时文件 → 0600 权限 → 原子 `rename`。

use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::ai::ModelSelection;
use super::{ConfigError, user_data_root};

/// 默认整棵根会话树最大并发子 agent 数。
pub const DEFAULT_MAX_CONCURRENT_SUBAGENTS: usize = 4;
/// 整棵根会话树最大并发子 agent 数下限。
pub const MIN_CONCURRENT_SUBAGENTS: usize = 1;
/// 整棵根会话树最大并发子 agent 数上限。
pub const MAX_CONCURRENT_SUBAGENTS_LIMIT: usize = 12;
/// 允许配置的最小 Agent 递归深度（1 = 仅主 Agent 可派发子 Agent）。
pub const MIN_AGENT_DEPTH: usize = 1;
/// 默认 Agent 递归深度（2 = 主 Agent -> 子 Agent -> 孙 Agent）。
pub const DEFAULT_MAX_AGENT_DEPTH: usize = 2;
/// 允许配置的最大 Agent 递归深度上限。
pub const MAX_AGENT_DEPTH_LIMIT: usize = 3;
/// 单个子 agent 单轮 Token 预算的最小值。
pub const MIN_SUBAGENT_MAX_TOKENS: u64 = 1_000;
/// 角色名最大长度（字节/ASCII 字符）。
pub const MAX_ROLE_NAME_LEN: usize = 32;
/// 角色简短描述最大字符数。
pub const MAX_ROLE_DESCRIPTION_CHARS: usize = 500;
/// 角色系统提示词最大字符数。
pub const MAX_ROLE_PROMPT_CHARS: usize = 16_000;
/// 单角色自定义昵称池最大条目数。
pub const MAX_NICKNAME_CANDIDATES: usize = 32;
/// 单个昵称最大字符数。
pub const MAX_NICKNAME_CHARS: usize = 24;

/// 内置 `explorer` 默认简短英文描述。
pub const DEFAULT_EXPLORER_DESCRIPTION: &str =
    "Read-only codebase and image investigator (`read`, `search`, `web_search`).";
/// 内置 `executor` 默认简短英文描述。
pub const DEFAULT_EXECUTOR_DESCRIPTION: &str =
    "Bounded implementation worker with file and process tools (`edit`, `write`, `exec_command`).";

const fn default_max_concurrent() -> usize {
    DEFAULT_MAX_CONCURRENT_SUBAGENTS
}

const fn default_max_depth() -> usize {
    DEFAULT_MAX_AGENT_DEPTH
}

/// 子 agent 的底层安全权限档位（决定工具白名单与默认内置系统提示词）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BaseRole {
    /// 只读调查：`read` / `search` / `web_search`（及只读 MCP）。
    Explorer,
    /// 有边界的实现：`read` / `search` / `web_search` / `edit` / `write` / `exec_command` / `write_stdin`。
    Executor,
}

impl BaseRole {
    /// 全部内置基础权限档位（固定顺序）。
    pub const ALL: [BaseRole; 2] = [Self::Explorer, Self::Executor];

    /// 稳定协议/存储值：`explorer` / `executor`。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Executor => "executor",
        }
    }

    /// 解析稳定值；仅接受 `"explorer"` 或 `"executor"`。
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "explorer" => Some(Self::Explorer),
            "executor" => Some(Self::Executor),
            _ => None,
        }
    }

    /// 该基础权限档位的内置简短英文描述。
    pub const fn default_description(self) -> &'static str {
        match self {
            Self::Explorer => DEFAULT_EXPLORER_DESCRIPTION,
            Self::Executor => DEFAULT_EXECUTOR_DESCRIPTION,
        }
    }
}

/// 校验角色名语法：`1..=32` 字符，首字符为小写字母 `a-z`，其余为 `a-z`、`0-9` 或 `_`。
pub fn is_valid_role_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_ROLE_NAME_LEN || !bytes[0].is_ascii_lowercase() {
        return false;
    }
    bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

/// `config/subagents.json` 根节点。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentsConfigFile {
    #[serde(default)]
    pub subagents: SubagentsSection,
}

/// `subagents` 配置段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentsSection {
    /// 整棵根会话树的最大并发运行子 agent 数（`1..=12`，默认 `4`）。
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// 最大递归派发深度（`1..=3`，默认 `2`：主 agent -> 子 agent -> 孙 agent）。
    #[serde(default = "default_max_depth")]
    pub max_depth: usize,
    /// 可选单轮默认 Token 预算上限（None 表示不限）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_max_tokens: Option<u64>,
    /// 内置角色（`explorer` / `executor`）的可选覆写项。
    #[serde(default)]
    pub builtins: BuiltinRoleOverrides,
    /// 用户自定义角色列表。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<CustomRoleConfig>,
}

impl Default for SubagentsSection {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_MAX_CONCURRENT_SUBAGENTS,
            max_depth: DEFAULT_MAX_AGENT_DEPTH,
            default_max_tokens: None,
            builtins: BuiltinRoleOverrides::default(),
            roles: Vec::new(),
        }
    }
}

/// 两个内置角色的可选覆写配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltinRoleOverrides {
    #[serde(default)]
    pub explorer: RoleOverrideConfig,
    #[serde(default)]
    pub executor: RoleOverrideConfig,
}

/// 内置角色的可选覆写字段（空值表示使用内置默认或继承父模型）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleOverrideConfig {
    /// 覆写角色简短描述（None 表示使用内置默认描述）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 覆写系统提示词（None 表示使用内置默认系统提示词）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// 默认模型与思考等级（None 表示继承父会话模型）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSelection>,
    /// 自定义昵称候选列表（空数组表示使用全局默认科学家昵称池）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nickname_candidates: Vec<String>,
}

impl RoleOverrideConfig {
    /// 是否没有任何覆写项。
    pub fn is_empty(&self) -> bool {
        self.description.is_none()
            && self.system_prompt.is_none()
            && self.model.is_none()
            && self.nickname_candidates.is_empty()
    }

    /// 归一化空白字符串为 None，清理空昵称。
    pub fn normalized(&self) -> Self {
        Self {
            description: self
                .description
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            system_prompt: self
                .system_prompt
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            model: self.model.as_ref().and_then(normalize_model_selection),
            nickname_candidates: normalize_nicknames(&self.nickname_candidates),
        }
    }
}

/// 用户自定义角色配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomRoleConfig {
    /// 唯一角色名（`[a-z][a-z0-9_]{0,31}`，禁止与 `"explorer"` / `"executor"` 重名）。
    pub name: String,
    /// 基础权限档位：`explorer`（只读）或 `executor`（读写）。
    pub base_role: BaseRole,
    /// 必填角色描述（注入 `spawn_agent` 工具说明）。
    pub description: String,
    /// 必填角色系统提示词。
    pub system_prompt: String,
    /// 可选默认模型与思考等级。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSelection>,
    /// 可选专属昵称池。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nickname_candidates: Vec<String>,
}

impl CustomRoleConfig {
    /// 归一化首尾空白与可选字段。
    pub fn normalized(&self) -> Self {
        Self {
            name: self.name.trim().to_string(),
            base_role: self.base_role,
            description: self.description.trim().to_string(),
            system_prompt: self.system_prompt.trim().to_string(),
            model: self.model.as_ref().and_then(normalize_model_selection),
            nickname_candidates: normalize_nicknames(&self.nickname_candidates),
        }
    }
}

/// 解析后的生效角色规格（供运行时 `spawn_agent` 与工具描述使用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoleSpec {
    pub name: String,
    pub base_role: BaseRole,
    pub is_builtin: bool,
    pub description: String,
    /// 角色覆写或自定义系统提示词（None 表示使用 `base_role` 的内置默认提示词）。
    pub system_prompt_override: Option<String>,
    pub default_model: Option<ModelSelection>,
    pub nickname_candidates: Vec<String>,
}

impl SubagentsSection {
    /// 归一化全部字段。
    pub fn normalized(&self) -> Self {
        Self {
            max_concurrent: self.max_concurrent,
            max_depth: self.max_depth,
            default_max_tokens: self.default_max_tokens,
            builtins: BuiltinRoleOverrides {
                explorer: self.builtins.explorer.normalized(),
                executor: self.builtins.executor.normalized(),
            },
            roles: self
                .roles
                .iter()
                .map(CustomRoleConfig::normalized)
                .collect(),
        }
    }

    /// 校验配置合法性。
    pub fn validate(&self) -> Result<(), SubagentsConfigError> {
        if !(MIN_CONCURRENT_SUBAGENTS..=MAX_CONCURRENT_SUBAGENTS_LIMIT)
            .contains(&self.max_concurrent)
        {
            return Err(SubagentsConfigError::Invalid(format!(
                "maxConcurrent must be between {MIN_CONCURRENT_SUBAGENTS} and {MAX_CONCURRENT_SUBAGENTS_LIMIT}"
            )));
        }
        if !(MIN_AGENT_DEPTH..=MAX_AGENT_DEPTH_LIMIT).contains(&self.max_depth) {
            return Err(SubagentsConfigError::Invalid(format!(
                "maxDepth must be between {MIN_AGENT_DEPTH} and {MAX_AGENT_DEPTH_LIMIT}"
            )));
        }
        if let Some(tokens) = self.default_max_tokens
            && tokens < MIN_SUBAGENT_MAX_TOKENS
        {
            return Err(SubagentsConfigError::Invalid(format!(
                "defaultMaxTokens must be at least {MIN_SUBAGENT_MAX_TOKENS}"
            )));
        }
        validate_override("explorer", &self.builtins.explorer)?;
        validate_override("executor", &self.builtins.executor)?;

        let mut seen = HashSet::new();
        for role in &self.roles {
            let name = role.name.trim();
            if name == "explorer" || name == "executor" {
                return Err(SubagentsConfigError::Invalid(format!(
                    "custom role name '{name}' is reserved for built-in roles"
                )));
            }
            if !is_valid_role_name(name) {
                return Err(SubagentsConfigError::Invalid(format!(
                    "custom role name '{name}' must match [a-z][a-z0-9_]{{0,31}}"
                )));
            }
            if !seen.insert(name.to_string()) {
                return Err(SubagentsConfigError::Invalid(format!(
                    "duplicate custom role name '{name}'"
                )));
            }
            let desc = role.description.trim();
            if desc.is_empty() {
                return Err(SubagentsConfigError::Invalid(format!(
                    "custom role '{name}' description must not be empty"
                )));
            }
            if desc.chars().count() > MAX_ROLE_DESCRIPTION_CHARS {
                return Err(SubagentsConfigError::Invalid(format!(
                    "custom role '{name}' description must be at most {MAX_ROLE_DESCRIPTION_CHARS} characters"
                )));
            }
            let prompt = role.system_prompt.trim();
            if prompt.is_empty() {
                return Err(SubagentsConfigError::Invalid(format!(
                    "custom role '{name}' systemPrompt must not be empty"
                )));
            }
            if prompt.chars().count() > MAX_ROLE_PROMPT_CHARS {
                return Err(SubagentsConfigError::Invalid(format!(
                    "custom role '{name}' systemPrompt must be at most {MAX_ROLE_PROMPT_CHARS} characters"
                )));
            }
            validate_model_selection(name, role.model.as_ref())?;
            validate_nicknames(name, &role.nickname_candidates)?;
        }
        Ok(())
    }

    /// 按角色名解析当前配置下的生效规格（内置 `explorer`/`executor` 或已配置的自定义角色）。
    pub fn resolve_role(&self, name: &str) -> Option<ResolvedRoleSpec> {
        match name.trim() {
            "explorer" => {
                let ov = self.builtins.explorer.normalized();
                Some(ResolvedRoleSpec {
                    name: "explorer".to_string(),
                    base_role: BaseRole::Explorer,
                    is_builtin: true,
                    description: ov
                        .description
                        .unwrap_or_else(|| BaseRole::Explorer.default_description().to_string()),
                    system_prompt_override: ov.system_prompt,
                    default_model: ov.model,
                    nickname_candidates: ov.nickname_candidates,
                })
            }
            "executor" => {
                let ov = self.builtins.executor.normalized();
                Some(ResolvedRoleSpec {
                    name: "executor".to_string(),
                    base_role: BaseRole::Executor,
                    is_builtin: true,
                    description: ov
                        .description
                        .unwrap_or_else(|| BaseRole::Executor.default_description().to_string()),
                    system_prompt_override: ov.system_prompt,
                    default_model: ov.model,
                    nickname_candidates: ov.nickname_candidates,
                })
            }
            custom_name => self
                .roles
                .iter()
                .find(|r| r.name.trim() == custom_name)
                .map(|r| {
                    let norm = r.normalized();
                    ResolvedRoleSpec {
                        name: norm.name,
                        base_role: norm.base_role,
                        is_builtin: false,
                        description: norm.description,
                        system_prompt_override: Some(norm.system_prompt),
                        default_model: norm.model,
                        nickname_candidates: norm.nickname_candidates,
                    }
                }),
        }
    }

    /// 按固定顺序返回所有生效角色（`explorer`、`executor` + 自定义角色列表）。
    pub fn all_roles(&self) -> Vec<ResolvedRoleSpec> {
        let mut out = Vec::with_capacity(2 + self.roles.len());
        if let Some(exp) = self.resolve_role("explorer") {
            out.push(exp);
        }
        if let Some(exe) = self.resolve_role("executor") {
            out.push(exe);
        }
        for custom in &self.roles {
            if let Some(resolved) = self.resolve_role(&custom.name) {
                out.push(resolved);
            }
        }
        out
    }
}

fn normalize_model_selection(sel: &ModelSelection) -> Option<ModelSelection> {
    let provider = sel.provider.trim();
    let model = sel.model.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    let variant = sel
        .variant
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(ModelSelection {
        provider: provider.to_string(),
        model: model.to_string(),
        variant,
    })
}

fn normalize_nicknames(list: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for raw in list {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(trimmed))
        {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn validate_override(role_name: &str, ov: &RoleOverrideConfig) -> Result<(), SubagentsConfigError> {
    if let Some(desc) = &ov.description
        && desc.trim().chars().count() > MAX_ROLE_DESCRIPTION_CHARS
    {
        return Err(SubagentsConfigError::Invalid(format!(
            "role '{role_name}' description must be at most {MAX_ROLE_DESCRIPTION_CHARS} characters"
        )));
    }
    if let Some(prompt) = &ov.system_prompt
        && prompt.trim().chars().count() > MAX_ROLE_PROMPT_CHARS
    {
        return Err(SubagentsConfigError::Invalid(format!(
            "role '{role_name}' systemPrompt must be at most {MAX_ROLE_PROMPT_CHARS} characters"
        )));
    }
    validate_model_selection(role_name, ov.model.as_ref())?;
    validate_nicknames(role_name, &ov.nickname_candidates)?;
    Ok(())
}

fn validate_model_selection(
    role_name: &str,
    model: Option<&ModelSelection>,
) -> Result<(), SubagentsConfigError> {
    let Some(sel) = model else {
        return Ok(());
    };
    if sel.provider.trim().is_empty() || sel.model.trim().is_empty() {
        return Err(SubagentsConfigError::Invalid(format!(
            "role '{role_name}' model selection requires non-empty provider and model"
        )));
    }
    Ok(())
}

fn validate_nicknames(role_name: &str, list: &[String]) -> Result<(), SubagentsConfigError> {
    if list.len() > MAX_NICKNAME_CANDIDATES {
        return Err(SubagentsConfigError::Invalid(format!(
            "role '{role_name}' nicknameCandidates must contain at most {MAX_NICKNAME_CANDIDATES} entries"
        )));
    }
    for item in list {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            return Err(SubagentsConfigError::Invalid(format!(
                "role '{role_name}' nicknameCandidates must not contain empty strings"
            )));
        }
        if trimmed.chars().count() > MAX_NICKNAME_CHARS {
            return Err(SubagentsConfigError::Invalid(format!(
                "role '{role_name}' nickname '{trimmed}' exceeds {MAX_NICKNAME_CHARS} characters"
            )));
        }
        if trimmed.chars().any(|c| c == ',' || c.is_control()) {
            return Err(SubagentsConfigError::Invalid(format!(
                "role '{role_name}' nickname '{trimmed}' must not contain commas or control characters"
            )));
        }
    }
    Ok(())
}

/// Subagents 配置错误。
#[derive(Debug)]
pub enum SubagentsConfigError {
    HomeMissing,
    Io(std::io::Error),
    Invalid(String),
}

impl fmt::Display for SubagentsConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid subagents config: {message}"),
        }
    }
}

impl std::error::Error for SubagentsConfigError {}

impl From<std::io::Error> for SubagentsConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ConfigError> for SubagentsConfigError {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::HomeMissing => Self::HomeMissing,
            ConfigError::Io(error) => Self::Io(error),
            ConfigError::Invalid(message) => Self::Invalid(message),
        }
    }
}

/// `<data>/config/subagents.json` 路径。
pub fn subagents_config_path() -> Result<PathBuf, SubagentsConfigError> {
    Ok(user_data_root()?.join("config").join("subagents.json"))
}

/// 读取配置；文件不存在返回 `Ok(None)`，损坏或校验失败返回 `Err`。
pub fn load_subagents(path: &Path) -> Result<Option<SubagentsConfigFile>, SubagentsConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SubagentsConfigError::Io(error)),
    };
    parse_subagents(&text).map(Some)
}

/// 读取生效的 `SubagentsSection`；文件不存在或出错时回退默认配置。
pub fn load_subagents_or_default() -> SubagentsSection {
    let Ok(path) = subagents_config_path() else {
        return SubagentsSection::default();
    };
    match load_subagents(&path) {
        Ok(Some(file)) => file.subagents,
        Ok(None) => SubagentsSection::default(),
        Err(error) => {
            tracing::warn!(error = %error, "failed to load subagents config, falling back to defaults");
            SubagentsSection::default()
        }
    }
}

/// 解析并校验配置文本。
pub fn parse_subagents(text: &str) -> Result<SubagentsConfigFile, SubagentsConfigError> {
    let mut parsed: SubagentsConfigFile = serde_json::from_str(text)
        .map_err(|error| SubagentsConfigError::Invalid(format!("malformed json: {error}")))?;
    parsed.subagents = parsed.subagents.normalized();
    parsed.subagents.validate()?;
    Ok(parsed)
}

/// 原子保存：先归一化并校验，再写临时文件 → 0600 → `rename`。
pub fn save_subagents(path: &Path, file: &SubagentsConfigFile) -> Result<(), SubagentsConfigError> {
    file.subagents.validate()?;
    let normalized = SubagentsConfigFile {
        subagents: file.subagents.normalized(),
    };
    normalized.subagents.validate()?;
    let body = serde_json::to_string_pretty(&normalized)
        .map_err(|error| SubagentsConfigError::Invalid(format!("serialize failed: {error}")))?;
    let body = format!("{body}\n");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        crate::platform::restrict_dir_permissions(parent)?;
    }
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_extension(format!("json.tmp.{}.{}", std::process::id(), sequence));
    std::fs::write(&temp, body.as_bytes())?;
    crate::platform::restrict_file_permissions(&temp)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(SubagentsConfigError::Io(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

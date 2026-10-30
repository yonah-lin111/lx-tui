//! MCP 配置（`config/mcp.json` 与项目级 `config/mcp/projects/<scope>.json`）。
//!
//! 与 `ai.json` 同一套约定：缺失文件返回 None，损坏文件报错（不静默覆盖），
//! 保存走临时文件 + 原子 rename，Unix 权限 0600、目录 0700。

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ConfigError;

/// 默认启动超时（秒）。
pub const DEFAULT_STARTUP_TIMEOUT_SEC: u64 = 30;
/// 默认单次工具调用超时（秒）。
pub const DEFAULT_TOOL_TIMEOUT_SEC: u64 = 300;
/// 超时上限：防止配置写成天级导致轮次永远挂起。
pub const MAX_TIMEOUT_SEC: u64 = 3_600;
/// server 名上限：给 `mcp__<server>__<tool>` 留出 64 字符工具名空间。
pub const MAX_SERVER_NAME_LEN: usize = 32;

/// MCP 配置文件根。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpConfigFile {
    /// 仅项目级文件携带；用户级为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_root: Option<PathBuf>,
    pub mcp: McpSection,
}

/// `mcp` 节点。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpSection {
    /// BTreeMap：序列化键序稳定（保存后 diff 可读）。
    #[serde(default)]
    pub servers: BTreeMap<String, McpServerConfig>,
}

/// 单个 MCP server 的配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(flatten)]
    pub transport: McpTransportConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_timeout_sec: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_timeout_sec: Option<u64>,
    /// None = 不限制；Some(列表) = 只注册列表内工具。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled_tools: Option<Vec<String>>,
    /// 在 `enabled_tools` 之后再剔除。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_tools: Vec<String>,
}

/// 传输配置；`transport` 字段决定变体。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpTransportConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<PathBuf>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        env_vars: Vec<String>,
    },
    Http {
        url: String,
        #[serde(default)]
        auth: McpHttpAuth,
        #[serde(default)]
        bearer_token: Option<String>,
        #[serde(default)]
        bearer_token_env_var: Option<String>,
        #[serde(default)]
        http_headers: BTreeMap<String, String>,
        #[serde(default)]
        env_http_headers: BTreeMap<String, String>,
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default)]
        client_id: Option<String>,
        #[serde(default)]
        client_secret: Option<String>,
    },
}

/// HTTP 认证方式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpHttpAuth {
    /// 不带认证头（或只靠 `http_headers`）。
    #[default]
    None,
    /// `Authorization: Bearer <bearer_token | env(bearer_token_env_var)>`。
    Bearer,
    /// OAuth（token 来自 `credentials.json`）。
    Oauth,
}

/// 配置作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum McpScope {
    User,
    Project,
}

impl McpScope {
    /// 稳定字符串值（设置页路由与凭据 key 共用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Project => "project",
        }
    }
}

/// 合并后的生效 server。
#[derive(Debug, Clone, PartialEq)]
pub struct EffectiveServer {
    pub name: String,
    pub scope: McpScope,
    pub config: McpServerConfig,
    /// 被项目级同名条目覆盖的用户级条目（设置页标注用）。
    pub shadowed_user: bool,
}

/// MCP 配置错误。
#[derive(Debug)]
pub enum McpConfigError {
    HomeMissing,
    Io(std::io::Error),
    Invalid(String),
}

impl fmt::Display for McpConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid mcp config: {message}"),
        }
    }
}

impl std::error::Error for McpConfigError {}

impl From<std::io::Error> for McpConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ConfigError> for McpConfigError {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::HomeMissing => Self::HomeMissing,
            ConfigError::Io(error) => Self::Io(error),
            ConfigError::Invalid(message) => Self::Invalid(message),
        }
    }
}

fn default_true() -> bool {
    true
}

impl McpServerConfig {
    /// 校验 server 名与传输参数；错误文案英文（会出现在设置页与 toast）。
    pub fn validate(&self, name: &str) -> Result<(), McpConfigError> {
        validate_server_name(name)?;
        if let Some(timeout) = self.startup_timeout_sec {
            validate_timeout("startup_timeout_sec", timeout)?;
        }
        if let Some(timeout) = self.tool_timeout_sec {
            validate_timeout("tool_timeout_sec", timeout)?;
        }
        if let Some(enabled) = &self.enabled_tools {
            let enabled: BTreeSet<&String> = enabled.iter().collect();
            if let Some(conflict) = self
                .disabled_tools
                .iter()
                .find(|tool| enabled.contains(tool))
            {
                return Err(McpConfigError::Invalid(format!(
                    "tool '{conflict}' is both enabled and disabled"
                )));
            }
        }
        match &self.transport {
            McpTransportConfig::Stdio {
                command, env_vars, ..
            } => {
                if command.trim().is_empty() {
                    return Err(McpConfigError::Invalid("command must not be empty".into()));
                }
                for variable in env_vars {
                    validate_env_name(variable)?;
                }
            }
            McpTransportConfig::Http {
                url,
                auth,
                bearer_token,
                bearer_token_env_var,
                http_headers,
                env_http_headers,
                ..
            } => {
                validate_http_url(url)?;
                match auth {
                    McpHttpAuth::Bearer => {
                        if bearer_token.is_some() == bearer_token_env_var.is_some() {
                            return Err(McpConfigError::Invalid(
                                "bearer auth needs exactly one of bearer_token or bearer_token_env_var"
                                    .into(),
                            ));
                        }
                    }
                    McpHttpAuth::None | McpHttpAuth::Oauth => {
                        if bearer_token.is_some() || bearer_token_env_var.is_some() {
                            return Err(McpConfigError::Invalid(
                                "bearer_token is only valid with auth = \"bearer\"".into(),
                            ));
                        }
                    }
                }
                if let Some(variable) = bearer_token_env_var {
                    validate_env_name(variable)?;
                }
                for name in http_headers.keys() {
                    validate_header_name(name)?;
                }
                for (name, variable) in env_http_headers {
                    validate_header_name(name)?;
                    validate_env_name(variable)?;
                }
            }
        }
        Ok(())
    }

    /// 启动超时（秒）。
    pub fn startup_timeout_sec(&self) -> u64 {
        self.startup_timeout_sec
            .unwrap_or(DEFAULT_STARTUP_TIMEOUT_SEC)
    }

    /// 工具调用超时（秒）。
    pub fn tool_timeout_sec(&self) -> u64 {
        self.tool_timeout_sec.unwrap_or(DEFAULT_TOOL_TIMEOUT_SEC)
    }

    /// 简短传输说明（供 `@` 面板 MCP Tab 次行展示）。
    pub fn summary(&self) -> String {
        match &self.transport {
            McpTransportConfig::Stdio { command, args, .. } => {
                if args.is_empty() {
                    format!("stdio · {command}")
                } else {
                    format!("stdio · {command} {}", args.join(" "))
                }
            }
            McpTransportConfig::Http { url, .. } => format!("http · {url}"),
        }
    }
}

/// server 名规则：1-32 个 `[A-Za-z0-9_-]`，且不含 `__`。
pub fn validate_server_name(name: &str) -> Result<(), McpConfigError> {
    if name.is_empty()
        || name.len() > MAX_SERVER_NAME_LEN
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        || name.contains("__")
    {
        return Err(McpConfigError::Invalid(format!(
            "invalid server name '{name}': use 1-32 of [A-Za-z0-9_-] without '__'"
        )));
    }
    Ok(())
}

fn validate_timeout(field: &str, timeout: u64) -> Result<(), McpConfigError> {
    if !(1..=MAX_TIMEOUT_SEC).contains(&timeout) {
        return Err(McpConfigError::Invalid(format!(
            "{field} must be within 1..={MAX_TIMEOUT_SEC}"
        )));
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), McpConfigError> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    if !valid {
        return Err(McpConfigError::Invalid(format!(
            "invalid environment variable name '{name}'"
        )));
    }
    Ok(())
}

/// RFC 7230 token；同时拒绝手写 Authorization（必须走 auth 字段）。
fn validate_header_name(name: &str) -> Result<(), McpConfigError> {
    let valid = !name.is_empty()
        && name.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(
                    ch,
                    '!' | '#'
                        | '$'
                        | '%'
                        | '&'
                        | '\''
                        | '*'
                        | '+'
                        | '-'
                        | '.'
                        | '^'
                        | '_'
                        | '`'
                        | '|'
                        | '~'
                )
        });
    if !valid {
        return Err(McpConfigError::Invalid(format!(
            "invalid header name '{name}'"
        )));
    }
    if name.eq_ignore_ascii_case("authorization") {
        return Err(McpConfigError::Invalid(
            "set Authorization through auth, not http_headers".into(),
        ));
    }
    Ok(())
}

/// `https://` 任意主机；`http://` 仅允许本机回环。
fn validate_http_url(url: &str) -> Result<(), McpConfigError> {
    if url.chars().any(|ch| (ch as u32) < 0x20 || ch == '\u{7f}') {
        return Err(McpConfigError::Invalid(
            "url contains control characters".into(),
        ));
    }
    let (scheme, rest) = url.split_once("://").ok_or_else(|| {
        McpConfigError::Invalid("url must be https (http only for localhost)".into())
    })?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Err(McpConfigError::Invalid(
            "url must be https (http only for localhost)".into(),
        ));
    }
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.split_once(']').map(|(host, _)| host))
        .unwrap_or_else(|| host.split(':').next().unwrap_or(host));
    match scheme {
        "https" => Ok(()),
        "http" if matches!(host, "127.0.0.1" | "localhost" | "::1") => Ok(()),
        _ => Err(McpConfigError::Invalid(
            "url must be https (http only for localhost)".into(),
        )),
    }
}

/// 用户级 `config/mcp.json`。
pub fn user_mcp_path() -> Result<PathBuf, McpConfigError> {
    Ok(super::user_data_root()?.join("config").join("mcp.json"))
}

/// 项目级 `config/mcp/projects/<scope>.json`。
pub fn project_mcp_path(project_root: &Path) -> Result<PathBuf, McpConfigError> {
    let scope = crate::platform::path_text::scope_dir_name(project_root).ok_or_else(|| {
        McpConfigError::Invalid(format!(
            "cannot derive a scope name from '{}'",
            project_root.display()
        ))
    })?;
    Ok(super::user_data_root()?
        .join("config")
        .join("mcp")
        .join("projects")
        .join(format!("{scope}.json")))
}

/// 读取配置；文件不存在返回 None，损坏返回错误。
pub fn load_mcp(path: &Path) -> Result<Option<McpConfigFile>, McpConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(McpConfigError::Io(error)),
    };
    parse_mcp(&text).map(Some)
}

/// 解析并校验配置文件文本。
pub fn parse_mcp(text: &str) -> Result<McpConfigFile, McpConfigError> {
    #[derive(Deserialize)]
    struct Root {
        #[serde(default)]
        project_root: Option<PathBuf>,
        #[serde(default)]
        mcp: McpSection,
    }
    let root: Root = serde_json::from_str(text)
        .map_err(|error| McpConfigError::Invalid(format!("malformed json: {error}")))?;
    for (name, server) in &root.mcp.servers {
        server.validate(name)?;
    }
    Ok(McpConfigFile {
        project_root: root.project_root,
        mcp: root.mcp,
    })
}

/// 原子保存：校验 → 临时文件 → 0600 → rename。
pub fn save_mcp(path: &Path, file: &McpConfigFile) -> Result<(), McpConfigError> {
    for (name, server) in &file.mcp.servers {
        server.validate(name)?;
    }
    if let Some(project_root) = &file.project_root
        && !project_root.is_absolute()
    {
        return Err(McpConfigError::Invalid(
            "project_root must be an absolute path".into(),
        ));
    }
    #[derive(Serialize)]
    struct Root<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        project_root: Option<&'a Path>,
        mcp: &'a McpSection,
    }
    let body = serde_json::to_string_pretty(&Root {
        project_root: file.project_root.as_deref(),
        mcp: &file.mcp,
    })
    .map_err(|error| McpConfigError::Invalid(format!("serialize failed: {error}")))?;
    let body = format!("{body}\n");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        crate::platform::restrict_dir_permissions(parent)?;
    }
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, body.as_bytes())?;
    crate::platform::restrict_file_permissions(&temp)?;
    if let Err(error) = std::fs::rename(&temp, path) {
        let _ = std::fs::remove_file(&temp);
        return Err(McpConfigError::Io(error));
    }
    Ok(())
}

/// 生效配置：用户级与项目级按 server 名整条合并，项目级覆盖。
///
/// 未信任项目整体忽略项目级配置；项目文件里的 `project_root` 与当前项目不一致
/// （哈希碰撞或手工拷贝）时同样忽略。
pub fn effective_servers(
    user: Option<&McpConfigFile>,
    project: Option<&McpConfigFile>,
    project_trusted: bool,
    project_root: Option<&Path>,
) -> Vec<EffectiveServer> {
    let project = project.filter(|file| {
        project_trusted
            && match (&file.project_root, project_root) {
                (Some(recorded), Some(root)) => recorded == root,
                _ => false,
            }
    });
    let user = user.map(|file| &file.mcp.servers);
    let mut merged: BTreeMap<String, EffectiveServer> = BTreeMap::new();
    if let Some(servers) = user {
        for (name, config) in servers {
            merged.insert(
                name.clone(),
                EffectiveServer {
                    name: name.clone(),
                    scope: McpScope::User,
                    config: config.clone(),
                    shadowed_user: false,
                },
            );
        }
    }
    if let Some(file) = project {
        for (name, config) in &file.mcp.servers {
            let shadowed_user = merged.contains_key(name);
            merged.insert(
                name.clone(),
                EffectiveServer {
                    name: name.clone(),
                    scope: McpScope::Project,
                    config: config.clone(),
                    shadowed_user,
                },
            );
        }
    }
    merged.into_values().collect()
}

#[cfg(test)]
mod tests;

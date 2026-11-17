//! 沙箱与权限模式配置（`config/sandbox.json`）。
//!
//! 与 `subagents.json` 同一套约定：缺失文件返回默认配置，损坏文件报错，
//! 保存走临时文件 → 0600 权限 → 原子 `rename`。

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::{ConfigError, user_data_root};

/// 额外可写根数量上限。
pub const MAX_WRITABLE_ROOTS: usize = 16;

/// 沙箱模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    /// 只读：shell 不能写，文件工具不能写。
    ReadOnly,
    /// 仅工作区可写（默认）。
    #[default]
    WorkspaceWrite,
    /// 无沙箱。
    DangerFullAccess,
}

impl SandboxMode {
    /// 全部模式（固定顺序，供设置页与 `/permission` 使用）。
    pub const ALL: [SandboxMode; 3] =
        [Self::ReadOnly, Self::WorkspaceWrite, Self::DangerFullAccess];

    /// 稳定协议/存储值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    /// 界面简短展示名（用于窗格左上角标题与 `/permission` 候选列表）。
    pub const fn short_label(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read-only",
            Self::WorkspaceWrite => "Workspace",
            Self::DangerFullAccess => "Full",
        }
    }

    /// 解析稳定值；仅接受三个 kebab-case 名称。
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "read-only" => Some(Self::ReadOnly),
            "workspace-write" => Some(Self::WorkspaceWrite),
            "danger-full-access" => Some(Self::DangerFullAccess),
            _ => None,
        }
    }

    /// 该模式是否允许进程级沙箱（`danger-full-access` 不套沙箱）。
    pub const fn process_sandboxed(self) -> bool {
        !matches!(self, Self::DangerFullAccess)
    }
}

impl fmt::Display for SandboxMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// 提权申请总开关。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum EscalationPolicy {
    /// 允许模型申请提权，由用户审批（默认）。
    #[default]
    Ask,
    /// 禁止一切提权申请。
    Never,
}

impl EscalationPolicy {
    /// 稳定协议/存储值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Never => "never",
        }
    }

    /// 解析稳定值。
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "ask" => Some(Self::Ask),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

impl fmt::Display for EscalationPolicy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

const fn default_mode() -> SandboxMode {
    SandboxMode::WorkspaceWrite
}

const fn default_escalation() -> EscalationPolicy {
    EscalationPolicy::Ask
}

/// `config/sandbox.json` 的内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxSettings {
    /// 全局默认模式。
    #[serde(default = "default_mode")]
    pub mode: SandboxMode,
    /// 受限档是否允许网络（默认关闭）。
    #[serde(default)]
    pub network_access: bool,
    /// 提权申请总开关。
    #[serde(default = "default_escalation")]
    pub escalation: EscalationPolicy,
    /// 额外可写根（绝对路径，`~` 在保存/加载时展开）。
    #[serde(default)]
    pub writable_roots: Vec<PathBuf>,
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            network_access: false,
            escalation: default_escalation(),
            writable_roots: Vec::new(),
        }
    }
}

impl SandboxSettings {
    /// 归一化并校验：展开 `~`、要求绝对路径、去重、限长。
    pub fn normalized(mut self) -> Result<Self, SandboxConfigError> {
        self.writable_roots = normalize_roots(&self.writable_roots)?;
        self.validate()?;
        Ok(self)
    }

    /// 校验已归一化的配置。
    pub fn validate(&self) -> Result<(), SandboxConfigError> {
        if self.writable_roots.len() > MAX_WRITABLE_ROOTS {
            return Err(SandboxConfigError::Invalid(format!(
                "writable_roots must contain at most {MAX_WRITABLE_ROOTS} entries"
            )));
        }
        for root in &self.writable_roots {
            if !root.is_absolute() {
                return Err(SandboxConfigError::Invalid(format!(
                    "writable root {} must be an absolute path",
                    root.display()
                )));
            }
            if path_has_control(root) {
                return Err(SandboxConfigError::Invalid(
                    "writable roots must not contain control characters".into(),
                ));
            }
        }
        Ok(())
    }
}

/// 沙箱配置错误。
#[derive(Debug)]
pub enum SandboxConfigError {
    /// HOME 缺失，无法定位配置或展开 `~`。
    HomeMissing,
    /// IO 错误。
    Io(std::io::Error),
    /// JSON 结构或取值非法。
    Invalid(String),
}

impl fmt::Display for SandboxConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid sandbox config: {message}"),
        }
    }
}

impl std::error::Error for SandboxConfigError {}

impl From<std::io::Error> for SandboxConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ConfigError> for SandboxConfigError {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::HomeMissing => Self::HomeMissing,
            ConfigError::Io(error) => Self::Io(error),
            ConfigError::Invalid(message) => Self::Invalid(message),
        }
    }
}

/// `<data>/config/sandbox.json` 路径。
pub fn sandbox_config_path() -> Result<PathBuf, SandboxConfigError> {
    Ok(user_data_root()?.join("config").join("sandbox.json"))
}

/// 读取配置；文件不存在返回 `Ok(None)`，损坏或校验失败返回 `Err`。
pub fn load_sandbox(path: &Path) -> Result<Option<SandboxSettings>, SandboxConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SandboxConfigError::Io(error)),
    };
    parse_sandbox(&text).map(Some)
}

/// 读取生效配置；文件不存在或出错时回退默认值。
pub fn load_sandbox_or_default() -> SandboxSettings {
    let Ok(path) = sandbox_config_path() else {
        return SandboxSettings::default();
    };
    match load_sandbox(&path) {
        Ok(Some(settings)) => settings,
        Ok(None) => SandboxSettings::default(),
        Err(error) => {
            tracing::warn!(error = %error, "failed to load sandbox config, falling back to defaults");
            SandboxSettings::default()
        }
    }
}

/// 解析并校验配置文本。
pub fn parse_sandbox(text: &str) -> Result<SandboxSettings, SandboxConfigError> {
    let parsed: SandboxSettings = serde_json::from_str(text)
        .map_err(|error| SandboxConfigError::Invalid(format!("malformed json: {error}")))?;
    let normalized = parsed.normalized()?;
    normalized.validate()?;
    Ok(normalized)
}

/// 原子保存：先归一化并校验，再写临时文件 → 0600 → `rename`。
pub fn save_sandbox(path: &Path, settings: &SandboxSettings) -> Result<(), SandboxConfigError> {
    let normalized = settings.clone().normalized()?;
    let body = serde_json::to_string_pretty(&normalized)
        .map_err(|error| SandboxConfigError::Invalid(format!("serialize failed: {error}")))?;
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
        return Err(SandboxConfigError::Io(error));
    }
    Ok(())
}

/// 展开 `~`、要求绝对路径、去重（保持首次出现顺序）。
pub fn normalize_roots(roots: &[PathBuf]) -> Result<Vec<PathBuf>, SandboxConfigError> {
    if roots.len() > MAX_WRITABLE_ROOTS {
        return Err(SandboxConfigError::Invalid(format!(
            "writable_roots must contain at most {MAX_WRITABLE_ROOTS} entries"
        )));
    }
    let mut normalized = Vec::with_capacity(roots.len());
    for root in roots {
        let expanded = expand_home(root)?;
        if !expanded.is_absolute() {
            return Err(SandboxConfigError::Invalid(format!(
                "writable root {} must be an absolute path",
                expanded.display()
            )));
        }
        if path_has_control(&expanded) {
            return Err(SandboxConfigError::Invalid(
                "writable roots must not contain control characters".into(),
            ));
        }
        if !normalized.contains(&expanded) {
            normalized.push(expanded);
        }
    }
    Ok(normalized)
}

/// 展开开头的 `~` / `~/...` / `~\...`；其余路径原样返回。
pub fn expand_home(path: &Path) -> Result<PathBuf, SandboxConfigError> {
    let text = path.to_string_lossy();
    let rest = if text == "~" {
        ""
    } else if let Some(rest) = text.strip_prefix("~/") {
        rest
    } else if let Some(rest) = text.strip_prefix("~\\") {
        rest
    } else {
        return Ok(path.to_path_buf());
    };
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|value| !value.is_empty())
        .ok_or(SandboxConfigError::HomeMissing)?;
    let mut expanded = PathBuf::from(home);
    if !rest.is_empty() {
        expanded.push(rest);
    }
    Ok(expanded)
}

fn path_has_control(path: &Path) -> bool {
    path.to_string_lossy().chars().any(char::is_control)
}

#[cfg(test)]
mod tests;

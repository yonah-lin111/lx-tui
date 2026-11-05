//! Skills 启停配置（`config/skills.json`）：按 canonical `SKILL.md` 路径禁用。
//!
//! 与 `mcp.json` 同一套约定：缺失文件返回空列表，损坏文件报错，原子保存 0600。

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ConfigError;

/// 配置文件根。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsConfigFile {
    #[serde(default)]
    pub skills: SkillsSection,
}

/// `skills` 节点。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsSection {
    /// 被禁用的 skill（canonical `SKILL.md` 绝对路径）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<PathBuf>,
    /// 显式启用的 `.agents` skill（canonical `SKILL.md` 绝对路径）；默认不加载。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled: Vec<PathBuf>,
}

impl SkillsConfigFile {
    /// 禁用集合（canonical 路径比较）。
    pub fn disabled_set(&self) -> BTreeSet<PathBuf> {
        self.skills.disabled.iter().cloned().collect()
    }

    /// 显式启用集合（仅 `.agents` 来源使用；默认关闭）。
    pub fn enabled_set(&self) -> BTreeSet<PathBuf> {
        self.skills.enabled.iter().cloned().collect()
    }
}

/// Skills 配置错误。
#[derive(Debug)]
pub enum SkillsConfigError {
    HomeMissing,
    Io(std::io::Error),
    Invalid(String),
}

impl fmt::Display for SkillsConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeMissing => write!(formatter, "home directory unavailable"),
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid skills config: {message}"),
        }
    }
}

impl std::error::Error for SkillsConfigError {}

impl From<std::io::Error> for SkillsConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ConfigError> for SkillsConfigError {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::HomeMissing => Self::HomeMissing,
            ConfigError::Io(error) => Self::Io(error),
            ConfigError::Invalid(message) => Self::Invalid(message),
        }
    }
}

/// 用户级 skill 根：`<data>/skills/`（排除 `projects/` 子目录）。
pub fn user_skills_root() -> Result<PathBuf, SkillsConfigError> {
    Ok(super::user_data_root()?.join("skills"))
}

/// 用户级 `.agents` skill 根：`~/.agents/skills/`。
pub fn user_agents_skills_root() -> Result<PathBuf, SkillsConfigError> {
    #[cfg(test)]
    if let Some(home) = crate::platform::test_home_override() {
        return Ok(home.join(".agents").join("skills"));
    }
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or(SkillsConfigError::HomeMissing)?;
    Ok(home.join(".agents").join("skills"))
}

/// 项目级 lx skill 根：`<data>/skills/projects/<scope>/`。
pub fn project_skills_root(project_root: &Path) -> Result<PathBuf, SkillsConfigError> {
    let scope = crate::platform::path_text::scope_dir_name(project_root).ok_or_else(|| {
        SkillsConfigError::Invalid(format!(
            "cannot derive a scope name from '{}'",
            project_root.display()
        ))
    })?;
    Ok(user_skills_root()?.join("projects").join(scope))
}

/// 项目级 `.agents` skill 根：`<项目根>/.agents/skills/`。
pub fn project_agents_skills_root(project_root: &Path) -> PathBuf {
    project_root.join(".agents").join("skills")
}

/// `config/skills.json` 路径。
pub fn skills_config_path() -> Result<PathBuf, SkillsConfigError> {
    Ok(super::user_data_root()?.join("config").join("skills.json"))
}

/// 读取配置；不存在返回 None，损坏返回错误。
pub fn load_skills(path: &Path) -> Result<Option<SkillsConfigFile>, SkillsConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(SkillsConfigError::Io(error)),
    };
    parse_skills(&text).map(Some)
}

/// 解析配置文本。
pub fn parse_skills(text: &str) -> Result<SkillsConfigFile, SkillsConfigError> {
    serde_json::from_str(text)
        .map_err(|error| SkillsConfigError::Invalid(format!("malformed json: {error}")))
}

/// 原子保存：临时文件 → 0600 → rename。
pub fn save_skills(path: &Path, file: &SkillsConfigFile) -> Result<(), SkillsConfigError> {
    let body = serde_json::to_string_pretty(file)
        .map_err(|error| SkillsConfigError::Invalid(format!("serialize failed: {error}")))?;
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
        return Err(SkillsConfigError::Io(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

//! 项目级子 agent 角色自动发现（`<workspace>/.lx/agents/*.json` 与 `<workspace>/.agents/roles/*.json`）。
//!
//! 安全边界（对齐 `src/skills/discover.rs`）：
//! 1. 仅在工作区已获信任（`workspace_trusted == true`）时扫描项目目录；
//! 2. 拒绝符号链接文件与隐藏文件，单文件大小上限 64 KiB，单工作区最多加载 32 个项目角色；
//! 3. 禁止覆盖内置角色名（`explorer` / `executor` / `monitor`）或全局 `subagents.json` 中已定义的自定义角色；
//! 4. 损坏或不合法的单文件仅记录 `tracing::warn!` 并跳过，不中断会话。

use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;

use crate::config::ai::ModelSelection;

use super::{
    BaseRole, CustomRoleConfig, MAX_ROLE_DESCRIPTION_CHARS, MAX_ROLE_PROMPT_CHARS,
    RoleCapabilities, SubagentsSection, is_valid_role_name, normalize_model_selection,
    normalize_nicknames, validate_model_selection, validate_nicknames,
};

/// 单个项目角色 JSON 文件的最大字节数（64 KiB）。
pub const MAX_ROLE_FILE_BYTES: u64 = 64 * 1024;
/// 单个工作区最多发现的项目级角色数。
pub const MAX_PROJECT_ROLES: usize = 32;

/// 项目级角色 JSON 反序列化载体（允许省略 `name`，省略时使用文件名词干）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectRoleFile {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    base_role: BaseRole,
    description: String,
    system_prompt: String,
    #[serde(default)]
    model: Option<ModelSelection>,
    #[serde(default)]
    lock_model: bool,
    #[serde(default, flatten)]
    capabilities: RoleCapabilities,
    #[serde(default)]
    nickname_candidates: Vec<String>,
}

/// 扫描受信任工作区下的 `.lx/agents/*.json` 与 `.agents/roles/*.json`。
pub fn discover_project_roles(workspace_root: &Path) -> Vec<CustomRoleConfig> {
    let candidate_dirs = [
        workspace_root.join(".lx").join("agents"),
        workspace_root.join(".agents").join("roles"),
    ];
    let mut out = Vec::new();
    let mut seen_names = HashSet::new();

    for dir in &candidate_dirs {
        let Ok(read_dir) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut entries: Vec<_> = read_dir.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            if out.len() >= MAX_PROJECT_ROLES {
                break;
            }
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if file_name.starts_with('.') || !file_name.ends_with(".json") {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.is_file() {
                tracing::warn!(path = %path.display(), "skipping non-regular project role file");
                continue;
            }
            if meta.len() > MAX_ROLE_FILE_BYTES {
                tracing::warn!(
                    path = %path.display(),
                    bytes = meta.len(),
                    "skipping oversized project role file (> 64 KiB)"
                );
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                tracing::warn!(path = %path.display(), "failed to read project role file");
                continue;
            };
            let parsed: ProjectRoleFile = match serde_json::from_str(&raw) {
                Ok(p) => p,
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        %error,
                        "skipping malformed project role json"
                    );
                    continue;
                }
            };
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .trim();
            let raw_name = parsed
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(stem);
            let role = CustomRoleConfig {
                name: raw_name.to_string(),
                base_role: parsed.base_role,
                description: parsed.description.trim().to_string(),
                system_prompt: parsed.system_prompt.trim().to_string(),
                model: parsed.model.as_ref().and_then(normalize_model_selection),
                lock_model: parsed.lock_model,
                capabilities: parsed.capabilities,
                nickname_candidates: normalize_nicknames(&parsed.nickname_candidates),
                is_project_scoped: true,
            };
            if let Err(reason) = validate_single_project_role(&role) {
                tracing::warn!(
                    path = %path.display(),
                    role = %role.name,
                    %reason,
                    "skipping invalid project role"
                );
                continue;
            }
            if !seen_names.insert(role.name.clone()) {
                tracing::warn!(
                    path = %path.display(),
                    role = %role.name,
                    "skipping duplicate project role"
                );
                continue;
            }
            out.push(role);
        }
    }
    out
}

/// 当 `workspace_trusted == true` 时，将工作区发现的项目角色合入基础 `SubagentsSection`
/// （全局 `base.roles` 同名优先，内置角色名永不覆盖）。
pub fn merge_project_roles(
    base: &SubagentsSection,
    workspace: Option<&Path>,
    workspace_trusted: bool,
) -> SubagentsSection {
    let mut merged = base.clone();
    if !workspace_trusted {
        return merged;
    }
    let Some(ws) = workspace else {
        return merged;
    };
    let existing: HashSet<String> = merged
        .roles
        .iter()
        .map(|r| r.name.trim().to_string())
        .collect();
    for role in discover_project_roles(ws) {
        if existing.contains(role.name.as_str()) {
            tracing::warn!(
                role = %role.name,
                "project role shadowed by global custom role in subagents.json"
            );
            continue;
        }
        merged.roles.push(role);
    }
    merged
}

fn validate_single_project_role(role: &CustomRoleConfig) -> Result<(), String> {
    let name = role.name.trim();
    if matches!(name, "explorer" | "executor" | "monitor") {
        return Err(format!("role name '{name}' is reserved for built-in roles"));
    }
    if !is_valid_role_name(name) {
        return Err(format!(
            "role name '{name}' must match [a-z][a-z0-9_]{{0,31}}"
        ));
    }
    if role.description.is_empty() || role.description.chars().count() > MAX_ROLE_DESCRIPTION_CHARS
    {
        return Err("invalid role description length".to_string());
    }
    if role.system_prompt.is_empty() || role.system_prompt.chars().count() > MAX_ROLE_PROMPT_CHARS {
        return Err("invalid role systemPrompt length".to_string());
    }
    validate_model_selection(name, role.model.as_ref()).map_err(|e| e.to_string())?;
    validate_nicknames(name, &role.nickname_candidates).map_err(|e| e.to_string())?;
    Ok(())
}

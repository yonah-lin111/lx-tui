//! Subagents 设置 API：全局并发上限、内置角色覆写/重置与自定义角色增删改查。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;

use super::handlers::{Request, Response};
use super::{ServiceState, SettingsEvent, ai_path};
use crate::chat::agent::{
    DEFAULT_NICKNAME_CANDIDATES, EXECUTOR_PROMPT, EXPLORER_PROMPT, MONITOR_PROMPT,
};
use crate::config::ai::{AiSettings, ModelSelection};
use crate::config::subagents::{
    BaseRole, CustomRoleConfig, DelegationMode, RoleCapabilities, RoleOverrideConfig,
    SubagentsConfigFile, SubagentsSection, load_subagents, save_subagents,
};

/// `/api/subagents` 系列入口。
pub async fn route(request: &Request, state: &ServiceState) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/subagents") => get_subagents(state),
        ("PUT", "/api/subagents") => put_subagents_all(request, state),
        ("PUT", "/api/subagents/settings") => put_settings(request, state),
        ("PUT", "/api/subagents/general") => put_general(request, state),
        ("PUT", "/api/subagents/builtins/explorer") => put_builtin("explorer", request, state),
        ("PUT", "/api/subagents/builtins/executor") => put_builtin("executor", request, state),
        ("PUT", "/api/subagents/builtins/monitor") => put_builtin("monitor", request, state),
        ("DELETE", "/api/subagents/builtins/explorer") => reset_builtin("explorer", state),
        ("DELETE", "/api/subagents/builtins/executor") => reset_builtin("executor", state),
        ("DELETE", "/api/subagents/builtins/monitor") => reset_builtin("monitor", state),
        ("POST", "/api/subagents/roles") => create_role(request, state),
        ("PUT", path) if path.starts_with("/api/subagents/roles/") => {
            let name = path.trim_start_matches("/api/subagents/roles/");
            update_role(name, request, state)
        }
        ("DELETE", path) if path.starts_with("/api/subagents/roles/") => {
            let name = path.trim_start_matches("/api/subagents/roles/");
            delete_role(name, state)
        }
        _ => Response::error(404, "not found"),
    }
}

fn subagents_path(state: &ServiceState) -> Result<PathBuf, String> {
    let root = state
        .user_root()
        .or_else(|| state.config_path().parent().map(Path::to_path_buf))
        .or_else(|| crate::config::user_data_root().ok())
        .ok_or_else(|| "home directory unavailable".to_string())?;
    Ok(root.join("config").join("subagents.json"))
}

fn load_current(state: &ServiceState) -> Result<(PathBuf, SubagentsConfigFile), Response> {
    let path = subagents_path(state).map_err(|err| Response::error(500, &err))?;
    let file = match load_subagents(&path) {
        Ok(Some(file)) => file,
        Ok(None) => SubagentsConfigFile::default(),
        Err(error) => return Err(Response::error(422, &error.to_string())),
    };
    Ok((path, file))
}

fn commit_and_notify(
    path: &Path,
    file: &SubagentsConfigFile,
    state: &ServiceState,
) -> Result<SubagentsSection, Response> {
    // 先校验原始输入，避免非法字段在 `save_subagents` 归一化时被静默丢弃。
    if let Err(error) = save_subagents(path, file) {
        return Err(Response::error(422, &error.to_string()));
    }
    let normalized = SubagentsConfigFile {
        subagents: file.subagents.normalized(),
    };
    let _ = state
        .sender()
        .send(SettingsEvent::SubagentsChanged(Box::new(
            normalized.subagents.clone(),
        )));
    Ok(normalized.subagents)
}

fn available_models_json(state: &ServiceState) -> serde_json::Value {
    let settings = crate::config::ai::load_ai(&ai_path(state))
        .ok()
        .flatten()
        .unwrap_or_else(AiSettings::empty);
    let mut list = Vec::new();
    for prov_id in &settings.enabled_providers {
        let Some(prov) = settings.providers.get(prov_id) else {
            continue;
        };
        let prov_name = if prov.name.trim().is_empty() {
            prov_id.as_str()
        } else {
            prov.name.trim()
        };
        let mut model_ids: Vec<&String> = prov.models.keys().collect();
        model_ids.sort();
        for model_id in model_ids {
            let Some(m) = prov.models.get(model_id) else {
                continue;
            };
            let model_name = if m.name.trim().is_empty() {
                model_id.as_str()
            } else {
                m.name.trim()
            };
            let mut variants: Vec<&String> = m.variants.keys().collect();
            variants.sort();
            list.push(json!({
                "provider": prov_id,
                "model": model_id,
                "label": format!("{prov_name} / {model_name}"),
                "variants": variants,
            }));
        }
    }
    serde_json::Value::Array(list)
}

fn build_response_json(section: &SubagentsSection, state: &ServiceState) -> Response {
    let body = json!({
        "maxConcurrent": section.max_concurrent,
        "maxDepth": section.max_depth,
        "defaultMaxTokens": section.default_max_tokens,
        "delegationMode": section.delegation_mode.as_str(),
        "builtins": section.builtins,
        "roles": section.roles,
        "defaults": {
            "explorer": {
                "baseRole": "explorer",
                "description": BaseRole::Explorer.default_description(),
                "systemPrompt": EXPLORER_PROMPT,
            },
            "executor": {
                "baseRole": "executor",
                "description": BaseRole::Executor.default_description(),
                "systemPrompt": EXECUTOR_PROMPT,
            },
            "monitor": {
                "baseRole": "monitor",
                "description": BaseRole::Monitor.default_description(),
                "systemPrompt": MONITOR_PROMPT,
            },
            "nicknameCandidates": DEFAULT_NICKNAME_CANDIDATES,
        },
        "models": available_models_json(state),
    });
    Response::json(200, body.to_string())
}

fn get_subagents(state: &ServiceState) -> Response {
    let (_, file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    build_response_json(&file.subagents, state)
}

/// 区分「字段缺省」与「显式 null」：缺省得到 `None`，`null` 得到 `Some(None)`。
fn deserialize_present_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PutAllPayload {
    max_concurrent: usize,
    #[serde(default)]
    max_depth: Option<usize>,
    /// 三态：缺省表示保留原值，`null` 表示清除，数字表示设定上限。
    #[serde(default, deserialize_with = "deserialize_present_option")]
    default_max_tokens: Option<Option<u64>>,
    #[serde(default)]
    delegation_mode: Option<DelegationMode>,
    #[serde(default)]
    builtins: Option<crate::config::subagents::BuiltinRoleOverrides>,
    #[serde(default)]
    roles: Option<Vec<CustomRoleConfig>>,
}

fn put_subagents_all(request: &Request, state: &ServiceState) -> Response {
    let (path, mut file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let payload: PutAllPayload = match serde_json::from_str(&request.body) {
        Ok(p) => p,
        Err(err) => return Response::error(400, &format!("invalid json: {err}")),
    };
    file.subagents.max_concurrent = payload.max_concurrent;
    if let Some(depth) = payload.max_depth {
        file.subagents.max_depth = depth;
    }
    if let Some(tokens) = payload.default_max_tokens {
        file.subagents.default_max_tokens = tokens;
    }
    if let Some(mode) = payload.delegation_mode {
        file.subagents.delegation_mode = mode;
    }
    if let Some(builtins) = payload.builtins {
        file.subagents.builtins = builtins;
    }
    if let Some(roles) = payload.roles {
        file.subagents.roles = roles;
    }
    match commit_and_notify(&path, &file, state) {
        Ok(saved) => build_response_json(&saved, state),
        Err(resp) => resp,
    }
}

/// `PUT /api/subagents/settings`：仅更新全局委派模式，不触碰并发与角色列表。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPayload {
    #[serde(default)]
    delegation_mode: Option<DelegationMode>,
}

fn put_settings(request: &Request, state: &ServiceState) -> Response {
    let (path, mut file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let payload: SettingsPayload = match serde_json::from_str(&request.body) {
        Ok(p) => p,
        Err(err) => return Response::error(400, &format!("invalid json: {err}")),
    };
    if let Some(mode) = payload.delegation_mode {
        file.subagents.delegation_mode = mode;
    }
    match commit_and_notify(&path, &file, state) {
        Ok(saved) => build_response_json(&saved, state),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeneralPayload {
    max_concurrent: usize,
    #[serde(default)]
    max_depth: Option<usize>,
    #[serde(default)]
    default_max_tokens: Option<u64>,
}

fn put_general(request: &Request, state: &ServiceState) -> Response {
    let (path, mut file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let payload: GeneralPayload = match serde_json::from_str(&request.body) {
        Ok(p) => p,
        Err(err) => return Response::error(400, &format!("invalid json: {err}")),
    };
    file.subagents.max_concurrent = payload.max_concurrent;
    if let Some(depth) = payload.max_depth {
        file.subagents.max_depth = depth;
    }
    file.subagents.default_max_tokens = payload.default_max_tokens;
    match commit_and_notify(&path, &file, state) {
        Ok(saved) => build_response_json(&saved, state),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltinOverridePayload {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    system_prompt: Option<String>,
    #[serde(default)]
    model: Option<ModelSelection>,
    #[serde(default)]
    lock_model: bool,
    #[serde(default, flatten)]
    capabilities: RoleCapabilities,
    #[serde(default)]
    nickname_candidates: Vec<String>,
}

fn put_builtin(role: &str, request: &Request, state: &ServiceState) -> Response {
    let (path, mut file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let payload: BuiltinOverridePayload = match serde_json::from_str(&request.body) {
        Ok(p) => p,
        Err(err) => return Response::error(400, &format!("invalid json: {err}")),
    };
    // 若提交的描述或系统提示词与内置默认值完全一致，自动视作未覆写（None）。
    let default_desc = match role {
        "explorer" => BaseRole::Explorer.default_description(),
        "executor" => BaseRole::Executor.default_description(),
        "monitor" => BaseRole::Monitor.default_description(),
        _ => "",
    };
    let default_prompt = match role {
        "explorer" => EXPLORER_PROMPT,
        "executor" => EXECUTOR_PROMPT,
        "monitor" => MONITOR_PROMPT,
        _ => "",
    };
    let ov = RoleOverrideConfig {
        description: payload.description.filter(|d| d.trim() != default_desc),
        system_prompt: payload
            .system_prompt
            .filter(|p| p.trim() != default_prompt.trim()),
        model: payload.model,
        lock_model: payload.lock_model,
        capabilities: payload.capabilities,
        nickname_candidates: payload.nickname_candidates,
    }
    .normalized();
    match role {
        "explorer" => file.subagents.builtins.explorer = ov,
        "executor" => file.subagents.builtins.executor = ov,
        "monitor" => file.subagents.builtins.monitor = ov,
        _ => return Response::error(404, "unknown built-in role"),
    }
    match commit_and_notify(&path, &file, state) {
        Ok(saved) => build_response_json(&saved, state),
        Err(resp) => resp,
    }
}

fn reset_builtin(role: &str, state: &ServiceState) -> Response {
    let (path, mut file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    match role {
        "explorer" => file.subagents.builtins.explorer = RoleOverrideConfig::default(),
        "executor" => file.subagents.builtins.executor = RoleOverrideConfig::default(),
        "monitor" => file.subagents.builtins.monitor = RoleOverrideConfig::default(),
        _ => return Response::error(404, "unknown built-in role"),
    }
    match commit_and_notify(&path, &file, state) {
        Ok(saved) => build_response_json(&saved, state),
        Err(resp) => resp,
    }
}

fn create_role(request: &Request, state: &ServiceState) -> Response {
    let (path, mut file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let custom: CustomRoleConfig = match serde_json::from_str(&request.body) {
        Ok(c) => c,
        Err(err) => return Response::error(400, &format!("invalid json: {err}")),
    };
    let norm = custom.normalized();
    if file
        .subagents
        .roles
        .iter()
        .any(|existing| existing.name.trim() == norm.name)
    {
        return Response::error(422, &format!("custom role '{}' already exists", norm.name));
    }
    file.subagents.roles.push(norm);
    match commit_and_notify(&path, &file, state) {
        Ok(saved) => build_response_json(&saved, state),
        Err(resp) => resp,
    }
}

fn update_role(name: &str, request: &Request, state: &ServiceState) -> Response {
    let (path, mut file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let custom: CustomRoleConfig = match serde_json::from_str(&request.body) {
        Ok(c) => c,
        Err(err) => return Response::error(400, &format!("invalid json: {err}")),
    };
    let norm = custom.normalized();
    let Some(idx) = file
        .subagents
        .roles
        .iter()
        .position(|r| r.name.trim() == name.trim())
    else {
        return Response::error(404, &format!("custom role '{name}' not found"));
    };
    file.subagents.roles[idx] = norm;
    match commit_and_notify(&path, &file, state) {
        Ok(saved) => build_response_json(&saved, state),
        Err(resp) => resp,
    }
}

fn delete_role(name: &str, state: &ServiceState) -> Response {
    let (path, mut file) = match load_current(state) {
        Ok(pair) => pair,
        Err(resp) => return resp,
    };
    let before = file.subagents.roles.len();
    file.subagents
        .roles
        .retain(|r| r.name.trim() != name.trim());
    if file.subagents.roles.len() == before {
        return Response::error(404, &format!("custom role '{name}' not found"));
    }
    match commit_and_notify(&path, &file, state) {
        Ok(saved) => build_response_json(&saved, state),
        Err(resp) => resp,
    }
}

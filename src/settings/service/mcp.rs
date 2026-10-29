//! MCP 设置 API：配置 CRUD、测试连接、重启与 OAuth 登录。
//!
//! scope 只从 TUI 推送的项目表解析；未知 scope 404、未信任项目 403。
//! GET 从不返回 token / client_secret 原文，只返回 `secret_flags`。

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::handlers::{Request, Response};
use super::{McpStatusView, ProjectView, ServiceState, SettingsEvent};
use crate::config::mcp::{
    EffectiveServer, McpConfigFile, McpHttpAuth, McpScope, McpServerConfig, McpTransportConfig,
    load_mcp, save_mcp,
};

/// `/api/mcp` 系列入口（回调在 handlers 中先行处理）。
pub async fn route(request: &Request, state: &ServiceState) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/mcp") => get_servers(state),
        (_, path) if path.starts_with("/api/mcp/") => derive_route(request, path, state).await,
        _ => Response::error(404, "not found"),
    }
}

async fn derive_route(request: &Request, path: &str, state: &ServiceState) -> Response {
    let rest = path.trim_start_matches("/api/mcp/");
    let mut segments: Vec<&str> = rest.split('/').collect();
    let action = match segments.last().copied() {
        Some("oauth") => {
            segments.pop();
            match segments.last().copied() {
                Some("start") => {
                    segments.pop();
                    Some("oauth/start")
                }
                Some("logout") => {
                    segments.pop();
                    Some("oauth/logout")
                }
                _ => None,
            }
        }
        Some("probe") | Some("restart") => segments.pop(),
        _ => None,
    };
    if segments.len() != 2 {
        return Response::error(404, "invalid mcp path");
    }
    let (scope, server) = (segments[0], segments[1]);
    let method = request.method.as_str();
    match (method, action) {
        ("PUT", None) => put_server(request, state, scope, server),
        ("DELETE", None) => delete_server(state, scope, server),
        ("POST", Some("probe")) => probe_server(state, scope, server).await,
        ("POST", Some("restart")) => restart_server(state, scope, server),
        ("POST", Some("oauth/start")) => oauth_start(state, scope, server).await,
        ("POST", Some("oauth/logout")) => oauth_logout_server(state, scope, server),
        _ => Response::error(405, "method not allowed"),
    }
}

fn data_root(state: &ServiceState) -> Result<PathBuf, String> {
    state
        .user_root()
        .or_else(|| crate::config::user_data_root().ok())
        .ok_or_else(|| "home directory unavailable".to_string())
}

fn user_config_path(root: &Path) -> PathBuf {
    root.join("config").join("mcp.json")
}

fn project_config_path(root: &Path, project_root: &Path) -> Option<PathBuf> {
    let scope = crate::platform::path_text::scope_dir_name(project_root)?;
    Some(
        root.join("config")
            .join("mcp")
            .join("projects")
            .join(format!("{scope}.json")),
    )
}

/// scope → (项目视图, 配置文件路径)。`user` 返回 None 项目。
fn resolve_scope(
    state: &ServiceState,
    scope: &str,
) -> Result<(Option<ProjectView>, PathBuf), Response> {
    let root = data_root(state).map_err(|error| Response::error(500, &error))?;
    if scope == "user" {
        return Ok((None, user_config_path(&root)));
    }
    let Some(project) = state.project_for_scope(scope) else {
        return Err(Response::error(404, "unknown project scope"));
    };
    if !project.trusted {
        return Err(Response::error(403, "project is not trusted"));
    }
    let Some(path) = project_config_path(&root, &project.root) else {
        return Err(Response::error(500, "cannot derive project scope"));
    };
    Ok((Some(project), path))
}

fn load_file(path: &Path) -> Result<McpConfigFile, Response> {
    load_mcp(path)
        .map_err(|error| Response::error(500, &error.to_string()))?
        .ok_or_else(|| Response::error(404, "mcp config not found"))
}

/// GET `/api/mcp`：配置（脱敏）+ 运行状态 + 项目列表。
fn get_servers(state: &ServiceState) -> Response {
    let root = match data_root(state) {
        Ok(root) => root,
        Err(error) => return Response::error(500, &error),
    };
    let user = match load_mcp(&user_config_path(&root)) {
        Ok(file) => file,
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let projects = state.projects();
    let statuses = state.mcp_status();
    let mut rows: Vec<Value> = Vec::new();
    let user_names: std::collections::BTreeSet<String> = user
        .as_ref()
        .map(|file| file.mcp.servers.keys().cloned().collect())
        .unwrap_or_default();
    if let Some(file) = &user {
        for (name, config) in &file.mcp.servers {
            let status = aggregate_status(&statuses, &projects, None, name);
            rows.push(server_json(
                name, "user", config, None, false, false, status,
            ));
        }
    }
    for project in &projects {
        if !project.trusted {
            continue;
        }
        let Some(path) = project_config_path(&root, &project.root) else {
            continue;
        };
        let Ok(Some(file)) = load_mcp(&path) else {
            continue;
        };
        for (name, config) in &file.mcp.servers {
            let status = aggregate_status(&statuses, &projects, Some(&project.root), name);
            rows.push(server_json(
                name,
                &project.scope,
                config,
                Some(&project.root),
                user_names.contains(name),
                false,
                status,
            ));
        }
    }
    // 用户级同名条目被项目级覆盖：加标记，供列表置灰。
    for row in &mut rows {
        if row.get("scope").and_then(Value::as_str) == Some("user")
            && let Some(name) = row.get("name").and_then(Value::as_str)
            && projects.iter().any(|project| {
                let Some(path) = project_config_path(&root, &project.root) else {
                    return false;
                };
                load_mcp(&path)
                    .ok()
                    .flatten()
                    .is_some_and(|file| file.mcp.servers.contains_key(name))
            })
        {
            row["shadowed_user"] = json!(true);
        }
    }
    let value = json!({
        "projects": projects.iter().map(project_json).collect::<Vec<_>>(),
        "servers": rows,
        "revision": credentials_revision(state),
    });
    Response::json(200, serde_json::to_string(&value).unwrap_or_default())
}

fn project_json(project: &ProjectView) -> Value {
    json!({
        "scope": project.scope,
        "name": project.name,
        "root": project.root.display().to_string(),
        "trusted": project.trusted,
    })
}

fn credentials_revision(state: &ServiceState) -> u64 {
    data_root(state)
        .ok()
        .map(|root| {
            crate::mcp::oauth::store_at(root.join("mcp").join("credentials.json")).revision()
        })
        .unwrap_or(0)
}

/// 单条 server 的脱敏 JSON。
fn server_json(
    name: &str,
    scope: &str,
    config: &McpServerConfig,
    project_root: Option<&Path>,
    overrides_user: bool,
    shadowed_user: bool,
    status: String,
) -> Value {
    let status = if config.enabled {
        status
    } else {
        "disabled".to_string()
    };
    let mut value = serde_json::to_value(config).unwrap_or_else(|_| json!({}));
    let mut secrets = Map::new();
    if let Some(object) = value.as_object_mut() {
        for key in ["bearer_token", "client_secret"] {
            let set = object
                .get(key)
                .and_then(Value::as_str)
                .is_some_and(|token| !token.is_empty());
            if set {
                secrets.insert(key.to_string(), json!(true));
            }
            object.insert(key.to_string(), json!(""));
        }
    }
    json!({
        "name": name,
        "scope": scope,
        "project_root": project_root.map(|root| root.display().to_string()),
        "overrides_user": overrides_user,
        "shadowed_user": shadowed_user,
        "status": status,
        "config": value,
        "secret_flags": secrets,
    })
}

fn aggregate_status(
    statuses: &[McpStatusView],
    projects: &[ProjectView],
    root: Option<&Path>,
    server: &str,
) -> String {
    let matching: Vec<&McpStatusView> = statuses
        .iter()
        .filter(|status| status.server == server)
        .filter(|status| match root {
            Some(root) => status.project == root,
            None => projects
                .iter()
                .any(|project| project.root == status.project),
        })
        .collect();
    if matching.is_empty() {
        return "unknown".into();
    }
    if matching.iter().any(|status| status.status == "ready") {
        return "ready".into();
    }
    if matching.iter().any(|status| status.status == "needs_login") {
        return "needs_login".into();
    }
    matching
        .first()
        .map(|status| status.status.clone())
        .unwrap_or_else(|| "unknown".into())
}

/// 保留旧密钥：空串/缺失表示保持，`null` 表示清除。
fn merge_secret(object: &mut Map<String, Value>, key: &str, previous: Option<&str>) {
    match object.get(key) {
        Some(Value::Null) => {
            object.insert(key.to_string(), Value::Null);
        }
        Some(Value::String(value)) if value.is_empty() => {
            if let Some(previous) = previous {
                object.insert(key.to_string(), json!(previous));
            } else {
                object.remove(key);
            }
        }
        Some(_) => {}
        None => {
            if let Some(previous) = previous {
                object.insert(key.to_string(), json!(previous));
            }
        }
    }
}

fn put_server(request: &Request, state: &ServiceState, scope: &str, server: &str) -> Response {
    let (project, path) = match resolve_scope(state, scope) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut file = match load_mcp(&path) {
        Ok(Some(file)) => file,
        Ok(None) => McpConfigFile::default(),
        Err(error) => return Response::error(500, &error.to_string()),
    };
    if project.is_some() {
        file.project_root = project.as_ref().map(|project| project.root.clone());
    }
    let previous = file.mcp.servers.get(server).cloned();
    let mut value: Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    let Some(object) = value.as_object_mut() else {
        return Response::error(422, "body must be a server object");
    };
    let auth = object
        .get("auth")
        .and_then(Value::as_str)
        .unwrap_or("none")
        .to_string();
    if let Some(previous) = &previous {
        match &previous.transport {
            McpTransportConfig::Http {
                bearer_token,
                client_secret,
                ..
            } if auth == "bearer" => {
                merge_secret(object, "bearer_token", bearer_token.as_deref());
            }
            McpTransportConfig::Http { client_secret, .. } if auth == "oauth" => {
                merge_secret(object, "client_secret", client_secret.as_deref());
            }
            _ => {}
        }
    }
    if auth != "bearer" {
        object.remove("bearer_token");
        object.remove("bearer_token_env_var");
    }
    if auth != "oauth" {
        object.remove("client_secret");
        object.remove("client_id");
    }
    // bearer 二选一：填写 env var 时清掉旧的明文字段（空串语义是保留）。
    if object.get("auth").and_then(Value::as_str) == Some("bearer") {
        let env_set = object
            .get("bearer_token_env_var")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty());
        if env_set {
            object.remove("bearer_token");
        } else {
            object.remove("bearer_token_env_var");
        }
    }
    let config: McpServerConfig = match serde_json::from_value(value) {
        Ok(config) => config,
        Err(error) => return Response::error(422, &format!("invalid server config: {error}")),
    };
    if let Err(error) = config.validate(server) {
        return Response::error(422, &error.to_string());
    }
    file.mcp.servers.insert(server.to_string(), config);
    if let Err(error) = save_mcp(&path, &file) {
        return Response::error(500, &error.to_string());
    }
    let _ = state.sender().send(SettingsEvent::McpChanged);
    Response::json(200, "{\"ok\":true}")
}

fn delete_server(state: &ServiceState, scope: &str, server: &str) -> Response {
    let (_project, path) = match resolve_scope(state, scope) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut file = match load_mcp(&path) {
        Ok(Some(file)) => file,
        Ok(None) => return Response::error(404, "server not found"),
        Err(error) => return Response::error(500, &error.to_string()),
    };
    if file.mcp.servers.remove(server).is_none() {
        return Response::error(404, "server not found");
    }
    if let Err(error) = save_mcp(&path, &file) {
        return Response::error(500, &error.to_string());
    }
    let _ = state.sender().send(SettingsEvent::McpChanged);
    Response::json(200, "{\"ok\":true}")
}

/// 取磁盘上的配置（探针/登录用）。
fn config_for(
    state: &ServiceState,
    scope: &str,
    server: &str,
) -> Result<McpServerConfig, Response> {
    let (_project, path) = resolve_scope(state, scope)?;
    let file = load_file(&path)?;
    file.mcp
        .servers
        .get(server)
        .cloned()
        .ok_or_else(|| Response::error(404, "server not found"))
}

async fn probe_server(state: &ServiceState, scope: &str, server: &str) -> Response {
    let config = match config_for(state, scope, server) {
        Ok(config) => config,
        Err(response) => return response,
    };
    let project = state.project_for_scope(scope);
    let project_root = project
        .as_ref()
        .map(|project| project.root.clone())
        .or_else(|| {
            state
                .projects()
                .into_iter()
                .find(|project| project.trusted)
                .map(|project| project.root)
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let effective = EffectiveServer {
        name: server.to_string(),
        scope: if project.is_some() {
            McpScope::Project
        } else {
            McpScope::User
        },
        config,
        shadowed_user: false,
    };
    match crate::mcp::global_manager()
        .probe(&effective, &project_root)
        .await
    {
        Ok(tools) => Response::json(200, json!({ "tools": tools }).to_string()),
        Err(error) => Response::error(422, &error),
    }
}

fn restart_server(state: &ServiceState, scope: &str, server: &str) -> Response {
    if let Err(response) = config_for(state, scope, server) {
        return response;
    }
    let _ = state.sender().send(SettingsEvent::McpRestart {
        server: server.to_string(),
    });
    Response::json(200, "{\"ok\":true}")
}

fn credentials_store(
    state: &ServiceState,
) -> Result<std::sync::Arc<crate::mcp::oauth::CredentialStore>, Response> {
    let root = data_root(state).map_err(|error| Response::error(500, &error))?;
    Ok(crate::mcp::oauth::store_at(
        root.join("mcp").join("credentials.json"),
    ))
}

async fn oauth_start(state: &ServiceState, scope: &str, server: &str) -> Response {
    let (project, _path) = match resolve_scope(state, scope) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let config = match config_for(state, scope, server) {
        Ok(config) => config,
        Err(response) => return response,
    };
    if !matches!(
        &config.transport,
        McpTransportConfig::Http {
            auth: McpHttpAuth::Oauth,
            ..
        }
    ) {
        return Response::error(422, "server auth is not oauth");
    }
    let effective = EffectiveServer {
        name: server.to_string(),
        scope: if project.is_some() {
            McpScope::Project
        } else {
            McpScope::User
        },
        config,
        shadowed_user: false,
    };
    let project_root = project
        .as_ref()
        .map(|project| project.root.clone())
        .or_else(|| {
            state
                .projects()
                .into_iter()
                .next()
                .map(|project| project.root)
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let redirect_uri = format!("http://127.0.0.1:{}/mcp/oauth/callback", state.port());
    let pending = match crate::mcp::oauth::begin_authorization(
        &effective,
        &project_root,
        &redirect_uri,
    )
    .await
    {
        Ok(pending) => pending,
        Err(error) => return Response::error(422, &error),
    };
    let authorize_url = pending.authorize_url.clone();
    let mut sessions = state.oauth().lock().await;
    let now = now_ms();
    sessions.retain(|_, pending| now.saturating_sub(pending.created_ms) < 600_000);
    sessions.insert(pending.state.clone(), pending);
    Response::json(200, json!({ "authorize_url": authorize_url }).to_string())
}

fn oauth_logout_server(state: &ServiceState, scope: &str, server: &str) -> Response {
    let config = match config_for(state, scope, server) {
        Ok(config) => config,
        Err(response) => return response,
    };
    let (project, _path) = match resolve_scope(state, scope) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let project_root = project
        .as_ref()
        .map(|project| project.root.clone())
        .or_else(|| {
            state
                .projects()
                .into_iter()
                .next()
                .map(|project| project.root)
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let effective = EffectiveServer {
        name: server.to_string(),
        scope: if project.is_some() {
            McpScope::Project
        } else {
            McpScope::User
        },
        config,
        shadowed_user: false,
    };
    let key = crate::mcp::oauth::credential_key(&effective, &project_root);
    let store = match credentials_store(state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    if let Err(error) = store.remove(&key) {
        return Response::error(500, &error);
    }
    let _ = state.sender().send(SettingsEvent::McpCredentialsChanged);
    Response::json(200, "{\"ok\":true}")
}

/// OAuth 回调：state 一次性 + 10 分钟过期；不校验 capability token。
pub async fn oauth_callback(request: &Request, state: &ServiceState) -> Response {
    let code = request.query.get("code").cloned().unwrap_or_default();
    let state_token = request.query.get("state").cloned().unwrap_or_default();
    if code.is_empty() || state_token.is_empty() {
        return html_response(400, "Login failed: missing code or state.");
    }
    let pending = {
        let mut sessions = state.oauth().lock().await;
        let pending = sessions.remove(&state_token);
        match pending {
            Some(pending) if now_ms().saturating_sub(pending.created_ms) < 600_000 => pending,
            _ => return html_response(400, "Login failed: state expired or invalid."),
        }
    };
    let store = match credentials_store(state) {
        Ok(store) => store,
        Err(response) => return response,
    };
    match crate::mcp::oauth::complete_authorization(&pending, &code, &store).await {
        Ok(()) => {
            let _ = state.sender().send(SettingsEvent::McpCredentialsChanged);
            html_response(200, "Login complete. You can close this tab.")
        }
        Err(error) => html_response(400, &format!("Login failed: {error}")),
    }
}

fn html_response(status: u16, message: &str) -> Response {
    let body = format!(
        "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>lx OAuth</title></head>\
         <body style=\"font:14px system-ui;padding:2rem\"><p>{}</p></body></html>",
        escape_html(message)
    );
    Response {
        status,
        content_type: "text/html; charset=utf-8",
        body,
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

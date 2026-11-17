//! Sandbox 设置 API：全局模式、网络、提权开关与额外可写根。

use std::path::PathBuf;

use serde_json::json;

use super::handlers::{Request, Response};
use super::{ServiceState, SettingsEvent};
use crate::config::sandbox::{
    SandboxConfigError, SandboxSettings, load_sandbox, sandbox_config_path, save_sandbox,
};

/// `/api/sandbox` 入口。
pub async fn route(request: &Request, state: &ServiceState) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/sandbox") => get_sandbox(state),
        ("PUT", "/api/sandbox") => {
            let _guard = state.config_write_lock().lock().await;
            put_sandbox(request, state)
        }
        ("POST", "/api/sandbox/open-rules") => open_rules(state).await,
        _ => Response::error(404, "not found"),
    }
}

fn sandbox_path(state: &ServiceState) -> Result<PathBuf, String> {
    // 与 settings.json / ai.json 同一数据根；用户目录不可用时退回数据根约定。
    if let Some(root) = state.user_root() {
        return Ok(root.join("config").join("sandbox.json"));
    }
    sandbox_config_path().map_err(|error| error.to_string())
}

fn rules_dir_for(state: &ServiceState) -> Result<PathBuf, String> {
    if let Some(root) = state.user_root() {
        return Ok(root.join(crate::chat::exec_policy::RULES_DIR_NAME));
    }
    crate::chat::exec_policy::rules_dir().map_err(|error| error.to_string())
}

fn sandbox_error_response(error: SandboxConfigError) -> Response {
    match error {
        SandboxConfigError::Invalid(_) => Response::error(422, &error.to_string()),
        SandboxConfigError::Io(_) | SandboxConfigError::HomeMissing => {
            Response::error(500, &error.to_string())
        }
    }
}

fn get_sandbox(state: &ServiceState) -> Response {
    let path = match sandbox_path(state) {
        Ok(path) => path,
        Err(error) => return Response::error(500, &error),
    };
    let (settings, warning) = match load_sandbox(&path) {
        Ok(Some(settings)) => (settings, None),
        Ok(None) => (SandboxSettings::default(), None),
        Err(error @ SandboxConfigError::Invalid(_)) => {
            let detail = error.to_string();
            tracing::warn!(
                error = %detail,
                path = %path.display(),
                "invalid sandbox config; falling back to defaults"
            );
            (SandboxSettings::default(), Some(detail))
        }
        Err(error @ (SandboxConfigError::Io(_) | SandboxConfigError::HomeMissing)) => {
            return Response::error(500, &error.to_string());
        }
    };
    Response::json(200, body(state, &settings, warning.as_deref()))
}

fn put_sandbox(request: &Request, state: &ServiceState) -> Response {
    let path = match sandbox_path(state) {
        Ok(path) => path,
        Err(error) => return Response::error(500, &error),
    };
    let parsed: SandboxSettings = match serde_json::from_str(&request.body) {
        Ok(parsed) => parsed,
        Err(error) => return Response::error(422, &format!("malformed json: {error}")),
    };
    let normalized = match parsed.normalized() {
        Ok(normalized) => normalized,
        Err(error) => return sandbox_error_response(error),
    };
    if let Err(error) = normalized.validate() {
        return sandbox_error_response(error);
    }
    if let Err(error) = save_sandbox(&path, &normalized) {
        return sandbox_error_response(error);
    }
    let _ = state
        .sender()
        .send(SettingsEvent::SandboxChanged(Box::new(normalized.clone())));
    Response::json(200, body(state, &normalized, None))
}

fn body(state: &ServiceState, settings: &SandboxSettings, warning: Option<&str>) -> String {
    let support = crate::platform::sandbox::support();
    let config = serde_json::to_value(settings).unwrap_or_else(|_| json!({}));
    let rules = match rules_dir_for(state) {
        Ok(dir) => crate::chat::exec_policy::rules_status_for_dir(&dir),
        Err(error) => crate::chat::exec_policy::RulesStatus {
            errors: vec![error],
            ..crate::chat::exec_policy::RulesStatus::default()
        },
    };
    let mut payload = json!({
        "config": config,
        "effective_default": settings.mode.as_str(),
        "capabilities": {
            "platform": std::env::consts::OS,
            "process_sandbox": support.as_str(),
            "reason": match support {
                crate::platform::sandbox::SandboxSupport::Unavailable => {
                    "install the platform sandbox backend or switch to danger-full-access"
                }
                crate::platform::sandbox::SandboxSupport::FileToolsOnly => {
                    "process sandbox is not implemented on Windows yet"
                }
                _ => "",
            },
        },
        "path": path_display(state),
        "rules": {
            "directory": rules.directory,
            "total_files": rules.total_files,
            "loaded_files": rules.loaded_files,
            "errors": rules.errors,
        },
    });
    if let Some(warning) = warning
        && let Some(object) = payload.as_object_mut()
    {
        object.insert("warning".to_string(), json!(warning));
    }
    payload.to_string()
}

/// `POST /api/sandbox/open-rules`：在系统文件管理器中打开规则目录。
async fn open_rules(state: &ServiceState) -> Response {
    let dir = match rules_dir_for(state) {
        Ok(dir) => dir,
        Err(error) => return Response::error(500, &error),
    };
    if let Err(error) = std::fs::create_dir_all(&dir) {
        return Response::error(500, &error.to_string());
    }
    let _ = crate::platform::restrict_dir_permissions(&dir);
    match tokio::task::spawn_blocking(move || crate::platform::reveal_in_file_manager(&dir)).await {
        Ok(Ok(())) => Response::json(200, "{\"ok\":true}"),
        Ok(Err(error)) => Response::error(500, &error.to_string()),
        Err(error) => Response::error(500, &error.to_string()),
    }
}

fn path_display(state: &ServiceState) -> String {
    sandbox_path(state)
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;

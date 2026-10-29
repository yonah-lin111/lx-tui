//! Skills 设置 API：目录列表、lx 自有根增删改与启停。
//!
//! `.agents` 来源只读；写请求只允许落在 `~/.lx-t[-dev]/skills/` 与
//! `.../skills/projects/<scope>/` 下。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::handlers::{Request, Response};
use super::{ProjectView, ServiceState, SettingsEvent};
use crate::config::skills as skills_config;
use crate::skills::{ProjectSkillRoots, SkillEntry, SkillSource, discover};

/// `/api/skills` 系列入口。
pub async fn route(request: &Request, state: &ServiceState) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/skills") => list_skills(state),
        ("GET", "/api/skills/detail") => skill_detail(request, state),
        ("PUT", "/api/skills/enabled") => set_enabled(request, state),
        (_, path) if path.starts_with("/api/skills/") => {
            edit_route(request, path.trim_start_matches("/api/skills/"), state)
        }
        _ => Response::error(404, "not found"),
    }
}

fn data_root(state: &ServiceState) -> Result<PathBuf, String> {
    state
        .user_root()
        .or_else(|| crate::config::user_data_root().ok())
        .ok_or_else(|| "home directory unavailable".to_string())
}

fn user_skills_root(root: &Path) -> PathBuf {
    root.join("skills")
}

fn project_skills_root(root: &Path, project_root: &Path) -> Option<PathBuf> {
    let scope = crate::platform::path_text::scope_dir_name(project_root)?;
    Some(root.join("skills").join("projects").join(scope))
}

fn skills_config_path(root: &Path) -> PathBuf {
    root.join("config").join("skills.json")
}

fn disabled_set(root: &Path) -> HashSet<PathBuf> {
    skills_config::load_skills(&skills_config_path(root))
        .ok()
        .flatten()
        .map(|file| file.disabled_set().into_iter().collect())
        .unwrap_or_default()
}

fn empty_root() -> &'static Path {
    Path::new("/nonexistent/lx-skills")
}

/// GET：全部作用域的条目 + 解析错误 + 项目列表。
fn list_skills(state: &ServiceState) -> Response {
    let root = match data_root(state) {
        Ok(root) => root,
        Err(error) => return Response::error(500, &error),
    };
    let disabled = disabled_set(&root);
    let user_root = user_skills_root(&root);
    let user_agents =
        skills_config::user_agents_skills_root().unwrap_or_else(|_| root.join("agents-skills"));
    let mut entries: Vec<SkillEntry> = Vec::new();
    let mut errors: Vec<(PathBuf, String)> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    let user = discover(&user_root, &user_agents, None, &disabled);
    for entry in user.entries {
        if seen.insert(entry.path.clone()) {
            entries.push(entry);
        }
    }
    errors.extend(user.errors);
    let mut project_scopes: Vec<(PathBuf, String)> = Vec::new();

    let projects = state.projects();
    for project in &projects {
        if !project.trusted {
            continue;
        }
        let Some(lx) = project_skills_root(&root, &project.root) else {
            continue;
        };
        let agents = skills_config::project_agents_skills_root(&project.root);
        project_scopes.push((lx.clone(), project.scope.clone()));
        project_scopes.push((agents.clone(), project.scope.clone()));
        let roots = ProjectSkillRoots {
            lx: Some(lx),
            agents: Some(agents),
        };
        let catalog = discover(empty_root(), empty_root(), Some(&roots), &disabled);
        for entry in catalog.entries {
            if seen.insert(entry.path.clone()) {
                entries.push(entry);
            }
        }
        errors.extend(catalog.errors);
    }

    let value = json!({
        "projects": projects.iter().map(|project| json!({
            "scope": project.scope,
            "name": project.name,
            "root": project.root.display().to_string(),
            "trusted": project.trusted,
        })).collect::<Vec<_>>(),
        "entries": entries.iter().map(|entry| {
            let scope = project_scopes.iter().find_map(|(root, scope)| {
                entry.path.starts_with(root).then_some(scope.as_str())
            }).unwrap_or_else(|| entry.scope.as_str());
            entry_json(entry, scope)
        }).collect::<Vec<_>>(),
        "errors": errors.iter().map(|(path, error)| json!({
            "path": path.display().to_string(),
            "error": error,
        })).collect::<Vec<_>>(),
    });
    Response::json(200, serde_json::to_string(&value).unwrap_or_default())
}

fn entry_json(entry: &SkillEntry, scope: &str) -> serde_json::Value {
    json!({
        "name": entry.name,
        "description": entry.description,
        "path": entry.path.display().to_string(),
        "scope": scope,
        "source": entry.source.as_str(),
        "enabled": entry.enabled,
        "editable": entry.source == SkillSource::Lx,
        "shadowed_by": entry.shadowed_by.as_ref().map(|path| path.display().to_string()),
    })
}

/// GET `/api/skills/detail?path=...`：读取一个已发现 skill 的正文。
fn skill_detail(request: &Request, state: &ServiceState) -> Response {
    let root = match data_root(state) {
        Ok(root) => root,
        Err(error) => return Response::error(500, &error),
    };
    let Some(path) = request.query.get("path") else {
        return Response::error(422, "missing `path`");
    };
    if !path_is_known_skill(state, &root, path) {
        return Response::error(403, "skill path is not in a managed root");
    }
    let path = PathBuf::from(path);
    match crate::skills::read_skill_at(&path) {
        Ok((name, description, body)) => {
            let scope = state
                .projects()
                .into_iter()
                .find(|project| {
                    path.starts_with(&project.root)
                        || project_skills_root(&root, &project.root)
                            .is_some_and(|root| path.starts_with(root))
                        || path
                            .starts_with(skills_config::project_agents_skills_root(&project.root))
                })
                .map(|project| project.scope)
                .unwrap_or_else(|| "user".into());
            let editable = path.starts_with(user_skills_root(&root))
                || state.projects().iter().any(|project| {
                    project.trusted
                        && project_skills_root(&root, &project.root)
                            .is_some_and(|root| path.starts_with(root))
                });
            Response::json(
                200,
                json!({
                    "name": name,
                    "description": description,
                    "body": body,
                    "scope": scope,
                    "editable": editable,
                    "path": path.display().to_string(),
                })
                .to_string(),
            )
        }
        Err(error) => Response::error(422, &error),
    }
}

fn edit_route(request: &Request, rest: &str, state: &ServiceState) -> Response {
    let mut segments = rest.split('/');
    let (Some(scope), Some(name)) = (segments.next(), segments.next()) else {
        return Response::error(404, "invalid skill path");
    };
    if segments.next().is_some() {
        return Response::error(404, "invalid skill path");
    }
    match request.method.as_str() {
        "PUT" => put_skill(request, state, scope, name),
        "DELETE" => delete_skill(state, scope, name),
        _ => Response::error(405, "method not allowed"),
    }
}

/// 解析可写根：user → `<data>/skills/`；项目 scope → `<data>/skills/projects/<scope>/`。
fn writable_root(
    state: &ServiceState,
    scope: &str,
) -> Result<(PathBuf, Option<ProjectView>), Response> {
    let root = data_root(state).map_err(|error| Response::error(500, &error))?;
    if scope == "user" {
        return Ok((user_skills_root(&root), None));
    }
    let Some(project) = state.project_for_scope(scope) else {
        return Err(Response::error(404, "unknown project scope"));
    };
    if !project.trusted {
        return Err(Response::error(403, "project is not trusted"));
    }
    let Some(path) = project_skills_root(&root, &project.root) else {
        return Err(Response::error(500, "cannot derive project scope"));
    };
    Ok((path, Some(project)))
}

fn put_skill(request: &Request, state: &ServiceState, scope: &str, name: &str) -> Response {
    let (root, _project) = match writable_root(state, scope) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let value: serde_json::Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    let description = value
        .get("description")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let body = value
        .get("body")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    match crate::skills::write_skill(&root, name, description, body) {
        Ok(_) => {
            let _ = state.sender().send(SettingsEvent::SkillsChanged);
            Response::json(200, "{\"ok\":true}")
        }
        Err(error) => Response::error(422, &error),
    }
}

fn delete_skill(state: &ServiceState, scope: &str, name: &str) -> Response {
    let (root, _project) = match writable_root(state, scope) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match crate::skills::delete_skill(&root, name) {
        Ok(()) => {
            let _ = state.sender().send(SettingsEvent::SkillsChanged);
            Response::json(200, "{\"ok\":true}")
        }
        Err(error) => Response::error(422, &error),
    }
}

/// PUT `/api/skills/enabled`：写 `skills.json` 的 disabled 列表。
fn set_enabled(request: &Request, state: &ServiceState) -> Response {
    let root = match data_root(state) {
        Ok(root) => root,
        Err(error) => return Response::error(500, &error),
    };
    let value: serde_json::Value = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(error) => return Response::error(422, &format!("invalid json: {error}")),
    };
    let Some(path) = value.get("path").and_then(serde_json::Value::as_str) else {
        return Response::error(422, "missing `path`");
    };
    let Some(enabled) = value.get("enabled").and_then(serde_json::Value::as_bool) else {
        return Response::error(422, "missing `enabled`");
    };
    if !path_is_known_skill(state, &root, path) {
        return Response::error(403, "skill path is not in a managed root");
    }
    let path = PathBuf::from(path);
    let config_path = skills_config_path(&root);
    let mut file = match skills_config::load_skills(&config_path) {
        Ok(Some(file)) => file,
        Ok(None) => skills_config::SkillsConfigFile::default(),
        Err(error) => return Response::error(500, &error.to_string()),
    };
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    if !enabled {
        if !file.skills.disabled.contains(&canonical) {
            file.skills.disabled.push(canonical);
        }
    } else {
        file.skills.disabled.retain(|entry| entry != &canonical);
    }
    if let Err(error) = skills_config::save_skills(&config_path, &file) {
        return Response::error(500, &error.to_string());
    }
    let _ = state.sender().send(SettingsEvent::SkillsChanged);
    Response::json(200, "{\"ok\":true}")
}

/// 只允许禁用系统实际发现的 `SKILL.md`（不能借 API 写任意路径）。
fn path_is_known_skill(state: &ServiceState, root: &Path, path: &str) -> bool {
    let path = PathBuf::from(path);
    let Some(name) = path.file_name() else {
        return false;
    };
    if name != "SKILL.md" {
        return false;
    }
    let canonical = std::fs::canonicalize(&path).unwrap_or(path);
    let user = user_skills_root(root);
    let user_agents = skills_config::user_agents_skills_root().ok();
    let mut roots: Vec<PathBuf> = vec![user];
    if let Some(agents) = user_agents {
        roots.push(agents);
    }
    for project in state.projects() {
        if !project.trusted {
            continue;
        }
        if let Some(lx) = project_skills_root(root, &project.root) {
            roots.push(lx);
        }
        roots.push(skills_config::project_agents_skills_root(&project.root));
    }
    roots.iter().any(|root| {
        canonical.starts_with(root)
            || std::fs::canonicalize(root)
                .map(|root| canonical.starts_with(root))
                .unwrap_or(false)
    })
}

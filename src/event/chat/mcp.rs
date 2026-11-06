//! 每轮冻结 MCP 快照：读配置、合并作用域、后台预热、取最近一次缓存。
//!
//! 首轮若 server 尚未就绪，本轮不带 MCP 工具；后台启动完成后下一轮即可用。
//! 快照本身在 turn 起点读取，同一 turn 的多次采样复用同一份。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app::state::AppState;
use crate::config::mcp::{
    EffectiveServer, effective_servers, load_mcp, project_mcp_path, user_mcp_path,
};
use crate::db::service::Persistence;
use crate::mcp::{McpToolSnapshot, global_manager};
use crate::settings::service::ProjectView;

/// 读磁盘配置并合并出某个项目的生效 server 列表。
pub(crate) fn effective_for_root(persistence: &Persistence, root: &Path) -> Vec<EffectiveServer> {
    let user = user_mcp_path()
        .ok()
        .and_then(|path| load_mcp(&path).ok().flatten());
    let project = project_mcp_path(root)
        .ok()
        .and_then(|path| load_mcp(&path).ok().flatten());
    let trusted = persistence.is_trusted(root);
    effective_servers(user.as_ref(), project.as_ref(), trusted, Some(root))
}

/// 后台预热一个项目；事件层在配置变化 / 登录后调用。
pub(crate) fn prewarm_root(persistence: &Persistence, root: &Path) {
    let servers = effective_for_root(persistence, root);
    global_manager().prewarm(root.to_path_buf(), servers);
}

/// 取本轮冻结的 MCP 快照。
pub(crate) fn snapshot_for_turn(
    persistence: &Persistence,
    workspace_root: Option<&Path>,
) -> Arc<McpToolSnapshot> {
    let Some(root) = workspace_root else {
        return Arc::new(McpToolSnapshot::default());
    };
    let servers = effective_for_root(persistence, root);
    let manager = global_manager();
    manager.prewarm(root.to_path_buf(), servers);
    manager.cached_snapshot(root)
}

/// 取本轮冻结且排除了当前窗格已禁用 server 的 MCP 快照。
pub(crate) fn snapshot_for_turn_with_disabled(
    persistence: &Persistence,
    workspace_root: Option<&Path>,
    disabled_servers: &BTreeSet<String>,
) -> Arc<McpToolSnapshot> {
    snapshot_for_turn(persistence, workspace_root).without_servers(disabled_servers)
}

/// 当前打开工作区的仓库根（去重、保持稳定顺序）。
pub(crate) fn project_roots(state: &AppState) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    for workspace in &state.workspaces {
        if let Some(root) = workspace.repo_key() {
            seen.insert(root.to_path_buf());
        }
    }
    seen.into_iter().collect()
}

/// 推送给设置页的项目视图。
pub(crate) fn project_views(state: &AppState, persistence: &Persistence) -> Vec<ProjectView> {
    state
        .workspaces
        .iter()
        .filter_map(|workspace| workspace.repo_key())
        .map(|root| {
            let name = root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workspace")
                .to_string();
            ProjectView {
                scope: crate::platform::path_text::scope_dir_name(root)
                    .unwrap_or_else(|| "workspace".into()),
                root: root.to_path_buf(),
                name,
                trusted: persistence.is_trusted(root),
            }
        })
        .fold(Vec::new(), |mut views, view| {
            if !views
                .iter()
                .any(|existing: &ProjectView| existing.root == view.root)
            {
                views.push(view);
            }
            views
        })
}

/// 配置变化后重新预热全部打开项目。
pub(crate) fn prewarm_projects(persistence: &Persistence, roots: &[PathBuf]) {
    for root in roots {
        prewarm_root(persistence, root);
    }
}

/// 强制重启某个 server（`None` = 全部）在所有打开项目里的连接。
pub(crate) fn restart_projects(persistence: &Persistence, roots: &[PathBuf], server: Option<&str>) {
    let jobs: Vec<(PathBuf, Vec<EffectiveServer>)> = roots
        .iter()
        .map(|root| (root.clone(), effective_for_root(persistence, root)))
        .collect();
    let server = server.map(str::to_string);
    tokio::spawn(async move {
        let manager = global_manager().clone();
        for (root, servers) in jobs {
            manager.restart(&root, server.as_deref()).await;
            manager.prewarm(root, servers);
        }
    });
}

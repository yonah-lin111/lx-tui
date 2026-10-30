//! 首页 MCP / skill 资源计数的启动装载与状态汇总。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tokio::sync::mpsc::UnboundedSender;

use crate::app::state::AppState;
use crate::app::state::lx::StartupCount;
use crate::db::service::Persistence;
use crate::settings::service::McpStatusView;

use super::AppEvent;

/// 启动（或首次打开工作区）时按配置初始化 MCP 计数并后台预热。
pub(crate) fn initialize_mcp(state: &mut AppState, persistence: &Persistence, roots: &[PathBuf]) {
    for root in roots {
        let total = super::chat::mcp::effective_for_root(persistence, root)
            .iter()
            .filter(|server| server.config.enabled)
            .count();
        set_mcp(
            state,
            root.as_path(),
            StartupCount {
                total,
                ready: 0,
                errors: 0,
                loading: total > 0,
            },
        );
    }
    super::chat::mcp::prewarm_projects(persistence, roots);
}

/// 配置变化后刷新总数：保留已结算的 ready/error，只为新增 server 打开 loading。
pub(crate) fn refresh_mcp(state: &mut AppState, persistence: &Persistence, roots: &[PathBuf]) {
    for root in roots {
        let total = super::chat::mcp::effective_for_root(persistence, root)
            .iter()
            .filter(|server| server.config.enabled)
            .count();
        for workspace in &mut state.workspaces {
            if workspace
                .repo_key()
                .is_none_or(|workspace_root| workspace_root != root.as_path())
            {
                continue;
            }
            for tab in &mut workspace.tabs {
                for id in tab.layout.pane_ids() {
                    if let Some(pane) = tab.pane_mut(id) {
                        let current = pane.lx.startup.mcp;
                        let errors = current.errors.min(total);
                        let ready = current.ready.min(total.saturating_sub(errors));
                        pane.lx.startup.mcp = StartupCount {
                            total,
                            ready,
                            errors,
                            loading: total > 0 && ready.saturating_add(errors) < total,
                        };
                    }
                }
            }
        }
    }
    super::chat::mcp::prewarm_projects(persistence, roots);
}

/// 单个 server 状态变化后，按项目重算 ready / error / loading。
pub(crate) fn update_mcp_status(
    state: &mut AppState,
    project: &Path,
    statuses: &BTreeMap<(PathBuf, String), McpStatusView>,
) {
    let mut ready = 0;
    let mut errors = 0;
    let mut loading = false;
    let mut total = 0;
    for ((root, _), status) in statuses {
        if root.as_path() != project {
            continue;
        }
        total += 1;
        match status.status.as_str() {
            "ready" => ready += 1,
            "failed" | "needs_login" => errors += 1,
            "starting" => loading = true,
            _ => {}
        }
    }
    for workspace in &mut state.workspaces {
        if workspace.repo_key().is_none_or(|root| root != project) {
            continue;
        }
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    let current = pane.lx.startup.mcp;
                    pane.lx.startup.mcp = StartupCount {
                        total: current.total.max(total),
                        ready,
                        errors,
                        loading: loading || current.loading,
                    };
                }
            }
        }
    }
}

/// 启动进度：完成前保持 loading，完成后按已结算状态停止 loading。
pub(crate) fn update_mcp_progress(state: &mut AppState, project: &Path, done: usize, total: usize) {
    for workspace in &mut state.workspaces {
        if workspace.repo_key().is_none_or(|root| root != project) {
            continue;
        }
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    let mut count = pane.lx.startup.mcp;
                    count.total = total;
                    count.loading = total > 0 && done < total;
                    pane.lx.startup.mcp = count;
                }
            }
        }
    }
}

/// 后台预扫描 skill 目录；结果经 [`AppEvent::SkillsLoaded`] 回到事件循环。
pub(crate) fn spawn_skills_preload(
    state: &mut AppState,
    persistence: &Persistence,
    sender: &UnboundedSender<AppEvent>,
) {
    let root = state
        .prompt_owner()
        .repo_key()
        .map(std::path::Path::to_path_buf);
    let Some(job) = super::chat::skills::prepare_preload(persistence, root.as_deref(), false)
    else {
        apply_skills_loaded(state, 0, 0);
        return;
    };
    for workspace in &mut state.workspaces {
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    pane.lx.startup.skills.loading = true;
                }
            }
        }
    }
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let (root, total, errors) = job.run();
        let _ = sender.send(AppEvent::SkillsLoaded {
            root,
            total,
            errors,
        });
    });
}

/// 写入后台扫描结果；首页统一显示当前 skill 目录总量与解析错误数。
pub(crate) fn apply_skills_loaded(state: &mut AppState, total: usize, errors: usize) {
    let skills = crate::skills::cached_entries();
    for workspace in &mut state.workspaces {
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    pane.lx.startup.skills = StartupCount {
                        total,
                        ready: total.saturating_sub(errors),
                        errors,
                        loading: false,
                    };
                    pane.lx.set_mention_skills(skills.clone());
                }
            }
        }
    }
}

fn set_mcp(state: &mut AppState, project: &Path, count: StartupCount) {
    for workspace in &mut state.workspaces {
        if workspace.repo_key().is_none_or(|root| root != project) {
            continue;
        }
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    pane.lx.startup.mcp = count;
                }
            }
        }
    }
}

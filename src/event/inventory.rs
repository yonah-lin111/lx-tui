//! 首页 MCP / skill 资源计数的启动装载与状态汇总。
//!
//! 计数按项目根集中存放，窗格只读取同步后的快照；这样新分割的窗格、切换
//! 工作区与多个窗格共享同一项目时不会各算一份或从 0 开始。

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
        // 已有项目保留状态：新开工作区只补齐新根，避免把已就绪的旧项目重置成
        // loading 后因为没有新的启动事件而永远卡住。
        if state.mcp_inventory.contains_key(root) {
            continue;
        }
        let total = super::chat::mcp::effective_for_root(persistence, root)
            .iter()
            .filter(|server| server.config.enabled)
            .count();
        state.mcp_inventory.insert(
            root.clone(),
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
        let current = state.mcp_inventory.get(root).copied().unwrap_or_default();
        let errors = current.errors.min(total);
        let ready = current.ready.min(total.saturating_sub(errors));
        state.mcp_inventory.insert(
            root.clone(),
            StartupCount {
                total,
                ready,
                errors,
                loading: total > 0 && ready.saturating_add(errors) < total,
            },
        );
    }
    super::chat::mcp::prewarm_projects(persistence, roots);
}

/// 单个 server 状态变化后，按项目重算 ready / error / loading。
pub(crate) fn update_mcp_status(
    state: &mut AppState,
    project: &Path,
    statuses: &std::collections::BTreeMap<(PathBuf, String), McpStatusView>,
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
    let current = state
        .mcp_inventory
        .get(project)
        .copied()
        .unwrap_or_default();
    state.mcp_inventory.insert(
        project.to_path_buf(),
        StartupCount {
            total: current.total.max(total),
            ready,
            errors,
            loading: loading || current.loading,
        },
    );
}

/// 启动进度：完成前保持 loading，完成后按已结算状态停止 loading。
pub(crate) fn update_mcp_progress(state: &mut AppState, project: &Path, done: usize, total: usize) {
    let mut count = state
        .mcp_inventory
        .get(project)
        .copied()
        .unwrap_or_default();
    count.total = total;
    count.loading = total > 0 && done < total;
    state.mcp_inventory.insert(project.to_path_buf(), count);
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
        apply_skills_loaded(state, root, 0, 0, &[]);
        return;
    };
    state
        .skill_inventory
        .entry(root.clone())
        .or_default()
        .loading = true;
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let (root, total, errors, entries) = job.run();
        let _ = sender.send(AppEvent::SkillsLoaded {
            root,
            total,
            errors,
            entries,
        });
    });
}

/// 写入后台扫描结果，并把 skill 候选复制到同一工作区的窗格。
pub(crate) fn apply_skills_loaded(
    state: &mut AppState,
    root: Option<PathBuf>,
    total: usize,
    errors: usize,
    entries: &[crate::skills::SkillEntry],
) {
    state.skill_inventory.insert(
        root.clone(),
        StartupCount {
            total,
            ready: total.saturating_sub(errors),
            errors,
            loading: false,
        },
    );
    for workspace in &mut state.workspaces {
        if workspace.repo_key().map(std::path::Path::to_path_buf) != root {
            continue;
        }
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    pane.lx.set_mention_skills(entries.to_vec());
                }
            }
        }
    }
}

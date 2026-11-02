//! 首页 MCP / skill 资源计数的启动装载与状态汇总。
//!
//! 计数按项目根集中存放，窗格只读取同步后的快照；这样新分割的窗格、切换
//! 工作区与多个窗格共享同一项目时不会各算一份或从 0 开始。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::app::state::AppState;
use crate::app::state::lx::StartupCount;
use crate::app::toast::Toast;
use crate::app::update;
use crate::config::Config;
use crate::db::service::Persistence;
use crate::layout::PaneId;
use crate::mcp::{McpEvent, ServerStatus};
use crate::settings::service::{McpStatusView, SettingsEvent, SettingsService};
use crate::ui::text;

use super::AppEvent;

/// 某个项目根下的全部 lx 窗格（MCP 反馈按项目落到对应窗格）。
pub(crate) fn panes_for_project(state: &AppState, project: &Path) -> Vec<PaneId> {
    let mut panes = Vec::new();
    for workspace in &state.workspaces {
        if workspace.repo_key().is_none_or(|root| root != project) {
            continue;
        }
        for tab in &workspace.tabs {
            panes.extend(tab.layout.pane_ids());
        }
    }
    panes
}

/// 当前所有打开工作区的 skill 作用域根（去重、保持稳定顺序）。
pub(crate) fn skill_roots(state: &AppState) -> Vec<Option<PathBuf>> {
    let mut roots: Vec<Option<PathBuf>> = Vec::new();
    for workspace in &state.workspaces {
        let root = workspace.repo_key().map(Path::to_path_buf);
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

/// 处理设置页事件：保存的配置、命令、模型、MCP 与 Skills 变更在事件循环内应用。
pub(crate) fn drain_settings_events(
    receiver: &mut UnboundedReceiver<SettingsEvent>,
    state: &mut AppState,
    persistence: &Persistence,
    config: &mut Config,
    commands_reload: &mut bool,
    skills_reload: &mut bool,
    mcp_statuses: &mut BTreeMap<(PathBuf, String), McpStatusView>,
) -> bool {
    let mut changed = false;
    while let Ok(event) = receiver.try_recv() {
        match event {
            SettingsEvent::ConfigChanged(updated) => *config = updated,
            SettingsEvent::CommandsChanged => *commands_reload = true,
            SettingsEvent::ModelsChanged(settings) => {
                // 模型配置已落盘：更新快照；在途请求继续持有旧快照到结束，
                // 下一轮发送才用新配置。
                update::apply_models_changed(state, &settings);
            }
            SettingsEvent::McpChanged => {
                let roots = super::chat::mcp::project_roots(state);
                prune_mcp_statuses(persistence, &roots, mcp_statuses);
                refresh_mcp(state, persistence, &roots, mcp_statuses);
            }
            SettingsEvent::McpRestart { server } => {
                let roots = super::chat::mcp::project_roots(state);
                super::chat::mcp::restart_projects(persistence, &roots, Some(&server));
            }
            SettingsEvent::McpCredentialsChanged => {
                crate::mcp::global_manager().bump_credentials();
                let roots = super::chat::mcp::project_roots(state);
                super::chat::mcp::prewarm_projects(persistence, &roots);
            }
            SettingsEvent::SkillsChanged => {
                super::chat::skills::invalidate_skill_cache();
                *skills_reload = true;
            }
        }
        changed = true;
    }
    changed
}

/// 处理 MCP 运行事件：状态快照、启动进度与失败/登录提示。
pub(crate) fn drain_mcp_events(
    receiver: &mut UnboundedReceiver<McpEvent>,
    state: &mut AppState,
    settings_service: Option<&SettingsService>,
    mcp_statuses: &mut BTreeMap<(PathBuf, String), McpStatusView>,
    mcp_toasted: &mut HashSet<(PathBuf, String, String)>,
) -> bool {
    let mut changed = false;
    while let Ok(event) = receiver.try_recv() {
        match event {
            McpEvent::Status {
                project,
                server,
                status,
            } => {
                let (label, error) = match &status {
                    ServerStatus::Disabled => ("disabled", None),
                    ServerStatus::Starting => ("starting", None),
                    ServerStatus::Ready { .. } => ("ready", None),
                    ServerStatus::Failed { error } => ("failed", Some(error.clone())),
                    ServerStatus::NeedsLogin => ("needs_login", None),
                };
                mcp_statuses.insert(
                    (project.clone(), server.clone()),
                    McpStatusView {
                        project: project.clone(),
                        server: server.clone(),
                        status: label.into(),
                        error: error.clone(),
                    },
                );
                update_mcp_status(state, &project, mcp_statuses);
                let notice = match &status {
                    ServerStatus::Failed { error } => Some((
                        text::mcp::start_failed(&server, error),
                        error.clone(),
                        crate::app::toast::ToastKind::Danger,
                    )),
                    ServerStatus::NeedsLogin => Some((
                        text::mcp::needs_login(&server),
                        "needs_login".to_string(),
                        crate::app::toast::ToastKind::Warning,
                    )),
                    _ => None,
                };
                if let Some((message, key, kind)) = notice
                    && mcp_toasted.insert((project.clone(), server.clone(), key))
                {
                    let panes = panes_for_project(state, &project);
                    for pane in panes {
                        update::show_lx_toast(
                            state,
                            pane,
                            Toast::new(kind, message.clone(), Some(pane), Instant::now()),
                        );
                    }
                }
                if let Some(service) = settings_service {
                    service.set_mcp_status(mcp_statuses.values().cloned().collect());
                }
                changed = true;
            }
            McpEvent::StartupProgress {
                project,
                done,
                total,
            } => {
                update_mcp_progress(state, &project, done, total);
                changed = true;
            }
        }
    }
    changed
}

/// 启动（或首次打开工作区）时按配置初始化 MCP 计数与 `@` 面板候选并后台预热。
pub(crate) fn initialize_mcp(state: &mut AppState, persistence: &Persistence, roots: &[PathBuf]) {
    for root in roots {
        let effective = super::chat::mcp::effective_for_root(persistence, root);
        let mentions: Vec<crate::app::prompt::McpMentionEntry> = effective
            .iter()
            .filter(|server| server.config.enabled)
            .map(|server| {
                crate::app::prompt::McpMentionEntry::new(
                    server.name.clone(),
                    server.config.summary(),
                )
            })
            .collect();
        apply_mcp_mentions(state, root, &mentions);
        // 已有项目保留状态：新开工作区只补齐新根，避免把已就绪的旧项目重置成
        // loading 后因为没有新的启动事件而永远卡住。
        if state.mcp_inventory.contains_key(root) {
            continue;
        }
        let total = mentions.len();
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

fn apply_mcp_mentions(
    state: &mut AppState,
    root: &Path,
    mentions: &[crate::app::prompt::McpMentionEntry],
) {
    let prompt_root = state
        .prompt_owner()
        .repo_key()
        .map(std::path::Path::to_path_buf);
    if prompt_root.as_deref() == Some(root) {
        state.prompt.set_mention_mcp_servers(mentions.to_vec());
    }
    for workspace in &mut state.workspaces {
        if workspace.repo_key() != Some(root) {
            continue;
        }
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(pane) = tab.pane_mut(id) {
                    pane.lx.set_mention_mcp_servers(mentions.to_vec());
                }
            }
        }
    }
}

/// 配置变化后清掉已移除 server 的旧状态，避免下一次事件把准备计数又加回来。
pub(crate) fn prune_mcp_statuses(
    persistence: &Persistence,
    roots: &[PathBuf],
    statuses: &mut std::collections::BTreeMap<(PathBuf, String), McpStatusView>,
) {
    let known: std::collections::HashSet<(PathBuf, String)> = roots
        .iter()
        .flat_map(|root| {
            super::chat::mcp::effective_for_root(persistence, root)
                .into_iter()
                .filter(|server| server.config.enabled)
                .map(|server| (root.clone(), server.name))
        })
        .collect();
    statuses.retain(|key, _| known.contains(key));
}

/// 配置变化后刷新总数、状态聚合与 `@` 面板候选。
pub(crate) fn refresh_mcp(
    state: &mut AppState,
    persistence: &Persistence,
    roots: &[PathBuf],
    statuses: &std::collections::BTreeMap<(PathBuf, String), McpStatusView>,
) {
    for root in roots {
        let effective = super::chat::mcp::effective_for_root(persistence, root);
        let mentions: Vec<crate::app::prompt::McpMentionEntry> = effective
            .iter()
            .filter(|server| server.config.enabled)
            .map(|server| {
                crate::app::prompt::McpMentionEntry::new(
                    server.name.clone(),
                    server.config.summary(),
                )
            })
            .collect();
        apply_mcp_mentions(state, root, &mentions);
        let total = mentions.len();
        state.mcp_inventory.entry(root.clone()).or_default().total = total;
        update_mcp_status(state, root, statuses);
    }
    super::chat::mcp::prewarm_projects(persistence, roots);
}

/// 单个 server 状态或配置变化后，按项目重算 ready / error / loading。
pub(crate) fn update_mcp_status(
    state: &mut AppState,
    project: &Path,
    statuses: &std::collections::BTreeMap<(PathBuf, String), McpStatusView>,
) {
    let mut ready: usize = 0;
    let mut errors: usize = 0;
    let mut starting: usize = 0;
    let mut tracked: usize = 0;
    for ((root, _), status) in statuses {
        if root.as_path() != project {
            continue;
        }
        tracked += 1;
        match status.status.as_str() {
            "ready" => ready += 1,
            "failed" | "needs_login" => errors += 1,
            "starting" => starting += 1,
            _ => {}
        }
    }
    let current = state
        .mcp_inventory
        .get(project)
        .copied()
        .unwrap_or_default();
    let total = current.total.max(tracked);
    let settled = ready.saturating_add(errors);
    state.mcp_inventory.insert(
        project.to_path_buf(),
        StartupCount {
            total,
            ready,
            errors,
            loading: starting > 0 || (total > 0 && settled < total),
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

/// 后台预扫描全部已打开工作区的 skill 目录；结果经 [`AppEvent::SkillsLoaded`] 回到事件循环。
pub(crate) fn spawn_skills_preload(
    state: &mut AppState,
    persistence: &Persistence,
    sender: &UnboundedSender<AppEvent>,
) {
    let has_runtime = tokio::runtime::Handle::try_current().is_ok();
    for root in skill_roots(state) {
        let Some(job) = super::chat::skills::prepare_preload(persistence, root.as_deref(), false)
        else {
            apply_skills_loaded(state, root, 0, 0, &[]);
            continue;
        };
        if !state.skill_inventory.contains_key(&root) {
            state
                .skill_inventory
                .entry(root.clone())
                .or_default()
                .loading = true;
        }
        if !has_runtime {
            let (root, total, errors, entries) = job.run();
            apply_skills_loaded(state, root, total, errors, &entries);
            continue;
        }
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
}

/// 写入后台扫描结果，并把 skill 候选复制到同一工作区的窗格与右侧 Prompt。
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
    if state
        .prompt_owner()
        .repo_key()
        .map(std::path::Path::to_path_buf)
        == root
    {
        state.prompt.set_mention_skills(entries.to_vec());
    }
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

/// 当右侧 Prompt 切换所属工作区时，同步该工作区的 MCP `@` 面板候选。
pub(crate) fn sync_prompt_mcp_mentions(state: &mut AppState, persistence: &Persistence) {
    let Some(root) = state
        .prompt_owner()
        .repo_key()
        .map(std::path::Path::to_path_buf)
    else {
        state.prompt.set_mention_mcp_servers(Vec::new());
        return;
    };
    let mentions: Vec<crate::app::prompt::McpMentionEntry> =
        super::chat::mcp::effective_for_root(persistence, &root)
            .into_iter()
            .filter(|server| server.config.enabled)
            .map(|server| {
                crate::app::prompt::McpMentionEntry::new(server.name, server.config.summary())
            })
            .collect();
    state.prompt.set_mention_mcp_servers(mentions);
}

/// 刷新所有已打开工作区的生效 `AGENTS.md` 文件数量（全局 `~/.lx-t[-dev]` + 已信任项目目录链）。
pub(crate) fn refresh_agents_md(state: &mut AppState, persistence: &Persistence) {
    let user_root = crate::config::user_data_root().ok();
    for root in skill_roots(state) {
        let trusted = root
            .as_deref()
            .is_some_and(|path| persistence.is_trusted(path));
        let count = crate::chat::agent::load_instructions(
            user_root.as_deref(),
            root.as_deref(),
            root.as_deref(),
            trusted,
        )
        .active_count();
        state.agents_md_inventory.insert(root, count);
    }
}

#[cfg(test)]
mod tests;

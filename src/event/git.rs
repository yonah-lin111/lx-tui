//! git 后台查询：worktree 列表与工作区元数据的执行与投递。

use std::collections::HashSet;
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::overlay::Overlay;
use crate::app::state::{AppState, WorkspaceGit};
use crate::app::update;
use crate::git;

use super::AppEvent;

/// 发起工作区元数据查询、信任仓库根解析与对话框列表查询；结果经 `AppEvent` 回投主循环。
///
/// `inflight` 记录对话框查询的仓库根，避免 loading 期间重复发起。
pub fn pump_git_queries(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
    inflight: &mut HashSet<PathBuf>,
) {
    for cwd in update::take_git_requests(state) {
        let sender = sender.clone();
        tokio::task::spawn_blocking(move || {
            let result = match git::query_worktrees(&cwd) {
                git::WorktreeQuery::Repository(list) => match git::checkout(&list, &cwd) {
                    Some(checkout) => update::GitRefresh::Resolved(Some(WorkspaceGit {
                        repo_root: checkout.repo_root,
                        checkout_path: checkout.checkout_path,
                        is_linked: checkout.is_linked,
                        branch: checkout.branch,
                        main_branch: checkout.main_branch,
                    })),
                    // 列表成功但 cwd 不落在任何 checkout（符号链接/挂载差异等）：状态未知。
                    None => update::GitRefresh::Failed,
                },
                git::WorktreeQuery::NotRepository => update::GitRefresh::Resolved(None),
                git::WorktreeQuery::Failed => {
                    tracing::warn!(cwd = %cwd.display(), "git workspace metadata query failed");
                    update::GitRefresh::Failed
                }
            };
            let _ = sender.send(AppEvent::GitRefreshed { cwd, result });
        });
    }

    for resolution in update::take_trust_resolutions(state) {
        let sender = sender.clone();
        tokio::task::spawn_blocking(move || {
            // 非仓库路径回退自身（此时信任目标就是该路径）。
            let root = git::list_worktrees(&resolution.path)
                .map_or_else(|| resolution.path.clone(), |list| list.repo_root);
            let _ = sender.send(AppEvent::TrustRootResolved {
                path: resolution.path,
                root,
                trigger: resolution.trigger,
            });
        });
    }

    let Some(repo_root) = dialog_repo_root(state) else {
        return;
    };
    if !inflight.insert(repo_root.clone()) {
        return;
    }
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let (entries, failed) = match git::list_worktrees(&repo_root) {
            Some(list) => (list.entries, false),
            None => (Vec::new(), true),
        };
        let _ = sender.send(AppEvent::WorktreeListed {
            repo_root,
            entries,
            failed,
        });
    });
}

/// 对话框待加载的仓库根；无对话框或已加载时 None。
fn dialog_repo_root(state: &AppState) -> Option<PathBuf> {
    match state.overlay.as_ref() {
        Some(Overlay::WorktreeOpen(dialog)) if dialog.loading => Some(dialog.repo_root.clone()),
        _ => None,
    }
}

//! git 后台查询：worktree 列表与工作区元数据的执行与投递。

use std::collections::HashSet;
use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::overlay::Overlay;
use crate::app::state::{AppState, WorkspaceGit};
use crate::app::update;
use crate::git;

use super::AppEvent;

/// 发起工作区元数据查询与对话框列表查询；结果经 `AppEvent` 回投主循环。
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
            let checkout = git::list_worktrees(&cwd)
                .and_then(|list| git::checkout(&list, &cwd))
                .map(|checkout| WorkspaceGit {
                    repo_root: checkout.repo_root,
                    checkout_path: checkout.checkout_path,
                    is_linked: checkout.is_linked,
                    branch: checkout.branch,
                });
            let _ = sender.send(AppEvent::GitRefreshed { cwd, checkout });
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

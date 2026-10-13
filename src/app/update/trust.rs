//! 信任行为：应用内触发（新建工作区、打开 worktree、终端同步）的浮层、决策与续作/回退。
//!
//! 启动进入的信任门在系统控制台完成（进入 TUI 前，见 `event::startup_trust`），
//! 不经过本模块的浮层。

use std::path::PathBuf;
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::overlay::Overlay;
use crate::app::state::{AppState, home_dir, workspace_label};
use crate::app::toast::{Toast, ToastKind};
use crate::app::trust::{TrustResolution, TrustTrigger, TrustWorkspace};
use crate::ui::text;

/// 打开信任浮层；`path` 为信任目标（git 工作区为其仓库主 checkout）。
pub fn open_trust_dialog(state: &mut AppState, path: PathBuf, trigger: TrustTrigger) {
    state.overlay = Some(Overlay::TrustWorkspace(TrustWorkspace { path, trigger }));
}

/// 信任通过：执行触发场景的续作（新建工作区、打开 worktree 或进入同步确认）。
pub fn resume_trust(state: &mut AppState, trigger: TrustTrigger) {
    match trigger {
        TrustTrigger::CreateWorkspace { cwd } => {
            super::create_workspace_with_cwd(state, Some(cwd));
        }
        TrustTrigger::SyncWorkspaceCwd { workspace, pane } => {
            super::open_confirm_sync_workspace_cwd(state, workspace, pane);
        }
        TrustTrigger::OpenWorktree {
            entry,
            repo_root,
            main_branch,
        } => {
            super::worktree::open_worktree_checkout(state, entry, repo_root, main_branch);
        }
    }
}

/// 登记仓库根解析请求：显式路径未信任时由事件层后台解析所属仓库主 checkout 后回投。
pub fn request_trust_root(state: &mut AppState, path: PathBuf, trigger: TrustTrigger) {
    state
        .trust_resolutions
        .push(TrustResolution { path, trigger });
}

/// 取走待解析的仓库根请求列表；事件层后台解析后回投 `TrustRootResolved`。
pub fn take_trust_resolutions(state: &mut AppState) -> Vec<TrustResolution> {
    std::mem::take(&mut state.trust_resolutions)
}

/// 信任浮层决策：授予则记录信任并续作；拒绝则取消操作并提示。
pub fn apply_trust_decision(state: &mut AppState, granted: bool) {
    let Some(Overlay::TrustWorkspace(trust)) = state.overlay.take() else {
        return;
    };
    if granted {
        let name = workspace_label(&trust.path, home_dir().as_deref());
        state.effects.push(Effect::GrantTrust {
            path: trust.path.clone(),
            name,
        });
        resume_trust(state, trust.trigger);
        return;
    }
    match trust.trigger {
        TrustTrigger::SyncWorkspaceCwd { .. } => toast_cancelled(state, text::TOAST_SYNC_CANCELLED),
        TrustTrigger::CreateWorkspace { .. } | TrustTrigger::OpenWorktree { .. } => {
            toast_cancelled(state, text::TOAST_CREATE_CANCELLED);
        }
    }
}

/// 取消操作的工作区告警 toast。
fn toast_cancelled(state: &mut AppState, message: &str) {
    let toast = Toast::new(ToastKind::Error, message, None, Instant::now())
        .with_title(text::TOAST_WORKSPACE_TITLE);
    super::show_toast(state, toast);
}

#[cfg(test)]
mod tests;

//! 信任行为：启动、新建工作区与终端同步三条路径的浮层、决策与续作/回退。

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::app::effects::Effect;
use crate::app::overlay::Overlay;
use crate::app::state::{AppState, home_dir, workspace_label};
use crate::app::toast::{Toast, ToastKind};
use crate::app::trust::{TrustTrigger, TrustWorkspace};
use crate::ui::text;

/// 打开信任浮层。
pub fn open_trust_dialog(state: &mut AppState, path: PathBuf, trigger: TrustTrigger) {
    state.overlay = Some(Overlay::TrustWorkspace(TrustWorkspace { path, trigger }));
}

/// 信任检查结果：已信任直接续作；未信任弹出信任浮层。
pub fn resolve_trust_check(
    state: &mut AppState,
    path: &Path,
    trusted: bool,
    trigger: TrustTrigger,
) {
    if trusted {
        resume_trust(state, trigger);
    } else {
        open_trust_dialog(state, path.to_path_buf(), trigger);
    }
}

/// 信任浮层决策：授予则记录信任并续作；拒绝按场景回退（启动退出 / 取消并提示）。
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
        TrustTrigger::Startup => {
            // 冷启动拒绝信任：干净退出，由入口恢复终端后输出提示。
            state.trust_refused = true;
            state.should_quit = true;
        }
        TrustTrigger::CreateWorkspace { .. } => {
            toast_cancelled(state, text::TOAST_CREATE_CANCELLED)
        }
        TrustTrigger::SyncWorkspaceCwd { .. } => toast_cancelled(state, text::TOAST_SYNC_CANCELLED),
    }
}

/// 信任授予后的续作：启动直接放行、新建创建工作区、同步打开二次确认。
fn resume_trust(state: &mut AppState, trigger: TrustTrigger) {
    match trigger {
        TrustTrigger::Startup => {}
        TrustTrigger::CreateWorkspace { cwd } => {
            super::create_workspace_with_cwd(state, Some(cwd));
        }
        TrustTrigger::SyncWorkspaceCwd { workspace, pane } => {
            super::open_confirm_sync_workspace_cwd(state, workspace, pane);
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

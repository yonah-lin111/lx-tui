//! 信任行为单元测试：三条触发路径的检查、授予续作与拒绝回退。

use std::path::PathBuf;

use super::*;
use crate::app::actions::OverlayKey;
use crate::app::effects::Effect;
use crate::app::overlay::{NewWorkspace, Overlay, TextInput};
use crate::app::state::AppState;
use crate::app::trust::TrustTrigger;
use crate::app::update::apply_overlay_key;

fn workspace_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("lx-tui-trust-{label}"))
}

/// 打开新建工作区弹窗并输入显式路径。
fn new_workspace_dialog_with(state: &mut AppState, path: &str) {
    state.overlay = Some(Overlay::NewWorkspace(NewWorkspace {
        input: TextInput::new(path),
    }));
}

#[test]
fn explicit_new_workspace_queues_trust_check() {
    let dir = workspace_dir("explicit");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let mut state = AppState::demo();
    new_workspace_dialog_with(&mut state, dir.to_str().expect("utf8 path"));
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(state.overlay.is_none(), "弹窗关闭");
    assert_eq!(state.workspaces.len(), 1, "未检查前不创建");
    let effect = state
        .effects
        .iter()
        .find(|effect| matches!(effect, Effect::CheckTrust { .. }))
        .cloned();
    let Some(Effect::CheckTrust {
        path,
        root,
        trigger,
    }) = effect
    else {
        panic!("trust check queued");
    };
    assert_eq!(path, std::fs::canonicalize(&dir).expect("canonical"));
    assert!(root.is_none(), "显式路径的仓库归属由事件层后台解析");
    assert!(matches!(trigger, TrustTrigger::CreateWorkspace { .. }));
}

#[test]
fn empty_new_workspace_inherits_path_without_check() {
    let mut state = AppState::demo();
    new_workspace_dialog_with(&mut state, "");
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert_eq!(state.workspaces.len(), 2, "继承路径直接创建");
    assert!(
        !state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::CheckTrust { .. }))
    );
}

#[test]
fn resume_creates_workspace_and_untrusted_requests_root_resolution() {
    let trusted = workspace_dir("trusted");
    let untrusted = workspace_dir("untrusted");
    let mut state = AppState::demo();
    resume_trust(
        &mut state,
        TrustTrigger::CreateWorkspace {
            cwd: trusted.clone(),
        },
    );
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(
        state.workspaces[1].cwd.as_deref().map(PathBuf::from),
        Some(trusted.clone())
    );

    request_trust_root(
        &mut state,
        untrusted.clone(),
        TrustTrigger::CreateWorkspace {
            cwd: untrusted.clone(),
        },
    );
    assert_eq!(state.workspaces.len(), 2, "未信任不创建");
    assert!(state.overlay.is_none(), "仓库根未解析前不弹浮层");
    assert!(matches!(
        state.trust_resolutions.as_slice(),
        [resolution] if resolution.path == untrusted
    ));
}

#[test]
fn take_trust_resolutions_drains_queue() {
    let mut state = AppState::demo();
    let path = workspace_dir("drain");
    request_trust_root(
        &mut state,
        path.clone(),
        TrustTrigger::CreateWorkspace { cwd: path },
    );
    let taken = take_trust_resolutions(&mut state);
    assert_eq!(taken.len(), 1);
    assert!(state.trust_resolutions.is_empty(), "取走后清空");
}

/// worktree 条目：构造对话框条目与触发场景。
fn worktree_trigger(entry_path: &str, repo_root: &str) -> TrustTrigger {
    TrustTrigger::OpenWorktree {
        entry: crate::app::overlay::WorktreeOpenEntry {
            path: PathBuf::from(entry_path),
            branch: Some("feature/x".into()),
            is_bare: false,
            is_linked: true,
            already_open: None,
        },
        repo_root: PathBuf::from(repo_root),
        main_branch: Some("main".into()),
    }
}

#[test]
fn grant_on_worktree_open_creates_child_and_main_checkout() {
    let mut state = AppState::demo();
    open_trust_dialog(
        &mut state,
        PathBuf::from("/repo"),
        worktree_trigger("/repo/.wt/feat", "/repo"),
    );
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(matches!(
        state.effects.as_slice(),
        [Effect::GrantTrust { path, .. }, ..] if path == std::path::Path::new("/repo")
    ));
    assert_eq!(
        state.workspaces.len(),
        3,
        "主 checkout 自动补开 + worktree 条目"
    );
    let child = state
        .workspaces
        .iter()
        .find(|workspace| workspace.cwd.as_deref() == Some(std::path::Path::new("/repo/.wt/feat")))
        .expect("worktree workspace created");
    assert!(child.git.as_ref().is_some_and(|git| git.is_linked));
    let main = state
        .workspaces
        .iter()
        .find(|workspace| workspace.cwd.as_deref() == Some(std::path::Path::new("/repo")))
        .expect("main checkout auto-opened");
    assert!(!main.git.as_ref().expect("git metadata").is_linked);
}

#[test]
fn refuse_on_worktree_open_cancels_with_toast() {
    let mut state = AppState::demo();
    open_trust_dialog(
        &mut state,
        PathBuf::from("/repo"),
        worktree_trigger("/repo/.wt/feat", "/repo"),
    );
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert_eq!(state.workspaces.len(), 1, "拒绝不创建任何条目");
    let toast = state.toast.as_ref().expect("cancelled toast");
    assert_eq!(toast.message, crate::ui::text::TOAST_CREATE_CANCELLED);
    assert!(state.effects.is_empty(), "拒绝不写库");
}

#[test]
fn grant_trust_records_and_resumes_creation() {
    let path = workspace_dir("grant");
    let mut state = AppState::demo();
    open_trust_dialog(
        &mut state,
        path.clone(),
        TrustTrigger::CreateWorkspace { cwd: path.clone() },
    );
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(matches!(
        state.effects.as_slice(),
        [Effect::GrantTrust { path: recorded, .. }, ..] if *recorded == path
    ));
    assert_eq!(state.workspaces.len(), 2, "信任后创建工作区");
}

#[test]
fn refuse_on_sync_cancels_with_toast() {
    let path = workspace_dir("sync-refuse");
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    open_trust_dialog(
        &mut state,
        path,
        TrustTrigger::SyncWorkspaceCwd { workspace: 0, pane },
    );
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert!(!state.should_quit);
    let toast = state.toast.as_ref().expect("cancelled toast");
    assert_eq!(toast.message, crate::ui::text::TOAST_SYNC_CANCELLED);
    assert!(state.effects.is_empty(), "拒绝不写库");
}

#[test]
fn grant_on_sync_opens_confirm_dialog() {
    let path = workspace_dir("sync-grant");
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    // 让窗格具备可同步的实时 cwd。
    if let Some(target) = state.active_tab_mut().pane_mut(pane) {
        target.cwd = Some(path.clone());
    }
    open_trust_dialog(
        &mut state,
        path,
        TrustTrigger::SyncWorkspaceCwd { workspace: 0, pane },
    );
    apply_overlay_key(&mut state, OverlayKey::Enter);
    assert!(matches!(
        state.overlay,
        Some(Overlay::ConfirmSyncWorkspaceCwd(_))
    ));
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::GrantTrust { .. }))
    );
}

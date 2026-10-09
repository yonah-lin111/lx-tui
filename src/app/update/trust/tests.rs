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
    let Some(Effect::CheckTrust { path, trigger }) = effect else {
        panic!("trust check queued");
    };
    assert_eq!(path, std::fs::canonicalize(&dir).expect("canonical"));
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
fn resolve_trusted_creates_workspace_and_untrusted_opens_dialog() {
    let trusted = workspace_dir("trusted");
    let untrusted = workspace_dir("untrusted");
    let mut state = AppState::demo();
    resolve_trust_check(
        &mut state,
        &trusted,
        true,
        TrustTrigger::CreateWorkspace {
            cwd: trusted.clone(),
        },
    );
    assert_eq!(state.workspaces.len(), 2);
    assert_eq!(
        state.workspaces[1].cwd.as_deref().map(PathBuf::from),
        Some(trusted.clone())
    );

    resolve_trust_check(
        &mut state,
        &untrusted,
        false,
        TrustTrigger::CreateWorkspace {
            cwd: untrusted.clone(),
        },
    );
    let Some(Overlay::TrustWorkspace(trust)) = &state.overlay else {
        panic!("trust dialog open");
    };
    assert_eq!(trust.path, untrusted);
    assert_eq!(state.workspaces.len(), 2, "未信任不创建");
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
fn refuse_on_startup_quits_cleanly() {
    let path = workspace_dir("startup-refuse");
    let mut state = AppState::demo();
    open_trust_dialog(&mut state, path, TrustTrigger::Startup);
    apply_overlay_key(&mut state, OverlayKey::Esc);
    assert!(state.should_quit);
    assert!(state.trust_refused);
    assert!(state.overlay.is_none());
    assert!(state.effects.is_empty(), "拒绝不写库");
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

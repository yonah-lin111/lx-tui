//! 沙箱权限切换时间线条目的状态更新：切档提交、列表回填与展开/折叠切换。

use crate::app::state::{AppState, PermissionSwitchNoticeView};
use crate::config::sandbox::SandboxMode;
use crate::db::chat::{PermissionSwitchCommit, PermissionSwitchRecord};
use crate::tiling::PaneId;

use super::slash::next_pane_sequence;

/// 替换窗格的权限切换记录列表（会话加载、切档落库或撤销重载时调用）。
pub fn set_switches(
    state: &mut AppState,
    pane: PaneId,
    records: &[PermissionSwitchRecord],
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let next: Vec<PermissionSwitchNoticeView> = records
        .iter()
        .map(PermissionSwitchNoticeView::from_record)
        .collect();
    if target.lx.permission_switch.switches == next {
        return false;
    }
    target.lx.permission_switch.replace(next);
    true
}

/// 切换一条权限切换提示的展开/折叠状态。
pub fn toggle_permission_switch(state: &mut AppState, pane: PaneId, switch_id: &str) -> bool {
    let exists = state.pane_anywhere(pane).is_some_and(|target| {
        target
            .lx
            .permission_switch
            .switches
            .iter()
            .any(|switch| switch.id == switch_id)
    });
    if !exists {
        return false;
    }
    super::pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.permission_switch.toggle(switch_id)
}

/// 应用 `/permission` 选中的沙箱模式：写会话覆盖、必要时清旧审批并记录时间线切换通知。
pub(crate) fn apply_permission_selection(
    state: &mut AppState,
    pane: PaneId,
    mode: SandboxMode,
) -> bool {
    let sandbox_settings = state.sandbox.clone();
    let workspace = state.chat_session_key_for_pane(pane);
    let pane_cwd = state.workspace_cwd_for_pane(pane);
    let session_id = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone());
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return true;
    };
    let prev_effective = target.lx.sandbox_mode.unwrap_or(sandbox_settings.mode);
    let mode_changed = mode != prev_effective;
    target.lx.slash = None;
    target.lx.sandbox_mode = Some(mode);
    let mut switch_commit = None;
    if mode_changed {
        target.lx.sandbox_exact_rules.clear();
        target.lx.sandbox_prefix_rules.clear();
        if session_id.is_some() && !target.lx.chat.messages.is_empty() {
            let injected_prompt = crate::chat::agent::render_permission_switch_update(
                prev_effective,
                mode,
                &sandbox_settings,
                workspace.as_deref(),
                pane_cwd.as_deref(),
                true,
            );
            let next_seq = next_pane_sequence(&target.lx);
            target
                .lx
                .permission_switch
                .switches
                .push(PermissionSwitchNoticeView {
                    id: crate::db::new_external_id("psw"),
                    sequence: next_seq,
                    from_mode: prev_effective.as_str().to_string(),
                    to_mode: mode.as_str().to_string(),
                    injected_prompt: injected_prompt.clone(),
                    created_at_ms: crate::db::now_ms(),
                });
            if let Some(session) = session_id.as_ref() {
                switch_commit = Some(PermissionSwitchCommit {
                    session_id: session.clone(),
                    from_mode: prev_effective.as_str().to_string(),
                    to_mode: mode.as_str().to_string(),
                    injected_prompt,
                });
            }
        }
    }
    if let (Some(workspace), Some(session_id)) = (workspace, session_id) {
        state
            .effects
            .push(crate::app::effects::Effect::PersistSandboxMode {
                workspace,
                session_id,
                mode: Some(mode),
                switch_commit,
            });
    }
    super::super::show_lx_toast(
        state,
        pane,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Default,
            format!(
                "Permission: {} (applies to the next turn)",
                mode.short_label()
            ),
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

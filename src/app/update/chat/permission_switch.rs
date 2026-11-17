//! 沙箱权限切换时间线条目的状态更新：切档提交、列表回填、跨窗格同步与展开/折叠切换。

use crate::app::state::{AppState, Lx, PermissionSwitchNoticeView, SlashPanel};
use crate::config::sandbox::{SandboxMode, SandboxSettings};
use crate::db::chat::{PermissionSwitchCommit, PermissionSwitchRecord};
use crate::db::sandbox::{SandboxRuleKind, SessionSandboxRule};
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

/// 将指定会话的沙箱模式覆盖同步到所有绑定该会话的窗格；`clear_rules` 为 true 时清空会话规则缓存。
pub fn sync_session_sandbox_mode(
    state: &mut AppState,
    session_id: &str,
    mode: Option<SandboxMode>,
    clear_rules: bool,
) -> bool {
    let global = state.sandbox.mode;
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        if target.lx.chat.session_id.as_deref() != Some(session_id) {
            continue;
        }
        if target.lx.sandbox_mode != mode {
            target.lx.sandbox_mode = mode;
            changed = true;
        }
        if clear_rules
            && (!target.lx.sandbox_exact_rules.is_empty()
                || !target.lx.sandbox_prefix_rules.is_empty())
        {
            target.lx.sandbox_exact_rules.clear();
            target.lx.sandbox_prefix_rules.clear();
            changed = true;
        }
        if let Some(SlashPanel::Permission(picker)) = target.lx.slash.as_mut() {
            changed |= picker.sync_modes(global, mode);
        }
    }
    changed
}

/// 替换窗格的会话沙箱规则缓存（会话加载、撤销重载或写库失败回滚时调用）。
pub fn set_sandbox_rules(state: &mut AppState, pane: PaneId, rules: &[SessionSandboxRule]) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    for rule in rules {
        match rule.kind {
            SandboxRuleKind::Exact => exact.push(rule.tokens.clone()),
            SandboxRuleKind::Prefix => prefix.push(rule.tokens.clone()),
        }
    }
    if target.lx.sandbox_exact_rules == exact && target.lx.sandbox_prefix_rules == prefix {
        return false;
    }
    target.lx.sandbox_exact_rules = exact;
    target.lx.sandbox_prefix_rules = prefix;
    true
}

/// 设置页沙箱配置变更后热更新 `AppState.sandbox`，并刷新所有已打开的 `/permission` 面板。
pub fn apply_sandbox_changed(state: &mut AppState, settings: SandboxSettings) -> bool {
    let mut changed = state.sandbox != settings;
    let global = settings.mode;
    state.sandbox = settings;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let session = target.lx.sandbox_mode;
        if let Some(SlashPanel::Permission(picker)) = target.lx.slash.as_mut() {
            changed |= picker.sync_modes(global, session);
        }
    }
    changed
}

/// 切换一条（或其所在连续合并区间）权限切换提示的展开/折叠状态。
pub fn toggle_permission_switch(state: &mut AppState, pane: PaneId, switch_id: &str) -> bool {
    let Some(group_ids) = state
        .pane_anywhere(pane)
        .map(|target| contiguous_permission_group(&target.lx, switch_id))
        .filter(|ids| !ids.is_empty())
    else {
        return false;
    };
    super::pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.permission_switch.toggle_range(&group_ids)
}

/// 找出 `switch_id` 所属的连续合并权限切换区间（与时间线合并口径一致）。
fn contiguous_permission_group(lx: &Lx, switch_id: &str) -> Vec<String> {
    let switches = &lx.permission_switch.switches;
    let Some(idx) = switches.iter().position(|switch| switch.id == switch_id) else {
        return Vec::new();
    };
    let mut blocking = Vec::new();
    let messages = &lx.chat.messages;
    for message in messages {
        let is_legacy_report = message.role == "user"
            && !message.reports.is_empty()
            && !messages
                .iter()
                .any(|other| other.turn_index == message.turn_index && other.role == "assistant");
        if !is_legacy_report {
            blocking.push(message.sequence);
        }
    }
    for notice in &lx.chat.notices {
        blocking.push(notice.sequence);
    }
    for checkpoint in &lx.compact.checkpoints {
        blocking.push(checkpoint.sequence);
    }
    for switch in &lx.model_switch.switches {
        blocking.push(switch.sequence);
    }
    let has_blocker = |prev_seq: i64, next_seq: i64| -> bool {
        blocking
            .iter()
            .any(|&seq| seq > prev_seq && seq <= next_seq)
    };
    let mut start = idx;
    while start > 0 && !has_blocker(switches[start - 1].sequence, switches[start].sequence) {
        start -= 1;
    }
    let mut end = idx;
    while end + 1 < switches.len()
        && !has_blocker(switches[end].sequence, switches[end + 1].sequence)
    {
        end += 1;
    }
    switches[start..=end]
        .iter()
        .map(|switch| switch.id.clone())
        .collect()
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
            crate::ui::text::permission_switched_toast(mode.short_label()),
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

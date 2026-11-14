//! Agents 面板追问：私聊/汇报投递与运行中子轮的轮内 steer。

use std::time::Instant;

use super::snapshot;
use super::*;

/// 面板普通追问：私聊只留在子会话，汇报沿用逐轮报告。
pub(super) fn send_from_panel(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    agent_id: &str,
    text: &str,
    notify_parent: bool,
) -> bool {
    snapshot::send_user_input_from_panel(
        state,
        streams,
        persistence,
        pane,
        session_id,
        agent_id,
        text,
        notify_parent,
    )
}

/// 面板 steer：目标仍在运行时挂到该子轮；已结束时回退为普通追问。
pub(super) fn steer_from_panel(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    agent_id: &str,
    text: &str,
    notify_parent: bool,
) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    if text.len() > crate::db::chat::MAX_MESSAGE_BYTES {
        crate::app::update::show_lx_toast(
            state,
            pane,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Warning,
                crate::ui::text::LX_STEER_TEXT_LIMIT.to_string(),
                Some(pane),
                Instant::now(),
            ),
        );
        return true;
    }
    let live = streams
        .subagents
        .values()
        .any(|handle| handle.agent_id == agent_id && handle.status == STATUS_RUNNING);
    if !live {
        return follow_up_or_fail(
            state,
            streams,
            persistence,
            pane,
            agent_id,
            text,
            notify_parent,
        );
    }
    // 当前窗格没有该 run（同一会话在另一窗格）时退化为排队追问，不能静默丢弃。
    let has_run = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.agents.by_agent(agent_id))
        .is_some();
    if !has_run {
        return follow_up_or_fail(
            state,
            streams,
            persistence,
            pane,
            agent_id,
            text,
            notify_parent,
        );
    }
    let Some(selection) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.agents.by_agent(agent_id))
        .and_then(|run| run.chat.effective_selection(&state.models))
        .or_else(|| state.models.default_selection())
    else {
        return follow_up_or_fail(
            state,
            streams,
            persistence,
            pane,
            agent_id,
            text,
            notify_parent,
        );
    };
    // steer 实际由事件层使用冻结模型注入；这里只为 pending 记录回退快照，
    // 不要求当前 provider 配置仍然可解析。
    let model = crate::db::chat::ModelSnapshot {
        provider: selection.provider,
        model: selection.model,
        variant: selection.variant,
    };
    let full = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        let Some(run) = target.lx.agents.by_agent_mut(agent_id) else {
            return false;
        };
        if run.chat.pending_steers.len() >= crate::app::state::lx::chat::steer::MAX_PENDING_STEERS {
            true
        } else {
            run.chat
                .pending_steers
                .push(crate::app::state::lx::chat::steer::PendingSteer {
                    text: text.to_string(),
                    attachments: Vec::new(),
                    model_images: Vec::new(),
                    model,
                });
            false
        }
    };
    if full {
        crate::app::update::show_lx_toast(
            state,
            pane,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Warning,
                crate::ui::text::LX_TASK_STEER_LIMIT.to_string(),
                Some(pane),
                Instant::now(),
            ),
        );
        return true;
    }
    let label = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.agents.by_agent(agent_id))
        .map(|run| run.display_identity())
        .unwrap_or_else(|| agent_id.to_string());
    crate::app::update::show_lx_toast(
        state,
        pane,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Default,
            format!("Steering {label}"),
            Some(pane),
            Instant::now(),
        ),
    );
    true
}

/// 非 live / 无 run / 无模型时退化为排队追问；失败也要给用户可见反馈。
#[allow(clippy::too_many_arguments)]
fn follow_up_or_fail(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    agent_id: &str,
    text: &str,
    notify_parent: bool,
) -> bool {
    let Some(session_id) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        show_steer_failed(state, pane);
        return true;
    };
    if !send_from_panel(
        state,
        streams,
        persistence,
        pane,
        &session_id,
        agent_id,
        text,
        notify_parent,
    ) {
        show_steer_failed(state, pane);
    }
    true
}

fn show_steer_failed(state: &mut AppState, pane: PaneId) {
    crate::app::update::show_lx_toast(
        state,
        pane,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Danger,
            crate::ui::text::LX_STEER_FAILED.to_string(),
            Some(pane),
            Instant::now(),
        ),
    );
}

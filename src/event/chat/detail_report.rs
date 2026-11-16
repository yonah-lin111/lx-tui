//! 详情面板 `/report` 事件层：一次性总结请求的准备与启动。
//!
//! 与 `/compact` 共用 `execute_compact` 的一次性请求/检查点管线，区别只在
//! `CompactRequest.kind = Report`、独立报告提示词与完成后填主输入框。

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::effects::CompactRequest;
use crate::app::state::{AppState, DetailTarget, FrozenModel};
use crate::app::toast::{Toast, ToastKind};
use crate::db::chat::CompactKind;
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::tiling::PaneId;
use crate::ui::text;

use super::compact::execute_compact;

fn workspace_for_pane(state: &AppState, pane: PaneId) -> Option<PathBuf> {
    state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.workspace_cwd_for_pane(pane))
}

fn agent_busy(state: &AppState, pane: PaneId, agent_id: &str) -> bool {
    state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.agents.by_agent(agent_id))
        .is_some_and(|run| {
            run.is_running()
                || run.queued_count > 0
                || !run.chat.pending_steers.is_empty()
                || run.compact.is_running()
        })
}

fn btw_busy(state: &AppState, pane: PaneId, thread_id: &str) -> bool {
    state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.btw.thread_by_thread_id(thread_id))
        .is_some_and(|thread| thread.is_loading() || thread.compact.is_running())
}

fn toast(state: &mut AppState, pane: PaneId, message: impl Into<String>, warning: bool) {
    crate::app::update::show_lx_toast(
        state,
        pane,
        Toast::new(
            if warning {
                ToastKind::Warning
            } else {
                ToastKind::Default
            },
            message.into(),
            Some(pane),
            std::time::Instant::now(),
        ),
    );
}

/// 准备并启动一次面板 `/report` 总结；返回是否产生执行。
pub(crate) fn prepare(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    pane: PaneId,
    target: DetailTarget,
    note: &str,
) -> bool {
    let Some(workspace) = workspace_for_pane(state, pane) else {
        toast(state, pane, text::detail::LX_REPORT_UNAVAILABLE, true);
        return true;
    };
    let Some(database) = persistence.database() else {
        toast(state, pane, text::detail::LX_REPORT_UNAVAILABLE, true);
        return true;
    };
    let child_session = match &target {
        DetailTarget::Agent(agent_id) => {
            let Some(main) = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.session_id.clone())
            else {
                toast(state, pane, text::detail::LX_REPORT_UNAVAILABLE, true);
                return true;
            };
            let Some(record) = database
                .resolve_owned_subagent(&workspace, &main, agent_id)
                .ok()
                .flatten()
            else {
                toast(state, pane, text::detail::LX_REPORT_UNAVAILABLE, true);
                return true;
            };
            if agent_busy(state, pane, agent_id) {
                toast(state, pane, text::detail::LX_DETAIL_REPORT_BUSY, true);
                return true;
            }
            record.child_session_id
        }
        DetailTarget::Btw(thread_id) => {
            let Some(thread) = database.btw_thread(&workspace, thread_id).ok().flatten() else {
                toast(state, pane, text::detail::LX_REPORT_UNAVAILABLE, true);
                return true;
            };
            if btw_busy(state, pane, thread_id) {
                toast(state, pane, text::detail::LX_DETAIL_REPORT_BUSY, true);
                return true;
            }
            thread.child_session_id
        }
    };
    // 模型：优先 child session 持久化模型；BTW 会话通常为空，回退当前窗格/默认模型。
    let selection = database
        .chat_session_model(&workspace, &child_session)
        .ok()
        .flatten()
        .map(|model| {
            state
                .models
                .normalize_selection(crate::config::ai::ModelSelection {
                    provider: model.provider,
                    model: model.model,
                    variant: model.variant,
                })
        })
        .or_else(|| {
            state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.effective_selection(&state.models))
        })
        .or_else(|| state.models.default_selection());
    let Some(selection) = selection else {
        toast(state, pane, text::LX_CHAT_NO_MODEL, true);
        return true;
    };
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        toast(state, pane, text::LX_CHAT_NO_MODEL, true);
        return true;
    };
    let Ok(revision) = database.session_revision(&child_session) else {
        toast(state, pane, text::detail::LX_REPORT_UNAVAILABLE, true);
        return true;
    };
    let operation_id = crate::db::new_external_id("detail-report");
    let model_label = state
        .models
        .model_display_name(&frozen.provider, &frozen.model);
    if !crate::app::update::on_detail_report_started(state, pane, &target, operation_id.clone()) {
        toast(state, pane, text::detail::LX_DETAIL_REPORT_BUSY, true);
        return true;
    }
    toast(state, pane, text::detail::LX_REPORT_GENERATING, false);
    execute_compact(
        sender,
        persistence,
        CompactRequest {
            pane,
            workspace,
            session_id: child_session,
            expected_revision: u64::try_from(revision).unwrap_or(0),
            base_checkpoint_id: None,
            model: crate::db::chat::ModelSnapshot {
                provider: frozen.provider.clone(),
                model: frozen.model.clone(),
                variant: frozen.variant.clone(),
            },
            model_label,
            transport: frozen.transport.clone(),
            base_url: frozen.base_url.clone(),
            api_key: frozen.api_key.clone(),
            variant_params: frozen.variant_params.clone(),
            idle_timeout_ms: frozen.idle_timeout_ms,
            operation_id,
            kind: CompactKind::Report,
            note: note.to_string(),
        },
    );
    true
}

//! 详情会话 `/compact` 的事件层：解析子会话/模型，复用主压缩管线并回投到详情。

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::effects::CompactRequest;
use crate::app::state::{AppState, DetailTarget, FrozenModel};
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::tiling::PaneId;
use crate::ui::text;

use super::compact::{CompactOutcome, execute_compact};

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

fn toast(state: &mut AppState, pane: PaneId, message: &str) {
    crate::app::update::show_lx_toast(
        state,
        pane,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Warning,
            message.to_string(),
            Some(pane),
            std::time::Instant::now(),
        ),
    );
}

/// 准备并启动一次详情压缩；返回是否产生执行。
pub(crate) fn prepare(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    pane: PaneId,
    target: DetailTarget,
) -> bool {
    let Some(workspace) = workspace_for_pane(state, pane) else {
        toast(state, pane, text::detail::LX_COMPACT_UNAVAILABLE);
        return true;
    };
    let Some(database) = persistence.database() else {
        toast(state, pane, text::detail::LX_COMPACT_UNAVAILABLE);
        return true;
    };
    let child_session = match &target {
        DetailTarget::Agent(agent_id) => {
            let Some(main) = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.session_id.clone())
            else {
                toast(state, pane, text::detail::LX_COMPACT_UNAVAILABLE);
                return true;
            };
            let Some(record) = database
                .resolve_owned_subagent(&workspace, &main, agent_id)
                .ok()
                .flatten()
            else {
                toast(state, pane, text::detail::LX_COMPACT_UNAVAILABLE);
                return true;
            };
            if agent_busy(state, pane, agent_id) {
                toast(state, pane, text::LX_COMPACT_BUSY);
                return true;
            }
            record.child_session_id
        }
        DetailTarget::Btw(thread_id) => {
            let Some(thread) = database.btw_thread(&workspace, thread_id).ok().flatten() else {
                toast(state, pane, text::detail::LX_COMPACT_UNAVAILABLE);
                return true;
            };
            if btw_busy(state, pane, thread_id) {
                toast(state, pane, text::LX_COMPACT_BUSY);
                return true;
            }
            thread.child_session_id
        }
    };
    // 模型：优先 child session 持久化的模型（subagent spawn 模型）；BTW 会话列通常
    // 为 NULL，回退当前窗格有效模型，再回退配置默认。摘要请求用稳定模型，
    // 避免中途换模型导致同一会话的压缩结果不可复现。
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
        toast(state, pane, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        toast(state, pane, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Ok(revision) = database.session_revision(&child_session) else {
        toast(state, pane, text::detail::LX_COMPACT_UNAVAILABLE);
        return true;
    };
    let base_checkpoint_id = database
        .active_checkpoint(&workspace, &child_session)
        .ok()
        .flatten()
        .map(|checkpoint| checkpoint.id);
    let operation_id = crate::db::new_external_id("detail-compact");
    let model_label = state
        .models
        .model_display_name(&frozen.provider, &frozen.model);
    if !crate::app::update::on_detail_compact_started(state, pane, &target, operation_id.clone()) {
        toast(state, pane, text::LX_COMPACT_BUSY);
        return true;
    }
    execute_compact(
        sender,
        persistence,
        CompactRequest {
            pane,
            workspace,
            session_id: child_session,
            expected_revision: u64::try_from(revision).unwrap_or(0),
            base_checkpoint_id,
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
            kind: crate::db::chat::CompactKind::Compact,
            note: String::new(),
            report_header: None,
            report_changed_paths: Vec::new(),
            report_changed_total: 0,
        },
    );
    true
}

/// 完成回执：只认对应详情会话的 `compact.running`，迟到回执返回 false。
pub(crate) fn handle_finished(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    workspace: &std::path::Path,
    operation_id: &str,
    outcome: &CompactOutcome,
) -> bool {
    let agent_running = state.pane_anywhere(pane).is_some_and(|target| {
        target
            .lx
            .agents
            .by_session(session_id)
            .is_some_and(|run| run.compact.running.as_deref() == Some(operation_id))
    });
    let btw_running = state.pane_anywhere(pane).is_some_and(|target| {
        target
            .lx
            .btw
            .thread_by_child_session(session_id)
            .is_some_and(|thread| thread.compact.running.as_deref() == Some(operation_id))
    });
    if !agent_running && !btw_running {
        return false;
    }
    let report = state.pane_anywhere(pane).is_some_and(|target| {
        target
            .lx
            .agents
            .by_session(session_id)
            .is_some_and(|run| run.compact.running_report)
            || target
                .lx
                .btw
                .thread_by_child_session(session_id)
                .is_some_and(|thread| thread.compact.running_report)
    });
    // 提交已成功；回填读失败时保留现有视图，但必须结束 running，否则
    // `compacting` 会永久占位并让心跳以 10Hz 空转。
    let checkpoints = match outcome {
        CompactOutcome::Committed => persistence
            .database()
            .and_then(|db| db.compact_checkpoints(workspace, session_id).ok()),
        _ => None,
    };
    let result = match outcome {
        CompactOutcome::Committed => crate::app::update::DetailCompactResult::Committed,
        CompactOutcome::Empty => crate::app::update::DetailCompactResult::Empty,
        CompactOutcome::Failed { message } => {
            crate::app::update::DetailCompactResult::Failed(message.clone())
        }
    };
    let changed = crate::app::update::on_detail_compact_finished(
        state,
        pane,
        session_id,
        operation_id,
        &result,
        checkpoints.as_deref(),
        report,
    );
    if !changed || !report || !matches!(outcome, CompactOutcome::Committed) {
        return changed;
    }
    let Some(checkpoints) = checkpoints.as_deref() else {
        crate::app::update::show_lx_toast(
            state,
            pane,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Warning,
                crate::ui::text::detail::LX_REPORT_UNAVAILABLE,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return changed;
    };
    crate::app::update::finish_generated(state, pane, session_id, checkpoints);
    true
}

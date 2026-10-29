//! 子 agent 终态结算、自动压缩检查点落库与父会话报告批次投递。

use tokio::sync::oneshot;

use super::runner::{schedule_next_queued_input, sync_agent_ui_queue};
use super::*;
use crate::app::state::{RequestKey, RequestState, SubagentReportView};
use crate::chat::agent::{format_report_envelope, reports_to_json};
use crate::chat::agent_loop::HostError;
use crate::chat::compact::AutoCompactNotice;
use crate::chat::events::StopReason;
use crate::db::chat::{CompactCommit, ModelSnapshot, ReportPayload};
use crate::db::new_external_id;
use crate::layout::PaneId;

/// 子 turn 终态：落库、逐轮报告、按状态机决定继续消费队列或释放恢复标记。
pub(crate) fn finish_subagent(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    reason: StopReason,
    usage: Usage,
) -> bool {
    let Some(mut handle) = streams.subagents.remove(&key.session) else {
        return false;
    };
    let was_interrupted_by_input = streams.interrupted_subagents.remove(&handle.agent_id);
    handle.status = match reason {
        StopReason::Complete => STATUS_COMPLETED,
        StopReason::Cancelled => STATUS_CANCELLED,
        StopReason::Failed { .. } => STATUS_FAILED,
    };
    let reason_for_chat = reason.clone();
    handle.chat.close_stream(&reason_for_chat);
    let parts = assistant_parts(&handle.chat, &handle.child_assistant_id).unwrap_or_default();
    let outcome = match &reason {
        StopReason::Complete => crate::db::chat::TurnOutcome::Complete {
            stop_reason: Some("complete".into()),
        },
        StopReason::Cancelled => crate::db::chat::TurnOutcome::Interrupted,
        StopReason::Failed { code } => crate::db::chat::TurnOutcome::Failed {
            error_code: code.clone(),
        },
    };
    let duration_ms = Some((crate::db::now_ms() - handle.started_at_ms).max(0));
    let mut report_view = None;
    if !matches!(reason, StopReason::Cancelled)
        && let Some(database) = persistence.database()
    {
        let body = report_body(&handle);
        let files_changed = database
            .count_changed_paths(
                &handle.workspace,
                &key.session,
                &handle.child_user_message_id,
            )
            .unwrap_or_default();
        let payload = ReportPayload {
            title: handle.title.clone(),
            body: body.clone(),
            files_changed,
            duration_ms,
        };
        if let Err(error) = database.finish_subagent_turn(
            &handle.workspace,
            &handle.agent_id,
            &handle.child_assistant_id,
            &parts,
            &outcome,
            usage,
            Some(&payload),
        ) {
            tracing::warn!(%error, agent_id = %handle.agent_id, "subagent finish failed");
        }
        report_view = Some(SubagentReportView {
            agent_id: handle.agent_id.clone(),
            title: handle.title.clone(),
            role: handle.role,
            status: handle.status.to_string(),
            files_changed,
            duration_ms,
            body,
            delivered_at_ms: crate::db::now_ms(),
        });
    } else if let Some(database) = persistence.database()
        && let Err(error) = database.finish_subagent_turn(
            &handle.workspace,
            &handle.agent_id,
            &handle.child_assistant_id,
            &parts,
            &outcome,
            usage,
            None,
        )
    {
        tracing::warn!(%error, agent_id = %handle.agent_id, "subagent cancel finish failed");
    }

    let _ = update::on_subagent_finished(state, &key.session, handle.status, report_view, usage);
    let cancel_key = RequestKey {
        pane: handle.parent.pane,
        session: handle.child_session_id.clone(),
        generation: handle.parent.generation,
    };
    // 清理本轮流状态，但保留子会话模型快照供后续轮次复用。
    let saved_frozen = streams.frozen.get(&handle.child_session_id).cloned();
    streams.finish(&cancel_key);
    if let Some(frozen) = saved_frozen {
        streams
            .frozen
            .insert(handle.child_session_id.clone(), frozen);
    }

    // 状态机流转（design.md §5.2 item 4）：
    // - Complete：若有 queued 输入则保持恢复标记并继续消费；若队列空则释放恢复标记进入终态。
    // - Failed：清除恢复标记并暂停整个 agent 的后续队列，保留未处理输入等显式 resume。
    // - Cancelled：若是 send_input(interrupt=true) 触发的取消，封口后立即启动队首新输入；
    //   否则（close_agent / 用户停止）清除恢复标记。
    let mut continued = false;
    match &reason {
        StopReason::Complete => {
            if streams.resumed_subagents.contains(&handle.agent_id) {
                match schedule_next_queued_input(
                    state,
                    streams,
                    persistence,
                    &handle.parent,
                    &handle.workspace,
                    &handle.agent_id,
                ) {
                    Ok(Some(_)) => {
                        continued = true;
                    }
                    Ok(None) => {
                        streams.resumed_subagents.remove(&handle.agent_id);
                    }
                    Err(_) => {
                        // 队首输入 blocked（如 image_missing）：保留队列，停止继续消费。
                    }
                }
            }
        }
        StopReason::Failed { .. } => {
            streams.resumed_subagents.remove(&handle.agent_id);
        }
        StopReason::Cancelled => {
            if was_interrupted_by_input {
                streams.resumed_subagents.insert(handle.agent_id.clone());
                if let Ok(Some(_)) = schedule_next_queued_input(
                    state,
                    streams,
                    persistence,
                    &handle.parent,
                    &handle.workspace,
                    &handle.agent_id,
                ) {
                    continued = true;
                }
            } else {
                streams.resumed_subagents.remove(&handle.agent_id);
            }
        }
    }

    sync_agent_ui_queue(
        state,
        streams,
        persistence,
        handle.parent.pane,
        &handle.workspace,
        &handle.parent.session,
    );
    update::refresh_subagent_cards(state, handle.parent.pane);
    if !continued || !matches!(reason, StopReason::Cancelled) {
        let _ = handle.done.send(true);
    }
    let _ = drain_reports(state, persistence, handle.parent.pane);
    true
}

/// 子 agent 自动压缩完成：先把检查点写入 `agent_compact_checkpoint`，再更新 UI 并回执。
pub(crate) fn handle_subagent_auto_compact(
    state: &mut AppState,
    streams: &ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    notice: &AutoCompactNotice,
    reply: Option<oneshot::Sender<Result<(), HostError>>>,
) -> bool {
    let workspace = streams
        .db_workspaces
        .get(key)
        .cloned()
        .or_else(|| super::workspace_for_pane(state, key.pane));
    if let (Some(database), Some(ws)) = (persistence.database(), workspace.as_ref()) {
        let messages = database
            .load_chat_session(ws, &key.session)
            .ok()
            .flatten()
            .unwrap_or_default();
        let end_sequence = messages.iter().map(|m| m.sequence).max().unwrap_or(0);
        let base_checkpoint_id = database
            .active_checkpoint(ws, &key.session)
            .ok()
            .flatten()
            .map(|cp| cp.id);
        let expected_revision = database.session_revision(&key.session).unwrap_or(0);
        let commit = CompactCommit {
            session_id: key.session.clone(),
            expected_revision,
            base_checkpoint_id,
            start_sequence: 0,
            end_sequence,
            summary_text: notice.summary.clone(),
            model_label: notice.model_label.clone(),
            operation_id: new_external_id("autocompact"),
        };
        if let Err(err) = database.commit_compact(ws, &commit) {
            if let Some(reply) = reply {
                let _ = reply.send(Err(HostError::Failed {
                    code: err.code().to_string(),
                }));
            }
            return false;
        }
    }
    let changed = update::on_subagent_auto_compact(state, &key.session, notice);
    if let Some(reply) = reply {
        let _ = reply.send(Ok(()));
    }
    changed
}

/// 父窗格是否空闲且没有排队提交（自动投递的唯一前置条件）。
fn parent_idle(state: &AppState, pane: PaneId) -> bool {
    state.pane_anywhere(pane).is_some_and(|target| {
        matches!(target.lx.chat.request, RequestState::Idle) && target.lx.chat.queue.is_empty()
    })
}

/// 把当前所有未投递的逐轮报告合并成一条报告消息 + 一个自动 assistant 回合（单事务标记 delivered）。
pub(crate) fn drain_reports(state: &mut AppState, persistence: &Persistence, pane: PaneId) -> bool {
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.lx.chat.main_stopped)
    {
        return false;
    }
    if !parent_idle(state, pane) {
        return false;
    }
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        return false;
    };
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
        return false;
    };
    let Some(selection) = target
        .lx
        .chat
        .effective_selection(&state.models)
        .or_else(|| state.models.default_selection())
    else {
        return false;
    };
    let Some(database) = persistence.database() else {
        return false;
    };
    let pending = match database.pending_reports(&workspace, &session_id) {
        Ok(pending) => pending,
        Err(error) => {
            tracing::warn!(%error, "pending subagent reports unavailable");
            return false;
        }
    };
    let mut agent_ids = Vec::with_capacity(pending.len());
    let mut report_ids = Vec::with_capacity(pending.len());
    let mut reports = Vec::with_capacity(pending.len());
    for record in pending {
        let Some(report) = SubagentReportView::from_json(&record.report_json) else {
            tracing::warn!(agent_id = %record.agent_id, report_id = %record.report_id, "subagent report JSON invalid");
            let _ = database.mark_reports_delivered_in_batch(
                &workspace,
                &session_id,
                std::slice::from_ref(&record.report_id),
            );
            continue;
        };
        if !agent_ids.contains(&record.agent_id) {
            agent_ids.push(record.agent_id);
        }
        report_ids.push(record.report_id);
        reports.push(report);
    }
    if reports.is_empty() {
        return false;
    }
    let envelope = format!(
        "[local runtime] subagent reports:\n{}",
        reports
            .iter()
            .map(format_report_envelope)
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    let reports_json = reports_to_json(&reports);
    let model = ModelSnapshot {
        provider: selection.provider,
        model: selection.model,
        variant: selection.variant,
    };
    let committed = match database.begin_report_batch_with_ids(
        &workspace,
        &session_id,
        &envelope,
        &reports_json,
        &agent_ids,
        &report_ids,
        &model,
    ) {
        Ok(committed) => committed,
        Err(error) => {
            tracing::warn!(%error, "subagent report batch not started");
            return false;
        }
    };
    update::on_report_batch_committed(state, pane, &committed, reports)
}

/// 报告正文：子会话最后一条 assistant 的正文，超限截断。
fn report_body(handle: &SubagentHandle) -> String {
    let text = handle
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .map(|message| message.text())
        .unwrap_or_default();
    crate::chat::agent::truncate_report_body(&text)
}

/// 从子会话消息模型取 assistant 的 parts（位置来自同一时间线）。
pub(super) fn assistant_parts(
    chat: &crate::app::state::LxChat,
    assistant_id: &str,
) -> Option<Vec<crate::db::chat::ChatPart>> {
    chat.messages
        .iter()
        .find(|message| message.id == assistant_id)
        .map(crate::db::chat::chat_message_part_views)
}

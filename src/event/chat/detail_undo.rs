//! 详情 composer 的 `/undo` 事件层：目标解析、共享 `execute_rewind` 执行与回投。

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::effects::DetailUndoRequest;
use crate::app::state::{AppState, DetailTarget, RewindAction, SnapshotState};
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::tiling::PaneId;
use crate::ui::text;

use super::rewind::RewindOutcome;

/// 解析工作区键：优先聊天会话键（`/resume` 共享键），回退窗格 cwd。
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
            run.is_running() || run.queued_count > 0 || !run.chat.pending_steers.is_empty()
        })
}

fn btw_busy(state: &AppState, pane: PaneId, thread_id: &str) -> bool {
    state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.btw.thread_by_thread_id(thread_id))
        .is_some_and(|thread| thread.is_loading())
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

/// 预检：只读 DB 元数据，成功才打开确认面板。
pub(crate) fn prepare(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    target: DetailTarget,
    input_at_open: String,
) -> bool {
    let Some(workspace) = workspace_for_pane(state, pane) else {
        return false;
    };
    let Some(database) = persistence.database() else {
        return false;
    };
    let plan = match target {
        DetailTarget::Agent(agent_id) => {
            let Some(main_session) = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.session_id.clone())
            else {
                return false;
            };
            let Some(record) = database
                .resolve_owned_subagent(&workspace, &main_session, &agent_id)
                .ok()
                .flatten()
            else {
                return false;
            };
            if agent_busy(state, pane, &agent_id) {
                toast(state, pane, text::detail::LX_DETAIL_UNDO_BUSY);
                return true;
            }
            let Some(turn) = database
                .latest_undoable_private_turn(&workspace, &record.child_session_id)
                .ok()
                .flatten()
            else {
                toast(state, pane, text::detail::LX_DETAIL_UNDO_NOTHING);
                return true;
            };
            // 用 rewind_availability 的 revision/user_text 作为权威快照：它内部会再次
            // 确认目标仍是最新轮，避免两次查询之间插入了新轮。
            let Ok((availability, revision, user_text)) = persistence.rewind_availability(
                &workspace,
                &record.child_session_id,
                &turn.user_message_id,
            ) else {
                toast(state, pane, text::detail::LX_DETAIL_UNDO_UNAVAILABLE);
                return true;
            };
            let snapshot = SnapshotState::from_db(&availability);
            if !snapshot.files_available() {
                toast(state, pane, text::detail::LX_DETAIL_UNDO_UNAVAILABLE);
                return true;
            }
            crate::app::state::DetailUndoPlan {
                target: DetailTarget::Agent(agent_id),
                workspace: workspace.clone(),
                child_session: record.child_session_id,
                user_message_id: turn.user_message_id,
                expected_revision: u64::try_from(revision).unwrap_or(0),
                user_text,
                action: RewindAction::RevertAndDelete,
                input_at_open,
                snapshot,
            }
        }
        DetailTarget::Btw(thread_id) => {
            let Some(thread) = database.btw_thread(&workspace, &thread_id).ok().flatten() else {
                return false;
            };
            if btw_busy(state, pane, &thread_id) {
                toast(state, pane, text::detail::LX_DETAIL_UNDO_BUSY);
                return true;
            }
            let Some(turn) = database
                .latest_user_turn(&workspace, &thread.child_session_id)
                .ok()
                .flatten()
            else {
                toast(state, pane, text::detail::LX_DETAIL_UNDO_NOTHING);
                return true;
            };
            crate::app::state::DetailUndoPlan {
                target: DetailTarget::Btw(thread_id),
                workspace: workspace.clone(),
                child_session: thread.child_session_id,
                user_message_id: turn.user_message_id,
                expected_revision: u64::try_from(turn.revision).unwrap_or(0),
                user_text: turn.text,
                action: RewindAction::DeleteOnly,
                input_at_open,
                snapshot: SnapshotState::KnownEmpty,
            }
        }
    };
    crate::app::update::on_detail_undo_prepared(state, pane, plan)
}

/// 后台执行：独立 SQLite 连接 + 文件 IO，结果回投事件循环。
pub(crate) fn execute(
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    request: DetailUndoRequest,
) {
    let Some(path) = persistence.database_path() else {
        let _ = sender.send(AppEvent::RewindFinished {
            pane: request.pane,
            session_id: request.request.session_id.clone(),
            workspace: request.request.workspace.clone(),
            operation_id: request.operation_id,
            outcome: RewindOutcome::Failed {
                message: "undo requires persistent storage".to_string(),
            },
        });
        return;
    };
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let outcome = match crate::db::Database::open(&path) {
            Ok(database) => crate::event::chat::rewind::execute_rewind(
                &database,
                &request.request.workspace,
                &request.request,
            ),
            Err(error) => RewindOutcome::Failed {
                message: error.code().to_string(),
            },
        };
        let _ = sender.send(AppEvent::RewindFinished {
            pane: request.pane,
            session_id: request.request.session_id.clone(),
            workspace: request.request.workspace.clone(),
            operation_id: request.operation_id,
            outcome,
        });
    });
}

/// 回执分流：只认当前详情执行；迟到/过期 operation id 忽略。
pub(crate) fn handle_finished(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    workspace: &std::path::Path,
    operation_id: &str,
    outcome: RewindOutcome,
) -> bool {
    let Some(plan) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.task.as_ref())
        .and_then(|page| {
            page.detail_undo_executing
                .as_ref()
                .filter(|exec| exec.operation_id == operation_id)
                .and_then(|_| page.detail_undo_pending.clone())
        })
    else {
        return false;
    };
    match outcome {
        RewindOutcome::Failed { message } => {
            crate::app::update::on_detail_undo_failed(state, pane, &message)
        }
        RewindOutcome::Succeeded {
            user_text,
            session_deleted,
            ..
        } => {
            let Some(database) = persistence.database() else {
                return crate::app::update::on_detail_undo_failed(
                    state,
                    pane,
                    "database unavailable",
                );
            };
            let messages = database
                .load_chat_session(workspace, &plan.child_session)
                .ok()
                .flatten()
                .unwrap_or_default();
            let checkpoints = database
                .compact_checkpoints(workspace, &plan.child_session)
                .unwrap_or_default();
            crate::app::update::on_detail_undo_succeeded(
                state,
                pane,
                &plan,
                &messages,
                &checkpoints,
                session_deleted,
                &user_text,
            )
        }
    }
}

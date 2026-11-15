//! 详情 composer 的 `/rewind` 事件层：目标解析、共享 `execute_rewind` 执行与回投。
//!
//! 目标与主 `/rewind` 同语义：取最新时间线事件，普通轮次走文件 + 对话撤销，
//! compact 检查点只回退上下文指针。只读预检在事件循环内完成。

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::effects::DetailRewindRequest;
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

/// 预检：只读 DB 元数据，成功才打开动作面板。
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
    // 先解析稳定目标与子会话：撤销永远落在子会话上。
    let (target, child_session) = match &target {
        DetailTarget::Agent(agent_id) => {
            let Some(main_session) = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.chat.session_id.clone())
            else {
                return false;
            };
            let Some(record) = database
                .resolve_owned_subagent(&workspace, &main_session, agent_id)
                .ok()
                .flatten()
            else {
                return false;
            };
            if agent_busy(state, pane, agent_id) {
                toast(state, pane, text::detail::LX_DETAIL_REWIND_BUSY);
                return true;
            }
            (
                DetailTarget::Agent(agent_id.clone()),
                record.child_session_id,
            )
        }
        DetailTarget::Btw(thread_id) => {
            let Some(thread) = database.btw_thread(&workspace, thread_id).ok().flatten() else {
                return false;
            };
            if btw_busy(state, pane, thread_id) {
                toast(state, pane, text::detail::LX_DETAIL_REWIND_BUSY);
                return true;
            }
            (
                DetailTarget::Btw(thread_id.clone()),
                thread.child_session_id,
            )
        }
    };
    // 时间线取最新事件：检查点比最新用户轮更晚时，`/rewind` 撤销压缩而不是轮次。
    let latest = database
        .latest_user_turn(&workspace, &child_session)
        .ok()
        .flatten();
    let checkpoint = database
        .compact_checkpoints(&workspace, &child_session)
        .ok()
        .and_then(|mut checkpoints| checkpoints.pop())
        .filter(|checkpoint| {
            latest
                .as_ref()
                .is_none_or(|turn| checkpoint.sequence > turn.sequence)
        });
    if let Some(checkpoint) = checkpoint {
        let Ok(revision) = database.session_revision(&child_session) else {
            toast(state, pane, text::detail::LX_DETAIL_REWIND_UNAVAILABLE);
            return true;
        };
        let plan = crate::app::state::DetailRewindPlan {
            target,
            workspace,
            child_session,
            user_message_id: checkpoint.id,
            expected_revision: u64::try_from(revision).unwrap_or(0),
            input_at_open,
            user_text: String::new(),
            action: RewindAction::RestoreCompact,
            snapshot: SnapshotState::KnownEmpty,
            kind: crate::app::state::RewindTargetKind::Compact,
        };
        return crate::app::update::on_detail_rewind_prepared(state, pane, plan);
    }
    // 轮次：Agents 只认人类私聊追问，BTW 只认线程内最新问答。
    let turn = match &target {
        DetailTarget::Agent(_) => database
            .latest_undoable_private_turn(&workspace, &child_session)
            .ok()
            .flatten(),
        DetailTarget::Btw(_) => latest,
    };
    let Some(turn) = turn else {
        toast(state, pane, text::detail::LX_DETAIL_REWIND_NOTHING);
        return true;
    };
    // 用 rewind_availability 的 revision/user_text 作为权威快照：它内部会再次
    // 确认目标仍是最新轮，避免两次查询之间插入了新轮。
    let Ok((availability, revision, user_text)) =
        persistence.rewind_availability(&workspace, &child_session, &turn.user_message_id)
    else {
        toast(state, pane, text::detail::LX_DETAIL_REWIND_UNAVAILABLE);
        return true;
    };
    let plan = crate::app::state::DetailRewindPlan {
        target,
        workspace,
        child_session,
        user_message_id: turn.user_message_id,
        expected_revision: u64::try_from(revision).unwrap_or(0),
        input_at_open,
        user_text,
        action: RewindAction::RevertAndDelete,
        snapshot: SnapshotState::from_db(&availability),
        kind: crate::app::state::RewindTargetKind::Turn,
    };
    crate::app::update::on_detail_rewind_prepared(state, pane, plan)
}

/// 后台执行：独立 SQLite 连接 + 文件 IO，结果回投事件循环。
pub(crate) fn execute(
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    request: DetailRewindRequest,
) {
    let Some(path) = persistence.database_path() else {
        let _ = sender.send(AppEvent::RewindFinished {
            pane: request.pane,
            session_id: request.request.session_id.clone(),
            workspace: request.request.workspace.clone(),
            operation_id: request.operation_id,
            outcome: RewindOutcome::Failed {
                message: "rewind requires persistent storage".to_string(),
            },
        });
        return;
    };
    let sender = sender.clone();
    tokio::task::spawn_blocking(move || {
        let outcome = match crate::db::Database::open(&path) {
            // compact 检查点只回退上下文指针，不碰消息与文件。
            Ok(database) if request.request.action == RewindAction::RestoreCompact => {
                let revert = crate::db::chat::CompactRevert {
                    operation_id: request.request.operation_id.clone(),
                    session_id: request.request.session_id.clone(),
                    checkpoint_id: request.request.target_user_message_id.clone(),
                    expected_revision: i64::try_from(request.request.expected_revision)
                        .unwrap_or(-1),
                };
                database
                    .revert_compact(&request.request.workspace, &revert)
                    .map(|_| RewindOutcome::Succeeded {
                        action: RewindAction::RestoreCompact,
                        restored_files: 0,
                        already_restored: false,
                        user_text: String::new(),
                        deleted: false,
                        session_deleted: false,
                    })
                    .unwrap_or_else(|error| RewindOutcome::Failed {
                        message: error.code().to_string(),
                    })
            }
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
            page.detail_rewind_executing
                .as_ref()
                .filter(|exec| exec.operation_id == operation_id)
                .and_then(|_| page.detail_rewind_pending.clone())
        })
    else {
        return false;
    };
    match outcome {
        RewindOutcome::Failed { message } => {
            crate::app::update::on_detail_rewind_failed(state, pane, &message)
        }
        RewindOutcome::Succeeded {
            user_text,
            session_deleted,
            ..
        } => {
            let Some(database) = persistence.database() else {
                return crate::app::update::on_detail_rewind_failed(
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
            crate::app::update::on_detail_rewind_succeeded(
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

//! `/compact` 的事件层执行：增量范围 → 独立摘要请求 → 事务写检查点 → 视图重载。
//!
//! 边界（对应 design §4.2 与 §6）：
//! - 摘要请求**不是**普通聊天轮次：不追加 user/assistant 消息、不落库、不执行工具；
//! - 网络请求失败/超限/取消一律走失败分支：不写检查点、不更新 active 指针、
//!   不改变原始消息与折叠状态；
//! - 成功只在**一个** DB 事务内完成：检查点写入、时间线取号与 revision 推进；
//! - 事件循环只做状态回填，网络在 Tokio 任务里执行。

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::app::effects::{CompactRequest, CompactRevertRequest, Effect};
use crate::app::state::AppState;
use crate::chat::compact::{self, CompactInput};
use crate::db::service::Persistence;
use crate::event::AppEvent;
use crate::layout::PaneId;

/// 一次压缩的最终结果（回投到唯一事件循环）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactOutcome {
    /// 成功写入检查点。
    Committed,
    /// 没有可压缩的增量（不写检查点，不算失败）。
    Empty,
    /// 失败原因（稳定、可展示；不写检查点）。
    Failed { message: String },
}

/// 取走并执行压缩类效应；返回是否发生变化。
pub fn apply_compact_effects(
    state: &mut AppState,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| matches!(effect, Effect::ExecuteCompact(_) | Effect::RevertCompact(_)))
        .cloned()
        .collect();
    if effects.is_empty() {
        return false;
    }
    state
        .effects
        .retain(|effect| !matches!(effect, Effect::ExecuteCompact(_) | Effect::RevertCompact(_)));
    for effect in effects {
        match effect {
            Effect::ExecuteCompact(request) => execute_compact(sender, persistence, request),
            Effect::RevertCompact(request) => execute_revert(sender, persistence, request),
            _ => {}
        }
    }
    true
}

/// 启动一次压缩：装载增量 → 装配摘要请求 → 后台流式执行。
fn execute_compact(
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    request: CompactRequest,
) {
    let Some(db) = persistence.database() else {
        reply_failed(sender, &request, "compact requires persistent storage");
        return;
    };
    // 读取增量：上一个 active 检查点之后的可回放完整轮次。
    let messages = match db.load_chat_session(&request.workspace, &request.session_id) {
        Ok(Some(messages)) => messages,
        Ok(None) => {
            reply_failed(sender, &request, "session not found");
            return;
        }
        Err(error) => {
            reply_failed(sender, &request, &format!("load failed: {}", error.code()));
            return;
        }
    };
    let start_after = match request.base_checkpoint_id.as_deref() {
        Some(id) => match db
            .compact_checkpoints(&request.workspace, &request.session_id)
            .map(|checkpoints| {
                checkpoints
                    .into_iter()
                    .find(|checkpoint| checkpoint.id == id)
                    .map(|checkpoint| checkpoint.end_sequence)
            }) {
            Ok(value) => value,
            Err(error) => {
                reply_failed(sender, &request, &format!("load failed: {}", error.code()));
                return;
            }
        },
        None => None,
    };
    let playable = crate::chat::compact::playable_messages(&messages);
    let input = compact::incremental(&playable, start_after);
    if input.is_empty() {
        let _ = sender.send(AppEvent::CompactFinished {
            pane: request.pane,
            session_id: request.session_id.clone(),
            workspace: request.workspace.clone(),
            operation_id: request.operation_id.clone(),
            outcome: CompactOutcome::Empty,
        });
        return;
    }
    let chat_request =
        compact::summary_request(&input, &request.model, request.variant_params.clone());
    let sender = sender.clone();
    let persistence_path = persistence.database_path();
    tokio::spawn(async move {
        let outcome = run_compact(persistence_path, request.clone(), input, chat_request).await;
        let _ = sender.send(AppEvent::CompactFinished {
            pane: request.pane,
            session_id: request.session_id.clone(),
            workspace: request.workspace.clone(),
            operation_id: request.operation_id.clone(),
            outcome,
        });
    });
}

/// 后台：执行摘要请求并提交检查点。
async fn run_compact(
    persistence_path: Option<PathBuf>,
    request: CompactRequest,
    input: CompactInput,
    chat_request: crate::chat::events::ChatRequest,
) -> CompactOutcome {
    // 摘要请求必须完整成功：任何增量丢弃、工具调用或非 Complete 都是失败。
    //
    // 这里没有 Esc 取消入口（`/compact` 只在空闲时发起、整段摘要是一锤子请求），
    // 因此用「不可取消」通道。**必须同时持有 sender**：只留 receiver 会让
    // `watch::Receiver::changed()` 立刻返回，请求在发出前就被当成用户取消
    // （历史 bug：`summary request failed: cancelled`）。
    let (_cancel_sender, cancel) = crate::chat::client::uncancellable();
    let outcome = crate::chat::client::run_stream(
        &request.transport,
        &request.base_url,
        Some(request.api_key.as_str()),
        std::time::Duration::from_millis(request.idle_timeout_ms),
        chat_request,
        cancel,
        |_| {},
    )
    .await;
    if outcome.stop != crate::chat::events::StopReason::Complete {
        return CompactOutcome::Failed {
            message: format!("summary request failed: {}", outcome.stop.as_str()),
        };
    }
    // 只接受纯文本：出现工具调用说明请求形状被误读，直接失败（不执行任何调用）。
    let mut text = String::new();
    for delta in &outcome.deltas {
        match delta {
            crate::chat::events::ChatDelta::Part(part)
                if part.kind == crate::chat::events::PartKind::Text =>
            {
                text.push_str(&part.text);
            }
            crate::chat::events::ChatDelta::Part(_) => {}
            crate::chat::events::ChatDelta::ToolInputStarted { .. }
            | crate::chat::events::ChatDelta::ToolInputDelta { .. }
            | crate::chat::events::ChatDelta::ToolInputEnded { .. } => {
                return CompactOutcome::Failed {
                    message: "summary request returned tool calls".to_string(),
                };
            }
            _ => {}
        }
    }
    let summary = match compact::validate_summary(&text) {
        Ok(summary) => summary,
        Err(message) => {
            return CompactOutcome::Failed {
                message: message.to_string(),
            };
        }
    };
    let Some(path) = persistence_path else {
        return CompactOutcome::Failed {
            message: "compact requires persistent storage".to_string(),
        };
    };
    // 最终事务在独立连接上执行（不与事件循环共享连接）。
    let commit = crate::db::chat::CompactCommit {
        session_id: request.session_id.clone(),
        expected_revision: i64::try_from(request.expected_revision).unwrap_or(-1),
        base_checkpoint_id: request.base_checkpoint_id.clone(),
        start_sequence: input.start_sequence,
        end_sequence: input.end_sequence,
        summary_text: summary,
        model_label: request.model_label.clone(),
        operation_id: request.operation_id.clone(),
    };
    match tokio::task::spawn_blocking(move || {
        let database = crate::db::Database::open(&path)?;
        database.commit_compact(&request.workspace, &commit)
    })
    .await
    {
        Ok(Ok(_)) => CompactOutcome::Committed,
        Ok(Err(error)) => CompactOutcome::Failed {
            message: format!("checkpoint commit failed: {}", error.code()),
        },
        Err(error) => CompactOutcome::Failed {
            message: format!("checkpoint commit aborted: {error}"),
        },
    }
}

/// 启动一次 compact 撤销：只切检查点状态与 revision。
fn execute_revert(
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    request: CompactRevertRequest,
) {
    let Some(path) = persistence.database_path() else {
        let _ = sender.send(AppEvent::CompactReverted {
            pane: request.pane,
            session_id: request.session_id.clone(),
            workspace: request.workspace.clone(),
            operation_id: request.operation_id.clone(),
            outcome: Err("compact rewind requires persistent storage".to_string()),
        });
        return;
    };
    let sender = sender.clone();
    let revert = crate::db::chat::CompactRevert {
        operation_id: request.operation_id.clone(),
        session_id: request.session_id.clone(),
        checkpoint_id: request.checkpoint_id.clone(),
        expected_revision: i64::try_from(request.expected_revision).unwrap_or(-1),
    };
    let workspace = request.workspace.clone();
    let operation_id = request.operation_id.clone();
    let pane = request.pane;
    let session_id = request.session_id.clone();
    tokio::task::spawn_blocking(move || {
        let outcome = match crate::db::Database::open(&path) {
            Ok(database) => database
                .revert_compact(&workspace, &revert)
                .map(|_| ())
                .map_err(|error| error.code().to_string()),
            Err(error) => Err(error.code().to_string()),
        };
        let _ = sender.send(AppEvent::CompactReverted {
            pane,
            session_id,
            workspace,
            operation_id,
            outcome,
        });
    });
}

/// 失败回投（装配阶段，没有进入网络）。
fn reply_failed(sender: &mpsc::UnboundedSender<AppEvent>, request: &CompactRequest, message: &str) {
    let _ = sender.send(AppEvent::CompactFinished {
        pane: request.pane,
        session_id: request.session_id.clone(),
        workspace: request.workspace.clone(),
        operation_id: request.operation_id.clone(),
        outcome: CompactOutcome::Failed {
            message: message.to_string(),
        },
    });
}

/// 压缩完成回执：按结果回填或提示。
///
/// 迟到回执（`operation_id` 对不上当前执行，或窗格已切走）一律不落地。
pub fn handle_compact_finished(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    workspace: &std::path::Path,
    operation_id: &str,
    outcome: CompactOutcome,
) -> bool {
    let running = state.pane_anywhere(pane).is_some_and(|target| {
        target.lx.chat.session_id.as_deref() == Some(session_id)
            && target.lx.compact.running.as_deref() == Some(operation_id)
    });
    match outcome {
        CompactOutcome::Failed { message } => {
            if !running {
                return false;
            }
            crate::app::update::on_compact_failed(state, pane, &message)
        }
        CompactOutcome::Empty => {
            if !running {
                return false;
            }
            crate::app::update::on_compact_empty(state, pane, session_id)
        }
        CompactOutcome::Committed => {
            // 提交已成功；回填读失败时保留现有视图与 revision，绝不用空列表/0 覆盖。
            let readback = persistence.database().and_then(|db| {
                let checkpoints = db.compact_checkpoints(workspace, session_id).ok()?;
                let revision = db.session_revision(session_id).ok()?;
                Some((checkpoints, revision))
            });
            let Some((checkpoints, revision)) = readback else {
                return running
                    && crate::app::update::on_compact_finished(state, pane, session_id, None);
            };
            let changed = if running {
                crate::app::update::on_compact_finished(
                    state,
                    pane,
                    session_id,
                    Some(checkpoints.clone()),
                )
            } else if state
                .pane_anywhere(pane)
                .is_some_and(|target| target.lx.chat.session_id.as_deref() == Some(session_id))
            {
                // 窗格已切回同一会话但没在跑这一次压缩：只刷新列表与 revision。
                crate::app::update::set_compact_checkpoints(state, pane, &checkpoints)
            } else {
                false
            };
            sync_revision(state, pane, session_id, revision);
            // 其他绑定同一会话的窗格同样需要刷新（多窗格共享会话）。
            let panes = state.all_pane_ids();
            for other in panes {
                if other != pane
                    && state.pane_anywhere(other).is_some_and(|target| {
                        target.lx.chat.session_id.as_deref() == Some(session_id)
                    })
                {
                    crate::app::update::set_compact_checkpoints(state, other, &checkpoints);
                    sync_revision(state, other, session_id, revision);
                }
            }
            changed
        }
    }
}

/// compact 撤销回执：成功回填检查点与 revision，失败保留原状并提示。
///
/// 回执只认发起时冻结的 `workspace` 与 `operation_id`：切工作区/重复执行都不会串。
pub fn handle_compact_reverted(
    state: &mut AppState,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    workspace: &std::path::Path,
    operation_id: &str,
    outcome: Result<(), String>,
) -> bool {
    // 迟到回执：窗格的执行中操作已换人或已清空时丢弃。
    let current = state.pane_anywhere(pane).and_then(|target| {
        target
            .lx
            .rewind
            .executing
            .as_ref()
            .map(|execution| execution.operation_id.clone())
    });
    if current.as_deref() != Some(operation_id) {
        return false;
    }
    match outcome {
        Ok(()) => {
            let readback = persistence.database().and_then(|db| {
                let checkpoints = db.compact_checkpoints(workspace, session_id).ok()?;
                let revision = db.session_revision(session_id).ok()?;
                Some((checkpoints, revision))
            });
            let Some((checkpoints, revision)) = readback else {
                // 撤销已提交但回填失败：本地把目标标成已撤销，上下文指针立刻回退；
                // revision 保持不变，下一次重载以 DB 为准。
                let checkpoint_id = state
                    .pane_anywhere(pane)
                    .and_then(|target| {
                        target
                            .lx
                            .rewind
                            .executing
                            .as_ref()
                            .map(|execution| execution.target.user_message_id.clone())
                    })
                    .unwrap_or_default();
                return crate::app::update::on_compact_reverted(
                    state,
                    pane,
                    session_id,
                    Some(&checkpoint_id),
                    None,
                );
            };
            let changed = crate::app::update::on_compact_reverted(
                state,
                pane,
                session_id,
                None,
                Some(checkpoints.clone()),
            );
            sync_revision(state, pane, session_id, revision);
            let panes = state.all_pane_ids();
            for other in panes {
                if other != pane
                    && state.pane_anywhere(other).is_some_and(|target| {
                        target.lx.chat.session_id.as_deref() == Some(session_id)
                    })
                {
                    crate::app::update::set_compact_checkpoints(state, other, &checkpoints);
                    sync_revision(state, other, session_id, revision);
                }
            }
            changed
        }
        Err(code) => {
            // 失败：保留会话与检查点原状，清掉执行态并提示“撤销失败”。
            crate::app::update::on_compact_revert_failed(state, pane, &code)
        }
    }
}

/// 只在该窗格仍绑定同一会话时同步 revision，且绝不把 revision 降回去。
fn sync_revision(state: &mut AppState, pane: PaneId, session_id: &str, revision: i64) {
    let Ok(revision) = u64::try_from(revision) else {
        return;
    };
    if let Some(target) = state.pane_mut_anywhere(pane)
        && target.lx.chat.session_id.as_deref() == Some(session_id)
    {
        target.lx.chat.revision = target.lx.chat.revision.max(revision);
    }
}

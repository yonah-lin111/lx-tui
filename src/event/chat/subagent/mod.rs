//! 子 agent 生命周期控制入口：spawn / send_input / resume / wait / close / list / cancel_input、
//! 增量分流、面板效应与运行状态投影。
//!
//! 子轮启动、跨轮历史装配、图片归档校验、终态结算与报告投递位于 [`runner`] 子模块；
//! 等待逻辑位于 [`wait`] 子模块；子轮工具握手位于 [`tools`] 子模块。

use std::path::PathBuf;
use std::time::Instant;

use tokio::sync::{oneshot, watch};

use super::*;

mod context;
mod runner;
mod settle;
#[cfg(test)]
mod tests;
mod tools;
mod wait;

#[cfg(test)]
pub(crate) use settle::finish_subagent;
pub(crate) use settle::{
    drain_reports, finish_subagent_with_billing, handle_subagent_auto_compact,
};
pub(crate) use tools::{begin_child_tool, finish_child_tool};
pub(crate) use wait::handle_subagent_wait_ready;
#[cfg(test)]
pub(crate) use wait::settle_wait;

use crate::app::state::RequestKey;
use crate::chat::agent::{
    AgentListItemView, AgentRole, FINISHED_CODE, INPUT_NOT_FOUND_CODE, MAX_CONCURRENT_SUBAGENTS,
    NOT_FOUND_CODE, SubagentInputItem, SubagentOp, SubagentOutcome, WaitMode,
};
use crate::db::chat::{SubagentQueueState, SubagentRecord};
use crate::event::AppEvent;
use crate::layout::PaneId;

/// 运行中状态值。
pub const STATUS_RUNNING: &str = "running";
/// 已恢复空闲等待输入状态值。
pub const STATUS_IDLE: &str = "idle";
/// 队列头因图片缺失/损坏阻断状态值。
pub const STATUS_BLOCKED: &str = "blocked";
/// 完成状态值。
pub const STATUS_COMPLETED: &str = "completed";
/// 失败状态值。
pub const STATUS_FAILED: &str = "failed";
/// 取消状态值。
pub const STATUS_CANCELLED: &str = "cancelled";
/// 中断状态值（崩溃恢复或插队中断）。
pub const STATUS_INTERRUPTED: &str = "interrupted";

/// 一个在跑的子 agent 句柄（只存在于事件循环）。
pub(crate) struct SubagentHandle {
    /// 父请求键：报告投递路由与 UI 归属。
    pub parent: RequestKey,
    /// 稳定 agent id。
    pub agent_id: String,
    /// 子会话 external id。
    pub child_session_id: String,
    /// 当前子轮落库的 assistant 行（子 turn 工具归属校验的锚点）。
    pub child_assistant_id: String,
    /// 当前子轮 user 消息 id（文件 mutation 归属）。
    pub child_user_message_id: String,
    /// 角色。
    pub role: AgentRole,
    /// 面向用户的短标题。
    pub title: String,
    /// 任务原文。
    pub task: String,
    /// 取消通道；close / interrupt / 会话删除 / 退出时发送。
    pub cancel: watch::Sender<bool>,
    /// 完成通知：`wait_agent` 用。
    pub done: watch::Sender<bool>,
    /// running / completed / failed / cancelled。
    pub status: &'static str,
    /// 启动时刻（UTC 毫秒）。
    pub started_at_ms: i64,
    /// 子会话消息模型（持久化来源；窗格关闭后仍存在）。
    pub chat: crate::app::state::LxChat,
    /// 子会话 DB 工作区键。
    pub workspace: PathBuf,
}

/// 投影子 agent 对外可见状态（工具结果与 UI 列表同源）。
pub(crate) fn projected_agent_status(
    streams: &ChatStreams,
    record: &SubagentRecord,
    queue: Option<&SubagentQueueState>,
) -> String {
    if streams
        .subagents
        .values()
        .any(|handle| handle.agent_id == record.agent_id && handle.status == STATUS_RUNNING)
    {
        return STATUS_RUNNING.to_string();
    }
    if queue.and_then(|q| q.head_error_code.as_ref()).is_some() {
        return STATUS_BLOCKED.to_string();
    }
    if streams.resumed_subagents.contains(&record.agent_id) {
        return STATUS_IDLE.to_string();
    }
    if record.status == STATUS_CANCELLED
        && queue.and_then(|q| q.last_input_status.as_deref()) == Some(STATUS_INTERRUPTED)
    {
        return STATUS_INTERRUPTED.to_string();
    }
    record.status.clone()
}

/// 处理控制工具请求（测试兼容无 `call_id` 入口）。
#[cfg(test)]
pub(crate) fn handle_subagent_op(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    op: SubagentOp,
    reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
) -> bool {
    handle_subagent_op_with_call(state, streams, persistence, key, None, op, reply)
}

/// 处理控制工具请求（携带工具 `call_id` 构造幂等去重键）；返回是否产生状态变化。
pub(crate) fn handle_subagent_op_with_call(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    call_id: Option<&str>,
    op: SubagentOp,
    reply: oneshot::Sender<Result<SubagentOutcome, HostError>>,
) -> bool {
    match op {
        SubagentOp::Spawn {
            role,
            title,
            task,
            items,
        } => {
            let effective_items = if items.is_empty() {
                vec![SubagentInputItem::Text { text: task.clone() }]
            } else {
                items
            };
            let result = runner::spawn(
                state,
                streams,
                persistence,
                &key,
                call_id,
                role,
                &title,
                &task,
                &effective_items,
            );
            match result {
                Ok(outcome) => {
                    let _ = reply.send(Ok(outcome));
                }
                Err(code) => {
                    let _ = reply.send(Err(HostError::Failed { code }));
                }
            }
            true
        }
        SubagentOp::SendInput {
            target,
            items,
            interrupt,
        } => {
            let result = runner::send_input(
                state,
                streams,
                persistence,
                &key,
                call_id,
                &target,
                &items,
                interrupt,
            );
            let changed = result.is_ok();
            match result {
                Ok(outcome) => {
                    let _ = reply.send(Ok(outcome));
                }
                Err(code) => {
                    let _ = reply.send(Err(HostError::Failed { code }));
                }
            }
            changed
        }
        SubagentOp::Resume { id } => {
            let result = runner::resume(state, streams, persistence, &key, &id);
            let changed = result.is_ok();
            match result {
                Ok(outcome) => {
                    let _ = reply.send(Ok(outcome));
                }
                Err(code) => {
                    let _ = reply.send(Err(HostError::Failed { code }));
                }
            }
            changed
        }
        SubagentOp::Close { target } => {
            let result = close(state, streams, persistence, &key, &target);
            let changed = result.is_ok();
            let _ = reply.send(result);
            changed
        }
        SubagentOp::List => {
            let result = list_agents(streams, persistence, &key);
            let _ = reply.send(result);
            false
        }
        SubagentOp::CancelInput { submission_id } => {
            let result = cancel_input(state, streams, persistence, &key, &submission_id);
            let changed = result.is_ok();
            let _ = reply.send(result);
            changed
        }
        SubagentOp::Wait {
            targets,
            mode,
            timeout_ms,
        } => {
            let all = targets.is_empty();
            let auto_continue = mode == WaitMode::All;
            wait::wait(
                streams,
                persistence,
                key,
                targets,
                all,
                mode,
                timeout_ms,
                auto_continue,
                reply,
            );
            false
        }
    }
}

/// close：关闭活动运行或 idle 身份，保留子会话、队列和历史（后续可 `resume_agent`）。
fn close(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    target: &str,
) -> Result<SubagentOutcome, HostError> {
    if let Some(handle) = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == target && handle.parent.session == key.session)
    {
        let previous = handle.status.to_string();
        if handle.status != STATUS_RUNNING {
            return Err(HostError::Failed {
                code: FINISHED_CODE.to_string(),
            });
        }
        streams.resumed_subagents.remove(target);
        streams.interrupted_subagents.remove(target);
        if handle.cancel.send(true).is_err() {
            return Err(HostError::Failed {
                code: FINISHED_CODE.to_string(),
            });
        }
        return Ok(SubagentOutcome::Closed {
            agent_id: target.to_string(),
            previous_status: previous,
        });
    }
    let Some(database) = persistence.database() else {
        return Err(HostError::Failed {
            code: NOT_FOUND_CODE.to_string(),
        });
    };
    let Some(workspace) = streams.db_workspaces.get(key).cloned() else {
        return Err(HostError::Failed {
            code: NOT_FOUND_CODE.to_string(),
        });
    };
    let Ok(Some(record)) = database.owned_subagent(&workspace, &key.session, target) else {
        return Err(HostError::Failed {
            code: NOT_FOUND_CODE.to_string(),
        });
    };
    if streams.resumed_subagents.remove(target) {
        streams.interrupted_subagents.remove(target);
        let _ = database.finish_input(
            &workspace,
            &key.session,
            target,
            None,
            STATUS_CANCELLED,
            None,
        );
        if let Some(pane) = state.pane_mut_anywhere(key.pane)
            && let Some(run) = pane.lx.agents.by_agent_mut(target)
        {
            run.status = STATUS_CANCELLED.to_string();
        }
        return Ok(SubagentOutcome::Closed {
            agent_id: target.to_string(),
            previous_status: STATUS_IDLE.to_string(),
        });
    }
    let _ = record;
    Err(HostError::Failed {
        code: FINISHED_CODE.to_string(),
    })
}

/// `list_agents`：仅列当前父会话与 workspace 下的子 agent 概览。
fn list_agents(
    streams: &ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
) -> Result<SubagentOutcome, HostError> {
    let Some(database) = persistence.database() else {
        return Err(HostError::Failed {
            code: "subagent persistence unavailable".into(),
        });
    };
    let Some(workspace) = streams.db_workspaces.get(key) else {
        return Err(HostError::Failed {
            code: "subagent workspace unavailable".into(),
        });
    };
    let records = database
        .list_subagents(workspace, &key.session)
        .map_err(|error| HostError::Failed {
            code: error.code().to_string(),
        })?;
    let queue_states = database
        .subagent_queue_states(workspace, &key.session)
        .unwrap_or_default();
    let agents = records
        .iter()
        .map(|record| {
            let q = queue_states.get(&record.agent_id);
            AgentListItemView {
                agent_id: record.agent_id.clone(),
                role: record.role,
                title: record.title.clone(),
                status: projected_agent_status(streams, record, q),
                queued_count: q.map_or(0, |s| s.queued_count),
            }
        })
        .collect();
    Ok(SubagentOutcome::Listed { agents })
}

/// `cancel_agent_input`：仅将 `queued`（含 `blocked`）转为 `cancelled`；`running`/终态返回现状。
fn cancel_input(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    submission_id: &str,
) -> Result<SubagentOutcome, HostError> {
    let Some(database) = persistence.database() else {
        return Err(HostError::Failed {
            code: "subagent persistence unavailable".into(),
        });
    };
    let Some(workspace) = streams.db_workspaces.get(key).cloned() else {
        return Err(HostError::Failed {
            code: "subagent workspace unavailable".into(),
        });
    };
    let outcome = database
        .cancel_input(&workspace, &key.session, submission_id)
        .map_err(|error| HostError::Failed {
            code: error.code().to_string(),
        })?;
    let Some((agent_id, status, mut queued_count)) = outcome else {
        return Err(HostError::Failed {
            code: INPUT_NOT_FOUND_CODE.to_string(),
        });
    };
    if status == STATUS_CANCELLED
        && streams.resumed_subagents.contains(&agent_id)
        && let Ok(Some(claimed)) = runner::schedule_next_queued_input(
            state,
            streams,
            persistence,
            key,
            &workspace,
            &agent_id,
        )
    {
        queued_count = claimed.queued_remaining;
    }
    runner::sync_agent_ui_queue(
        state,
        streams,
        persistence,
        key.pane,
        &workspace,
        &key.session,
    );
    Ok(SubagentOutcome::InputCancelled {
        submission_id: submission_id.to_string(),
        agent_id,
        status,
        queued_count,
    })
}

/// 子 turn 的网络事件分流（`Delta` / `Finished`）。
pub(crate) fn handle_subagent_net_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    event: ChatNetEvent,
) -> bool {
    match event {
        ChatNetEvent::Delta { key, deltas } => {
            let bytes: usize = deltas.iter().map(ChatDelta::payload_bytes).sum();
            let entry = streams
                .checkpoints
                .entry(key.clone())
                .or_insert_with(|| (Instant::now(), 0));
            entry.1 += bytes;
            let due = entry.1 >= CHECKPOINT_BYTES || entry.0.elapsed() >= CHECKPOINT_INTERVAL;
            let mut changed = false;
            if let Some(handle) = streams.subagents.get_mut(&key.session) {
                changed |= update::apply_subagent_deltas(&mut handle.chat, &deltas);
                if due {
                    *entry = (Instant::now(), 0);
                    if let Some(parts) =
                        settle::assistant_parts(&handle.chat, &handle.child_assistant_id)
                    {
                        state.effects.push(Effect::CheckpointChatTurn {
                            workspace: handle.workspace.clone(),
                            assistant_id: handle.child_assistant_id.clone(),
                            parts,
                        });
                    }
                }
            }
            changed |= update::on_subagent_delta(state, &key.session, &deltas);
            changed
        }
        ChatNetEvent::Finished {
            key,
            reason,
            usage,
            billing,
        } => {
            if let Some(sender) = streams.event_sender.clone() {
                let _ = sender.send(AppEvent::SubagentFinished {
                    key,
                    reason,
                    usage,
                    billing,
                });
            }
            false
        }
    }
}

/// 会话被删除或用户手动停止：取消该会话下所有在册子 turn。
pub(crate) fn cancel_for_session(
    streams: &mut ChatStreams,
    session_id: &str,
    agent_id: Option<&str>,
) -> bool {
    let mut cancelled = false;
    let mut dead = Vec::new();
    let mut cleared_agents = Vec::new();
    for (child, handle) in &streams.subagents {
        let selected = agent_id.is_none_or(|target| handle.agent_id == target);
        let belongs = match agent_id {
            Some(_) => handle.parent.session == session_id,
            None => handle.parent.session == session_id || handle.child_session_id == session_id,
        };
        if selected && belongs {
            cleared_agents.push(handle.agent_id.clone());
            if handle.cancel.send(true).is_ok() {
                cancelled = true;
            } else {
                dead.push(child.clone());
            }
        }
    }
    for id in cleared_agents {
        streams.resumed_subagents.remove(&id);
        streams.interrupted_subagents.remove(&id);
    }
    if let Some(target) = agent_id {
        streams.resumed_subagents.remove(target);
        streams.interrupted_subagents.remove(target);
    }
    for child in dead {
        streams.subagents.remove(&child);
        streams.frozen.remove(&child);
    }
    cancelled
}

/// 应用子 agent 效应（列表、懒加载、停止、报告投递）。
pub fn apply_subagent_effects(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
) -> bool {
    let effects: Vec<Effect> = state
        .effects
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::ListSubagents { .. }
                    | Effect::LoadSubagentSession { .. }
                    | Effect::CancelSubagents { .. }
                    | Effect::DrainSubagentReports { .. }
            )
        })
        .cloned()
        .collect();
    if effects.is_empty() {
        return false;
    }
    state.effects.retain(|effect| {
        !matches!(
            effect,
            Effect::ListSubagents { .. }
                | Effect::LoadSubagentSession { .. }
                | Effect::CancelSubagents { .. }
                | Effect::DrainSubagentReports { .. }
        )
    });
    let mut changed = false;
    for effect in effects {
        match effect {
            Effect::ListSubagents {
                pane,
                parent_session,
            } => {
                let Some(database) = persistence.database() else {
                    continue;
                };
                let Some(workspace) = workspace_for_pane(state, pane) else {
                    continue;
                };
                match database.list_subagents(&workspace, &parent_session) {
                    Ok(mut records) => {
                        let queue_states = database
                            .subagent_queue_states(&workspace, &parent_session)
                            .unwrap_or_default();
                        for record in &mut records {
                            record.status = projected_agent_status(
                                streams,
                                record,
                                queue_states.get(&record.agent_id),
                            );
                        }
                        changed |=
                            update::set_subagent_list(state, pane, &parent_session, &records);
                        runner::sync_agent_ui_queue(
                            state,
                            streams,
                            persistence,
                            pane,
                            &workspace,
                            &parent_session,
                        );
                    }
                    Err(error) => {
                        tracing::warn!(%error, "listing subagents failed");
                    }
                }
            }
            Effect::LoadSubagentSession {
                pane,
                agent_id,
                child_session,
            } => {
                let Some(database) = persistence.database() else {
                    continue;
                };
                let Some(workspace) = workspace_for_pane(state, pane) else {
                    continue;
                };
                match database.load_chat_session(&workspace, &child_session) {
                    Ok(Some(messages)) => {
                        let checkpoints: Vec<crate::app::state::CompactNoticeView> = database
                            .compact_checkpoints(&workspace, &child_session)
                            .unwrap_or_default()
                            .iter()
                            .map(crate::app::state::CompactNoticeView::from_record)
                            .collect();
                        changed |= update::on_subagent_loaded(state, pane, &agent_id, &messages);
                        if !checkpoints.is_empty()
                            && let Some(target) = state.pane_mut_anywhere(pane)
                            && let Some(run) = target.lx.agents.by_agent_mut(&agent_id)
                        {
                            run.compact.replace(checkpoints);
                            run.rows_dirty = true;
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "loading subagent session failed");
                    }
                }
            }
            Effect::CancelSubagents {
                session_id,
                agent_id,
            } => {
                changed |= cancel_for_session(streams, &session_id, agent_id.as_deref());
            }
            Effect::DrainSubagentReports { pane } => {
                changed |= drain_reports(state, persistence, pane);
            }
            _ => {}
        }
    }
    changed
}

/// 窗格的会话 DB 工作区键（面板打开时没有在途轮次，不能读 `db_workspaces`）。
pub(super) fn workspace_for_pane(state: &AppState, pane: PaneId) -> Option<PathBuf> {
    state
        .chat_session_key_for_pane(pane)
        .or_else(|| state.workspace_cwd_for_pane(pane))
}

/// 运行中的子 agent 摘要（`runtime_context_summary` 追加段）。
pub(crate) fn running_subagent_lines(streams: &ChatStreams, parent_session: &str) -> Vec<String> {
    let running: Vec<&SubagentHandle> = streams
        .subagents
        .values()
        .filter(|handle| handle.parent.session == parent_session && handle.status == STATUS_RUNNING)
        .collect();
    if running.is_empty() {
        return Vec::new();
    }
    let mut lines =
        vec!["[local runtime] running subagents (use wait_agent / close_agent):".to_string()];
    for handle in running.iter().take(MAX_CONCURRENT_SUBAGENTS) {
        let head: String = handle
            .task
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect();
        lines.push(format!(
            "- agent_id={} role={} · {head}",
            handle.agent_id,
            handle.role.as_str()
        ));
    }
    lines
}

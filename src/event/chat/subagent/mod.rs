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
mod launch;
mod model_resolve;
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
    AgentListItemView, AgentRole, FINISHED_CODE, INPUT_NOT_FOUND_CODE, NOT_FOUND_CODE,
    SubagentInputItem, SubagentOp, SubagentOutcome, WaitMode, path_matches_prefix,
};
use crate::db::chat::{SubagentQueueState, SubagentRecord};
use crate::event::AppEvent;
use crate::layout::PaneId;

/// 运行中状态值。
pub const STATUS_RUNNING: &str = "running";
/// 已恢复空闲等待输入状态值。
pub const STATUS_IDLE: &str = "idle";
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
    /// 可选唯一昵称。
    pub nickname: Option<String>,
    /// 规范层级路径（如 `/root/turing`、`/root/turing/curie`）。
    pub agent_path: String,
    /// 所属根会话 id（整树并发计数与整树等待共用）。
    pub root_session: String,
    /// 当前层级深度（主 agent 派发为 1）。
    pub depth: usize,
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
    /// 本轮 Token 预算（spawn 覆写或全局默认）；None 表示不限。
    pub max_tokens: Option<u64>,
    /// `close_agent` / 窗格或应用关闭：结算时不生成半程汇报。
    pub closed_without_report: bool,
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
    let running = streams
        .subagents
        .values()
        .any(|handle| handle.agent_id == record.agent_id && handle.status == STATUS_RUNNING);
    let resumed_or_paused = streams.resumed_subagents.contains(&record.agent_id)
        || streams.paused_by_interrupt.contains(&record.agent_id);
    let has_blocked_head = queue.and_then(|q| q.head_error_code.as_ref()).is_some();
    let projected = crate::chat::agent::projected_agent_status(
        running,
        resumed_or_paused,
        has_blocked_head,
        &record.status,
    );
    // 崩溃恢复/插队中断留下的 `cancelled + interrupted` 组合投影为中断态。
    if projected == crate::chat::agent::AgentStatus::Cancelled
        && queue.and_then(|q| q.last_input_status.as_deref()) == Some(STATUS_INTERRUPTED)
    {
        return STATUS_INTERRUPTED.to_string();
    }
    projected.as_str().to_string()
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
            task_name,
            task,
            items,
            fork_context,
            fork_turns,
            max_tokens,
            model,
            reasoning_effort,
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
                task_name.as_deref(),
                &task,
                &effective_items,
                fork_context,
                fork_turns,
                max_tokens,
                model.as_deref(),
                reasoning_effort.as_deref(),
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
            trigger_turn,
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
                trigger_turn,
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
        SubagentOp::Interrupt { target } => {
            let result = interrupt(state, streams, persistence, &key, &target);
            let changed = result.is_ok();
            let _ = reply.send(result);
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
        SubagentOp::List { path_prefix } => {
            let result = list_agents(streams, persistence, &key, path_prefix.as_deref());
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

/// `interrupt_agent`：中止目标及其正在运行的后代，保留已生成内容与半程汇报，维持 `idle`。
fn interrupt(
    _state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    target: &str,
) -> Result<SubagentOutcome, HostError> {
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
    let tree = runner::load_subagent_tree(database, &workspace, &key.session);
    let Some(record) = runner::resolve_tree_target(&tree, target).cloned() else {
        return Err(HostError::Failed {
            code: NOT_FOUND_CODE.to_string(),
        });
    };
    let canonical_id = record.agent_id.clone();
    let queued_count = database
        .subagent_queue_states(&workspace, &record.parent_session_id)
        .ok()
        .and_then(|map| map.get(&canonical_id).map(|q| q.queued_count))
        .unwrap_or(0);
    let subtree = runner::subtree_paths(&tree.records, &record.agent_path);

    let mut previous_status = None;
    for handle in streams.subagents.values_mut() {
        if handle.status != STATUS_RUNNING || !subtree.contains(&handle.agent_path) {
            continue;
        }
        streams.paused_by_interrupt.insert(handle.agent_id.clone());
        streams.resumed_subagents.insert(handle.agent_id.clone());
        streams.interrupted_subagents.remove(&handle.agent_id);
        if handle.agent_id == canonical_id {
            previous_status = Some(STATUS_RUNNING.to_string());
        }
        let _ = handle.cancel.send(true);
    }
    if let Some(previous) = previous_status {
        return Ok(SubagentOutcome::Interrupted {
            agent_id: canonical_id,
            nickname: record.nickname,
            previous_status: previous,
            queued_count,
        });
    }

    if streams.resumed_subagents.contains(&canonical_id) {
        streams.paused_by_interrupt.insert(canonical_id.clone());
        return Ok(SubagentOutcome::Interrupted {
            agent_id: canonical_id,
            nickname: record.nickname,
            previous_status: STATUS_IDLE.to_string(),
            queued_count,
        });
    }

    Err(HostError::Failed {
        code: crate::chat::agent::NOT_ACTIVE_CODE.to_string(),
    })
}

/// close：关闭目标及其整棵子树（不产生半程汇报），保留子会话、队列和历史（后续可 `resume_agent`）。
fn close(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    target: &str,
) -> Result<SubagentOutcome, HostError> {
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
    let tree = runner::load_subagent_tree(database, &workspace, &key.session);
    let Some(record) = runner::resolve_tree_target(&tree, target).cloned() else {
        return Err(HostError::Failed {
            code: NOT_FOUND_CODE.to_string(),
        });
    };
    let canonical_id = record.agent_id.clone();
    let subtree = runner::subtree_paths(&tree.records, &record.agent_path);
    let handle_status = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == canonical_id)
        .map(|handle| handle.status);
    let idle = streams.resumed_subagents.contains(&canonical_id);
    if handle_status.is_none() && !idle {
        return Err(HostError::Failed {
            code: FINISHED_CODE.to_string(),
        });
    }
    let previous_status = handle_status.unwrap_or(STATUS_IDLE).to_string();

    // 级联：目标自身及其所有存活后代都标记「关闭不产报告」并取消在途轮次。
    for handle in streams.subagents.values_mut() {
        if !subtree.contains(&handle.agent_path) {
            continue;
        }
        streams.resumed_subagents.remove(&handle.agent_id);
        streams.interrupted_subagents.remove(&handle.agent_id);
        streams.paused_by_interrupt.remove(&handle.agent_id);
        handle.closed_without_report = true;
        let _ = handle.cancel.send(true);
    }
    let targets: Vec<SubagentRecord> = tree
        .records
        .iter()
        .filter(|candidate| subtree.contains(&candidate.agent_path))
        .cloned()
        .collect();
    for agent in &targets {
        let _ = database.close_idle_subagent(&workspace, &agent.parent_session_id, &agent.agent_id);
        streams.resumed_subagents.remove(&agent.agent_id);
        streams.interrupted_subagents.remove(&agent.agent_id);
        streams.paused_by_interrupt.remove(&agent.agent_id);
        if let Some(pane) = state.pane_mut_anywhere(key.pane)
            && let Some(run) = pane.lx.agents.by_agent_mut(&agent.agent_id)
            && run.status == STATUS_IDLE
        {
            run.status = STATUS_CANCELLED.to_string();
        }
    }
    Ok(SubagentOutcome::Closed {
        agent_id: canonical_id,
        previous_status,
    })
}

/// 停止窗格所属整棵会话树里全部正在运行的子 agent（`Agents` 面板 `^a`）。
pub(crate) fn stop_all_subagents_for_pane(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
) -> usize {
    let Some(session) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return 0;
    };
    let root_session = persistence
        .database()
        .zip(workspace_for_pane(state, pane))
        .and_then(|(database, workspace)| {
            database
                .resolve_session_tree_root(&workspace, &session)
                .ok()
                .map(|(root, _, _)| root)
        })
        .unwrap_or_else(|| session.clone());

    let mut stopped = 0usize;
    let mut dead = Vec::new();
    for (child, handle) in &mut streams.subagents {
        if handle.root_session != root_session || handle.status != STATUS_RUNNING {
            continue;
        }
        handle.closed_without_report = true;
        streams.resumed_subagents.remove(&handle.agent_id);
        streams.interrupted_subagents.remove(&handle.agent_id);
        streams.paused_by_interrupt.remove(&handle.agent_id);
        if handle.cancel.send(true).is_ok() {
            stopped = stopped.saturating_add(1);
        } else {
            dead.push(child.clone());
        }
    }
    for child in dead {
        streams.subagents.remove(&child);
        streams.frozen.remove(&child);
    }
    stopped
}

/// `list_agents`：列出根会话树下的子 agent 概览，可按 `path_prefix` 过滤。
fn list_agents(
    streams: &ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    path_prefix: Option<&str>,
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
    let tree = runner::load_subagent_tree(database, workspace, &key.session);
    let queue_states = runner::tree_queue_states(database, workspace, &tree);
    let agents = tree
        .records
        .iter()
        .filter(|record| {
            path_prefix.is_none_or(|prefix| path_matches_prefix(&record.agent_path, prefix))
        })
        .map(|record| {
            let q = queue_states.get(&record.agent_id);
            AgentListItemView {
                agent_id: record.agent_id.clone(),
                nickname: record.nickname.clone(),
                agent_path: Some(record.agent_path.clone()),
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
    // 输入行按目标的真实父会话归属：整树寻址时逐个候选父会话尝试。
    let tree = runner::load_subagent_tree(database, &workspace, &key.session);
    let mut parent_sessions = vec![key.session.clone()];
    for record in &tree.records {
        if !parent_sessions.contains(&record.parent_session_id) {
            parent_sessions.push(record.parent_session_id.clone());
        }
    }
    let mut cancelled = None;
    let mut owner_session = key.session.clone();
    for parent_session in parent_sessions {
        match database.cancel_input(&workspace, &parent_session, submission_id) {
            Ok(Some(result)) => {
                owner_session = parent_session;
                cancelled = Some(result);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                return Err(HostError::Failed {
                    code: error.code().to_string(),
                });
            }
        }
    }
    let Some((agent_id, status, mut queued_count)) = cancelled else {
        return Err(HostError::Failed {
            code: INPUT_NOT_FOUND_CODE.to_string(),
        });
    };
    let owner_key = RequestKey {
        pane: key.pane,
        session: owner_session.clone(),
        generation: key.generation,
    };
    if status == STATUS_CANCELLED
        && !streams.paused_by_interrupt.contains(&agent_id)
        && streams.resumed_subagents.contains(&agent_id)
        && let Ok(Some(claimed)) = runner::schedule_next_queued_input(
            state,
            streams,
            persistence,
            &owner_key,
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
        &owner_session,
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

/// 会话被删除或用户手动停止：取消该根会话树下所有在册子 turn（级联后代，不产报告）。
pub(crate) fn cancel_for_session(
    streams: &mut ChatStreams,
    session_id: &str,
    agent_id: Option<&str>,
) -> bool {
    // 种子：显式 agent 自身，或该会话直接相关的句柄；再沿 `agent_path` 前缀级联后代。
    let seeds: Vec<String> = streams
        .subagents
        .values()
        .filter(|handle| match agent_id {
            Some(id) => handle.agent_id == id,
            None => handle.parent.session == session_id || handle.child_session_id == session_id,
        })
        .map(|handle| handle.agent_path.clone())
        .collect();
    let selected = |path: &str| {
        seeds.iter().any(|seed| {
            path.eq_ignore_ascii_case(seed)
                || path
                    .to_ascii_lowercase()
                    .starts_with(&format!("{}/", seed.to_ascii_lowercase()))
        })
    };

    let mut cancelled = false;
    let mut cleared_agents = Vec::new();
    let mut dead = Vec::new();
    for (child, handle) in &mut streams.subagents {
        if !selected(&handle.agent_path) {
            continue;
        }
        cleared_agents.push(handle.agent_id.clone());
        // 用户手动停止 / 会话删除 / 退出：不生成半程汇报。
        handle.closed_without_report = true;
        if handle.cancel.send(true).is_ok() {
            cancelled = true;
        } else {
            dead.push(child.clone());
        }
    }
    for id in cleared_agents {
        streams.resumed_subagents.remove(&id);
        streams.interrupted_subagents.remove(&id);
        streams.paused_by_interrupt.remove(&id);
    }
    if let Some(target) = agent_id {
        streams.resumed_subagents.remove(target);
        streams.interrupted_subagents.remove(target);
        streams.paused_by_interrupt.remove(target);
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
                let tree = runner::load_subagent_tree(database, &workspace, &parent_session);
                let queue_states = runner::tree_queue_states(database, &workspace, &tree);
                let mut records = tree.records;
                for record in &mut records {
                    record.status =
                        projected_agent_status(streams, record, queue_states.get(&record.agent_id));
                }
                changed |= update::set_subagent_list(state, pane, &parent_session, &records);
                runner::sync_agent_ui_queue(
                    state,
                    streams,
                    persistence,
                    pane,
                    &workspace,
                    &parent_session,
                );
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
                changed |= match agent_id.as_deref() {
                    Some(id) => cancel_for_session(streams, &session_id, Some(id)),
                    None => {
                        // 停全部：优先按窗格所属根会话树级联；找不到窗格时退回在册句柄。
                        let pane = state.all_pane_ids().into_iter().find(|pane| {
                            state
                                .pane_anywhere(*pane)
                                .and_then(|target| target.lx.chat.session_id.as_deref())
                                == Some(session_id.as_str())
                        });
                        match pane {
                            Some(pane) => {
                                stop_all_subagents_for_pane(state, streams, persistence, pane) > 0
                            }
                            None => cancel_for_session(streams, &session_id, None),
                        }
                    }
                };
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
    for handle in running
        .iter()
        .take(crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT)
    {
        let head: String = handle
            .task
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect();
        let nickname = handle
            .nickname
            .as_deref()
            .map(str::trim)
            .filter(|nick| !nick.is_empty())
            .map(|nick| format!(" nickname={nick}"))
            .unwrap_or_default();
        let depth = if handle.depth > 1 {
            format!(" depth={}", handle.depth)
        } else {
            String::new()
        };
        lines.push(format!(
            "- agent_id={} role={}{nickname}{depth} · {head}",
            handle.agent_id,
            handle.role.as_str()
        ));
    }
    lines
}

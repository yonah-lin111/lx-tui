//! 子 agent 环境快照与面板内联追问调度。
//!
//! `<subagents>` 快照只展示可复用子树（排除 `cancelled`），按活跃度排序并限制
//! 8 条 / 1024 B；面板追问在目标已结束时先自动 `resume` 再投递 `send_input`。

use std::path::Path;
use std::time::Instant;

use super::*;
use crate::chat::agent::{AgentStatus, SubagentInputItem};
use crate::db::Database;

/// 环境快照最多列出的子 agent 条数。
pub const MAX_SNAPSHOT_SUBAGENTS: usize = 8;
/// 环境快照（含 `<subagents>` 标签）的最大字节数。
pub const MAX_SNAPSHOT_SUBAGENT_BYTES: usize = 1024;

/// 快照条目：DB 记录与内存句柄归一化后的展示数据。
struct SnapshotEntry {
    agent_path: String,
    agent_id: String,
    nickname: String,
    role: String,
    status: String,
    title: String,
    created_at_ms: i64,
}

/// 生成当前根会话树下已知子 agent 的结构化环境摘要（含 running / idle / interrupted /
/// completed，排除 cancelled）。无 DB 时回退到内存句柄（单测路径）。
pub(crate) fn known_subagent_lines(
    streams: &ChatStreams,
    database: Option<&Database>,
    workspace: Option<&Path>,
    parent_session: &str,
) -> Vec<String> {
    let mut entries = Vec::new();
    let mut known_ids = std::collections::HashSet::new();
    if let (Some(database), Some(workspace)) = (database, workspace) {
        let tree = runner::load_subagent_tree(database, workspace, parent_session);
        let queues = runner::tree_queue_states(database, workspace, &tree);
        for record in &tree.records {
            let status = projected_agent_status(streams, record, queues.get(&record.agent_id));
            if AgentStatus::parse(&status) == Some(AgentStatus::Cancelled) {
                continue;
            }
            known_ids.insert(record.agent_id.clone());
            entries.push(SnapshotEntry {
                agent_path: record.agent_path.clone(),
                agent_id: record.agent_id.clone(),
                nickname: display_nickname(record.nickname.as_deref()),
                role: record.role.as_str().to_string(),
                status,
                title: record.title.clone(),
                created_at_ms: record.created_at_ms,
            });
        }
    }
    for handle in streams.subagents.values() {
        if handle.parent.session != parent_session && handle.root_session != parent_session {
            continue;
        }
        if known_ids.contains(&handle.agent_id) || handle.status == STATUS_CANCELLED {
            continue;
        }
        entries.push(SnapshotEntry {
            agent_path: handle.agent_path.clone(),
            agent_id: handle.agent_id.clone(),
            nickname: display_nickname(handle.nickname.as_deref()),
            role: handle.role.as_str().to_string(),
            status: handle.status.to_string(),
            title: if handle.title.trim().is_empty() {
                handle.task.clone()
            } else {
                handle.title.clone()
            },
            created_at_ms: handle.started_at_ms,
        });
    }
    if entries.is_empty() {
        return Vec::new();
    }
    entries.sort_by(|left, right| {
        status_priority(&left.status)
            .cmp(&status_priority(&right.status))
            .then(right.created_at_ms.cmp(&left.created_at_ms))
    });

    let header = "<subagents>";
    let footer = "</subagents>";
    let mut bytes = header.len() + footer.len() + 2;
    let mut lines = vec![header.to_string()];
    for entry in entries.into_iter().take(MAX_SNAPSHOT_SUBAGENTS) {
        let title = first_line(&entry.title);
        let line = if title.is_empty() {
            format!(
                "- path={} id={} nickname={} role={} status={}",
                entry.agent_path, entry.agent_id, entry.nickname, entry.role, entry.status
            )
        } else {
            format!(
                "- path={} id={} nickname={} role={} status={} · {}",
                entry.agent_path, entry.agent_id, entry.nickname, entry.role, entry.status, title
            )
        };
        let extra = line.len() + 1;
        if bytes.saturating_add(extra) > MAX_SNAPSHOT_SUBAGENT_BYTES {
            continue;
        }
        bytes = bytes.saturating_add(extra);
        lines.push(line);
    }
    if lines.len() == 1 {
        return Vec::new();
    }
    lines.push(footer.to_string());
    lines
}

/// 兼容旧入口：仅基于内存句柄的环境快照。
#[allow(dead_code)]
pub(crate) fn running_subagent_lines(streams: &ChatStreams, parent_session: &str) -> Vec<String> {
    known_subagent_lines(streams, None, None, parent_session)
}

/// 面板内联追问：目标已结束时自动 `resume`，随后投递并立即触发新一轮。
pub(super) fn send_user_input_from_panel(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    session_id: &str,
    agent_id: &str,
    text: &str,
) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let generation = state
        .pane_anywhere(pane)
        .map_or(0, |target| target.lx.chat.generation_counter);
    let key = RequestKey {
        pane,
        session: session_id.to_string(),
        generation,
    };
    if !streams.db_workspaces.contains_key(&key)
        && let Some(workspace) = super::workspace_for_pane(state, pane)
    {
        streams.db_workspaces.insert(key.clone(), workspace);
    }
    if !streams.frozen.contains_key(session_id)
        && let Some(frozen) = runner::resolve_frozen_for_pane(state, pane)
    {
        streams.frozen.insert(session_id.to_string(), frozen);
    }

    let record = persistence.database().and_then(|database| {
        streams.db_workspaces.get(&key).and_then(|workspace| {
            database
                .resolve_owned_subagent(workspace, session_id, agent_id)
                .ok()
                .flatten()
        })
    });
    let closed_in_memory = streams
        .subagents
        .values()
        .any(|handle| handle.agent_id == agent_id && handle.closed_without_report);
    let closed_in_db = record.as_ref().is_some_and(|rec| {
        let queue = persistence.database().and_then(|db| {
            streams.db_workspaces.get(&key).and_then(|ws| {
                db.subagent_queue_states(ws, &rec.parent_session_id)
                    .ok()
                    .and_then(|mut map| map.remove(&rec.agent_id))
            })
        });
        super::projected_agent_status(streams, rec, queue.as_ref()) == STATUS_CANCELLED
    });
    if closed_in_memory || closed_in_db {
        show_panel_toast(
            state,
            pane,
            ToastKind::Warning,
            crate::ui::text::LX_TASK_AGENT_REPLY_CLOSED.to_string(),
        );
        return true;
    }

    let is_active = streams.subagents.values().any(|handle| {
        handle.agent_id == agent_id
            && handle.status == STATUS_RUNNING
            && !handle.closed_without_report
    }) || streams.resumed_subagents.contains(agent_id);
    if !is_active && let Err(error) = runner::resume(state, streams, persistence, &key, agent_id) {
        show_panel_toast(state, pane, ToastKind::Danger, error);
        return true;
    }
    let nickname = record
        .as_ref()
        .and_then(|record| record.nickname.as_deref())
        .map(str::trim)
        .filter(|nickname| !nickname.is_empty())
        .map(str::to_string);
    let child_session = record
        .as_ref()
        .map(|record| record.child_session_id.clone());

    let outcome = runner::send_input(
        state,
        streams,
        persistence,
        &key,
        None,
        agent_id,
        &[SubagentInputItem::Text {
            text: text.to_string(),
        }],
        false,
        true,
    );
    match outcome {
        Ok(_) => {
            let message = nickname
                .map(|nickname| format!("Sent input to {nickname}"))
                .unwrap_or_else(|| "Sent input to subagent".to_string());
            show_panel_toast(state, pane, ToastKind::Default, message);
            let unloaded = state
                .pane_anywhere(pane)
                .and_then(|target| target.lx.agents.by_agent(agent_id))
                .is_some_and(|run| !run.loaded);
            if unloaded && let Some(child_session) = child_session {
                state.effects.push(Effect::LoadSubagentSession {
                    pane,
                    agent_id: agent_id.to_string(),
                    child_session,
                });
            }
            true
        }
        Err(error) => {
            show_panel_toast(state, pane, ToastKind::Danger, error);
            true
        }
    }
}

fn show_panel_toast(state: &mut AppState, pane: PaneId, kind: ToastKind, message: String) {
    update::show_lx_toast(
        state,
        pane,
        Toast::new(kind, message, Some(pane), Instant::now()),
    );
}

fn status_priority(status: &str) -> u8 {
    match AgentStatus::parse(status) {
        Some(AgentStatus::Running) => 0,
        Some(AgentStatus::Blocked) => 1,
        Some(AgentStatus::Idle) => 2,
        Some(AgentStatus::Interrupted) => 3,
        Some(AgentStatus::Failed) => 4,
        Some(AgentStatus::Completed) => 5,
        Some(AgentStatus::Cancelled) => u8::MAX,
        None => 6,
    }
}

fn display_nickname(nickname: Option<&str>) -> String {
    nickname
        .map(str::trim)
        .filter(|nickname| !nickname.is_empty())
        .unwrap_or("-")
        .to_string()
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(120)
        .collect()
}

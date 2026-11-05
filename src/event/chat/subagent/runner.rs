//! 子 agent 运行调度：新身份登记（`spawn`）、排队与插队（`send_input`）与显式恢复（`resume`）。
//!
//! 已登记子轮的启动位于 `launch` 子模块。

use std::path::Path;

use tokio::sync::watch;

use super::context::{ChildTurnLimits, assemble_child_context, prepare_input_items};
use super::*;
use crate::app::state::RequestKey;
use crate::chat::agent::{
    AgentRole, DEPTH_LIMIT_REACHED_CODE, ForkSpec, IMAGE_MISSING_CODE, LIMIT_REACHED_CODE,
    NOT_ACTIVE_CODE, NOT_FOUND_CODE, SubagentInputItem, SubagentOutcome, allocate_child_path,
    joined_input_text,
};
use crate::db::chat::{ModelSnapshot, PersistedInputItem, SubagentQueueState, SubagentRecord};
use crate::db::{Database, new_external_id};
use crate::layout::PaneId;

/// 根会话树快照：根会话、调用者深度/路径与整树子 agent 记录。
pub(super) struct SubagentTree {
    /// 根会话 external id。
    pub root_session: String,
    /// 调用者（发起控制工具的会话）所处深度；根为 0。
    pub caller_depth: usize,
    /// 调用者的规范路径；根为 `/root`。
    pub caller_path: String,
    /// 整棵树的子 agent 记录（按创建时间升序）。
    pub records: Vec<SubagentRecord>,
}

/// 读取根会话树快照；旧库或空树时回退到当前会话的直接子 agent 列表。
pub(super) fn load_subagent_tree(
    database: &Database,
    workspace: &Path,
    session: &str,
) -> SubagentTree {
    let (root_session, caller_depth, caller_path) = database
        .resolve_session_tree_root(workspace, session)
        .unwrap_or_else(|_| {
            (
                session.to_string(),
                0,
                crate::chat::agent::ROOT_AGENT_PATH.to_string(),
            )
        });
    let records = database
        .list_subagent_tree(workspace, &root_session)
        .or_else(|_| database.list_subagents(workspace, session))
        .unwrap_or_default();
    SubagentTree {
        root_session,
        caller_depth,
        caller_path,
        records,
    }
}

/// 三合一寻址（`agent_id` / `nickname` / 绝对或相对 `agent_path`）在整棵树内解析目标。
pub(super) fn resolve_tree_target<'a>(
    tree: &'a SubagentTree,
    target: &str,
) -> Option<&'a SubagentRecord> {
    tree.records.iter().find(|record| {
        crate::chat::agent::matches_agent_target(
            &record.agent_id,
            record.nickname.as_deref(),
            Some(&record.agent_path),
            &tree.caller_path,
            target,
        )
    })
}

/// 目标自身与其所有后代路径（按树的记录集合展开）。
pub(super) fn subtree_paths(records: &[SubagentRecord], target_path: &str) -> Vec<String> {
    let prefix = format!(
        "{}/",
        target_path.trim_end_matches('/').to_ascii_lowercase()
    );
    let mut paths = vec![target_path.to_string()];
    for record in records {
        let candidate = record.agent_path.to_ascii_lowercase();
        if candidate != target_path.to_ascii_lowercase() && candidate.starts_with(&prefix) {
            paths.push(record.agent_path.clone());
        }
    }
    paths
}

/// 整棵根会话树中正在运行的子 agent 数量。
pub(super) fn running_in_tree(streams: &ChatStreams, root_session: &str) -> usize {
    streams
        .subagents
        .values()
        .filter(|handle| handle.root_session == root_session && handle.status == STATUS_RUNNING)
        .count()
}

/// 整树排队状态：按每个子 agent 的真实父会话分别读取后合并（孙 agent 的队列也可见）。
pub(super) fn tree_queue_states(
    database: &Database,
    workspace: &Path,
    tree: &SubagentTree,
) -> std::collections::HashMap<String, SubagentQueueState> {
    let mut parents: Vec<&str> = vec![tree.root_session.as_str()];
    for record in &tree.records {
        let parent = record.parent_session_id.as_str();
        if !parents.contains(&parent) {
            parents.push(parent);
        }
    }
    let mut states = std::collections::HashMap::new();
    for parent in parents {
        for (agent_id, state) in database
            .subagent_queue_states(workspace, parent)
            .unwrap_or_default()
        {
            states.insert(agent_id, state);
        }
    }
    states
}

/// spawn：角色与模型校验 → 树深度/整树并发检查 → 图片严格归档 → 事务落库 → 注册句柄并标记活动 → 启动首轮 → 回执。
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    call_id: Option<&str>,
    role: AgentRole,
    title: &str,
    task_name: Option<&str>,
    task: &str,
    items: &[SubagentInputItem],
    fork_context: bool,
    fork_turns: Option<u32>,
    max_tokens: Option<u64>,
    model_override: Option<&str>,
    reasoning_effort_override: Option<&str>,
) -> Result<SubagentOutcome, String> {
    let _sender = streams
        .event_sender
        .as_ref()
        .ok_or_else(|| "subagent event channel unavailable".to_string())?;
    let role_spec = state
        .subagents_config
        .resolve_role(role.as_str())
        .ok_or_else(|| format!("unknown subagent role '{}'", role.as_str()))?;
    let role = role.with_base(role_spec.base_role);

    let parent_session = key.session.clone();
    let workspace = streams
        .db_workspaces
        .get(key)
        .cloned()
        .ok_or_else(|| "subagent workspace unavailable".to_string())?;
    let database = persistence
        .database()
        .ok_or_else(|| "subagent persistence unavailable".to_string())?;
    let tree = load_subagent_tree(database, &workspace, &parent_session);
    let max_depth = state
        .subagents_config
        .max_depth
        .clamp(1, crate::config::subagents::MAX_AGENT_DEPTH_LIMIT);
    let next_depth = tree.caller_depth.saturating_add(1);
    if next_depth > max_depth {
        return Err(DEPTH_LIMIT_REACHED_CODE.to_string());
    }
    let max_concurrent = state
        .subagents_config
        .max_concurrent
        .clamp(1, crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT);
    if running_in_tree(streams, &tree.root_session) >= max_concurrent {
        return Err(LIMIT_REACHED_CODE.to_string());
    }
    let parent_frozen = streams
        .frozen
        .get(&parent_session)
        .cloned()
        .or_else(|| resolve_frozen_for_pane(state, key.pane))
        .ok_or_else(|| "subagent model snapshot unavailable".to_string())?;
    let resolved_model = super::model_resolve::resolve_subagent_model(
        &state.models,
        &parent_frozen,
        role_spec.default_model.as_ref(),
        model_override,
        reasoning_effort_override,
    )?;
    let frozen = resolved_model.frozen;
    let cwd = state
        .workspace_cwd_for_pane(key.pane)
        .unwrap_or_else(|| workspace.clone());
    // 确认进程运行实例可用，避免 DB 落库后无法启动执行器。
    if streams.process_sender.is_none() {
        return Err("process runtime unavailable".to_string());
    }

    let used_nicknames: Vec<String> = tree
        .records
        .iter()
        .filter_map(|record| record.nickname.clone())
        .collect();
    let nickname =
        crate::chat::agent::allocate_nickname(&used_nicknames, &role_spec.nickname_candidates);
    let existing_paths: Vec<String> = tree
        .records
        .iter()
        .map(|record| record.agent_path.clone())
        .collect();
    let agent_path = allocate_child_path(
        &tree.caller_path,
        task_name.unwrap_or(&nickname),
        &existing_paths,
    );
    let fork_spec = ForkSpec::from_params(fork_context, fork_turns);
    let effective_max_tokens = max_tokens.or(state.subagents_config.default_max_tokens);

    let child_session_id = new_external_id("sess");
    let image_rewrite = if fork_spec.is_enabled() {
        crate::db::chat::copy_session_images(&parent_session, &child_session_id)
    } else {
        None
    };
    let (persisted_items, _initial_model_items) =
        match prepare_input_items(&child_session_id, &cwd, frozen.supports_image_input, items) {
            Ok(prepared) => prepared,
            Err(err) => {
                crate::platform::remove_session_images(&child_session_id);
                return Err(err);
            }
        };
    let task_text = if task.trim().is_empty() {
        joined_input_text(items)
    } else {
        task.to_string()
    };
    let effective_title = if title.trim().is_empty() {
        task_text
            .lines()
            .next()
            .unwrap_or("subagent")
            .chars()
            .take(80)
            .collect::<String>()
    } else {
        title.to_string()
    };
    let spawn_message_id = active_assistant_id(state, key);
    let dedupe_key = call_id.map(|cid| {
        format!(
            "spawn:{}:{}:{cid}",
            parent_session,
            spawn_message_id.as_deref().unwrap_or_default()
        )
    });
    let model = ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let committed = match database.begin_subagent_turn_with_full_options(
        &workspace,
        &parent_session,
        Some(&child_session_id),
        Some(&nickname),
        role,
        &effective_title,
        &task_text,
        &persisted_items,
        &model,
        spawn_message_id.as_deref(),
        dedupe_key.as_deref(),
        call_id,
        fork_spec.is_enabled(),
        Some(&agent_path),
        fork_spec.fork_turns(),
        &image_rewrite,
    ) {
        Ok(committed) => committed,
        Err(error) => {
            crate::platform::remove_session_images(&child_session_id);
            return Err(error.code().to_string());
        }
    };

    let record = SubagentRecord {
        agent_id: committed.agent_id.clone(),
        nickname: Some(nickname.clone()),
        agent_path: agent_path.clone(),
        child_session_id: committed.child_session_id.clone(),
        parent_session_id: parent_session.clone(),
        spawn_message_id,
        role,
        model_provider: frozen.provider.clone(),
        model_id: frozen.model.clone(),
        model_variant: frozen.variant.clone(),
        status: STATUS_RUNNING.to_string(),
        title: effective_title.clone(),
        created_at_ms: committed.created_at_ms,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    };
    streams.resumed_subagents.insert(committed.agent_id.clone());
    streams
        .frozen
        .insert(committed.child_session_id.clone(), frozen.clone());
    streams
        .subagent_budgets
        .insert(committed.agent_id.clone(), effective_max_tokens);
    if resolved_model.pinned {
        streams
            .pinned_subagent_models
            .insert(committed.child_session_id.clone(), frozen.clone());
    }

    let limits = ChildTurnLimits {
        depth: next_depth,
        max_depth,
        max_tokens: effective_max_tokens,
    };
    super::launch::launch_child_turn(
        state,
        streams,
        persistence,
        key,
        &workspace,
        &cwd,
        &frozen,
        &record,
        &committed.child_user_message_id,
        &committed.child_assistant_id,
        committed.created_at_ms,
        &task_text,
        &persisted_items,
        true,
        None,
        limits,
        &tree.root_session,
    )?;

    Ok(SubagentOutcome::Spawned {
        agent_id: committed.agent_id,
        nickname: Some(nickname),
        agent_path: Some(agent_path),
        role,
        title: effective_title,
        model: resolved_model.model_override_label,
        reasoning_effort: resolved_model.effort_override_label,
        fork_context: fork_spec.is_enabled(),
        fork_turns: fork_spec.fork_turns(),
        max_tokens: effective_max_tokens,
    })
}

/// `send_input`：校验归属与活动态 → 严格归档图片 → 原子入队 →
/// `trigger_turn=true` 时若 idle 立即调度，`false` 时仅入队（`queued`，不唤醒）。
#[allow(clippy::too_many_arguments)]
pub(super) fn send_input(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    call_id: Option<&str>,
    target: &str,
    items: &[SubagentInputItem],
    interrupt: bool,
    trigger_turn: bool,
) -> Result<SubagentOutcome, String> {
    let parent_session = key.session.clone();
    let workspace = streams
        .db_workspaces
        .get(key)
        .cloned()
        .ok_or_else(|| "subagent workspace unavailable".to_string())?;
    let database = persistence
        .database()
        .ok_or_else(|| "subagent persistence unavailable".to_string())?;
    let tree = load_subagent_tree(database, &workspace, &parent_session);
    let record = resolve_tree_target(&tree, target)
        .cloned()
        .ok_or_else(|| NOT_FOUND_CODE.to_string())?;
    let target = record.agent_id.as_str();
    // 队列/归属操作必须使用目标真正的父会话（目标可能是孙 agent）。
    let owner_session = record.parent_session_id.clone();
    let owner_key = RequestKey {
        pane: key.pane,
        session: owner_session.clone(),
        generation: key.generation,
    };

    // 整树寻址：目标可能不是调用者的直接子 agent，句柄按全局唯一 `agent_id` 匹配。
    let is_running = streams
        .subagents
        .values()
        .any(|h| h.agent_id == target && h.status == STATUS_RUNNING && !h.closed_without_report);
    let is_resumed = streams.resumed_subagents.contains(target);
    if !is_running && !is_resumed {
        return Err(NOT_ACTIVE_CODE.to_string());
    }
    // `trigger_turn=false` 只入队：`interrupt` 在解析层已禁止与它同传。
    let interrupt = interrupt && trigger_turn;
    let was_cancelling_from_interrupt = streams.paused_by_interrupt.remove(target);
    let max_concurrent = state
        .subagents_config
        .max_concurrent
        .clamp(1, crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT);

    // P1-1：非运行态（idle）下入库前先校验队首未阻断且存在可用并发槽，避免入库后再报错遗留孤儿 queued 项。
    if !is_running && trigger_turn {
        if let Ok(Some(head)) = database.next_queued_input(&workspace, &owner_session, target)
            && let Some(code) = head.error_code
        {
            return Err(format!("subagent input blocked: {code}"));
        }
        if running_in_tree(streams, &tree.root_session) >= max_concurrent {
            return Err(LIMIT_REACHED_CODE.to_string());
        }
    }

    let frozen = {
        let parent_frozen = resolve_frozen_for_pane(state, key.pane)
            .or_else(|| streams.frozen.get(&owner_session).cloned())
            .or_else(|| streams.frozen.get(&record.child_session_id).cloned())
            .ok_or_else(|| "subagent model snapshot unavailable".to_string())?;
        super::model_resolve::child_frozen_for(state, streams, &record, &parent_frozen)
    };
    streams
        .frozen
        .insert(record.child_session_id.clone(), frozen.clone());

    let cwd = state
        .workspace_cwd_for_pane(key.pane)
        .unwrap_or_else(|| workspace.clone());
    let (persisted_items, _) = prepare_input_items(
        &record.child_session_id,
        &cwd,
        frozen.supports_image_input,
        items,
    )?;

    let parent_message_id = active_assistant_id(state, key);
    let dedupe_key = match call_id.filter(|cid| !cid.trim().is_empty()) {
        Some(cid) => format!(
            "send:{}:{}:{cid}",
            parent_session,
            parent_message_id.as_deref().unwrap_or_default()
        ),
        None => format!("send:{}:{}", parent_session, new_external_id("dedupe")),
    };

    let enqueued = database
        .enqueue_input(
            &workspace,
            &owner_session,
            target,
            &dedupe_key,
            call_id,
            parent_message_id.as_deref(),
            &persisted_items,
            interrupt,
        )
        .map_err(|err| err.code().to_string())?;

    if !trigger_turn {
        sync_agent_ui_queue(
            state,
            streams,
            persistence,
            key.pane,
            &workspace,
            &owner_session,
        );
        let queue = database
            .subagent_queue_states(&workspace, &owner_session)
            .unwrap_or_default();
        let status = super::projected_agent_status(streams, &record, queue.get(target));
        return Ok(SubagentOutcome::InputAccepted {
            submission_id: enqueued.submission_id,
            agent_id: target.to_string(),
            status,
            queued_count: enqueued.queued_count,
            interrupt: false,
            trigger_turn: false,
        });
    }

    if is_running {
        if (interrupt || was_cancelling_from_interrupt) && enqueued.created {
            streams.interrupted_subagents.insert(target.to_string());
            if let Some(handle) = streams.subagents.values().find(|h| h.agent_id == target) {
                let _ = handle.cancel.send(true);
            }
        }
        sync_agent_ui_queue(
            state,
            streams,
            persistence,
            key.pane,
            &workspace,
            &owner_session,
        );
        return Ok(SubagentOutcome::InputAccepted {
            submission_id: enqueued.submission_id,
            agent_id: target.to_string(),
            status: enqueued.status,
            queued_count: enqueued.queued_count,
            interrupt,
            trigger_turn: true,
        });
    }

    // 目标处于已恢复 idle 态：尝试立即消费队首输入。
    let mut final_status = enqueued.status;
    let mut final_queued = enqueued.queued_count;
    match schedule_next_queued_input(state, streams, persistence, &owner_key, &workspace, target) {
        Ok(Some(claimed)) => {
            if claimed.submission_id == enqueued.submission_id {
                final_status = STATUS_RUNNING.to_string();
            }
            final_queued = claimed.queued_remaining;
        }
        Ok(None) => {}
        Err(err) => {
            sync_agent_ui_queue(
                state,
                streams,
                persistence,
                key.pane,
                &workspace,
                &owner_session,
            );
            return Err(err);
        }
    }
    sync_agent_ui_queue(
        state,
        streams,
        persistence,
        key.pane,
        &workspace,
        &owner_session,
    );
    Ok(SubagentOutcome::InputAccepted {
        submission_id: enqueued.submission_id,
        agent_id: target.to_string(),
        status: final_status,
        queued_count: final_queued,
        interrupt,
        trigger_turn: true,
    })
}

/// `resume_agent`：校验归属 → 修复崩溃遗留状态 → 登记恢复标记 → 若有 queued 项则启动首项，否则保持 idle。
pub(super) fn resume(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    agent_id: &str,
) -> Result<SubagentOutcome, String> {
    let parent_session = key.session.clone();
    let workspace = streams
        .db_workspaces
        .get(key)
        .cloned()
        .ok_or_else(|| "subagent workspace unavailable".to_string())?;
    let database = persistence
        .database()
        .ok_or_else(|| "subagent persistence unavailable".to_string())?;
    let tree = load_subagent_tree(database, &workspace, &parent_session);
    let record = resolve_tree_target(&tree, agent_id)
        .cloned()
        .ok_or_else(|| NOT_FOUND_CODE.to_string())?;
    let agent_id = record.agent_id.as_str();
    // 队列/归属操作必须使用目标真正的父会话（目标可能是孙 agent）。
    let owner_session = record.parent_session_id.clone();
    let owner_key = RequestKey {
        pane: key.pane,
        session: owner_session.clone(),
        generation: key.generation,
    };
    // resume 先判运行态：interrupt 已发出取消但子轮尚未落定时，这里只是把「恢复」意图
    // 转交结算路径（`finish` 看到 interrupted 标记后会继续消费队列）。
    let is_running = streams
        .subagents
        .values()
        .any(|h| h.agent_id == agent_id && h.status == STATUS_RUNNING && !h.closed_without_report);
    if streams.paused_by_interrupt.remove(agent_id) && is_running {
        streams.interrupted_subagents.insert(agent_id.to_string());
    }

    let queue_map = database
        .subagent_queue_states(&workspace, &owner_session)
        .unwrap_or_default();
    let queued_count = queue_map.get(agent_id).map_or(0, |q| q.queued_count);

    // 已在运行中：幂等返回 running，绝不启动第二个并发轮次。
    if is_running {
        streams.resumed_subagents.insert(agent_id.to_string());
        return Ok(SubagentOutcome::Resumed {
            agent_id: agent_id.to_string(),
            status: STATUS_RUNNING.to_string(),
            queued_count,
        });
    }

    let max_concurrent = state
        .subagents_config
        .max_concurrent
        .clamp(1, crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT);
    if queued_count > 0 && running_in_tree(streams, &tree.root_session) >= max_concurrent {
        return Err(LIMIT_REACHED_CODE.to_string());
    }

    database
        .recover_subagent_for_resume(&workspace, &owner_session, agent_id)
        .map_err(|err| err.code().to_string())?;
    streams.resumed_subagents.insert(agent_id.to_string());

    if let Some(parent_frozen) = streams
        .frozen
        .get(&owner_session)
        .cloned()
        .or_else(|| resolve_frozen_for_pane(state, key.pane))
        .or_else(|| streams.frozen.get(&record.child_session_id).cloned())
    {
        let frozen =
            super::model_resolve::child_frozen_for(state, streams, &record, &parent_frozen);
        streams
            .frozen
            .insert(record.child_session_id.clone(), frozen);
    }

    match schedule_next_queued_input(
        state,
        streams,
        persistence,
        &owner_key,
        &workspace,
        agent_id,
    ) {
        Ok(Some(claimed)) => {
            sync_agent_ui_queue(
                state,
                streams,
                persistence,
                key.pane,
                &workspace,
                &owner_session,
            );
            Ok(SubagentOutcome::Resumed {
                agent_id: agent_id.to_string(),
                status: STATUS_RUNNING.to_string(),
                queued_count: claimed.queued_remaining,
            })
        }
        Ok(None) => {
            sync_agent_ui_queue(
                state,
                streams,
                persistence,
                key.pane,
                &workspace,
                &owner_session,
            );
            Ok(SubagentOutcome::Resumed {
                agent_id: agent_id.to_string(),
                status: STATUS_IDLE.to_string(),
                queued_count: 0,
            })
        }
        Err(err) => {
            sync_agent_ui_queue(
                state,
                streams,
                persistence,
                key.pane,
                &workspace,
                &owner_session,
            );
            Err(err)
        }
    }
}

/// 若该 agent 空闲且队首有可执行的 `queued` 输入，校验图片与历史并认领启动下一轮。
pub(crate) fn schedule_next_queued_input(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    parent_key: &RequestKey,
    workspace: &Path,
    agent_id: &str,
) -> Result<Option<crate::db::chat::ClaimedSubagentTurn>, String> {
    schedule_next_queued_input_with_done(
        state,
        streams,
        persistence,
        parent_key,
        workspace,
        agent_id,
        None,
    )
}

/// 若该 agent 空闲且队首有可执行的 `queued` 输入，校验图片与历史并认领启动下一轮（可复用上一轮 `done` 通道）。
pub(crate) fn schedule_next_queued_input_with_done(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    parent_key: &RequestKey,
    workspace: &Path,
    agent_id: &str,
    reused_done: Option<watch::Sender<bool>>,
) -> Result<Option<crate::db::chat::ClaimedSubagentTurn>, String> {
    let parent_session = parent_key.session.clone();
    if streams
        .subagents
        .values()
        .any(|h| h.agent_id == agent_id && h.status == STATUS_RUNNING)
    {
        return Ok(None);
    }
    let database = persistence
        .database()
        .ok_or_else(|| "subagent persistence unavailable".to_string())?;
    let record = database
        .owned_subagent(workspace, &parent_session, agent_id)
        .map_err(|err| err.code().to_string())?
        .ok_or_else(|| NOT_FOUND_CODE.to_string())?;
    let Some(next_input) = database
        .next_queued_input(workspace, &parent_session, agent_id)
        .map_err(|err| err.code().to_string())?
    else {
        return Ok(None);
    };

    // 队首输入此前已标为错误（如 `image_missing`）时保持 blocked，不越过队首。
    if let Some(code) = &next_input.error_code {
        return Err(format!("subagent input blocked: {code}"));
    }

    let max_concurrent = state
        .subagents_config
        .max_concurrent
        .clamp(1, crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT);
    let (root_session, _, _) = database
        .resolve_session_tree_root(workspace, &parent_session)
        .unwrap_or_else(|_| {
            (
                parent_session.clone(),
                0,
                crate::chat::agent::ROOT_AGENT_PATH.to_string(),
            )
        });
    if running_in_tree(streams, &root_session) >= max_concurrent {
        return Err(LIMIT_REACHED_CODE.to_string());
    }
    let max_depth = state
        .subagents_config
        .max_depth
        .clamp(1, crate::config::subagents::MAX_AGENT_DEPTH_LIMIT);
    // 队列轮沿用 spawn 时冻结的预算覆写（无记录时回退当前默认值）。
    let max_tokens = streams
        .subagent_budgets
        .get(agent_id)
        .copied()
        .unwrap_or(state.subagents_config.default_max_tokens);
    let limits = ChildTurnLimits {
        depth: crate::chat::agent::path_depth(&record.agent_path),
        max_depth,
        max_tokens,
    };

    // 若该子会话有显式/角色绑定的固定模型，优先沿用固定模型；
    // 否则继承父会话最新模型，再回退到数据库里持久化的子 agent 模型快照。
    let parent_frozen = resolve_frozen_for_pane(state, parent_key.pane)
        .or_else(|| streams.frozen.get(&parent_session).cloned())
        .or_else(|| streams.frozen.get(&record.child_session_id).cloned())
        .ok_or_else(|| "subagent model snapshot unavailable".to_string())?;
    let frozen = super::model_resolve::child_frozen_for(state, streams, &record, &parent_frozen);
    streams
        .frozen
        .insert(record.child_session_id.clone(), frozen.clone());

    // 认领前先严格校验本条输入及保留历史中的归档图片：缺失或损坏则标为 blocked（不进 running）。
    let cwd = state
        .workspace_cwd_for_pane(parent_key.pane)
        .unwrap_or_else(|| workspace.to_path_buf());
    let model = ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let inputs = super::context::capture_child_prompt_inputs(
        state,
        persistence,
        workspace,
        &cwd,
        &frozen,
        record.role,
    );
    if let Err(err) = assemble_child_context(
        database,
        workspace,
        &parent_session,
        agent_id,
        &record.child_session_id,
        "",
        record.role,
        super::context::ChildPromptContext {
            system_prompt_override: inputs.system_prompt_override.as_deref(),
            model_identity: Some(&inputs.model_identity),
            environment: &inputs.environment,
            instructions: inputs.instructions.as_deref(),
            skills: inputs.skills.as_deref(),
            mcp: &inputs.mcp,
        },
        &cwd,
        &model,
        frozen.variant_params.clone(),
        frozen.context_limit,
        frozen.output_limit,
        frozen.supports_image_input,
        &next_input.items,
        &state.subagents_config,
        limits,
    ) {
        let code = if err.contains(IMAGE_MISSING_CODE) {
            IMAGE_MISSING_CODE
        } else {
            err.as_str()
        };
        let _ = database.set_input_error(
            workspace,
            &parent_session,
            &next_input.submission_id,
            Some(code),
        );
        return Err(format!("subagent input blocked: {code}"));
    }

    let claimed = database
        .claim_input(
            workspace,
            &parent_session,
            agent_id,
            &next_input.submission_id,
            &model,
        )
        .map_err(|err| err.code().to_string())?;

    let task_text = PersistedInputItem::joined_text(&claimed.items);
    super::launch::launch_child_turn(
        state,
        streams,
        persistence,
        parent_key,
        workspace,
        &cwd,
        &frozen,
        &record,
        &claimed.child_user_message_id,
        &claimed.child_assistant_id,
        claimed.started_at_ms,
        &task_text,
        &claimed.items,
        false,
        reused_done,
        limits,
        &root_session,
    )?;

    Ok(Some(claimed))
}

/// 同步窗格 `Agents` 列表中的状态投影与 `queued_count`。
pub(super) fn sync_agent_ui_queue(
    state: &mut AppState,
    streams: &ChatStreams,
    persistence: &Persistence,
    pane: PaneId,
    workspace: &Path,
    parent_session: &str,
) {
    let Some(database) = persistence.database() else {
        return;
    };
    let Ok(records) = database.list_subagents(workspace, parent_session) else {
        return;
    };
    let queue_states = database
        .subagent_queue_states(workspace, parent_session)
        .unwrap_or_default();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    for record in &records {
        let q = queue_states.get(&record.agent_id);
        let projected = super::projected_agent_status(streams, record, q);
        let queued_count = q.map_or(0, |s| s.queued_count);
        if let Some(run) = target.lx.agents.by_agent_mut(&record.agent_id) {
            run.status = projected;
            run.queued_count = queued_count;
        }
    }
}

/// 从窗格当前模型配置解析 `FrozenModel`。
fn resolve_frozen_for_pane(
    state: &AppState,
    pane: PaneId,
) -> Option<crate::app::state::FrozenModel> {
    let target = state.pane_anywhere(pane)?;
    let selection = target
        .lx
        .chat
        .effective_selection(&state.models)
        .or_else(|| state.models.default_selection())?;
    crate::app::state::FrozenModel::resolve(&state.models, &selection)
}

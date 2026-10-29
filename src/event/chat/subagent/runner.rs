//! 子 agent 运行调度：新身份登记（`spawn`）、排队与插队（`send_input`）、显式恢复（`resume`）
//! 与启动已登记子轮（`launch_child_turn`）。

use std::path::Path;
use std::time::Duration;

use tokio::sync::watch;

use super::context::{assemble_child_context, prepare_input_items};
use super::*;
use crate::app::state::lx::agent::{SubagentRun, seed_chat_model};
use crate::app::state::{RequestKey, RequestState};
use crate::chat::agent::{
    AgentRole, IMAGE_MISSING_CODE, LIMIT_REACHED_CODE, MAX_CONCURRENT_SUBAGENTS, NOT_ACTIVE_CODE,
    NOT_FOUND_CODE, SubagentInputItem, SubagentOutcome, joined_input_text,
};
use crate::chat::events::{ChatRequest, RequestPurpose, StopReason};
use crate::db::chat::{ModelSnapshot, PersistedInputItem, SubagentRecord};
use crate::db::new_external_id;
use crate::event::AppEvent;
use crate::layout::PaneId;

/// spawn：并发检查 → 图片严格归档 → 事务落库 → 注册句柄并标记活动 → 启动首轮 → 回执。
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    call_id: Option<&str>,
    role: AgentRole,
    title: &str,
    task: &str,
    items: &[SubagentInputItem],
) -> Result<SubagentOutcome, String> {
    let _sender = streams
        .event_sender
        .as_ref()
        .ok_or_else(|| "subagent event channel unavailable".to_string())?;
    let parent_session = key.session.clone();
    let running = running_count_for_parent(streams, &parent_session);
    if running >= MAX_CONCURRENT_SUBAGENTS {
        return Err(LIMIT_REACHED_CODE.to_string());
    }
    let frozen = streams
        .frozen
        .get(&parent_session)
        .cloned()
        .ok_or_else(|| "subagent model snapshot unavailable".to_string())?;
    let workspace = streams
        .db_workspaces
        .get(key)
        .cloned()
        .ok_or_else(|| "subagent workspace unavailable".to_string())?;
    let database = persistence
        .database()
        .ok_or_else(|| "subagent persistence unavailable".to_string())?;
    let cwd = state
        .workspace_cwd_for_pane(key.pane)
        .unwrap_or_else(|| workspace.clone());
    // 确认进程运行实例可用，避免 DB 落库后无法启动执行器。
    if streams.process_sender.is_none() {
        return Err("process runtime unavailable".to_string());
    }

    let child_session_id = new_external_id("sess");
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
    let committed = match database.begin_subagent_turn_with_items(
        &workspace,
        &parent_session,
        Some(&child_session_id),
        role,
        &effective_title,
        &task_text,
        &persisted_items,
        &model,
        spawn_message_id.as_deref(),
        dedupe_key.as_deref(),
        call_id,
    ) {
        Ok(committed) => committed,
        Err(error) => {
            crate::platform::remove_session_images(&child_session_id);
            return Err(error.code().to_string());
        }
    };

    let record = SubagentRecord {
        agent_id: committed.agent_id.clone(),
        child_session_id: committed.child_session_id.clone(),
        parent_session_id: parent_session.clone(),
        spawn_message_id,
        role,
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

    launch_child_turn(
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
    )?;

    Ok(SubagentOutcome::Spawned {
        agent_id: committed.agent_id,
        role,
        title: effective_title,
    })
}

/// `send_input`：校验归属与活动态 → 严格归档图片 → 原子入队 → 若 interrupt 则取消当前轮，若 idle 则立即调度。
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
    let record = database
        .owned_subagent(&workspace, &parent_session, target)
        .map_err(|err| err.code().to_string())?
        .ok_or_else(|| NOT_FOUND_CODE.to_string())?;

    let is_running = streams.subagents.values().any(|h| {
        h.agent_id == target && h.parent.session == parent_session && h.status == STATUS_RUNNING
    });
    let is_resumed = streams.resumed_subagents.contains(target);
    if !is_running && !is_resumed {
        return Err(NOT_ACTIVE_CODE.to_string());
    }

    // P1-1：非运行态（idle）下入库前先校验队首未阻断且存在可用并发槽，避免入库后再报错遗留孤儿 queued 项。
    if !is_running {
        if let Ok(Some(head)) = database.next_queued_input(&workspace, &parent_session, target)
            && let Some(code) = head.error_code
        {
            return Err(format!("subagent input blocked: {code}"));
        }
        if running_count_for_parent(streams, &parent_session) >= MAX_CONCURRENT_SUBAGENTS {
            return Err(LIMIT_REACHED_CODE.to_string());
        }
    }

    let frozen = streams
        .frozen
        .get(&record.child_session_id)
        .or_else(|| streams.frozen.get(&parent_session))
        .cloned()
        .or_else(|| resolve_frozen_for_pane(state, key.pane))
        .ok_or_else(|| "subagent model snapshot unavailable".to_string())?;
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
            &parent_session,
            target,
            &dedupe_key,
            call_id,
            parent_message_id.as_deref(),
            &persisted_items,
            interrupt,
        )
        .map_err(|err| err.code().to_string())?;

    if is_running {
        if interrupt && enqueued.created {
            streams.interrupted_subagents.insert(target.to_string());
            if let Some(handle) = streams
                .subagents
                .values()
                .find(|h| h.agent_id == target && h.parent.session == parent_session)
            {
                let _ = handle.cancel.send(true);
            }
        }
        sync_agent_ui_queue(
            state,
            streams,
            persistence,
            key.pane,
            &workspace,
            &parent_session,
        );
        return Ok(SubagentOutcome::InputAccepted {
            submission_id: enqueued.submission_id,
            agent_id: target.to_string(),
            status: enqueued.status,
            queued_count: enqueued.queued_count,
            interrupt,
        });
    }

    // 目标处于已恢复 idle 态：尝试立即消费队首输入。
    let mut final_status = enqueued.status;
    let mut final_queued = enqueued.queued_count;
    match schedule_next_queued_input(state, streams, persistence, key, &workspace, target) {
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
                &parent_session,
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
        &parent_session,
    );
    Ok(SubagentOutcome::InputAccepted {
        submission_id: enqueued.submission_id,
        agent_id: target.to_string(),
        status: final_status,
        queued_count: final_queued,
        interrupt,
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
    let record = database
        .owned_subagent(&workspace, &parent_session, agent_id)
        .map_err(|err| err.code().to_string())?
        .ok_or_else(|| NOT_FOUND_CODE.to_string())?;

    let queue_map = database
        .subagent_queue_states(&workspace, &parent_session)
        .unwrap_or_default();
    let queued_count = queue_map.get(agent_id).map_or(0, |q| q.queued_count);

    // 已在运行中：幂等返回 running，绝不启动第二个并发轮次。
    if streams.subagents.values().any(|h| {
        h.agent_id == agent_id && h.parent.session == parent_session && h.status == STATUS_RUNNING
    }) {
        streams.resumed_subagents.insert(agent_id.to_string());
        return Ok(SubagentOutcome::Resumed {
            agent_id: agent_id.to_string(),
            status: STATUS_RUNNING.to_string(),
            queued_count,
        });
    }

    if queued_count > 0
        && running_count_for_parent(streams, &parent_session) >= MAX_CONCURRENT_SUBAGENTS
    {
        return Err(LIMIT_REACHED_CODE.to_string());
    }

    database
        .recover_subagent_for_resume(&workspace, &parent_session, agent_id)
        .map_err(|err| err.code().to_string())?;
    streams.resumed_subagents.insert(agent_id.to_string());

    if let Some(frozen) = streams
        .frozen
        .get(&parent_session)
        .cloned()
        .or_else(|| resolve_frozen_for_pane(state, key.pane))
    {
        streams
            .frozen
            .entry(record.child_session_id.clone())
            .or_insert(frozen);
    }

    match schedule_next_queued_input(state, streams, persistence, key, &workspace, agent_id) {
        Ok(Some(claimed)) => {
            sync_agent_ui_queue(
                state,
                streams,
                persistence,
                key.pane,
                &workspace,
                &parent_session,
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
                &parent_session,
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
                &parent_session,
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

    let running = running_count_for_parent(streams, &parent_session);
    if running >= MAX_CONCURRENT_SUBAGENTS {
        return Err(LIMIT_REACHED_CODE.to_string());
    }

    let frozen = streams
        .frozen
        .get(&record.child_session_id)
        .or_else(|| streams.frozen.get(&parent_session))
        .cloned()
        .or_else(|| resolve_frozen_for_pane(state, parent_key.pane))
        .ok_or_else(|| "subagent model snapshot unavailable".to_string())?;
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
    let environment = crate::chat::prompt::environment_snapshot(
        &crate::platform::EnvironmentFacts::capture(),
        Some(&cwd),
        Some(&cwd),
        &[],
    );
    let skills = super::super::skills::catalog_prompt(persistence, Some(&cwd));
    let mcp_snapshot = super::super::mcp::snapshot_for_turn(persistence, Some(&cwd));
    if let Err(err) = assemble_child_context(
        database,
        workspace,
        &parent_session,
        agent_id,
        &record.child_session_id,
        "",
        record.role,
        &environment,
        skills.as_deref(),
        &mcp_snapshot,
        &cwd,
        &model,
        frozen.variant_params.clone(),
        frozen.context_limit,
        frozen.output_limit,
        frozen.supports_image_input,
        &next_input.items,
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
    launch_child_turn(
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
    )?;

    Ok(Some(claimed))
}

/// 启动一个已在 DB 登记 user/assistant 消息的子轮（`spawn` 与队列消费/恢复共用）。
#[allow(clippy::too_many_arguments)]
fn launch_child_turn(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    parent_key: &RequestKey,
    workspace: &Path,
    cwd: &Path,
    frozen: &crate::app::state::FrozenModel,
    record: &SubagentRecord,
    child_user_message_id: &str,
    child_assistant_id: &str,
    started_at_ms: i64,
    task_text: &str,
    items: &[PersistedInputItem],
    is_new_agent: bool,
    reused_done: Option<watch::Sender<bool>>,
) -> Result<(), String> {
    let sender = streams
        .event_sender
        .clone()
        .ok_or_else(|| "subagent event channel unavailable".to_string())?;
    let database = persistence
        .database()
        .ok_or_else(|| "subagent persistence unavailable".to_string())?;
    let model = ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let environment = crate::chat::prompt::environment_snapshot(
        &crate::platform::EnvironmentFacts::capture(),
        Some(cwd),
        Some(cwd),
        &[],
    );
    let skills = super::super::skills::catalog_prompt(persistence, Some(cwd));
    let mcp_snapshot = super::super::mcp::snapshot_for_turn(persistence, Some(cwd));
    let context = assemble_child_context(
        database,
        workspace,
        &parent_key.session,
        &record.agent_id,
        &record.child_session_id,
        child_user_message_id,
        record.role,
        &environment,
        skills.as_deref(),
        &mcp_snapshot,
        cwd,
        &model,
        frozen.variant_params.clone(),
        frozen.context_limit,
        frozen.output_limit,
        frozen.supports_image_input,
        items,
    )?;

    let child_key = RequestKey {
        pane: parent_key.pane,
        session: record.child_session_id.clone(),
        generation: parent_key.generation,
    };

    let existing_messages = database
        .load_chat_session(workspace, &record.child_session_id)
        .ok()
        .flatten();
    let existing_checkpoints: Vec<crate::app::state::CompactNoticeView> = database
        .compact_checkpoints(workspace, &record.child_session_id)
        .unwrap_or_default()
        .iter()
        .map(crate::app::state::CompactNoticeView::from_record)
        .collect();
    let models_snapshot = state.models.clone();
    let build_child_chat = || {
        let mut chat = crate::app::state::LxChat::new();
        if let Some(ref existing) = existing_messages {
            chat.session_id = Some(record.child_session_id.clone());
            chat.messages = existing
                .iter()
                .map(|msg| crate::app::update::view_message_of(msg, &models_snapshot))
                .collect();
            chat.request = RequestState::Streaming {
                key: child_key.clone(),
                assistant_id: child_assistant_id.to_string(),
            };
            chat.status.streaming = true;
        } else {
            seed_chat_model(
                &mut chat,
                &record.child_session_id,
                task_text,
                child_key.clone(),
                child_assistant_id,
            );
        }
        chat
    };
    let child_chat = build_child_chat();
    let ui_chat = build_child_chat();

    if is_new_agent {
        let mut run = SubagentRun::from_record(record);
        run.task = task_text.to_string();
        run.status = STATUS_RUNNING.to_string();
        run.chat = ui_chat;
        run.compact.replace(existing_checkpoints);
        run.loaded = true;
        update::on_subagent_started(state, parent_key.pane, run);
    } else if let Some(pane) = state.pane_mut_anywhere(parent_key.pane) {
        if let Some(run) = pane.lx.agents.by_agent_mut(&record.agent_id) {
            run.status = STATUS_RUNNING.to_string();
            run.completed_at_ms = None;
            run.started_at_ms = started_at_ms;
            run.task = task_text.to_string();
            run.chat = ui_chat;
            run.compact.replace(existing_checkpoints);
            run.loaded = true;
            run.rows_dirty = true;
        } else {
            let mut run = SubagentRun::from_record(record);
            run.status = STATUS_RUNNING.to_string();
            run.started_at_ms = started_at_ms;
            run.task = task_text.to_string();
            run.chat = ui_chat;
            run.compact.replace(existing_checkpoints);
            run.loaded = true;
            pane.lx.agents.runs.push(run);
        }
    }

    let (cancel_sender, cancel) = watch::channel(false);
    let done_sender = reused_done.unwrap_or_else(|| watch::channel(false).0);
    let (manager, turn) = match streams.runtime_for(cwd.to_path_buf(), &record.child_session_id) {
        Some(runtime) => (
            runtime.manager.clone(),
            runtime.turn(&parent_key.generation.to_string()),
        ),
        None => return Err("process runtime unavailable".to_string()),
    };

    let handle = SubagentHandle {
        parent: parent_key.clone(),
        agent_id: record.agent_id.clone(),
        child_session_id: record.child_session_id.clone(),
        child_assistant_id: child_assistant_id.to_string(),
        child_user_message_id: child_user_message_id.to_string(),
        role: record.role,
        title: record.title.clone(),
        task: task_text.to_string(),
        cancel: cancel_sender.clone(),
        done: done_sender,
        status: STATUS_RUNNING,
        started_at_ms,
        chat: child_chat,
        workspace: workspace.to_path_buf(),
    };
    streams.cancels.insert(child_key.clone(), cancel_sender);
    streams
        .db_workspaces
        .insert(child_key.clone(), workspace.to_path_buf());
    streams
        .subagents
        .insert(record.child_session_id.clone(), handle);

    let net_sender = sender.clone();
    let stream_key = child_key;
    let reads = crate::chat::tools::read_registry(&record.child_session_id);
    let recorder = super::super::file_snapshots::HostRecorderFactory::new(
        net_sender.clone(),
        workspace.to_path_buf(),
        record.child_session_id.clone(),
        child_user_message_id.to_string(),
    );
    let executor = AgentTools::new(manager, turn)
        .with_recorder(std::sync::Arc::new(recorder))
        .with_reads(reads)
        .with_image_support(frozen.supports_image_input)
        .with_mcp(
            crate::mcp::global_manager().clone(),
            mcp_snapshot,
            record.child_session_id.clone(),
        );
    let transport = frozen.transport.clone();
    let base_url = frozen.base_url.clone();
    let api_key = frozen.api_key.clone();
    let idle_timeout = Duration::from_millis(frozen.idle_timeout_ms);
    let guard_session = record.child_session_id.clone();

    tokio::spawn(async move {
        let _guard = match crate::platform::try_lock_session(&guard_session) {
            Ok(Some(guard)) => guard,
            Ok(None) | Err(_) => {
                let _ = net_sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
                    key: stream_key,
                    reason: StopReason::Failed {
                        code: "subagent session busy".into(),
                    },
                    usage: Usage::default(),
                }));
                return;
            }
        };
        let live_sender = net_sender.clone();
        let live_key = stream_key.clone();
        let stream_one = move |request: ChatRequest, cancel: watch::Receiver<bool>| {
            let sender = live_sender.clone();
            let key = live_key.clone();
            let transport = transport.clone();
            let base_url = base_url.clone();
            let api_key = api_key.clone();
            let emit = request.purpose != RequestPurpose::CompactSummary;
            async move {
                crate::chat::client::run_stream(
                    &transport,
                    &base_url,
                    Some(api_key.as_str()),
                    idle_timeout,
                    request,
                    cancel,
                    move |deltas| {
                        if emit {
                            let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Delta {
                                key: key.clone(),
                                deltas: deltas.to_vec(),
                            }));
                        }
                    },
                )
                .await
            }
        };
        let summary = host::drive_agent_turn(
            stream_key.clone(),
            context,
            net_sender.clone(),
            executor,
            stream_one,
            cancel,
        )
        .await;
        let _ = net_sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
            key: stream_key,
            reason: summary.stop_reason(),
            usage: summary.usage,
        }));
    });
    Ok(())
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

/// 统计当前父会话下正在运行的子 agent 数量。
fn running_count_for_parent(streams: &ChatStreams, parent_session: &str) -> usize {
    streams
        .subagents
        .values()
        .filter(|handle| handle.parent.session == parent_session && handle.status == STATUS_RUNNING)
        .count()
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

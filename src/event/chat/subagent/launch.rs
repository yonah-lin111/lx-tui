//! 子轮启动：把已在 DB 登记 user/assistant 消息的一轮交给进程运行时执行。

use std::path::Path;
use std::time::Duration;

use tokio::sync::watch;

use super::context::{ChildTurnLimits, assemble_child_context_with_permission};
use super::*;
use crate::app::state::lx::agent::{SubagentRun, seed_chat_model};
use crate::app::state::{RequestKey, RequestState};
use crate::chat::events::{ChatRequest, RequestPurpose, StopReason};
use crate::db::chat::{ModelSnapshot, PersistedInputItem, SubagentRecord};

/// 启动一个已在 DB 登记 user/assistant 消息的子轮（`spawn` 与队列消费/恢复共用）。
#[allow(clippy::too_many_arguments)]
pub(super) fn launch_child_turn(
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
    notify_parent: bool,
    reused_done: Option<watch::Sender<bool>>,
    limits: ChildTurnLimits,
    root_session: &str,
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
    let workspace_trusted = persistence.is_trusted(cwd) || persistence.is_trusted(workspace);
    let effective_subagents = crate::config::subagents::merge_project_roles_for_paths(
        &state.subagents_config,
        Some(workspace),
        Some(cwd),
        workspace_trusted,
    );
    let (effective_sandbox, exact_rules, prefix_rules) = super::effective_sandbox_for_session(
        state,
        Some(database),
        workspace,
        parent_key.pane,
        root_session,
    );
    let prev_child_mode = database
        .session_sandbox_mode(workspace, &record.child_session_id)
        .ok()
        .flatten()
        .and_then(|text| crate::config::sandbox::SandboxMode::parse(&text));
    if let Err(error) = database.set_session_sandbox_mode(
        workspace,
        &record.child_session_id,
        Some(effective_sandbox.mode.as_str()),
    ) {
        tracing::warn!(%error, session = %record.child_session_id, "failed to sync child sandbox mode");
    }
    let permission_update = if !is_new_agent
        && let Some(prev) = prev_child_mode
        && prev != effective_sandbox.mode
    {
        Some(crate::chat::agent::render_permission_switch_update(
            prev,
            effective_sandbox.mode,
            &effective_sandbox,
            Some(workspace),
            Some(cwd),
            false,
        ))
    } else {
        None
    };
    let inputs = super::context::capture_child_prompt_inputs_with_sandbox(
        state,
        persistence,
        parent_key.pane,
        workspace,
        cwd,
        frozen,
        record.role,
        &effective_sandbox,
    );
    let context = assemble_child_context_with_permission(
        database,
        workspace,
        &parent_key.session,
        &record.agent_id,
        &record.child_session_id,
        child_user_message_id,
        record.role,
        super::context::ChildPromptContext {
            system_prompt_override: inputs.system_prompt_override.as_deref(),
            model_identity: Some(&inputs.model_identity),
            environment: &inputs.environment,
            instructions: inputs.instructions.as_deref(),
            skills: inputs.skills.as_deref(),
            mcp: &inputs.mcp,
        },
        cwd,
        &model,
        frozen.variant_params.clone(),
        frozen.context_limit,
        frozen.output_limit,
        frozen.supports_image_input,
        items,
        &effective_subagents,
        limits,
        permission_update.as_deref(),
        effective_sandbox.escalation == crate::config::sandbox::EscalationPolicy::Never,
    )?;
    let mcp_snapshot = inputs.mcp;

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
            chat.rebuild_steered_user_messages(existing);
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
    // 私聊追问不更新父可见任务摘要：`task` 会进入 `@` 引用块与快照标题兜底。
    let previous_task = state
        .pane_anywhere(parent_key.pane)
        .and_then(|pane| pane.lx.agents.by_agent(&record.agent_id))
        .map(|run| run.task.clone());
    let visible_task = crate::app::state::lx::agent::visible_task(
        task_text,
        notify_parent,
        previous_task.as_deref(),
        &record.title,
    );

    if is_new_agent {
        let mut run = SubagentRun::from_record(record);
        run.task = visible_task.clone();
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
            run.task = visible_task.clone();
            run.chat = ui_chat;
            run.compact.replace(existing_checkpoints);
            run.loaded = true;
            run.rows_dirty = true;
        } else {
            let mut run = SubagentRun::from_record(record);
            run.status = STATUS_RUNNING.to_string();
            run.started_at_ms = started_at_ms;
            run.task = visible_task.clone();
            run.chat = ui_chat;
            run.compact.replace(existing_checkpoints);
            run.loaded = true;
            pane.lx.agents.runs.push(run);
        }
        pane.lx.sync_mention_subagent_runs();
    }
    let turn_user_message_ids = vec![child_user_message_id.to_string()];

    let (cancel_sender, cancel) = watch::channel(false);
    let done_sender = reused_done.unwrap_or_else(|| watch::channel(false).0);
    // steer 会切换当前轮的 mutation 归属；句柄与 recorder 共享同一份 id。
    let mutation_user_id =
        std::sync::Arc::new(std::sync::RwLock::new(child_user_message_id.to_string()));
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
        nickname: record.nickname.clone(),
        agent_path: record.agent_path.clone(),
        root_session: root_session.to_string(),
        depth: limits.depth,
        child_session_id: record.child_session_id.clone(),
        child_assistant_id: child_assistant_id.to_string(),
        child_user_message_id: child_user_message_id.to_string(),
        turn_user_message_ids,
        role: record.role,
        title: record.title.clone(),
        task: visible_task,
        cancel: cancel_sender.clone(),
        done: done_sender,
        status: STATUS_RUNNING,
        started_at_ms,
        max_tokens: limits.max_tokens,
        closed_without_report: false,
        notify_parent,
        mutation_user_id: mutation_user_id.clone(),
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
    let stream_log = crate::chat::client::StreamLog {
        kind: "subagent",
        pane: Some(stream_key.pane.raw()),
        session_id: Some(stream_key.session.clone()),
        generation: Some(stream_key.generation),
        agent_id: Some(record.agent_id.clone()),
        parent_session: Some(parent_key.session.clone()),
        ..crate::chat::client::StreamLog::default()
    };
    tracing::info!(
        target: "lx::agent",
        pane = parent_key.pane.raw(),
        session = %record.child_session_id,
        parent_session = %parent_key.session,
        agent_id = %record.agent_id,
        role = record.role.as_str(),
        "subagent turn started"
    );
    let reads = crate::chat::tools::read_registry(&record.child_session_id);
    let recorder = super::super::file_snapshots::HostRecorderFactory::new(
        net_sender.clone(),
        workspace.to_path_buf(),
        record.child_session_id.clone(),
        mutation_user_id,
    );
    crate::chat::exec_policy::reload_if_changed();
    let executor = AgentTools::new(manager, turn)
        .with_recorder(std::sync::Arc::new(recorder))
        .with_reads(reads)
        .with_sandbox(
            crate::chat::sandbox::SandboxPlan::for_turn(&effective_sandbox, cwd)
                .with_exact_rules(exact_rules, prefix_rules),
        )
        .with_image_support(frozen.supports_image_input)
        .with_web_search_keys(
            state.models.settings.web_search.exa_api_key.clone(),
            state.models.settings.web_search.tavily_api_key.clone(),
        )
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
                    budget_exceeded: false,
                    usage: Usage::default(),
                    billing: Usage::default(),
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
            let stream_log = stream_log.clone();
            let emit = request.purpose != RequestPurpose::CompactSummary;
            async move {
                crate::chat::client::run_stream_with_log(
                    &transport,
                    &base_url,
                    Some(api_key.as_str()),
                    idle_timeout,
                    request,
                    &stream_log,
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
            budget_exceeded: summary.budget_exceeded,
            usage: summary.usage,
            billing: summary.billing,
        }));
    });
    Ok(())
}

/// 解析会话当前生效的沙箱配置与会话级规则（实时窗格状态优先，否则回查 DB）。
pub(crate) fn effective_sandbox_for_session(
    state: &AppState,
    database: Option<&crate::db::Database>,
    workspace: &Path,
    pane: crate::tiling::PaneId,
    session_id: &str,
) -> (
    crate::config::sandbox::SandboxSettings,
    Vec<Vec<String>>,
    Vec<Vec<String>>,
) {
    let mut settings = state.sandbox.clone();
    if let Some(target) = state.pane_anywhere(pane)
        && target.lx.chat.session_id.as_deref() == Some(session_id)
    {
        if let Some(mode) = target.lx.sandbox_mode {
            settings.mode = mode;
        }
        return (
            settings,
            target.lx.sandbox_exact_rules.clone(),
            target.lx.sandbox_prefix_rules.clone(),
        );
    }
    if let Some(db) = database {
        if let Some(mode) = db
            .session_sandbox_mode(workspace, session_id)
            .ok()
            .flatten()
            .and_then(|text| crate::config::sandbox::SandboxMode::parse(&text))
        {
            settings.mode = mode;
        }
        let rules = db.sandbox_rules(workspace, session_id).unwrap_or_default();
        let exact = rules
            .iter()
            .filter(|rule| rule.kind == crate::db::sandbox::SandboxRuleKind::Exact)
            .map(|rule| rule.tokens.clone())
            .collect();
        let prefix = rules
            .iter()
            .filter(|rule| rule.kind == crate::db::sandbox::SandboxRuleKind::Prefix)
            .map(|rule| rule.tokens.clone())
            .collect();
        return (settings, exact, prefix);
    }
    (settings, Vec::new(), Vec::new())
}

pub(super) fn wake_subagent_approvals(
    state: &mut AppState,
    streams: &mut ChatStreams,
    keys: Vec<RequestKey>,
) {
    for child_key in keys {
        super::super::approval::cancel(state, streams, &child_key);
    }
}

/// 停止窗格所属整棵会话树里全部正在运行的子 agent（会话删除/退出路径使用）。
pub(crate) fn stop_all_subagents_for_pane(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    pane: crate::tiling::PaneId,
) -> usize {
    let Some(session) = state
        .pane_anywhere(pane)
        .and_then(|target| target.lx.chat.session_id.clone())
    else {
        return 0;
    };
    let root_session = persistence
        .database()
        .zip(super::workspace_for_pane(state, pane))
        .and_then(|(database, workspace)| {
            database
                .resolve_session_tree_root(&workspace, &session)
                .ok()
                .map(|(root, _, _)| root)
        })
        .unwrap_or_else(|| session.clone());

    let mut stopped = 0usize;
    let mut dead = Vec::new();
    let mut cancelled_keys = Vec::new();
    for (child, handle) in &mut streams.subagents {
        if handle.root_session != root_session || handle.status != STATUS_RUNNING {
            continue;
        }
        handle.closed_without_report = true;
        streams.resumed_subagents.remove(&handle.agent_id);
        streams.interrupted_subagents.remove(&handle.agent_id);
        streams.paused_by_interrupt.remove(&handle.agent_id);
        cancelled_keys.push(RequestKey {
            pane: handle.parent.pane,
            session: handle.child_session_id.clone(),
            generation: handle.parent.generation,
        });
        if handle.cancel.send(true).is_ok() {
            stopped = stopped.saturating_add(1);
        } else {
            dead.push(child.clone());
        }
    }
    wake_subagent_approvals(state, streams, cancelled_keys);
    for child in dead {
        streams.subagents.remove(&child);
        streams.frozen.remove(&child);
    }
    stopped
}

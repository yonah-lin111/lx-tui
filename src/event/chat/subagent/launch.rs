//! 子轮启动：把已在 DB 登记 user/assistant 消息的一轮交给进程运行时执行。

use std::path::Path;
use std::time::Duration;

use tokio::sync::watch;

use super::context::assemble_child_context;
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
    let inputs = super::context::capture_child_prompt_inputs(
        state,
        persistence,
        workspace,
        cwd,
        frozen,
        record.role,
    );
    let context = assemble_child_context(
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
        nickname: record.nickname.clone(),
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
            billing: summary.billing,
        }));
    });
    Ok(())
}

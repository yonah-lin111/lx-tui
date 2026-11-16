//! 单个 BTW 回答者的启动与结算（含 `/report` 批次聚合）。

use super::*;

/// 启动一个回答者（DB 认领 + 上下文装配 + 网络任务）。
pub(super) fn start_recipient(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    sender: &mpsc::UnboundedSender<AppEvent>,
    pending: BtwPending,
) -> bool {
    let Some(database) = persistence.database() else {
        return false;
    };
    // 工作区/cwd 取提交时冻结值：源窗格关闭或工作区切换后仍按原归属运行。
    let workspace = pending.workspace.clone();
    // BTW child session 与主轮共用跨进程 guard：启动恢复只处理拿得到 guard 的
    // session，不能把另一个 lx 实例仍在流式输出的回答标成 interrupted。
    let guard = match crate::platform::try_lock_session(&pending.child_session_id) {
        Ok(Some(guard)) => guard,
        Ok(None) => {
            tracing::warn!(
                session = %pending.child_session_id,
                "btw recipient skipped: session is owned by another instance"
            );
            let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
                pane: pending.pane,
                qa_id: Some(pending.qa_id.clone()),
                message: crate::ui::text::LX_BTW_SUBMIT_FAILED.to_string(),
            }));
            return false;
        }
        Err(error) => {
            tracing::warn!(%error, session = %pending.child_session_id, "btw session guard unavailable");
            let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
                pane: pending.pane,
                qa_id: Some(pending.qa_id.clone()),
                message: crate::ui::text::LX_BTW_SUBMIT_FAILED.to_string(),
            }));
            return false;
        }
    };
    let start = match database.start_btw_recipient(&workspace, &pending.recipient_id) {
        Ok(Some(start)) => start,
        Ok(None) => {
            // 已被恢复流程/其它路径结算：收敛批次状态，避免永久停在 queued/running。
            let _ = database.sync_btw_batch_status(&pending.batch_id);
            let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
                pane: pending.pane,
                qa_id: Some(pending.qa_id.clone()),
                message: crate::ui::text::LX_BTW_SUBMIT_FAILED.to_string(),
            }));
            return false;
        }
        Err(error) => {
            tracing::warn!(code = error.code(), "btw recipient claim failed");
            let _ = database.fail_btw_recipient(&pending.recipient_id, "recipient claim failed");
            let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
                pane: pending.pane,
                qa_id: Some(pending.qa_id.clone()),
                message: crate::ui::text::LX_BTW_SUBMIT_FAILED.to_string(),
            }));
            return false;
        }
    };
    let pane = pending.pane;
    let cwd = pending.cwd.clone();
    let selection = state
        .models
        .normalize_selection(crate::config::ai::ModelSelection {
            provider: start.model.provider.clone(),
            model: start.model.model.clone(),
            variant: start.model.variant.clone(),
        });
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        let _ = database.finish_btw_recipient(
            &workspace,
            &pending.recipient_id,
            &start.assistant_message_id,
            None,
            &TurnOutcome::Failed {
                error_code: "model unavailable".to_string(),
            },
            Default::default(),
            Default::default(),
        );
        let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
            pane: pending.pane,
            qa_id: Some(pending.qa_id.clone()),
            message: crate::ui::text::LX_BTW_SUBMIT_FAILED.to_string(),
        }));
        return false;
    };
    let mcp_snapshot = super::mcp::snapshot_for_turn_with_disabled(
        persistence,
        state
            .project_root_for_pane(pane)
            .as_deref()
            .or(Some(workspace.as_path())),
        &std::collections::BTreeSet::new(),
    );
    let references = agent_references(persistence, &workspace, &start, &pending.thread_id);
    let context = match assemble_btw_context(BtwContextInput {
        database,
        workspace: &workspace,
        parent_session_id: &start.parent_session_id,
        qa_anchor_id: &start.qa_message_id,
        child_session_id: &start.child_session_id,
        current_user_message_id: &start.user_message_id,
        question: &start.question,
        answerer_label: &start.display_nickname,
        references: &references,
        model: &start.model,
        variant_params: frozen.variant_params.clone(),
        cwd: &cwd,
        context_limit: frozen.context_limit,
        mcp: &mcp_snapshot,
    }) {
        Ok(context) => context,
        Err(error) => {
            tracing::warn!(%error, "btw context assembly failed");
            let _ = database.finish_btw_recipient(
                &workspace,
                &pending.recipient_id,
                &start.assistant_message_id,
                None,
                &TurnOutcome::Failed {
                    error_code: "context unavailable".to_string(),
                },
                Default::default(),
                Default::default(),
            );
            let _ = sender.send(AppEvent::Btw(BtwEvent::Failed {
                pane: pending.pane,
                qa_id: Some(pending.qa_id.clone()),
                message: crate::ui::text::LX_BTW_SUBMIT_FAILED.to_string(),
            }));
            return false;
        }
    };
    let Some(sender) = Some(sender.clone()) else {
        return false;
    };
    let (manager, turn) = match streams.runtime_for(cwd.clone(), &start.child_session_id) {
        Some(runtime) => (runtime.manager.clone(), runtime.turn(&pending.recipient_id)),
        None => return false,
    };
    streams.btw.generation = streams.btw.generation.saturating_add(1);
    let key = RequestKey {
        pane,
        session: start.child_session_id.clone(),
        generation: streams.btw.generation,
    };
    streams
        .btw
        .active_batches
        .insert(pending.thread_id.clone(), pending.batch_id.clone());
    let (cancel_sender, cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender.clone());
    streams.db_workspaces.insert(key.clone(), workspace.clone());
    // 统计行展示回答者模型，而不是回答者身份；与 DB reload 的 view_message_of 同源，
    // 同一条线程不会因加载路径不同而变样。
    let model_label = state
        .models
        .model_display_name(&start.model.provider, &start.model.model);
    let mut buffer = crate::app::state::LxChat::new();
    buffer
        .messages
        .push(crate::app::update::new_btw_assistant_message(
            &start.assistant_message_id,
            &model_label,
        ));
    let run = BtwRun {
        thread_id: pending.thread_id.clone(),
        batch_id: pending.batch_id.clone(),
        recipient_id: pending.recipient_id.clone(),
        assistant_message_id: start.assistant_message_id.clone(),
        child_session_id: start.child_session_id.clone(),
        parent_session_id: start.parent_session_id.clone(),
        buffer,
        _guard: guard,
        started_at_ms: crate::db::now_ms(),
        last_checkpoint_ms: crate::db::now_ms(),
        label: model_label,
    };
    let label = run.label.clone();
    streams.btw.runs.insert(key.clone(), run);

    let reads = crate::chat::tools::read_registry(&start.child_session_id);
    let recorder = super::file_snapshots::HostRecorderFactory::new(
        sender.clone(),
        workspace.clone(),
        start.child_session_id.clone(),
        std::sync::Arc::new(std::sync::RwLock::new(start.user_message_id.clone())),
    );
    let mut executor = AgentTools::new(manager, turn)
        .with_recorder(std::sync::Arc::new(recorder))
        .with_reads(reads)
        .with_image_support(frozen.supports_image_input)
        .with_mcp(
            crate::mcp::global_manager().clone(),
            mcp_snapshot,
            start.child_session_id.clone(),
        );
    let _ = &mut executor;
    let transport = frozen.transport.clone();
    let base_url = frozen.base_url.clone();
    let api_key = frozen.api_key.clone();
    let idle_timeout = Duration::from_millis(frozen.idle_timeout_ms);
    let stream_key = key.clone();
    let stream_log = crate::chat::client::StreamLog {
        kind: "btw",
        pane: Some(pane.raw()),
        session_id: Some(key.session.clone()),
        generation: Some(key.generation),
        parent_session: Some(start.parent_session_id.clone()),
        ..crate::chat::client::StreamLog::default()
    };
    let _ = sender.send(AppEvent::Btw(BtwEvent::RecipientStarted {
        child_session_id: start.child_session_id.clone(),
        assistant_message_id: start.assistant_message_id.clone(),
        label,
    }));
    let finish_sender = sender.clone();
    tokio::spawn(async move {
        let task_sender = sender;
        let task_key = stream_key.clone();
        // 内层任务捕获 panic：否则 panic 后既不会回投 Finished，也不会结算回答者，
        // 批次会永久停在 running、后续追问永久排队（表现为「AI 无法输出」）。
        let handle = tokio::spawn(async move {
            let live_sender = task_sender.clone();
            let live_key = task_key.clone();
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
            super::host::drive_agent_turn(
                task_key,
                context,
                task_sender,
                executor,
                stream_one,
                cancel,
            )
            .await
        });
        let summary = match handle.await {
            Ok(summary) => summary,
            Err(error) => {
                tracing::error!(%error, "btw turn task failed");
                crate::chat::agent_loop::TurnSummary {
                    outcome: crate::chat::agent_loop::TurnOutcome::Failed {
                        code: "btw task aborted".to_string(),
                    },
                    usage: Default::default(),
                    billing: Default::default(),
                    budget_exceeded: false,
                }
            }
        };
        let _ = finish_sender.send(AppEvent::ChatNet(ChatNetEvent::Finished {
            key: stream_key,
            reason: summary.stop_reason(),
            budget_exceeded: summary.budget_exceeded,
            usage: summary.usage,
            billing: summary.billing,
        }));
    });
    true
}

fn agent_references(
    persistence: &Persistence,
    workspace: &Path,
    start: &crate::db::chat::BtwRecipientStart,
    _thread_id: &str,
) -> Vec<BtwAgentReference> {
    let Some(database) = persistence.database() else {
        return Vec::new();
    };
    let Some(agent_id) = start.agent_id.as_deref() else {
        return Vec::new();
    };
    let records = database
        .list_subagents(workspace, &start.parent_session_id)
        .unwrap_or_default();
    records
        .iter()
        .filter(|record| record.agent_id == agent_id)
        .map(|record| BtwAgentReference {
            agent_id: record.agent_id.clone(),
            nickname: record
                .nickname
                .clone()
                .unwrap_or_else(|| record.agent_id.clone()),
            role: record.role.as_str().to_string(),
            status: record.status.clone(),
            summary: format!(
                "title: {}\nstatus: {}\nchild_session: {}",
                record.title, record.status, record.child_session_id
            ),
        })
        .collect()
}

/// 停止某线程的全部活动与排队回答；不影响原任务。
pub(super) fn stop_thread(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    thread_id: Option<&str>,
) -> bool {
    let Some(thread_id) = thread_id else {
        return false;
    };
    let keys: Vec<RequestKey> = streams
        .btw
        .runs
        .iter()
        .filter(|(_, run)| run.thread_id == thread_id)
        .map(|(key, _)| key.clone())
        .collect();
    let pending_batches: Vec<String> = streams
        .btw
        .pending
        .iter()
        .filter(|pending| pending.thread_id == thread_id)
        .map(|pending| pending.batch_id.clone())
        .collect();
    let changed = cancel_threads(streams, &[thread_id.to_string()]);
    if let Some(database) = persistence.database() {
        let mut batches = pending_batches;
        batches.sort();
        batches.dedup();
        for batch in batches {
            let _ = database.cancel_btw_queued(&batch);
        }
    }
    let _ = state;
    changed || !keys.is_empty()
}

/// 取消若干线程的在途回答与排队项（不写库）。
pub(super) fn cancel_threads(streams: &mut ChatStreams, thread_ids: &[String]) -> bool {
    let keys: Vec<RequestKey> = streams
        .btw
        .runs
        .iter()
        .filter(|(_, run)| thread_ids.iter().any(|id| id == &run.thread_id))
        .map(|(key, _)| key.clone())
        .collect();
    let mut changed = false;
    for key in keys {
        changed |= streams.cancel(&key);
    }
    let before = streams.btw.pending.len();
    streams
        .btw
        .pending
        .retain(|pending| !thread_ids.iter().any(|id| id == &pending.thread_id));
    for thread in thread_ids {
        streams.btw.active_batches.remove(thread);
    }
    changed || streams.btw.pending.len() != before
}

/// 旁路网络事件分流（Delta / Finished）。
pub(crate) fn handle_net_event(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    event: ChatNetEvent,
) -> bool {
    let key = match &event {
        ChatNetEvent::Delta { key, .. } | ChatNetEvent::Finished { key, .. } => key.clone(),
    };
    let Some(run) = streams.btw.runs.get(&key) else {
        // 已结束或已删除的 BTW 事件：丢弃，绝不回落主路径。
        return false;
    };
    let assistant_id = run.assistant_message_id.clone();
    let child_session = run.child_session_id.clone();
    match event {
        ChatNetEvent::Delta { deltas, .. } => {
            // UI 可能在切主会话/关闭 pane 后找不到 thread；无论可见与否，都把
            // 同一批增量应用到 run 自己的缓冲，结算时不会只剩最后一次 checkpoint。
            if let Some(run) = streams.btw.runs.get_mut(&key) {
                run.buffer.stream_assistant = Some(assistant_id.clone());
                let _ = crate::app::update::apply_btw_buffer_deltas(&mut run.buffer, &deltas);
                run.buffer.stream_assistant = None;
            }
            let changed =
                crate::app::update::on_btw_delta(state, &child_session, &assistant_id, &deltas);
            // 流中 checkpoint：面板关闭后仍可在库里留下已生成正文；节流 500ms。
            let now = crate::db::now_ms();
            if let Some(run) = streams.btw.runs.get_mut(&key)
                && now.saturating_sub(run.last_checkpoint_ms) >= 500
            {
                run.last_checkpoint_ms = now;
                let mut parts = crate::app::update::btw_parts(state, &child_session, &assistant_id);
                if parts.is_empty() {
                    parts = buffer_parts(&run.buffer, &assistant_id);
                }
                if !parts.is_empty()
                    && let Some(workspace) = streams.db_workspaces.get(&key).cloned()
                {
                    state
                        .effects
                        .push(crate::app::effects::Effect::CheckpointChatTurn {
                            workspace,
                            assistant_id: assistant_id.clone(),
                            parts,
                        });
                }
            }
            changed
        }
        ChatNetEvent::Finished {
            reason,
            usage,
            billing,
            ..
        } => finish_run(
            state,
            streams,
            persistence,
            &key,
            &child_session,
            &assistant_id,
            reason,
            usage,
            billing,
        ),
    }
}

/// 从无头流缓冲提取 assistant 正文/思考 parts。
fn buffer_parts(
    chat: &crate::app::state::LxChat,
    assistant_id: &str,
) -> Vec<crate::db::chat::ChatPart> {
    chat.messages
        .iter()
        .find(|message| message.id == assistant_id)
        .map(crate::db::chat::chat_message_part_views)
        .unwrap_or_default()
}

/// 结算一个旁路回答并推进同线程下一批次。
#[allow(clippy::too_many_arguments)]
pub(super) fn finish_run(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    child_session: &str,
    assistant_id: &str,
    reason: StopReason,
    usage: crate::db::chat::Usage,
    billing: crate::db::chat::Usage,
) -> bool {
    let Some(run) = streams.btw.runs.remove(key) else {
        return false;
    };
    let workspace = streams.db_workspaces.get(key).cloned().unwrap_or_default();
    let duration_ms = crate::db::now_ms().saturating_sub(run.started_at_ms).max(0);
    // 先在内存收尾（关闭思考块/在途工具），再取 parts 落库；顺序反过来会丢最后一次收尾。
    let changed = crate::app::update::on_btw_finished(
        state,
        child_session,
        assistant_id,
        &reason,
        usage,
        duration_ms,
    );
    let mut parts = crate::app::update::btw_parts(state, child_session, assistant_id);
    if parts.is_empty() {
        parts = buffer_parts(&run.buffer, assistant_id);
    }
    let outcome = match &reason {
        StopReason::Complete => TurnOutcome::Complete {
            stop_reason: Some("complete".to_string()),
        },
        StopReason::Cancelled => TurnOutcome::Interrupted,
        StopReason::Failed { code } => TurnOutcome::Failed {
            error_code: code.clone(),
        },
    };
    let mut batch_terminal = false;
    if let Some(database) = persistence.database() {
        match database.finish_btw_recipient(
            &workspace,
            &run.recipient_id,
            assistant_id,
            if parts.is_empty() {
                None
            } else {
                Some(parts.as_slice())
            },
            &outcome,
            usage,
            billing,
        ) {
            Ok(completion) => {
                batch_terminal = completion.batch_terminal;
                streams
                    .btw
                    .last_status
                    .insert(run.thread_id.clone(), completion.recipient_status);
            }
            Err(error) => tracing::warn!(code = error.code(), "btw settle failed"),
        }
    }
    // 只清本 run 的执行资源；不按 session 清理其他目标的模型。
    streams.cancels.remove(key);
    streams.db_workspaces.remove(key);
    streams.btw.runs.remove(key);
    if batch_terminal
        && streams
            .btw
            .active_batches
            .get(&run.thread_id)
            .is_some_and(|batch| batch == &run.batch_id)
    {
        streams.btw.active_batches.remove(&run.thread_id);
    }
    changed
}

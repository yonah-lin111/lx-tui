//! 启动一次聊天网络流：校验在途请求、装配上下文并 spawn agent loop。
//!
//! 与事件分发分离，避免 `event/chat/mod.rs` 继续膨胀。

use super::*;

/// 启动一轮：装配上下文、冻结模型与工作区并 spawn agent turn。
pub(super) fn start_stream(
    state: &mut AppState,
    streams: &mut ChatStreams,
    sender: &mpsc::UnboundedSender<AppEvent>,
    persistence: &Persistence,
    key: RequestKey,
    assistant_id: String,
) -> bool {
    // 效应可能在 session 切换/新会话生效之后才被泵到：key 必须仍对应当前窗格的
    // 在途请求与 assistant 占位。否则会把新会话的消息装配到旧 session（或反过来）。
    let live = state.pane_anywhere(key.pane).is_some_and(|pane| {
        pane.lx.chat.session_id.as_deref() == Some(key.session.as_str())
            && matches!(
                &pane.lx.chat.request,
                RequestState::Streaming {
                    key: active,
                    assistant_id: active_assistant,
                } if active == &key && active_assistant == &assistant_id
            )
            && pane.lx.chat.messages.iter().any(|message| {
                message.id == assistant_id
                    && message.role == "assistant"
                    && message.status == "streaming"
            })
    });
    if !live {
        return false;
    }
    // 已有同 key 在途流：不重复启动。
    if streams.cancels.contains_key(&key) {
        return false;
    }
    let turn_index = state.pane_anywhere(key.pane).and_then(|pane| {
        pane.lx
            .chat
            .messages
            .iter()
            .find(|message| message.id == assistant_id)
            .map(|message| message.turn_index)
    });
    tracing::info!(
        target: "lx::agent",
        pane = key.pane.raw(),
        session = %key.session,
        generation = key.generation,
        turn = ?turn_index,
        "agent turn started"
    );
    // 冻结模型快照：从窗格当前消息流状态读取该轮的选择（发送时冻结语义：Committing 已把
    // 模型写入 DB；流启动时按 `effective_selection` 重解析，补上默认 variant 后再冻结）。
    let (
        frozen,
        assembled,
        current_plan,
        mcp_snapshot,
        skill_omitted,
        workspace_root,
        agents_md_count,
        effective_subagents,
    ) = {
        let Some(pane) = state.pane_anywhere(key.pane) else {
            return false;
        };
        let Some(selection) = pane.lx.chat.effective_selection(&state.models) else {
            // 无有效模型：直接按失败收尾。
            fail_turn(sender, key, "no model available");
            return true;
        };
        let Some(frozen) = crate::app::state::FrozenModel::resolve(&state.models, &selection)
        else {
            fail_turn(sender, key, "model configuration invalid");
            return true;
        };
        // 上下文：从窗格内存中的完整轮次装配（DB 历史与内存一致；partial 排除）。
        // 本轮的 compact 摘要与环境快照在这里冻结：本轮所有采样复用同一份。
        let compact = pane
            .lx
            .compact
            .active()
            .map(|checkpoint| context::CompactContext {
                end_sequence: checkpoint.end_sequence,
                summary: checkpoint.summary.clone(),
                model_label: checkpoint.model_label.clone(),
            });
        // 环境快照：工作区根 + 冻结 cwd + 受管后台任务摘要 + 会话生效的沙箱配置。
        let workspace_root = state.workspace_cwd_for_pane(key.pane);
        let effective_sandbox = state.effective_sandbox_for_pane(key.pane);
        let environment = crate::chat::prompt::environment_snapshot_with_sandbox(
            &crate::platform::EnvironmentFacts::capture(),
            workspace_root.as_deref(),
            workspace_root.as_deref(),
            &background_tasks(pane.lx.chat.session_id.as_deref(), streams),
            &effective_sandbox,
        );
        // skill 目录与 MCP 快照在 turn 起点冻结（配置根取仓库主 checkout，执行 cwd 为 workspace_root）。
        let config_root = state
            .chat_session_key_for_pane(key.pane)
            .or_else(|| workspace_root.clone());
        let (skills, skill_omitted) =
            skills::catalog_prompt(persistence, config_root.as_deref(), false);
        let (draft_text, _) = timeline::latest_user_draft_and_attachments(&pane.lx.chat.messages);
        // 合成汇报轮的草稿是注入的报告文本，不是用户输入：跳过 `@` 引用块提取。
        let synthetic_report = subagent::snapshot::is_synthetic_report_turn(&pane.lx.chat.messages);
        let mut skill_blocks = if synthetic_report {
            Vec::new()
        } else {
            skills::reference_blocks(&draft_text)
        };
        let mcp_snapshot = mcp::snapshot_for_turn_with_disabled(
            persistence,
            config_root.as_deref(),
            pane.lx.disabled_mcp_servers(),
        );
        let mcp_prompt = mcp_snapshot.instructions_prompt();
        let model_identity = crate::chat::agent::ModelIdentity {
            display_name: state
                .models
                .model_display_name(&frozen.provider, &frozen.model),
            provider_id: frozen.provider.clone(),
            model_id: frozen.model.clone(),
            variant: frozen.variant.clone(),
            supports_image_input: frozen.supports_image_input,
        }
        .render_prompt_section();
        let workspace_trusted = [workspace_root.as_deref(), config_root.as_deref()]
            .into_iter()
            .flatten()
            .any(|path| persistence.is_trusted(path));
        let effective_subagents = crate::config::subagents::merge_project_roles_for_paths(
            &state.subagents_config,
            config_root.as_deref(),
            workspace_root.as_deref(),
            workspace_trusted,
        );
        if !synthetic_report {
            skill_blocks.extend(subagent::snapshot::subagent_reference_blocks(
                &draft_text,
                &effective_subagents,
                &pane.lx.agents.runs,
            ));
        }
        let loaded_instructions = crate::chat::agent::load_instructions(
            crate::config::user_data_root().ok().as_deref(),
            config_root.as_deref(),
            workspace_root.as_deref(),
            workspace_trusted,
        );
        let agents_md_count = loaded_instructions.active_count();
        let instructions = loaded_instructions.render_prompt_section();
        let model_switches: Vec<crate::db::chat::ModelSwitchRecord> = pane
            .lx
            .model_switch
            .switches
            .iter()
            .map(crate::app::state::ModelSwitchNoticeView::to_record)
            .collect();
        let permission_switches: Vec<crate::db::chat::PermissionSwitchRecord> = pane
            .lx
            .permission_switch
            .switches
            .iter()
            .map(crate::app::state::PermissionSwitchNoticeView::to_record)
            .collect();
        let mut assembled = assemble_turn_request_with_context(
            &pane.lx.chat.messages,
            &frozen,
            compact.as_ref(),
            Some(&model_identity),
            Some(&environment),
            instructions.as_deref(),
            skills.as_deref(),
            mcp_prompt.as_deref(),
            &model_switches,
            &permission_switches,
            &skill_blocks,
        );
        let mut tool_decls = crate::chat::tools::turn_tools_with_subagents(
            crate::chat::agent::ToolSurface::Main,
            &mcp_snapshot,
            Some(&effective_subagents),
        );
        if effective_sandbox.escalation == crate::config::sandbox::EscalationPolicy::Never {
            crate::chat::tools::trim_escalation_parameters(&mut tool_decls);
        }
        assembled.request.tools = std::sync::Arc::new(tool_decls);
        // 计划快照先克隆：运行上下文摘要要在后面的可变借用窗外拼装。
        let current_plan = pane.lx.chat.plan.clone();
        (
            frozen,
            assembled,
            current_plan,
            mcp_snapshot,
            skill_omitted,
            workspace_root,
            agents_md_count,
            effective_subagents,
        )
    };
    state
        .agents_md_inventory
        .insert(workspace_root, agents_md_count);
    if skill_omitted > 0 {
        update::show_lx_toast(
            state,
            key.pane,
            Toast::new(
                ToastKind::Warning,
                text::skills::catalog_omitted(skill_omitted),
                Some(key.pane),
                Instant::now(),
            ),
        );
    }
    // 省略提示（一次性显示）。
    if assembled.omitted_turns > 0
        && let Some(pane) = state.pane_mut_anywhere(key.pane)
    {
        pane.lx.chat.omitted_turns_notice = Some(assembled.omitted_turns);
    }
    // 当前用户消息单独超限：整轮失败并报错，绝不截断用户输入。
    if assembled.draft_exceeds {
        fail_turn(sender, key, DRAFT_EXCEEDS_CONTEXT_CODE);
        return true;
    }
    let mut request = assembled.request;
    if let Err(code) = timeline::hydrate_item_images(
        state,
        persistence,
        &key,
        frozen.supports_image_input,
        &assembled.item_image_attachments,
        &mut request,
    ) {
        fail_turn(sender, key, code);
        return true;
    }
    // 冻结工作区根：本轮所有工具的相对路径都以它为准（后续切换窗格不影响）。
    let Some(workspace) = state.workspace_cwd_for_pane(key.pane) else {
        fail_turn(sender, key, "workspace path unavailable");
        return true;
    };
    let mut prepared = prepare_turn(request, workspace);
    // 跨进程 session guard：续轮在 `BeginChatTurn` 之前已由事件循环取得并暂存，
    // 这里取走；新会话没有并发者，才在拿到 session id 后补取。
    let guard = match streams.pending_guards.remove(&(key.pane, key.generation)) {
        Some(guard) => guard,
        None => match crate::platform::try_lock_session(&key.session) {
            Ok(Some(guard)) => guard,
            Ok(None) => {
                fail_turn(sender, key, "chat session is busy in another instance");
                return true;
            }
            Err(error) => {
                tracing::warn!(%error, "session guard unavailable");
                fail_turn(sender, key, "chat session guard unavailable");
                return true;
            }
        },
    };
    streams.guards.insert(key.clone(), guard);
    // 本轮已进入初始上下文的命令标记为已见（初始运行中的命令仍可在完成后吸收）。
    user_shell::mark_initial_seen(state, streams, &key);
    // 取消通道；sender 由 ChatStreams 持有。
    let (cancel_sender, cancel) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_sender);
    // DB 归属键与执行 cwd 分离：linked worktree 下工具行与快照仍存在会话所属工作区。
    let session_key = state
        .chat_session_key_for_pane(key.pane)
        .unwrap_or_else(|| prepared.cwd.clone());
    streams
        .db_workspaces
        .insert(key.clone(), session_key.clone());
    // spawn 时冻结的模型：父会话登记一份，子 agent 从这里取。
    streams.frozen.insert(key.session.clone(), frozen.clone());
    // 每轮绑定稳定 user message id：文件 mutation 的归属来自这里，不按 turn_index 猜。
    let Some(user_message_id) = state.pane_anywhere(key.pane).and_then(|pane| {
        pane.lx
            .chat
            .messages
            .iter()
            .rev()
            // 本地命令卡片同样是 `role == "user"`，不能作为本轮聊天消息的锚点。
            .find(|message| message.role == "user" && !message.is_user_shell())
            .map(|message| message.id.clone())
    }) else {
        fail_turn(sender, key, "user message unavailable");
        return true;
    };
    let mutation_user = std::sync::Arc::new(std::sync::RwLock::new(user_message_id));
    streams
        .mutation_users
        .insert(key.clone(), mutation_user.clone());
    // 新会话的首条消息（或 `/permission` 在会话落库前选择）：把 pending 覆盖写入会话列。
    if let Some(mode) = state
        .pane_anywhere(key.pane)
        .and_then(|pane| pane.lx.sandbox_mode)
        && let Some(database) = persistence.database()
        && let Err(error) =
            database.set_session_sandbox_mode(&session_key, &key.session, Some(mode.as_str()))
    {
        tracing::warn!(%error, "failed to persist pending sandbox mode");
    }
    // 运行实例：同一聊天会话跨轮复用同一进程管理器；本轮执行上下文独立。
    let turn_slot = key.session.clone();
    let Some(runtime) = streams.runtime_for(prepared.cwd.clone(), &turn_slot) else {
        // 没有进程事件通道（测试或非进程环境）：仍可执行文件工具，进程工具按失败返回。
        fail_turn(sender, key, "process runtime unavailable");
        return true;
    };
    let turn = runtime.turn(&key.generation.to_string());
    let recorder = file_snapshots::HostRecorderFactory::new(
        sender.clone(),
        session_key.clone(),
        key.session.clone(),
        mutation_user,
    );
    // 读后版本保护：登记表按会话隔离，跨轮保留（读过的文件在后续轮次仍可用同一 token）。
    let reads = crate::chat::tools::read_registry(&key.session);
    // turn 起点：规则文件变化时重载（不阻塞启动；坏文件只告警）。
    crate::chat::exec_policy::reload_if_changed();
    let executor = AgentTools::new(runtime.manager.clone(), turn)
        .with_recorder(std::sync::Arc::new(recorder))
        .with_reads(reads)
        .with_sandbox(
            crate::chat::sandbox::SandboxPlan::for_turn(
                &state.effective_sandbox_for_pane(key.pane),
                &prepared.cwd,
            )
            .with_exact_rules(
                state
                    .pane_anywhere(key.pane)
                    .map(|pane| pane.lx.sandbox_exact_rules.clone())
                    .unwrap_or_default(),
                state
                    .pane_anywhere(key.pane)
                    .map(|pane| pane.lx.sandbox_prefix_rules.clone())
                    .unwrap_or_default(),
            ),
        )
        .with_image_support(frozen.supports_image_input)
        .with_web_search_keys(
            state.models.settings.web_search.exa_api_key.clone(),
            state.models.settings.web_search.tavily_api_key.clone(),
        )
        .with_mcp(
            crate::mcp::global_manager().clone(),
            mcp_snapshot,
            key.session.clone(),
        );
    // 本地运行上下文：存活进程 + 当前计划 + 在册子 agent + 委派模式。
    let mut summary = runtime_context_summary(runtime.manager.as_ref(), &current_plan);
    if let Some(guidance) = crate::chat::agent::delegation_mode_guidance(
        effective_subagents.delegation_mode,
        frozen.variant.as_deref(),
    ) {
        append_summary(&mut summary, guidance);
    }
    let subagent_lines = subagent::known_subagent_lines(
        streams,
        persistence.database(),
        Some(&session_key),
        &key.session,
    );
    if !subagent_lines.is_empty() {
        append_summary(&mut summary, &subagent_lines.join("\n"));
    }
    prepared.context = prepared.context.with_runtime_summary(summary);
    let provider = frozen.provider.clone();
    let transport = frozen.transport.clone();
    let base_url = frozen.base_url.clone();
    let api_key = frozen.api_key.clone();
    let idle_timeout = Duration::from_millis(frozen.idle_timeout_ms);
    let net_sender = sender.clone();
    let stream_key = key.clone();
    let stream_log = crate::chat::client::StreamLog {
        kind: "chat",
        pane: Some(key.pane.raw()),
        session_id: Some(key.session.clone()),
        turn_index,
        generation: Some(key.generation),
        ..crate::chat::client::StreamLog::default()
    };
    tokio::spawn(async move {
        let api_key = connect::chat_api_key(&provider, api_key).await;
        // 每次采样一次模型流；增量即时回投（打字机效果），整轮终态在 loop 结束时上报。
        let live_sender = net_sender.clone();
        let live_key = stream_key.clone();
        let stream_one = move |request: ChatRequest, cancel: watch::Receiver<bool>| {
            let sender = live_sender.clone();
            let key = live_key.clone();
            let transport = transport.clone();
            let base_url = base_url.clone();
            let api_key = api_key.clone();
            let stream_log = stream_log.clone();
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
                        let _ = sender.send(AppEvent::ChatNet(ChatNetEvent::Delta {
                            key: key.clone(),
                            deltas: deltas.to_vec(),
                        }));
                    },
                )
                .await
            }
        };
        let summary = host::drive_agent_turn(
            stream_key.clone(),
            prepared.context,
            net_sender.clone(),
            executor,
            stream_one,
            cancel,
        )
        .await;
        report_turn(net_sender, stream_key, summary, turn_index);
    });
    true
}

/// 追加一段运行上下文摘要（非空块之间空行分隔）。
fn append_summary(summary: &mut Option<String>, block: &str) {
    let text = summary.get_or_insert_with(String::new);
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(block);
}

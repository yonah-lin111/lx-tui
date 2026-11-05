//! 子轮输入归档与子轮宿主事件落库。
//!
//! 上下文装配见 [`super::context`]；由 [`super::ExecSubagentPool`] 的池控制入口调用，
//! 所有方法直接读写 Database，不接触事件层状态。等待与终态结算见 [`super::settle`]。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, watch};

use crate::chat::agent::{
    AgentRole, DEPTH_LIMIT_REACHED_CODE, ForkSpec, allocate_child_path, joined_input_text,
};
use crate::chat::agent_loop::{AgentTools, TurnExecution};
use crate::chat::events::{ChatDelta, ChatRequest};
use crate::chat::process::{ProcessManager, ProcessOwner};
use crate::db::chat::PersistedInputItem;
use crate::db::{Database, new_external_id};

use super::super::runner::{append_deltas_to_parts, db_result_of, new_mutation_recorder_factory};
use super::*;

impl ExecSubagentPool {
    /// 预校验并归档一次输入的全部图片；失败时由调用方决定是否清理会话图片目录。
    pub(super) fn prepare_input_items(
        &self,
        child_session_id: &str,
        items: &[SubagentInputItem],
        supports_image_input: bool,
    ) -> Result<Vec<PersistedInputItem>, HostError> {
        if items.is_empty() {
            return Err(failed("subagent input items must not be empty"));
        }
        let has_images = items
            .iter()
            .any(|item| !matches!(item, SubagentInputItem::Text { .. }));
        if has_images && !supports_image_input {
            return Err(failed("current model does not support image input"));
        }
        let mut out = Vec::with_capacity(items.len());
        let (mut count, mut total) = (0usize, 0usize);
        for item in items {
            let processed = match item {
                SubagentInputItem::Text { text } => {
                    out.push(PersistedInputItem::Text { text: text.clone() });
                    continue;
                }
                SubagentInputItem::LocalImage { path } => {
                    count += 1;
                    ensure_image(count, total)?;
                    let resolved = resolve_image_path(&self.cwd, path)?;
                    crate::platform::load_and_process_image_file(&resolved).map_err(|error| {
                        failed(&format!("invalid local_image: {}", error.code()))
                    })?
                }
                SubagentInputItem::Image { image_url } => {
                    count += 1;
                    ensure_image(count, total)?;
                    crate::platform::process_data_url_image(image_url, count.saturating_sub(1))
                        .map_err(|error| {
                            failed(&format!("invalid inline image: {}", error.code()))
                        })?
                }
            };
            total = total.saturating_add(processed.original_byte_count);
            ensure_image(count, total)?;
            out.push(archive_image(child_session_id, &processed)?);
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_child(
        &mut self,
        db: &Database,
        agent_id: &str,
        parent_session_id: &str,
        child_session_id: &str,
        child_assistant_id: &str,
        child_user_message_id: &str,
        agent_path: &str,
        depth: usize,
        max_depth: usize,
        max_tokens: Option<u64>,
        role: AgentRole,
        title: &str,
        started_at_ms: i64,
        current_items: &[PersistedInputItem],
        done_tx: watch::Sender<bool>,
    ) -> Result<(), HostError> {
        let context = self
            .child_context(
                db,
                agent_id,
                parent_session_id,
                child_session_id,
                role,
                current_items,
                depth,
                max_depth,
                max_tokens,
            )
            .map_err(|error| failed(&error))?;
        let processes = Arc::new(ProcessManager::new(
            ProcessOwner {
                workspace: self.cwd.clone(),
                chat_session_id: child_session_id.to_string(),
                runtime_epoch: new_external_id("epoch"),
            },
            None,
        ));
        let child_model = self
            .child_models
            .get(child_session_id)
            .cloned()
            .unwrap_or_else(|| self.model.clone());
        let executor = AgentTools::new(
            Arc::clone(&processes),
            Arc::new(Mutex::new(TurnExecution::default())),
        )
        .with_recorder(new_mutation_recorder_factory(
            self.sender.clone(),
            self.workspace.clone(),
            child_session_id.to_string(),
            child_user_message_id.to_string(),
        ))
        .with_reads(crate::chat::tools::read_registry(child_session_id))
        .with_image_support(child_model.supports_image_input)
        .with_web_search_keys(self.exa_api_key.clone(), self.tavily_api_key.clone())
        .with_mcp(
            crate::mcp::global_manager().clone(),
            Arc::clone(&self.mcp_snapshot),
            child_session_id.to_string(),
        );
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let stream = {
            let factory = Arc::clone(&self.stream_factory);
            let model = child_model;
            let sender = self.sender.clone();
            let ids = (
                agent_id.to_string(),
                child_session_id.to_string(),
                child_assistant_id.to_string(),
            );
            move |request: ChatRequest, cancel: watch::Receiver<bool>| {
                let factory = Arc::clone(&factory);
                let model = model.clone();
                let sender = sender.clone();
                let (agent_id, child_session_id, child_assistant_id) = ids.clone();
                async move {
                    let on_delta = Box::new(move |deltas: &[ChatDelta]| {
                        let _ = sender.send(ExecLoopEvent::Subagent(ExecSubagentEvent::Delta {
                            agent_id: agent_id.clone(),
                            child_session_id: child_session_id.clone(),
                            child_assistant_id: child_assistant_id.clone(),
                            deltas: deltas.to_vec(),
                        }));
                    });
                    factory.stream(&model, request, cancel, on_delta).await
                }
            }
        };
        let host = ChildHost {
            sender: self.sender.clone(),
            agent_id: agent_id.to_string(),
            child_session_id: child_session_id.to_string(),
            child_assistant_id: child_assistant_id.to_string(),
        };
        let finished = {
            let sender = self.sender.clone();
            let ids = (
                agent_id.to_string(),
                child_session_id.to_string(),
                child_assistant_id.to_string(),
                child_user_message_id.to_string(),
            );
            move |summary: TurnSummary| {
                let (agent_id, child_session_id, child_assistant_id, child_user_message_id) =
                    ids.clone();
                let _ = sender.send(ExecLoopEvent::Subagent(ExecSubagentEvent::Finished {
                    agent_id,
                    child_session_id,
                    child_assistant_id,
                    child_user_message_id,
                    summary,
                }));
            }
        };
        let join_handle = tokio::spawn(async move {
            let summary = run_agent_turn(context, host, executor, stream, cancel_rx).await;
            finished(summary);
        });
        self.running.insert(
            agent_id.to_string(),
            ChildHandle {
                agent_id: agent_id.to_string(),
                parent_session_id: parent_session_id.to_string(),
                child_session_id: child_session_id.to_string(),
                child_assistant_id: child_assistant_id.to_string(),
                child_user_message_id: child_user_message_id.to_string(),
                agent_path: agent_path.to_string(),
                role,
                title: title.to_string(),
                started_at_ms,
                parts: Vec::new(),
                next_position: 0,
                open_part_index: None,
                cancel_tx,
                done_tx,
                processes,
                join_handle: Some(join_handle),
                uncheckpointed_bytes: 0,
                closed_without_report: false,
            },
        );
        Ok(())
    }

    pub(super) fn handle_delta(
        &mut self,
        db: &Database,
        agent_id: &str,
        child_session_id: &str,
        child_assistant_id: &str,
        deltas: &[ChatDelta],
    ) {
        let Some(handle) = self.running.get_mut(agent_id) else {
            return;
        };
        if handle.child_session_id != child_session_id
            || handle.child_assistant_id != child_assistant_id
        {
            return;
        }
        append_deltas_to_parts(
            &mut handle.parts,
            &mut handle.next_position,
            &mut handle.open_part_index,
            deltas,
            |_, _| {},
        );
        handle.uncheckpointed_bytes = handle
            .uncheckpointed_bytes
            .saturating_add(deltas.iter().map(ChatDelta::payload_bytes).sum::<usize>());
        if handle.uncheckpointed_bytes < CHECKPOINT_BYTES {
            return;
        }
        handle.uncheckpointed_bytes = 0;
        let _ = db.checkpoint(&self.workspace, child_assistant_id, &handle.parts);
    }

    pub(super) fn handle_host(
        &mut self,
        db: &Database,
        agent_id: &str,
        child_session_id: &str,
        child_assistant_id: &str,
        request: AgentHostRequest,
    ) {
        match request {
            AgentHostRequest::BeginTool { call, reply } => {
                let Some(handle) = self.running.get_mut(agent_id) else {
                    let _ = reply.send(Err(HostError::Cancelled));
                    return;
                };
                if handle.child_session_id != child_session_id
                    || handle.child_assistant_id != child_assistant_id
                {
                    let _ = reply.send(Err(HostError::Cancelled));
                    return;
                }
                close_open_part(&mut handle.parts, &mut handle.open_part_index);
                let result = db
                    .checkpoint(&self.workspace, &handle.child_assistant_id, &handle.parts)
                    .map_err(|error| tool_failed("tool call pending record failed", error.code()))
                    .and_then(|()| {
                        db.begin_tool_call(
                            &self.workspace,
                            child_session_id,
                            child_assistant_id,
                            &call.call_id,
                            &call.name,
                            &call.arguments_json,
                        )
                        .map_err(|error| {
                            tool_failed("tool call pending record failed", error.code())
                        })
                    })
                    .and_then(|ticket| {
                        handle.next_position =
                            handle.next_position.max(ticket.position.saturating_add(1));
                        db.mark_tool_running(&self.workspace, &ticket)
                            .map_err(|error| {
                                tool_failed("tool call running record failed", error.code())
                            })
                            .map(|()| ticket)
                    });
                let _ = reply.send(result);
            }
            AgentHostRequest::FinishTool {
                ticket,
                outcome,
                reply,
            } => {
                let result = db_result_of(&outcome);
                let sent = match db.finish_tool_call(&self.workspace, &ticket, &result) {
                    Ok(()) => Ok(()),
                    Err(error) => Err(tool_failed("tool result persistence failed", error.code())),
                };
                let _ = reply.send(sent);
            }
            AgentHostRequest::AskUser { reply, .. } => {
                let _ = reply.send(Err(failed("unsupported in subagent")));
            }
            AgentHostRequest::Subagent { call_id, op, reply } => {
                let Some(cancel_rx) = self
                    .running
                    .get(agent_id)
                    .filter(|h| {
                        h.child_session_id == child_session_id
                            && h.child_assistant_id == child_assistant_id
                    })
                    .map(|h| h.cancel_tx.subscribe())
                else {
                    let _ = reply.send(Err(HostError::Cancelled));
                    return;
                };
                let saved_parent =
                    std::mem::replace(&mut self.parent_session_id, child_session_id.to_string());
                self.handle_op(
                    db,
                    child_assistant_id,
                    call_id.as_deref(),
                    op,
                    &cancel_rx,
                    reply,
                );
                self.parent_session_id = saved_parent;
            }
            AgentHostRequest::UserShellUpdates { reply } => {
                let _ = reply.send(Vec::new());
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn spawn(
        &mut self,
        db: &Database,
        parent_assistant_id: &str,
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
    ) -> Result<SubagentOutcome, HostError> {
        let role_spec = self
            .subagents_config
            .resolve_role(role.as_str())
            .ok_or_else(|| failed(&format!("unknown subagent role '{}'", role.as_str())))?;
        let role = role.with_base(role_spec.base_role);
        let (root_session, caller_depth, caller_path) = db
            .resolve_session_tree_root(&self.workspace, &self.parent_session_id)
            .unwrap_or_else(|_| {
                (
                    self.parent_session_id.clone(),
                    0,
                    crate::chat::agent::ROOT_AGENT_PATH.to_string(),
                )
            });
        let max_depth = self
            .subagents_config
            .max_depth
            .clamp(1, crate::config::subagents::MAX_AGENT_DEPTH_LIMIT);
        let next_depth = caller_depth.saturating_add(1);
        if next_depth > max_depth {
            return Err(failed(DEPTH_LIMIT_REACHED_CODE));
        }
        let max_concurrent = self
            .subagents_config
            .max_concurrent
            .clamp(1, crate::config::subagents::MAX_CONCURRENT_SUBAGENTS_LIMIT);
        if self.running.len() >= max_concurrent {
            return Err(failed(LIMIT_REACHED_CODE));
        }
        let locked = role_spec.lock_model && role_spec.default_model.is_some();
        let (effective_model_override, effective_effort_override) = if locked {
            (None, None)
        } else {
            (model_override, reasoning_effort_override)
        };
        let child_target = self.resolve_child_model(
            role_spec.base_role,
            role_spec.default_model.as_ref(),
            role_spec.lock_model,
            effective_model_override,
            effective_effort_override,
        )?;
        let tree_records = db
            .list_subagent_tree(&self.workspace, &root_session)
            .or_else(|_| db.list_subagents(&self.workspace, &self.parent_session_id))
            .unwrap_or_default();
        let used_nicknames: Vec<String> = tree_records
            .iter()
            .filter_map(|record| record.nickname.clone())
            .collect();
        let nickname =
            crate::chat::agent::allocate_nickname(&used_nicknames, &role_spec.nickname_candidates);
        let existing_paths: Vec<String> = tree_records
            .iter()
            .map(|record| record.agent_path.clone())
            .collect();
        let agent_path = allocate_child_path(
            &caller_path,
            task_name.unwrap_or(&nickname),
            &existing_paths,
        );
        let fork_spec = ForkSpec::from_params(fork_context, fork_turns);
        let effective_max_tokens = max_tokens.or(self.subagents_config.default_max_tokens);
        let child_session_id = new_external_id("sess");
        let image_rewrite = if fork_spec.is_enabled() {
            crate::db::chat::copy_session_images(&self.parent_session_id, &child_session_id)
        } else {
            None
        };
        let source: Vec<SubagentInputItem> = if items.is_empty() {
            vec![SubagentInputItem::Text {
                text: task.to_string(),
            }]
        } else {
            items.to_vec()
        };
        let persisted = match self.prepare_input_items(
            &child_session_id,
            &source,
            child_target.supports_image_input,
        ) {
            Ok(persisted) => persisted,
            Err(error) => {
                crate::platform::remove_session_images(&child_session_id);
                return Err(error);
            }
        };
        let task_text = if task.trim().is_empty() {
            joined_input_text(&source)
        } else {
            task.to_string()
        };
        if task_text.trim().is_empty() {
            crate::platform::remove_session_images(&child_session_id);
            return Err(failed("subagent task must not be empty"));
        }
        let title = if title.trim().is_empty() {
            first_line_title(&task_text)
        } else {
            title.to_string()
        };
        let dedupe = call_id.map(|cid| {
            format!(
                "spawn:{}:{}:{cid}",
                self.parent_session_id, parent_assistant_id
            )
        });
        let committed = match db.begin_subagent_turn_with_full_options(
            &self.workspace,
            &self.parent_session_id,
            Some(&child_session_id),
            Some(&nickname),
            role,
            &title,
            &task_text,
            &persisted,
            &child_target.snapshot(),
            Some(parent_assistant_id),
            dedupe.as_deref(),
            call_id,
            fork_spec.is_enabled(),
            Some(&agent_path),
            fork_spec.fork_turns(),
            &image_rewrite,
        ) {
            Ok(committed) => committed,
            Err(error) => {
                crate::platform::remove_session_images(&child_session_id);
                return Err(db_failed(error));
            }
        };
        self.child_models
            .insert(committed.child_session_id.clone(), child_target.clone());
        self.resumed.insert(committed.agent_id.clone());
        self.agent_budgets
            .insert(committed.agent_id.clone(), effective_max_tokens);
        let parent_session_id = self.parent_session_id.clone();
        let started = self.start_child(
            db,
            &committed.agent_id,
            &parent_session_id,
            &committed.child_session_id,
            &committed.child_assistant_id,
            &committed.child_user_message_id,
            &agent_path,
            next_depth,
            max_depth,
            effective_max_tokens,
            role,
            &title,
            committed.created_at_ms,
            &persisted,
            watch::channel(false).0,
        );
        if let Err(error) = started {
            self.resumed.remove(&committed.agent_id);
            return Err(error);
        }
        let model_label = if effective_model_override.is_some()
            || child_target.provider != self.model.provider
            || child_target.model != self.model.model
        {
            Some(format!("{}/{}", child_target.provider, child_target.model))
        } else {
            None
        };
        let effort_label =
            if effective_effort_override.is_some() || child_target.variant != self.model.variant {
                child_target.variant.clone()
            } else {
                None
            };
        Ok(SubagentOutcome::Spawned {
            agent_id: committed.agent_id,
            nickname: Some(nickname),
            agent_path: Some(agent_path),
            role,
            title,
            model: model_label,
            reasoning_effort: effort_label,
            fork_context: fork_spec.is_enabled(),
            fork_turns: fork_spec.fork_turns(),
            max_tokens: effective_max_tokens,
        })
    }

    fn resolve_child_model(
        &self,
        base_role: crate::config::subagents::BaseRole,
        role_default: Option<&crate::config::ai::ModelSelection>,
        lock_model: bool,
        model_override: Option<&str>,
        effort_override: Option<&str>,
    ) -> Result<ExecModelTarget, HostError> {
        let locked = lock_model && role_default.is_some();
        let explicit_model = if locked {
            None
        } else {
            model_override.map(str::trim).filter(|s| !s.is_empty())
        };
        let explicit_effort = if locked {
            None
        } else {
            effort_override.map(str::trim).filter(|s| !s.is_empty())
        };
        // 父 fallback 取调用者自身冻结的模型：嵌套 executor 的子 agent 必须继承它，而不是池根模型。
        let parent_model = self
            .child_models
            .get(&self.parent_session_id)
            .cloned()
            .unwrap_or_else(|| self.model.clone());
        let settings = crate::config::ai::ai_config_path()
            .ok()
            .and_then(|path| {
                crate::config::ai::connect::load_runtime_ai(&path)
                    .ok()
                    .flatten()
            })
            .unwrap_or_else(crate::config::ai::AiSettings::empty);
        let monitor_default = base_role == crate::config::subagents::BaseRole::Monitor
            && explicit_effort.is_none()
            && role_default
                .and_then(|sel| sel.variant.as_deref())
                .is_none();
        if explicit_model.is_none() && explicit_effort.is_none() && role_default.is_none() {
            if monitor_default && supports_low_variant(&settings, &parent_model) {
                return resolve_with_low(&settings, &parent_model);
            }
            return Ok(parent_model);
        }
        if settings.providers.is_empty() {
            // 只有单元测试环境（无 ai.json）才允许用父模型做兜底；真实运行中配置缺失
            // 必须显式失败，不能把 `provider/model` 静默塞进 model 字段。
            if !cfg!(test) {
                return Err(failed("model configuration unavailable"));
            }
            let mut fallback = parent_model;
            if let Some(m) = explicit_model {
                apply_reference_to_target(&mut fallback, m);
            } else if let Some(sel) = role_default {
                fallback.provider = sel.provider.clone();
                fallback.model = sel.model.clone();
                if sel.variant.is_some() {
                    fallback.variant = sel.variant.clone();
                }
            }
            if let Some(e) = explicit_effort {
                fallback.variant = Some(e.to_string());
            }
            return Ok(fallback);
        }
        let raw_spec = if let Some(m) = explicit_model {
            match explicit_effort {
                Some(e) => format!("{m}#{e}"),
                None => m.to_string(),
            }
        } else if let Some(sel) = role_default {
            let eff = explicit_effort.or(sel.variant.as_deref());
            match eff {
                Some(e) => format!("{}/{}#{e}", sel.provider, sel.model),
                None => format!("{}/{}", sel.provider, sel.model),
            }
        } else {
            let e = explicit_effort.unwrap_or_default();
            format!("{}/{}#{e}", parent_model.provider, parent_model.model)
        };
        let target =
            crate::chat::agent::exec::resolve_model_target(&settings, Some(&raw_spec), None)
                .map_err(|err| failed(&err))?;
        if monitor_default && supports_low_variant(&settings, &target) {
            return resolve_with_low(&settings, &target);
        }
        Ok(target)
    }

    /// 子 agent 的模型目标：内存缓存 > 数据库持久化快照（重启后恢复）> 父模型。
    pub(super) fn child_model_for(&mut self, record: &SubagentRecord) -> &ExecModelTarget {
        let differs = !record.model_provider.trim().is_empty()
            && !record.model_id.trim().is_empty()
            && (record.model_provider != self.model.provider
                || record.model_id != self.model.model
                || record.model_variant != self.model.variant);
        if differs && !self.child_models.contains_key(&record.child_session_id) {
            let settings = crate::config::ai::ai_config_path()
                .ok()
                .and_then(|path| {
                    crate::config::ai::connect::load_runtime_ai(&path)
                        .ok()
                        .flatten()
                })
                .unwrap_or_else(crate::config::ai::AiSettings::empty);
            let reference = match record
                .model_variant
                .as_deref()
                .map(str::trim)
                .filter(|variant| !variant.is_empty())
            {
                Some(variant) => format!("{}/{}#{variant}", record.model_provider, record.model_id),
                None => format!("{}/{}", record.model_provider, record.model_id),
            };
            if let Ok(target) =
                crate::chat::agent::exec::resolve_model_target(&settings, Some(&reference), None)
            {
                self.child_models
                    .insert(record.child_session_id.clone(), target);
            }
        }
        self.child_models
            .get(&record.child_session_id)
            .unwrap_or(&self.model)
    }
}

fn resolve_image_path(cwd: &Path, raw: &str) -> Result<PathBuf, HostError> {
    let path = Path::new(raw.trim());
    if path.as_os_str().is_empty() {
        return Err(failed("image path must not be empty"));
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if cwd.as_os_str().is_empty() || !cwd.is_dir() {
        return Err(failed("workspace cwd unavailable for relative image path"));
    }
    Ok(cwd.join(path))
}

fn ensure_image(count: usize, total: usize) -> Result<(), HostError> {
    if count > crate::platform::MAX_ATTACHED_IMAGES {
        return Err(failed(&format!(
            "too many images: at most {} images per input",
            crate::platform::MAX_ATTACHED_IMAGES
        )));
    }
    if total > crate::platform::MAX_TOTAL_IMAGE_RAW_BYTES {
        return Err(failed("total image bytes exceed the 40 MiB limit"));
    }
    Ok(())
}

fn archive_image(
    child_session_id: &str,
    processed: &crate::platform::ProcessedImage,
) -> Result<PersistedInputItem, HostError> {
    let archived = crate::platform::archive_processed_image(child_session_id, processed)
        .map_err(|error| failed(&format!("image archive failed: {}", error.code())))?;
    Ok(PersistedInputItem::Image {
        label: archived.label,
        archive_path: archived.archive_path.to_string_lossy().into_owned(),
        mime: archived.mime,
        content_hash: archived.content_hash,
        width: archived.width,
        height: archived.height,
    })
}

/// 目标模型是否声明 `low` 思考档位。
fn supports_low_variant(
    settings: &crate::config::ai::AiSettings,
    target: &ExecModelTarget,
) -> bool {
    settings
        .providers
        .get(&target.provider)
        .and_then(|provider| provider.models.get(&target.model))
        .is_some_and(|model| model.variants.contains_key("low"))
        && target.variant.as_deref() != Some("low")
}

/// 以 `low` 档位重新解析目标（同时刷新 `variant_params`）。
fn resolve_with_low(
    settings: &crate::config::ai::AiSettings,
    target: &ExecModelTarget,
) -> Result<ExecModelTarget, HostError> {
    let raw = format!("{}/{}#low", target.provider, target.model);
    crate::chat::agent::exec::resolve_model_target(settings, Some(&raw), None)
        .map_err(|err| failed(&err))
}

/// 测试兜底模型解析：`<provider>/<model>` 拆分写入 provider/model，纯模型 ID 只改 model。
fn apply_reference_to_target(target: &mut ExecModelTarget, reference: &str) {
    match reference.split_once('/') {
        Some((provider, model)) if !provider.trim().is_empty() && !model.trim().is_empty() => {
            target.provider = provider.trim().to_string();
            target.model = model.trim().to_string();
        }
        _ => target.model = reference.trim().to_string(),
    }
}

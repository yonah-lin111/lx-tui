//! 子轮上下文装配、输入归档与子轮宿主事件落库。
//!
//! 由 [`super::ExecSubagentPool`] 的池控制入口调用：所有方法直接读写 Database，
//! 不接触事件层状态。等待与终态结算见 [`super::settle`]。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, watch};

use crate::chat::agent::{AgentRole, IMAGE_MISSING_CODE, ToolSurface};
use crate::chat::agent_loop::{AgentTools, ModelContext, TurnExecution};
use crate::chat::events::{
    ChatDelta, ChatRequest, ChatTurn, ModelImage, ModelItem, RequestPurpose,
};
use crate::chat::process::{ProcessManager, ProcessOwner};
use crate::chat::prompt::PromptParts;
use crate::db::chat::PersistedInputItem;
use crate::db::{Database, new_external_id};

use super::super::runner::{append_deltas_to_parts, db_result_of, new_mutation_recorder_factory};
use super::*;

impl ExecSubagentPool {
    /// 子轮请求：历史按已完成轮次回放（工具步骤与历史图片保留），末尾接当前输入。
    fn child_context(
        &self,
        db: &Database,
        agent_id: &str,
        child_session_id: &str,
        role: AgentRole,
        current_items: &[PersistedInputItem],
    ) -> Result<ModelContext, String> {
        let child_model = self
            .child_models
            .get(child_session_id)
            .unwrap_or(&self.model);
        let supports_image_input = child_model.supports_image_input;
        let (mut turns, mut items) = (Vec::new(), Vec::new());
        let checkpoint = db
            .active_checkpoint(&self.workspace, child_session_id)
            .map_err(|error| error.code().to_string())?;
        let start_after = checkpoint.as_ref().map_or(i64::MIN, |cp| cp.end_sequence);
        let messages: Vec<_> = db
            .load_chat_session(&self.workspace, child_session_id)
            .map_err(|error| error.code().to_string())?
            .unwrap_or_default()
            .into_iter()
            .filter(|message| message.sequence > start_after)
            .collect();
        let turn_items = db
            .load_subagent_turn_items(&self.workspace, &self.parent_session_id, agent_id)
            .unwrap_or_default();
        let mut index = 0;
        while index < messages.len() {
            let turn_index = messages[index].turn_index;
            let (mut user, mut assistant) = (None, None);
            while index < messages.len() && messages[index].turn_index == turn_index {
                let message = &messages[index];
                match message.role.as_str() {
                    "user" if user.is_none() => user = Some(message),
                    "assistant" if assistant.is_none() => assistant = Some(message),
                    _ => {}
                }
                index += 1;
            }
            let (Some(user), Some(assistant)) = (user, assistant) else {
                continue;
            };
            if user.status != "complete" || assistant.status != "complete" {
                continue;
            }
            let user_text = message_text(user);
            turns.push(ChatTurn {
                role: "user".into(),
                text: user_text.clone(),
                reasoning_metadata: None,
            });
            turns.push(ChatTurn {
                role: "assistant".into(),
                text: message_text(assistant),
                reasoning_metadata: None,
            });
            match turn_items.get(&user.external_id) {
                Some(stored) if has_images(stored) => {
                    items.extend(replay_input_items(
                        child_session_id,
                        supports_image_input,
                        false,
                        stored,
                    )?);
                }
                None if supports_image_input => {
                    // fork 克隆的历史轮次没有子输入项记录：回放消息附件里仍在磁盘上的图片。
                    let attachments: Vec<&crate::db::chat::MessageAttachmentMeta> = user
                        .attachments
                        .iter()
                        .filter(|att| {
                            att.kind == crate::db::chat::MessageAttachmentKind::Image
                                && att
                                    .source_path
                                    .as_deref()
                                    .is_some_and(|path| Path::new(path).is_file())
                        })
                        .collect();
                    if attachments.is_empty() {
                        items.push(ModelItem::UserText(user_text));
                    } else {
                        items.extend(replay_message_attachments(
                            db,
                            &self.workspace,
                            child_session_id,
                            user_text,
                            &attachments,
                        ));
                    }
                }
                _ => items.push(ModelItem::UserText(user_text)),
            }
            items.extend(crate::chat::context::assistant_items(assistant));
        }
        let current_text = PersistedInputItem::joined_text(current_items);
        turns.push(ChatTurn {
            role: "user".into(),
            text: current_text,
            reasoning_metadata: None,
        });
        items.extend(replay_input_items(
            child_session_id,
            supports_image_input,
            true,
            current_items,
        )?);
        let identity = crate::chat::agent::ModelIdentity {
            display_name: child_model.display_name.clone(),
            provider_id: child_model.provider.clone(),
            model_id: child_model.model.clone(),
            variant: child_model.variant.clone(),
            supports_image_input: child_model.supports_image_input,
        }
        .render_prompt_section();
        let environment = crate::chat::prompt::environment_snapshot(
            &crate::platform::EnvironmentFacts::capture(),
            Some(&self.workspace),
            Some(&self.cwd),
            &[],
        );
        let instructions = crate::chat::agent::load_instructions(
            crate::config::user_data_root().ok().as_deref(),
            Some(&self.workspace),
            Some(&self.cwd),
            true,
        )
        .render_prompt_section();
        let skills = super::super::skills_catalog_prompt(&self.workspace, true);
        let mcp_prompt = self.mcp_snapshot.instructions_prompt();
        let custom_prompt = self
            .subagents_config
            .resolve_role(role.as_str())
            .and_then(|r| r.system_prompt_override);
        let system_prompt = crate::chat::agent::compose_subagent_prompt_with_override(
            role,
            custom_prompt.as_deref(),
            PromptParts {
                model_identity: Some(&identity),
                environment: Some(&environment),
                instructions: instructions.as_deref(),
                skills: skills.as_deref(),
                mcp: mcp_prompt.as_deref(),
                compact: None,
            },
        );
        let request = ChatRequest {
            turns,
            items,
            model: child_model.snapshot(),
            variant_params: child_model.variant_params.clone(),
            purpose: RequestPurpose::Subagent(role),
            system_prompt: Some(system_prompt),
            tools: Arc::new(crate::chat::tools::turn_tools(
                ToolSurface::from_role(role),
                &self.mcp_snapshot,
            )),
        };
        let context = ModelContext::new(request, self.cwd.clone())
            .with_context_limit(child_model.context_limit);
        Ok(match checkpoint {
            Some(checkpoint) => context.with_compact_summary(Some(checkpoint.summary_text)),
            None => context,
        })
    }

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
        child_session_id: &str,
        child_assistant_id: &str,
        child_user_message_id: &str,
        role: AgentRole,
        title: &str,
        started_at_ms: i64,
        current_items: &[PersistedInputItem],
        done_tx: watch::Sender<bool>,
    ) -> Result<(), HostError> {
        let context = self
            .child_context(db, agent_id, child_session_id, role, current_items)
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
                child_session_id: child_session_id.to_string(),
                child_assistant_id: child_assistant_id.to_string(),
                child_user_message_id: child_user_message_id.to_string(),
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
            AgentHostRequest::Subagent { reply, .. } => {
                let _ = reply.send(Err(failed("unsupported in subagent")));
            }
            AgentHostRequest::UserShellUpdates { reply } => {
                let _ = reply.send(Vec::new());
            }
        }
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

/// 持久化输入 → 模型项：当前输入加载失败即拒绝，历史轮次降级为可读占位文本。
fn replay_input_items(
    child_session_id: &str,
    supports_image_input: bool,
    is_current_input: bool,
    items: &[PersistedInputItem],
) -> Result<Vec<ModelItem>, String> {
    if has_images(items) && !supports_image_input {
        if is_current_input {
            return Err("current model does not support image input".to_string());
        }
        let mut lines = Vec::new();
        for item in items {
            match item {
                PersistedInputItem::Text { text } => {
                    if !text.is_empty() {
                        lines.push(text.clone());
                    }
                }
                PersistedInputItem::Image { label, .. } => lines.push(format!(
                    "ERROR: Cannot read \"{label}\" (this model does not support image input). Inform the user."
                )),
            }
        }
        return Ok(vec![ModelItem::UserText(lines.join("\n"))]);
    }
    if !has_images(items) {
        return Ok(vec![ModelItem::UserText(PersistedInputItem::joined_text(
            items,
        ))]);
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            PersistedInputItem::Text { text } => out.push(ModelItem::UserContent {
                text: text.clone(),
                images: Vec::new(),
            }),
            PersistedInputItem::Image {
                archive_path,
                mime,
                content_hash,
                width,
                height,
                ..
            } => {
                let bytes = crate::platform::verify_archived_image(
                    child_session_id,
                    Path::new(archive_path),
                    mime,
                    content_hash,
                    *width,
                    *height,
                )
                .map_err(|_| IMAGE_MISSING_CODE.to_string())?;
                out.push(ModelItem::UserContent {
                    text: String::new(),
                    images: vec![ModelImage {
                        mime: mime.clone(),
                        bytes: Arc::new(bytes),
                    }],
                });
            }
        }
    }
    Ok(out)
}

fn has_images(items: &[PersistedInputItem]) -> bool {
    items
        .iter()
        .any(|item| matches!(item, PersistedInputItem::Image { .. }))
}

/// 回放 fork 克隆历史轮的图片附件（这些消息没有 `agent_subagent_input` 记录）。
fn replay_message_attachments(
    db: &Database,
    workspace: &Path,
    child_session_id: &str,
    text: String,
    attachments: &[&crate::db::chat::MessageAttachmentMeta],
) -> Vec<ModelItem> {
    let ids: Vec<&str> = attachments.iter().map(|att| att.id.as_str()).collect();
    let payloads = db
        .load_image_payloads(workspace, child_session_id, &ids)
        .unwrap_or_default();
    let mut images = Vec::new();
    for att in attachments {
        if let Some(bytes) = payloads.get(&att.id) {
            images.push(ModelImage {
                mime: att.mime.clone().unwrap_or_else(|| "image/png".to_string()),
                bytes: Arc::new(bytes.clone()),
            });
        }
    }
    if images.is_empty() {
        return vec![ModelItem::UserText(text)];
    }
    vec![ModelItem::UserContent { text, images }]
}

fn message_text(message: &crate::db::chat::ChatMessage) -> String {
    message
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| part.content.as_str())
        .collect::<Vec<_>>()
        .join("")
}

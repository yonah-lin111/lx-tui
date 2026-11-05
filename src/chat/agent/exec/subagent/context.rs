//! 无头子轮上下文装配：已完成/被中断轮次回放、`<turn_aborted>` 标记、
//! `RoleCapabilities` 能力裁剪与后续 `send_input` 的结构化信封。
//!
//! 由 [`super::ExecSubagentPool`] 的子轮启动路径调用；所有方法直接读写 Database，
//! 不接触事件层状态。

use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::chat::agent::ToolSurface;
use crate::chat::agent_loop::ModelContext;
use crate::chat::events::{ChatRequest, ChatTurn, ModelItem, RequestPurpose};
use crate::chat::prompt::PromptParts;
use crate::db::Database;
use crate::db::chat::PersistedInputItem;

impl ExecSubagentPool {
    /// 子轮请求：历史按已完成/被中断轮次回放（工具步骤与历史图片保留），末尾接当前输入。
    // 子轮上下文需要工作区库、标识、角色、当前输入项与深度/预算护栏参数；保持显式传参比额外包装结构更直观。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn child_context(
        &self,
        db: &Database,
        agent_id: &str,
        parent_session_id: &str,
        child_session_id: &str,
        role: AgentRole,
        current_items: &[PersistedInputItem],
        depth: usize,
        max_depth: usize,
        max_tokens: Option<u64>,
    ) -> Result<ModelContext, String> {
        let child_model = self
            .child_models
            .get(child_session_id)
            .unwrap_or(&self.model);
        let supports_image_input = child_model.supports_image_input;
        let role_spec = self.subagents_config.resolve_role(role.as_str());
        let capabilities = role_spec
            .as_ref()
            .map(|spec| spec.capabilities)
            .unwrap_or_default();
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
            .load_subagent_turn_items(&self.workspace, parent_session_id, agent_id)
            .unwrap_or_default();
        // 非克隆子轮来自 `agent_subagent_input`（当前轮已 claim 并计入）；> 1 即第 2 轮及
        // 之后的 `send_input` 跟进（fork 克隆的父历史没有输入记录，不会误判）。
        let is_follow_up = turn_items.len() > 1;
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
            let aborted = matches!(assistant.status.as_str(), "interrupted" | "cancelled");
            if user.status != "complete" || !(assistant.status == "complete" || aborted) {
                continue;
            }
            let user_text = message_text(user);
            turns.push(ChatTurn {
                role: "user".into(),
                text: user_text.clone(),
                reasoning_metadata: None,
            });
            let mut assistant_text = message_text(assistant);
            if aborted {
                assistant_text.push('\n');
                assistant_text.push_str(crate::chat::agent::SUBAGENT_TURN_ABORTED_MARKER);
            }
            turns.push(ChatTurn {
                role: "assistant".into(),
                text: assistant_text,
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
            let mut assistant_items = crate::chat::context::assistant_items(assistant);
            if aborted {
                assistant_items.push(ModelItem::UserText(
                    crate::chat::agent::SUBAGENT_TURN_ABORTED_MARKER.to_string(),
                ));
            }
            items.extend(assistant_items);
        }
        let raw_current_text = PersistedInputItem::joined_text(current_items);
        let (child_path, sender_path) =
            envelope_paths(db, &self.workspace, parent_session_id, agent_id);
        let current_text = if is_follow_up {
            crate::chat::agent::format_inter_agent_envelope(
                crate::chat::agent::InterAgentMessageType::NewTask,
                &child_path,
                &sender_path,
                &raw_current_text,
            )
        } else {
            raw_current_text
        };
        turns.push(ChatTurn {
            role: "user".into(),
            text: current_text,
            reasoning_metadata: None,
        });
        let mut current_model_items =
            replay_input_items(child_session_id, supports_image_input, true, current_items)?;
        if is_follow_up {
            current_model_items =
                wrap_envelope_items(current_model_items, &child_path, &sender_path);
        }
        items.extend(current_model_items);
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
        let skills = if capabilities.allow_skills {
            super::super::skills_catalog_prompt(&self.workspace, true)
        } else {
            None
        };
        let empty_mcp = crate::mcp::McpToolSnapshot::default();
        let mcp = if capabilities.allow_mcp {
            self.mcp_snapshot.as_ref()
        } else {
            &empty_mcp
        };
        let mcp_prompt = mcp.instructions_prompt();
        let custom_prompt = role_spec.and_then(|spec| spec.system_prompt_override);
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
            tools: Arc::new(crate::chat::tools::turn_tools_for_subagent(
                ToolSurface::from_role_at_depth(role, depth, max_depth),
                capabilities,
                mcp,
                Some(&self.subagents_config),
            )),
        };
        let context = ModelContext::new(request, self.cwd.clone())
            .with_context_limit(child_model.context_limit)
            .with_subagent_depth(depth, max_depth)
            .with_token_budget(max_tokens);
        Ok(match checkpoint {
            Some(checkpoint) => context.with_compact_summary(Some(checkpoint.summary_text)),
            None => context,
        })
    }
}

/// 子 agent 与发送者的规范路径（用于后续 `send_input` 轮次的结构化信封）。
fn envelope_paths(
    db: &Database,
    workspace: &Path,
    parent_session_id: &str,
    agent_id: &str,
) -> (String, String) {
    let child_path = db
        .owned_subagent(workspace, parent_session_id, agent_id)
        .ok()
        .flatten()
        .map(|record| record.agent_path)
        .filter(|path| !path.trim().is_empty())
        .unwrap_or_else(|| "/root/subagent".to_string());
    let sender_path = db
        .resolve_session_tree_root(workspace, parent_session_id)
        .map(|(_, _, path)| path)
        .unwrap_or_else(|_| crate::chat::agent::ROOT_AGENT_PATH.to_string());
    (child_path, sender_path)
}

/// 只在模型请求侧包裹后续 `send_input` 的用户文本（DB 与 TUI 保持原文）。
fn wrap_envelope_items(
    items: Vec<ModelItem>,
    child_path: &str,
    sender_path: &str,
) -> Vec<ModelItem> {
    let mut wrapped = false;
    items
        .into_iter()
        .map(|item| match item {
            ModelItem::UserText(text) if !wrapped => {
                wrapped = true;
                ModelItem::UserText(crate::chat::agent::format_inter_agent_envelope(
                    crate::chat::agent::InterAgentMessageType::NewTask,
                    child_path,
                    sender_path,
                    &text,
                ))
            }
            ModelItem::UserContent { text, images } if !wrapped && !text.trim().is_empty() => {
                wrapped = true;
                ModelItem::UserContent {
                    text: crate::chat::agent::format_inter_agent_envelope(
                        crate::chat::agent::InterAgentMessageType::NewTask,
                        child_path,
                        sender_path,
                        &text,
                    ),
                    images,
                }
            }
            other => other,
        })
        .collect()
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
                .map_err(|_| crate::chat::agent::IMAGE_MISSING_CODE.to_string())?;
                out.push(ModelItem::UserContent {
                    text: String::new(),
                    images: vec![crate::chat::events::ModelImage {
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
            images.push(crate::chat::events::ModelImage {
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

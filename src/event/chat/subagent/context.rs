//! 子 agent 输入项的图片归档、回放校验与多轮上下文装配。
//!
//! 严格遵循 `design.md` §5.4 与 §6：
//! - 接收输入前校验模型图像能力、尺寸/像素/总量上限，并将图片原子归档到
//!   `~/.lx-t[-dev]/images/<child_session_id>/`；
//! - 历史与当前输入的文本和图片保持原始交错顺序（`ModelItem::UserContent` 序列）；
//! - 跨轮或重启恢复时重新校验归档图片的存在性、可读性、MIME、尺寸与摘要；
//!   缺失或损坏即返回 `image_missing`，绝不静默降级为纯文本，也不重读外部原路径；
//! - 复用完整轮次预算裁剪规则与持久化自动压缩检查点（`system -> environment -> summary -> 后续轮次`）。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::chat::agent::{AgentRole, IMAGE_MISSING_CODE, SubagentInputItem};
use crate::chat::agent_loop::ModelContext;
use crate::chat::events::{ChatRequest, ChatTurn, ModelImage, ModelItem, RequestPurpose};
use crate::db::Database;
use crate::db::chat::{ChatMessage, ModelSnapshot, PersistedInputItem};
use crate::platform::{
    MAX_ATTACHED_IMAGES, MAX_TOTAL_IMAGE_RAW_BYTES, archive_processed_image,
    load_and_process_image_file, process_data_url_image, verify_archived_image,
};

/// 预校验并归档一次 `spawn_agent` / `send_input` 的有序结构化 `items`。
///
/// 成功返回可落库的 `Vec<PersistedInputItem>` 与立即可用于请求的 `Vec<ModelItem>`。
pub(super) fn prepare_input_items(
    child_session_id: &str,
    cwd: &Path,
    supports_image_input: bool,
    items: &[SubagentInputItem],
) -> Result<(Vec<PersistedInputItem>, Vec<ModelItem>), String> {
    if items.is_empty() {
        return Err("subagent input items must not be empty".to_string());
    }
    let has_images = items.iter().any(|item| {
        matches!(
            item,
            SubagentInputItem::LocalImage { .. } | SubagentInputItem::Image { .. }
        )
    });
    if has_images && !supports_image_input {
        return Err("current model does not support image input".to_string());
    }

    let mut persisted = Vec::with_capacity(items.len());
    let mut model_items = Vec::with_capacity(items.len());
    let mut image_count = 0usize;
    let mut total_raw_bytes = 0usize;

    for item in items {
        match item {
            SubagentInputItem::Text { text } => {
                persisted.push(PersistedInputItem::Text { text: text.clone() });
                if has_images {
                    model_items.push(ModelItem::UserContent {
                        text: text.clone(),
                        images: Vec::new(),
                    });
                } else {
                    model_items.push(ModelItem::UserText(text.clone()));
                }
            }
            SubagentInputItem::LocalImage { path } => {
                image_count = image_count.saturating_add(1);
                if image_count > MAX_ATTACHED_IMAGES {
                    return Err(format!(
                        "too many images: at most {MAX_ATTACHED_IMAGES} images per input"
                    ));
                }
                let resolved = resolve_image_path(cwd, path)?;
                let processed = load_and_process_image_file(&resolved)
                    .map_err(|err| format!("invalid local_image: {}", err.code()))?;
                total_raw_bytes = total_raw_bytes.saturating_add(processed.original_byte_count);
                if total_raw_bytes > MAX_TOTAL_IMAGE_RAW_BYTES {
                    return Err("total image bytes exceed the 40 MiB limit".to_string());
                }
                let archived = archive_processed_image(child_session_id, &processed)
                    .map_err(|err| format!("image archive failed: {}", err.code()))?;
                persisted.push(PersistedInputItem::Image {
                    label: archived.label,
                    archive_path: archived.archive_path.to_string_lossy().into_owned(),
                    mime: archived.mime.clone(),
                    content_hash: archived.content_hash,
                    width: archived.width,
                    height: archived.height,
                });
                model_items.push(ModelItem::UserContent {
                    text: String::new(),
                    images: vec![ModelImage {
                        mime: archived.mime,
                        bytes: Arc::new(archived.bytes),
                    }],
                });
            }
            SubagentInputItem::Image { image_url } => {
                image_count = image_count.saturating_add(1);
                if image_count > MAX_ATTACHED_IMAGES {
                    return Err(format!(
                        "too many images: at most {MAX_ATTACHED_IMAGES} images per input"
                    ));
                }
                let processed = process_data_url_image(image_url, image_count.saturating_sub(1))
                    .map_err(|err| format!("invalid inline image: {}", err.code()))?;
                total_raw_bytes = total_raw_bytes.saturating_add(processed.original_byte_count);
                if total_raw_bytes > MAX_TOTAL_IMAGE_RAW_BYTES {
                    return Err("total image bytes exceed the 40 MiB limit".to_string());
                }
                let archived = archive_processed_image(child_session_id, &processed)
                    .map_err(|err| format!("image archive failed: {}", err.code()))?;
                persisted.push(PersistedInputItem::Image {
                    label: archived.label,
                    archive_path: archived.archive_path.to_string_lossy().into_owned(),
                    mime: archived.mime.clone(),
                    content_hash: archived.content_hash,
                    width: archived.width,
                    height: archived.height,
                });
                model_items.push(ModelItem::UserContent {
                    text: String::new(),
                    images: vec![ModelImage {
                        mime: archived.mime,
                        bytes: Arc::new(archived.bytes),
                    }],
                });
            }
        }
    }
    if !has_images && model_items.len() > 1 {
        let joined = PersistedInputItem::joined_text(&persisted);
        model_items = vec![ModelItem::UserText(joined)];
    }
    Ok((persisted, model_items))
}

/// 相对路径以父轮冻结的工作区 cwd 解析；绝对路径原样使用。
fn resolve_image_path(cwd: &Path, raw: &str) -> Result<std::path::PathBuf, String> {
    let path = Path::new(raw.trim());
    if path.as_os_str().is_empty() {
        return Err("image path must not be empty".to_string());
    }
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if cwd.as_os_str().is_empty() || !cwd.is_dir() {
        return Err("workspace cwd unavailable for relative image path".to_string());
    }
    Ok(cwd.join(path))
}

/// 子会话系统提示词冻结输入段。
pub(crate) struct ChildPromptContext<'a> {
    pub system_prompt_override: Option<&'a str>,
    pub model_identity: Option<&'a str>,
    pub environment: &'a str,
    pub instructions: Option<&'a str>,
    pub skills: Option<&'a str>,
    pub mcp: &'a crate::mcp::McpToolSnapshot,
}

/// 子轮层级与预算：决定工具面（是否开放控制工具）与 Token 预算护栏。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ChildTurnLimits {
    /// 当前子 agent 所处深度（根为 0，直接子 agent 为 1）。
    pub depth: usize,
    /// 配置的最大递归深度。
    pub max_depth: usize,
    /// 单轮 Token 预算；None 表示不限。
    pub max_tokens: Option<u64>,
}

/// 采集子轮系统提示词所需的冻结输入段。
pub(crate) struct CapturedChildPromptInputs {
    pub system_prompt_override: Option<String>,
    pub model_identity: String,
    pub environment: String,
    pub instructions: Option<String>,
    pub skills: Option<String>,
    pub mcp: Arc<crate::mcp::McpToolSnapshot>,
}

pub(crate) fn capture_child_prompt_inputs(
    state: &crate::app::state::AppState,
    persistence: &crate::db::service::Persistence,
    workspace: &Path,
    cwd: &Path,
    frozen: &crate::app::state::FrozenModel,
    role: AgentRole,
) -> CapturedChildPromptInputs {
    let environment = crate::chat::prompt::environment_snapshot(
        &crate::platform::EnvironmentFacts::capture(),
        Some(cwd),
        Some(cwd),
        &[],
    );
    let workspace_trusted = persistence.is_trusted(cwd) || persistence.is_trusted(workspace);
    let effective_subagents = crate::config::subagents::merge_project_roles_for_paths(
        &state.subagents_config,
        Some(workspace),
        Some(cwd),
        workspace_trusted,
    );
    let role_spec = effective_subagents.resolve_role(role.as_str());
    let capabilities = role_spec
        .as_ref()
        .map(|spec| spec.capabilities)
        .unwrap_or_default();
    let (skills, _) = if capabilities.allow_skills {
        super::super::skills::catalog_prompt(persistence, Some(workspace), true)
    } else {
        (None, 0)
    };
    let mcp = if capabilities.allow_mcp {
        super::super::mcp::snapshot_for_turn(persistence, Some(workspace))
    } else {
        Arc::new(crate::mcp::McpToolSnapshot::default())
    };
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
    let instructions = crate::chat::agent::load_instructions(
        crate::config::user_data_root().ok().as_deref(),
        Some(workspace),
        Some(cwd),
        persistence.is_trusted(cwd) || persistence.is_trusted(workspace),
    )
    .render_prompt_section();
    let system_prompt_override = role_spec.and_then(|spec| spec.system_prompt_override);
    CapturedChildPromptInputs {
        system_prompt_override,
        model_identity,
        environment,
        instructions,
        skills,
        mcp,
    }
}

/// 从持久化的 `PersistedInputItem` 重放有序 `ModelItem`（严格校验归档文件与摘要）。
///
/// - `is_current_input == true`：若含图片但当前模型不支持图片输入，返回错误拒绝本轮；
/// - `is_current_input == false`（历史轮次回放）：若当前模型不支持图片输入，降级为英文占位文本。
pub(super) fn replay_persisted_items(
    child_session_id: &str,
    supports_image_input: bool,
    is_current_input: bool,
    items: &[PersistedInputItem],
) -> Result<Vec<ModelItem>, String> {
    if items.is_empty() {
        return Err(IMAGE_MISSING_CODE.to_string());
    }
    let has_images = items
        .iter()
        .any(|item| matches!(item, PersistedInputItem::Image { .. }));
    if has_images && !supports_image_input {
        if is_current_input {
            return Err("current model does not support image input".to_string());
        }
        let mut degraded_lines = Vec::new();
        for item in items {
            match item {
                PersistedInputItem::Text { text } => {
                    if !text.is_empty() {
                        degraded_lines.push(text.clone());
                    }
                }
                PersistedInputItem::Image { label, .. } => {
                    degraded_lines.push(format!(
                        "ERROR: Cannot read \"{label}\" (this model does not support image input). Inform the user."
                    ));
                }
            }
        }
        return Ok(vec![ModelItem::UserText(degraded_lines.join("\n"))]);
    }
    if !has_images {
        return Ok(vec![ModelItem::UserText(PersistedInputItem::joined_text(
            items,
        ))]);
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        match item {
            PersistedInputItem::Text { text } => {
                out.push(ModelItem::UserContent {
                    text: text.clone(),
                    images: Vec::new(),
                });
            }
            PersistedInputItem::Image {
                archive_path,
                mime,
                content_hash,
                width,
                height,
                ..
            } => {
                let bytes = verify_archived_image(
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

/// 回放 fork 克隆历史轮的图片附件（这些消息没有 `agent_subagent_input` 记录）。
///
/// 仅加载仍存在于磁盘的归档图片；全部不可读时退化为纯文本，不阻断本轮回放。
fn historical_attachment_items(
    database: &Database,
    workspace: &Path,
    child_session_id: &str,
    user_text: &str,
    attachments: &[&crate::db::chat::MessageAttachmentMeta],
) -> Vec<ModelItem> {
    let ids: Vec<&str> = attachments.iter().map(|att| att.id.as_str()).collect();
    let payloads = database
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
        return vec![ModelItem::UserText(user_text.to_string())];
    }
    vec![ModelItem::UserContent {
        text: user_text.to_string(),
        images,
    }]
}

/// 组装子会话新一轮的完整 `ModelContext`（含持久化压缩检查点、完整轮次预算裁剪与有序多模态输入）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_child_context(
    database: &Database,
    workspace: &Path,
    parent_session: &str,
    agent_id: &str,
    child_session_id: &str,
    current_user_message_id: &str,
    role: AgentRole,
    prompt_ctx: ChildPromptContext<'_>,
    cwd: &Path,
    model: &ModelSnapshot,
    variant_params: Option<serde_json::Value>,
    context_limit: Option<u64>,
    output_limit: Option<u64>,
    supports_image_input: bool,
    current_items: &[PersistedInputItem],
    subagents: &crate::config::subagents::SubagentsSection,
    limits: ChildTurnLimits,
) -> Result<ModelContext, String> {
    let checkpoint = database
        .active_checkpoint(workspace, child_session_id)
        .map_err(|err| err.code().to_string())?;
    let start_after = checkpoint.as_ref().map_or(i64::MIN, |cp| cp.end_sequence);
    let compact_summary = checkpoint.map(|cp| cp.summary_text);

    let messages = database
        .load_chat_session(workspace, child_session_id)
        .map_err(|err| err.code().to_string())?
        .unwrap_or_default();
    let turn_items_map = database
        .load_subagent_turn_items(workspace, parent_session, agent_id)
        .map_err(|err| err.code().to_string())?;
    let turn_envelopes = database
        .load_subagent_turn_envelopes(workspace, parent_session, agent_id)
        .unwrap_or_default();

    let retained: Vec<&ChatMessage> = messages
        .iter()
        .filter(|msg| msg.sequence > start_after && msg.external_id != current_user_message_id)
        .collect();

    let planned = plan_child_historical_turns(&retained, &turn_items_map, supports_image_input);
    // 非克隆子轮来自 `agent_subagent_input`；当前轮若已落库也会计入，故按是否包含当前消息
    // 调整阈值。该判断基于完整历史，不受 compact 检查点裁剪影响。
    let non_cloned_user_count = messages
        .iter()
        .filter(|msg| msg.role == "user" && turn_items_map.contains_key(&msg.external_id))
        .count();
    let current_tracked = messages
        .iter()
        .any(|msg| msg.external_id == current_user_message_id);
    let is_follow_up = non_cloned_user_count > usize::from(current_tracked);
    let raw_current_text = PersistedInputItem::joined_text(current_items);
    let mut current_model_items =
        replay_persisted_items(child_session_id, supports_image_input, true, current_items)?;
    let current_text = if is_follow_up {
        let (msg_type, child_path, sender_path) = resolve_current_envelope(
            database,
            workspace,
            parent_session,
            agent_id,
            current_user_message_id,
            &turn_envelopes,
        );
        current_model_items =
            wrap_envelope_items(current_model_items, msg_type, &child_path, &sender_path);
        crate::chat::agent::format_inter_agent_envelope(
            msg_type,
            &child_path,
            &sender_path,
            &raw_current_text,
        )
    } else {
        raw_current_text
    };
    let current_tokens = estimate_items_tokens(&current_model_items);

    let output_reserve = output_limit.unwrap_or(0).min(8192);
    let budget = context_limit.map(|limit| {
        limit
            .saturating_sub(output_reserve)
            .saturating_sub(256)
            .saturating_sub(crate::chat::context::USER_SHELL_INJECTION_RESERVE_TOKENS)
    });

    let mut kept = Vec::new();
    let mut used = current_tokens;
    for turn in planned.iter().rev() {
        if let Some(budget) = budget
            && used.saturating_add(turn.tokens) > budget
        {
            break;
        }
        used = used.saturating_add(turn.tokens);
        kept.push(turn);
    }
    kept.reverse();

    let mut turns = Vec::with_capacity(kept.len() * 2 + 1);
    let mut items = Vec::new();
    for turn in kept {
        let user_items = match turn.stored_user_items {
            Some(stored) => {
                replay_persisted_items(child_session_id, supports_image_input, false, stored)?
            }
            None if !turn.user_image_attachments.is_empty() => historical_attachment_items(
                database,
                workspace,
                child_session_id,
                &turn.user_text,
                &turn.user_image_attachments,
            ),
            None => vec![ModelItem::UserText(turn.user_text.clone())],
        };
        turns.push(ChatTurn {
            role: "user".into(),
            text: turn.user_text.clone(),
            reasoning_metadata: None,
        });
        turns.push(ChatTurn {
            role: "assistant".into(),
            text: turn.assistant_text.clone(),
            reasoning_metadata: None,
        });
        items.extend(user_items);
        items.extend(turn.assistant_items.iter().cloned());
    }
    turns.push(ChatTurn {
        role: "user".into(),
        text: current_text,
        reasoning_metadata: None,
    });
    items.extend(current_model_items);

    let capabilities = subagents
        .resolve_role(role.as_str())
        .map(|spec| spec.capabilities)
        .unwrap_or_default();
    let mcp_prompt = prompt_ctx.mcp.instructions_prompt();
    let system_prompt = crate::chat::agent::compose_subagent_prompt_with_capabilities(
        role,
        prompt_ctx.system_prompt_override,
        capabilities,
        crate::chat::prompt::PromptParts {
            model_identity: prompt_ctx.model_identity,
            environment: Some(prompt_ctx.environment),
            instructions: prompt_ctx.instructions,
            skills: prompt_ctx.skills,
            mcp: mcp_prompt.as_deref(),
            compact: None,
        },
    );
    let request = ChatRequest {
        turns,
        items,
        model: model.clone(),
        variant_params,
        purpose: RequestPurpose::Subagent(role),
        system_prompt: Some(system_prompt),
        tools: std::sync::Arc::new(child_tool_decls(
            crate::chat::agent::ToolSurface::from_role_at_depth(
                role,
                limits.depth,
                limits.max_depth,
            ),
            capabilities,
            prompt_ctx.mcp,
            subagents,
        )),
    };
    Ok(ModelContext::new(request, cwd.to_path_buf())
        .with_context_limit(context_limit)
        .with_subagent_depth(limits.depth, limits.max_depth)
        .with_token_budget(limits.max_tokens)
        .with_compact_summary(compact_summary))
}

/// 子 agent 的工具声明：按角色能力收紧 `web_search` 与 MCP 工具。
fn child_tool_decls(
    surface: crate::chat::agent::ToolSurface,
    capabilities: crate::config::subagents::RoleCapabilities,
    mcp: &crate::mcp::McpToolSnapshot,
    subagents: &crate::config::subagents::SubagentsSection,
) -> Vec<crate::chat::tools::ToolDecl> {
    crate::chat::tools::turn_tools_for_subagent(surface, capabilities, mcp, Some(subagents))
}

/// 测试专用：与子上下文使用同一入口生成工具名集合。
#[cfg(test)]
pub(crate) fn child_tool_names(
    surface: crate::chat::agent::ToolSurface,
    capabilities: crate::config::subagents::RoleCapabilities,
    mcp: &crate::mcp::McpToolSnapshot,
    subagents: &crate::config::subagents::SubagentsSection,
) -> Vec<String> {
    child_tool_decls(surface, capabilities, mcp, subagents)
        .into_iter()
        .map(|decl| decl.name)
        .collect()
}

/// 解析当前跟进轮次的信封类型与 `(child_path, sender_path)`。
fn resolve_current_envelope(
    database: &Database,
    workspace: &Path,
    parent_session: &str,
    agent_id: &str,
    current_user_message_id: &str,
    turn_envelopes: &HashMap<String, crate::db::chat::SubagentTurnEnvelopeMeta>,
) -> (crate::chat::agent::InterAgentMessageType, String, String) {
    if !current_user_message_id.is_empty()
        && let Some(meta) = turn_envelopes.get(current_user_message_id)
    {
        let (child_path, sender_path) = envelope_paths(
            database,
            workspace,
            parent_session,
            agent_id,
            meta.caller_session_id.as_deref(),
        );
        return (
            crate::chat::agent::InterAgentMessageType::from_trigger_turn(meta.trigger_turn),
            child_path,
            sender_path,
        );
    }
    if let Ok(Some(queued)) = database.next_queued_input(workspace, parent_session, agent_id) {
        let (child_path, sender_path) = envelope_paths(
            database,
            workspace,
            parent_session,
            agent_id,
            queued.caller_session_id(),
        );
        return (
            crate::chat::agent::InterAgentMessageType::from_trigger_turn(queued.trigger_turn()),
            child_path,
            sender_path,
        );
    }
    if let Some(latest) = turn_envelopes.values().max_by_key(|m| m.position) {
        let (child_path, sender_path) = envelope_paths(
            database,
            workspace,
            parent_session,
            agent_id,
            latest.caller_session_id.as_deref(),
        );
        return (
            crate::chat::agent::InterAgentMessageType::from_trigger_turn(latest.trigger_turn),
            child_path,
            sender_path,
        );
    }
    let (child_path, sender_path) =
        envelope_paths(database, workspace, parent_session, agent_id, None);
    (
        crate::chat::agent::InterAgentMessageType::NewTask,
        child_path,
        sender_path,
    )
}

/// 子 agent 与发送者的规范路径（用于 `send_input` 后续轮次的结构化信封）。
fn envelope_paths(
    database: &Database,
    workspace: &Path,
    parent_session: &str,
    agent_id: &str,
    caller_session: Option<&str>,
) -> (String, String) {
    let child_path = database
        .owned_subagent(workspace, parent_session, agent_id)
        .ok()
        .flatten()
        .map(|record| record.agent_path)
        .filter(|path| !path.trim().is_empty())
        .unwrap_or_else(|| "/root/subagent".to_string());
    let sender_session = caller_session
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(parent_session);
    let sender_path = database
        .resolve_session_tree_root(workspace, sender_session)
        .map(|(_, _, path)| path)
        .unwrap_or_else(|_| crate::chat::agent::ROOT_AGENT_PATH.to_string());
    (child_path, sender_path)
}

/// 只在模型请求侧包裹后续 `send_input` 的用户文本（DB 与 TUI 保持原文）。
fn wrap_envelope_items(
    items: Vec<ModelItem>,
    message_type: crate::chat::agent::InterAgentMessageType,
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
                    message_type,
                    child_path,
                    sender_path,
                    &text,
                ))
            }
            ModelItem::UserContent { text, images } if !wrapped => {
                wrapped = true;
                let payload = if text.trim().is_empty() {
                    format!("[{} image(s)]", images.len())
                } else {
                    text
                };
                ModelItem::UserContent {
                    text: crate::chat::agent::format_inter_agent_envelope(
                        message_type,
                        child_path,
                        sender_path,
                        &payload,
                    ),
                    images,
                }
            }
            other => other,
        })
        .collect()
}

struct PlannedChildTurn<'a> {
    user_text: String,
    assistant_text: String,
    stored_user_items: Option<&'a [PersistedInputItem]>,
    /// 无 `agent_subagent_input` 记录（fork 克隆历史）时，从消息附件恢复的有序图片。
    user_image_attachments: Vec<&'a crate::db::chat::MessageAttachmentMeta>,
    assistant_items: Vec<ModelItem>,
    tokens: u64,
}

fn plan_child_historical_turns<'a>(
    messages: &'a [&'a ChatMessage],
    turn_items_map: &'a HashMap<String, Vec<PersistedInputItem>>,
    supports_image_input: bool,
) -> Vec<PlannedChildTurn<'a>> {
    let mut planned = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        let turn_index = messages[index].turn_index;
        let mut user: Option<&ChatMessage> = None;
        let mut assistant: Option<&ChatMessage> = None;
        while index < messages.len() && messages[index].turn_index == turn_index {
            let msg = messages[index];
            match msg.role.as_str() {
                "user" if user.is_none() => user = Some(msg),
                "assistant" if assistant.is_none() => assistant = Some(msg),
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
        let stored_user_items = turn_items_map.get(&user.external_id).map(Vec::as_slice);
        // fork 克隆的历史轮次没有子输入项记录：回放消息附件里仍在磁盘上的图片。
        let user_image_attachments = if stored_user_items.is_none() && supports_image_input {
            user.attachments
                .iter()
                .filter(|att| {
                    att.kind == crate::db::chat::MessageAttachmentKind::Image
                        && att
                            .source_path
                            .as_deref()
                            .is_some_and(|path| Path::new(path).is_file())
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut assistant_text = message_text(assistant);
        let mut assistant_items = crate::chat::context::assistant_items(assistant);
        if aborted {
            assistant_items
                .retain(|item| !matches!(item, ModelItem::AssistantText(text) if text.is_empty()));
            assistant_items.push(ModelItem::UserText(
                crate::chat::agent::SUBAGENT_TURN_ABORTED_MARKER.to_string(),
            ));
            if !assistant_text.is_empty() {
                assistant_text.push('\n');
            }
            assistant_text.push_str(crate::chat::agent::SUBAGENT_TURN_ABORTED_MARKER);
        }
        planned.push(PlannedChildTurn {
            user_text,
            assistant_text,
            stored_user_items,
            user_image_attachments,
            assistant_items,
            tokens: 0,
        });
    }
    crate::chat::context::trim_tool_images_in_turn_items(
        planned.iter_mut().map(|turn| &mut turn.assistant_items),
        supports_image_input,
    );
    for turn in &mut planned {
        let user_tokens = match turn.stored_user_items {
            Some(stored) => estimate_persisted_items_tokens(stored),
            None => (crate::chat::context::estimate_tokens(&turn.user_text) as u64)
                .saturating_add((turn.user_image_attachments.len() as u64) * 1600),
        };
        turn.tokens = user_tokens.saturating_add(estimate_items_tokens(&turn.assistant_items));
    }
    planned
}

fn message_text(message: &ChatMessage) -> String {
    message
        .parts
        .iter()
        .filter(|part| part.kind == "text")
        .map(|part| part.content.as_str())
        .collect::<Vec<_>>()
        .join("")
}

fn estimate_persisted_items_tokens(items: &[PersistedInputItem]) -> u64 {
    let mut tokens = 0u64;
    for item in items {
        let step = match item {
            PersistedInputItem::Text { text } => crate::chat::context::estimate_tokens(text) as u64,
            PersistedInputItem::Image { .. } => 1600,
        };
        tokens = tokens.saturating_add(step);
    }
    tokens
}

fn estimate_items_tokens(items: &[ModelItem]) -> u64 {
    let mut tokens = 0u64;
    for item in items {
        let step = match item {
            ModelItem::UserText(text) | ModelItem::AssistantText(text) => {
                crate::chat::context::estimate_tokens(text) as u64
            }
            ModelItem::UserContent { text, images } => (crate::chat::context::estimate_tokens(text)
                as u64)
                .saturating_add((images.len() as u64) * 1600),
            ModelItem::AssistantToolCall {
                call_id,
                name,
                input,
            } => {
                let args = input.to_string();
                (crate::chat::context::estimate_tokens(call_id)
                    + crate::chat::context::estimate_tokens(name)
                    + crate::chat::context::estimate_tokens(&args)) as u64
            }
            ModelItem::ToolResult {
                call_id,
                content,
                images,
                ..
            } => ((crate::chat::context::estimate_tokens(call_id)
                + crate::chat::context::estimate_tokens(content)) as u64)
                .saturating_add((images.len() as u64) * 1600),
        };
        tokens = tokens.saturating_add(step);
    }
    tokens
}

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

use crate::chat::agent::{
    AgentRole, IMAGE_MISSING_CODE, SubagentInputItem, compose_subagent_prompt,
};
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

/// 从持久化的 `PersistedInputItem` 重放有序 `ModelItem`（严格校验归档文件与摘要）。
pub(super) fn replay_persisted_items(
    child_session_id: &str,
    supports_image_input: bool,
    items: &[PersistedInputItem],
) -> Result<Vec<ModelItem>, String> {
    if items.is_empty() {
        return Err(IMAGE_MISSING_CODE.to_string());
    }
    let has_images = items
        .iter()
        .any(|item| matches!(item, PersistedInputItem::Image { .. }));
    if has_images && !supports_image_input {
        return Err("current model does not support image input".to_string());
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

/// 组装子会话新一轮的完整 `ModelContext`（含持久化压缩检查点、完整轮次预算裁剪与有序多模态输入）。
#[allow(clippy::too_many_arguments)]
pub(super) fn assemble_child_context(
    database: &Database,
    workspace: &Path,
    parent_session: &str,
    agent_id: &str,
    child_session_id: &str,
    current_user_message_id: &str,
    role: AgentRole,
    environment: &str,
    cwd: &Path,
    model: &ModelSnapshot,
    variant_params: Option<serde_json::Value>,
    context_limit: Option<u64>,
    output_limit: Option<u64>,
    supports_image_input: bool,
    current_items: &[PersistedInputItem],
) -> Result<ModelContext, String> {
    let current_model_items =
        replay_persisted_items(child_session_id, supports_image_input, current_items)?;
    let current_text = PersistedInputItem::joined_text(current_items);
    let current_tokens = estimate_items_tokens(&current_model_items);

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

    let retained: Vec<&ChatMessage> = messages
        .iter()
        .filter(|msg| msg.sequence > start_after && msg.external_id != current_user_message_id)
        .collect();

    let planned = plan_child_historical_turns(&retained, &turn_items_map);

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
            Some(stored) => replay_persisted_items(child_session_id, supports_image_input, stored)?,
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

    let system_prompt = compose_subagent_prompt(role, Some(environment));
    let request = ChatRequest {
        turns,
        items,
        model: model.clone(),
        variant_params,
        purpose: RequestPurpose::Subagent(role),
        system_prompt: Some(system_prompt),
    };
    Ok(ModelContext::new(request, cwd.to_path_buf())
        .with_context_limit(context_limit)
        .with_compact_summary(compact_summary))
}

struct PlannedChildTurn<'a> {
    user_text: String,
    assistant_text: String,
    stored_user_items: Option<&'a [PersistedInputItem]>,
    assistant_items: Vec<ModelItem>,
    tokens: u64,
}

fn plan_child_historical_turns<'a>(
    messages: &[&ChatMessage],
    turn_items_map: &'a HashMap<String, Vec<PersistedInputItem>>,
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
        if user.status != "complete" || assistant.status != "complete" {
            continue;
        }
        let user_text = message_text(user);
        let stored_user_items = turn_items_map.get(&user.external_id).map(Vec::as_slice);
        let user_tokens = match stored_user_items {
            Some(stored) => estimate_persisted_items_tokens(stored),
            None => crate::chat::context::estimate_tokens(&user_text) as u64,
        };
        let assistant_text = message_text(assistant);
        let assistant_items = crate::chat::context::assistant_items(assistant);
        let tokens = user_tokens.saturating_add(estimate_items_tokens(&assistant_items));
        planned.push(PlannedChildTurn {
            user_text,
            assistant_text,
            stored_user_items,
            assistant_items,
            tokens,
        });
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
                call_id, content, ..
            } => {
                (crate::chat::context::estimate_tokens(call_id)
                    + crate::chat::context::estimate_tokens(content)) as u64
            }
        };
        tokens = tokens.saturating_add(step);
    }
    tokens
}

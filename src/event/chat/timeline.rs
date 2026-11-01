//! 时间线与上下文装配的辅助：步骤 position、parts 快照、checkpoint 与草稿提取。

use super::*;

/// 该 call 步骤在 app 时间线内的 position（找不到返回 None）。
pub(super) fn tool_step_position(state: &AppState, key: &RequestKey, call_id: &str) -> Option<i64> {
    let assistant_id = active_assistant_id(state, key)?;
    let pane = state.pane_anywhere(key.pane)?;
    let message = pane
        .lx
        .chat
        .messages
        .iter()
        .find(|message| message.id == assistant_id)?;
    message.tool(call_id).map(|call| call.position)
}

/// 该工具步骤**之前**的 parts 快照（保持真实 position，不重新编号）。
pub(super) fn parts_before(
    state: &AppState,
    key: &RequestKey,
    assistant_id: &str,
    position: i64,
) -> Vec<crate::db::chat::ChatPart> {
    let Some(pane) = state.pane_anywhere(key.pane) else {
        return Vec::new();
    };
    let Some(message) = pane
        .lx
        .chat
        .messages
        .iter()
        .find(|message| message.id == assistant_id)
    else {
        return Vec::new();
    };
    crate::db::chat::chat_part_views(message.steps.iter().filter_map(|step| match step {
        crate::app::state::AssistantStepView::Part { position: at, part } if *at < position => {
            Some((*at, part))
        }
        _ => None,
    }))
}

/// 入队一次 checkpoint：把窗格当前内存 parts 快照交给持久化层。
pub(crate) fn schedule_checkpoint(state: &mut AppState, key: &RequestKey) {
    let Some(target) = state.pane_anywhere(key.pane) else {
        return;
    };
    let assistant_id = match &target.lx.chat.request {
        RequestState::Streaming {
            key: active,
            assistant_id,
        } if active == key => assistant_id.clone(),
        _ => return,
    };
    let parts = target
        .lx
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.id == assistant_id)
        .map(crate::db::chat::chat_message_part_views)
        .unwrap_or_default();
    let Some(workspace) = state.chat_session_key_for_pane(key.pane) else {
        return;
    };
    state
        .effects
        .push(crate::app::effects::Effect::CheckpointChatTurn {
            workspace,
            assistant_id,
            parts,
        });
}

/// 本轮用户草稿：取**最近一条 user 消息**的正文拼接。
///
/// 发送时该轮 assistant 占位已追加在末尾，故不能用 `messages.last()`；
/// 必须按 role 反向查找 user 消息，否则得到空串（请求携带空 user 消息被拒）。
#[cfg(test)]
pub(crate) fn latest_user_draft_for_test(
    messages: &[crate::app::state::ChatMessageView],
) -> String {
    latest_user_draft(messages)
}

#[allow(dead_code)]
pub(crate) fn latest_user_draft(messages: &[crate::app::state::ChatMessageView]) -> String {
    latest_user_draft_and_attachments(messages).0
}

pub(crate) fn latest_user_draft_and_attachments(
    messages: &[crate::app::state::ChatMessageView],
) -> (String, Vec<crate::db::chat::MessageAttachmentMeta>) {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "user" && !message.is_user_shell())
        .map(|message| (message.text(), message.attachments.clone()))
        .unwrap_or_default()
}

/// 窗格聊天视图 → DB 消息记录（上下文装配输入）。
///
/// 委托给 `db::chat::chat_message_views_to_db`：把 parts 与 tools 同时映射成记录，
/// 使「当前轮内存视图」与「`/resume` 加载的历史」走同一条装配来源。
pub(crate) fn chat_part_views_to_db(
    messages: &[crate::app::state::ChatMessageView],
) -> Vec<crate::db::chat::ChatMessage> {
    crate::db::chat::chat_message_views_to_db(messages)
}

/// 由窗格消息视图装配本轮请求（兼容旧 7 参数签名，不带模型身份/AGENTS.md/切换记录）。
#[cfg(test)]
pub(crate) fn assemble_turn_request(
    messages: &[crate::app::state::ChatMessageView],
    frozen: &crate::app::state::FrozenModel,
    compact: Option<&context::CompactContext>,
    environment: Option<&str>,
    skills: Option<&str>,
    mcp: Option<&str>,
    skill_blocks: &[String],
) -> AssembledTurn {
    assemble_turn_request_with_context(
        messages,
        frozen,
        compact,
        None,
        environment,
        None,
        skills,
        mcp,
        &[],
        skill_blocks,
    )
}

/// 由窗格消息视图装配本轮完整请求（含模型身份声明、AGENTS.md 指令段与模型切换记录）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_turn_request_with_context(
    messages: &[crate::app::state::ChatMessageView],
    frozen: &crate::app::state::FrozenModel,
    compact: Option<&context::CompactContext>,
    model_identity: Option<&str>,
    environment: Option<&str>,
    instructions: Option<&str>,
    skills: Option<&str>,
    mcp: Option<&str>,
    model_switches: &[crate::db::chat::ModelSwitchRecord],
    skill_blocks: &[String],
) -> AssembledTurn {
    let (draft, draft_attachments) = latest_user_draft_and_attachments(messages);
    let db_messages = chat_part_views_to_db(messages);
    let model = crate::db::chat::ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let assembly = context::assemble(&AssemblyInput {
        messages: &db_messages,
        draft: &draft,
        draft_attachments: &draft_attachments,
        model: &model,
        context_limit: frozen.context_limit,
        output_limit: frozen.output_limit,
        variant_params: frozen.variant_params.clone(),
        compact,
        model_identity,
        environment,
        instructions,
        skills,
        mcp,
        model_switches,
        skill_blocks,
        supports_image_input: frozen.supports_image_input,
    });
    AssembledTurn {
        request: ChatRequest {
            turns: assembly.request.turns,
            items: assembly.request.items,
            model,
            variant_params: frozen.variant_params.clone(),
            purpose: crate::chat::events::RequestPurpose::Chat,
            // 装配出的系统提示已包含冻结环境与 compact 摘要：请求不再现场采集。
            system_prompt: Some(assembly.system_prompt),
            tools: assembly.request.tools,
        },
        omitted_turns: assembly.omitted_turns,
        draft_exceeds: assembly.draft_exceeds,
        item_image_attachments: assembly.item_image_attachments,
    }
}

/// 装配结果：请求 + 需要调用方处理的两种信号。
pub(crate) struct AssembledTurn {
    pub request: ChatRequest,
    /// 省略的完整轮次数（一次性提示）。
    pub omitted_turns: usize,
    /// 当前用户消息单独超限：调用方报错，**不截断**用户输入。
    pub draft_exceeds: bool,
    /// 选中的每个 UserContent 步骤对应的图片附件列表。
    pub item_image_attachments: Vec<(usize, Vec<context::ItemImageAttachment>)>,
}

/// 当前用户消息超出模型上下文预算时的稳定错误码（调用方报错，不截断输入）。
pub const DRAFT_EXCEEDS_CONTEXT_CODE: &str = "message exceeds model context";

/// 一轮的可执行快照：冻结的上下文 + 冻结的工作区根。
///
/// 与 spawn 分离，使测试能用假 transport 直接驱动同一 agent turn（不经真实 HTTP）。
pub(crate) struct PreparedTurn {
    pub context: ModelContext,
    pub cwd: PathBuf,
}

/// 由已装配请求与冻结 cwd 构造可执行快照。
pub(crate) fn prepare_turn(request: ChatRequest, cwd: PathBuf) -> PreparedTurn {
    let context = ModelContext::new(request, cwd.clone());
    PreparedTurn { context, cwd }
}

/// 按需从本地路径加载用户图片字节并写入 `request.items`：
/// - 当前轮缺图或模型不支持图片：返回 `Err(稳定错误码)`；
/// - 历史轮次缺图：跳过该图片；模型不支持图片时降级为显式提示文本。
pub(super) fn hydrate_item_images(
    state: &AppState,
    persistence: &Persistence,
    key: &RequestKey,
    supports_image_input: bool,
    attachments: &[(usize, Vec<context::ItemImageAttachment>)],
    request: &mut ChatRequest,
) -> Result<(), &'static str> {
    if attachments.is_empty() {
        return Ok(());
    }
    let db_workspace = state.chat_session_key_for_pane(key.pane).or_else(|| {
        state
            .active_workspace()
            .repo_key()
            .map(std::path::Path::to_path_buf)
    });
    let Some(ws) = db_workspace else {
        return Err("workspace missing");
    };
    let total_items = request.items.len();
    for (item_idx, atts) in attachments {
        let is_current_turn = *item_idx + 1 == total_items;
        if !supports_image_input {
            if is_current_turn {
                return Err("current model does not support image input");
            }
            if let Some(ModelItem::UserContent { text, .. }) = request.items.get_mut(*item_idx) {
                let mut replaced = std::mem::take(text);
                for att in atts {
                    if !replaced.is_empty() {
                        replaced.push('\n');
                    }
                    replaced.push_str(&format!(
                        "ERROR: Cannot read \"{}\" (this model does not support image input). Inform the user.",
                        att.label
                    ));
                }
                request.items[*item_idx] = ModelItem::UserText(replaced);
            }
            continue;
        }
        let mut loaded_images: Vec<crate::chat::events::ModelImage> = Vec::new();
        for att in atts {
            match persistence.load_image_payloads(&ws, &key.session, &[att.id.as_str()]) {
                Ok(mut map) => {
                    if let Some(bytes) = map.remove(&att.id) {
                        if !loaded_images
                            .iter()
                            .any(|existing| existing.bytes.as_slice() == bytes.as_slice())
                        {
                            loaded_images.push(crate::chat::events::ModelImage {
                                mime: att.mime.clone(),
                                bytes: std::sync::Arc::new(bytes),
                            });
                        }
                    } else if is_current_turn {
                        return Err("image payload unavailable");
                    }
                }
                Err(_) if is_current_turn => {
                    return Err("image payload unavailable");
                }
                Err(_) => {}
            }
        }
        if let Some(item) = request.items.get_mut(*item_idx) {
            if loaded_images.is_empty() && !is_current_turn {
                if let ModelItem::UserContent { text, .. } = item {
                    *item = ModelItem::UserText(std::mem::take(text));
                }
            } else if let ModelItem::UserContent { images, .. } = item {
                *images = loaded_images;
            }
        }
    }
    Ok(())
}

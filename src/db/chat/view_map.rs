//! App 聊天视图（`ChatMessageView` / `ChatPartView`）到 DB 装配记录的无损映射。

use super::{ChatMessage, ChatPart, ToolCallRecord, UserShellRecord};

/// app 层聊天步骤视图 → 存储记录 parts。
///
/// position 由调用方的时间线给出（不再按枚举重排），`completed_at_ms` 原样携带；
/// 重复或负值位置会在写入时被 `replace_parts` 拒绝为 `PositionConflict`。
pub fn chat_part_views<'a>(
    parts: impl Iterator<Item = (i64, &'a crate::app::state::ChatPartView)>,
) -> Vec<ChatPart> {
    parts
        .map(|(position, part)| ChatPart {
            position,
            kind: part.kind_str().to_string(),
            content: part.content.clone(),
            provider_metadata_json: part.provider_metadata_json.clone(),
            started_at_ms: part.started_at_ms,
            ended_at_ms: part.ended_at_ms,
            completed_at_ms: part.completed_at_ms,
        })
        .collect()
}

/// 消息步骤时间线 → DB parts（使用步骤自带的真实 `position`，不重新编号）。
///
/// 工具步骤不是内容块：它们由 `agent_tool_call` 表承载（见 `chat/tools.rs`），
/// 此处只取 `AssistantStepView::Part`。
pub fn chat_message_part_views(message: &crate::app::state::ChatMessageView) -> Vec<ChatPart> {
    chat_part_views(message.steps.iter().filter_map(|step| match step {
        crate::app::state::AssistantStepView::Part { position, part } => Some((*position, part)),
        crate::app::state::AssistantStepView::Tool { .. } => None,
    }))
}

/// 消息步骤时间线 → DB 工具记录（装配输入用；不写入 DB）。
///
/// `position` 取步骤自身编号（与 parts 共用同一编号空间，不重新编号）；
/// `result_json` 只在结算后存在，`error_text` 只在失败/中断时有值——都不伪造。
pub fn chat_message_tool_views(
    message: &crate::app::state::ChatMessageView,
) -> Vec<ToolCallRecord> {
    message
        .steps
        .iter()
        .filter_map(|step| match step {
            crate::app::state::AssistantStepView::Tool { position, call } => Some(ToolCallRecord {
                call_id: call.call_id.clone(),
                message_id: message.id.clone(),
                name: call.name.clone(),
                arguments_json: call.arguments_json.clone(),
                position: *position,
                status: call.status.as_str().to_string(),
                result_json: call.result_json.clone(),
                error_text: call.error.clone(),
                started_at_ms: 0,
                completed_at_ms: None,
                duration_ms: None,
            }),
            crate::app::state::AssistantStepView::Part { .. } => None,
        })
        .collect()
}

/// 展示视图 → 持久化记录（上下文装配与 checkpoint 用；字段一一对应，不丢事实）。
fn user_shell_record_of(view: &crate::app::state::lx::chat::UserShellView) -> UserShellRecord {
    UserShellRecord {
        command: view.command.clone(),
        cwd: view.cwd.clone(),
        shell: view.shell.clone(),
        state: view.state.as_str().to_string(),
        exit_code: view.exit_code,
        duration_ms: view.duration_ms,
        output: view.output.clone(),
        output_start: view.output_start,
        output_end: view.output_end,
        dropped_bytes: view.dropped_bytes,
        truncated: view.truncated,
        runtime_epoch: view.runtime_epoch.clone(),
        process_id: view.process_id,
        revision: view.revision,
    }
}

/// 窗格聊天视图 → 装配用 DB 消息记录（parts 与 tools 同源映射）。
///
/// **选择**：`/resume` 后的下一轮装配走「窗格内存视图 → 记录 → `context::assemble`」，
/// 不重新查库。理由：窗格视图已是「DB 历史 + 本轮增量」的唯一时间线
/// （`on_session_loaded` 按 position 合并 parts 与 tools，`on_stream_delta` 追加本轮步骤），
/// 再查库既拿不到本轮未完成步骤，也会与内存状态形成两份真相。
/// 记录里的 `sequence` 用视图下标（装配按消息顺序分组，不使用该字段的绝对值）。
pub fn chat_message_views_to_db(
    messages: &[crate::app::state::ChatMessageView],
) -> Vec<ChatMessage> {
    let has_real_sequences = messages.iter().any(|message| message.sequence > 0);
    messages
        .iter()
        .enumerate()
        .map(|(index, message)| ChatMessage {
            external_id: message.id.clone(),
            sequence: if has_real_sequences {
                message.sequence
            } else {
                index as i64
            },
            turn_index: message.turn_index,
            kind: message.kind.clone(),
            role: message.role.clone(),
            status: message.status.clone(),
            model_provider: None,
            model_id: None,
            variant: None,
            stop_reason: None,
            error_code: message.error_code.clone(),
            input_tokens: message.input_tokens,
            output_tokens: message.output_tokens,
            cache_read_tokens: message.cache_read_tokens,
            cache_write_tokens: None,
            created_at_ms: 0,
            completed_at_ms: None,
            user_shell: message.shell.as_ref().map(user_shell_record_of),
            subagent_reports: None,
            parts: chat_message_part_views(message),
            tools: chat_message_tool_views(message),
            attachments: message.attachments.clone(),
        })
        .collect()
}

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

pub(crate) fn latest_user_draft(messages: &[crate::app::state::ChatMessageView]) -> String {
    messages
        .iter()
        .rev()
        .find(|message| message.role == "user" && !message.is_user_shell())
        .map(|message| message.text())
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

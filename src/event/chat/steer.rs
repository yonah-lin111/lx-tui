//! 流中 steer 的事件层交付：DB 关旧片段 + 开新 user/assistant，再回执 loop。

use tokio::sync::oneshot;

use crate::app::state::{AppState, AssistantStepView, ChatMessageView, ChatPartView, RequestState};
use crate::chat::agent_loop::{HostError, SteerInput};
use crate::chat::events::{PartKind, StopReason};
use crate::db::chat::chat_message_part_views;
use crate::db::service::Persistence;

use super::{ChatStreams, active_assistant_id};

/// `begin_steer_turn` 落库失败时的稳定错误码前缀。
pub(crate) const STEER_TURN_BEGIN_FAILED_CODE: &str = "steer turn begin failed";
/// pending 图片与冻结模型能力不匹配。
pub(crate) const STEER_IMAGE_UNSUPPORTED_CODE: &str = "steer images unsupported";

/// 把当前 pending steer 批次注入当前轮。
///
/// 返回是否需要重绘；pending 在 DB 成功前不消费。
pub(crate) fn deliver(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: crate::app::state::RequestKey,
    reply: oneshot::Sender<Result<Vec<SteerInput>, HostError>>,
) -> bool {
    // 子 agent 的轮次复用 EventHost：它的 pending 在自己的 run.chat 里，steer 需要
    // 同步切 DB 片段并更新 handle 的 assistant/user 锚点，不能走主对话的窗格路径。
    if streams.subagents.contains_key(&key.session) {
        return deliver_child(state, streams, persistence, key, reply);
    }
    let Some(assistant_id) = active_assistant_id(state, &key) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    if state
        .pane_anywhere(key.pane)
        .is_none_or(|pane| pane.lx.chat.session_id.as_deref() != Some(key.session.as_str()))
    {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    }
    let Some(batch) = state.pane_anywhere(key.pane).and_then(|pane| {
        crate::app::state::lx::chat::steer::build_batch(&pane.lx.chat.pending_steers)
    }) else {
        // 没有 pending 是最常见路径：不要求 frozen/workspace，也不写库。
        let _ = reply.send(Ok(Vec::new()));
        return false;
    };
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Failed {
            code: format!("{STEER_TURN_BEGIN_FAILED_CODE} · workspace missing"),
        }));
        return false;
    };
    let Some(frozen) = streams.frozen.get(&key.session).cloned() else {
        let _ = reply.send(Err(HostError::Failed {
            code: format!("{STEER_TURN_BEGIN_FAILED_CODE} · model missing"),
        }));
        return false;
    };
    if !batch.model_images.is_empty() && !frozen.supports_image_input {
        let _ = reply.send(Err(HostError::Failed {
            code: STEER_IMAGE_UNSUPPORTED_CODE.to_string(),
        }));
        return false;
    }
    // 关闭当前片段并取最终 parts；安全边界保证工具已全部结算。
    let parts = {
        let Some(target) = state.pane_mut_anywhere(key.pane) else {
            let _ = reply.send(Err(HostError::Cancelled));
            return false;
        };
        target.lx.chat.close_stream(&StopReason::Complete);
        target
            .lx
            .chat
            .messages
            .iter()
            .rev()
            .find(|message| message.id == assistant_id)
            .map(chat_message_part_views)
            .unwrap_or_default()
    };
    let model = crate::db::chat::ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let committed = match persistence.begin_steer_turn(
        &workspace,
        &key.session,
        &assistant_id,
        &parts,
        &batch.text,
        &batch.attachments,
        &model,
    ) {
        Ok(committed) => committed,
        Err(error) => {
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{STEER_TURN_BEGIN_FAILED_CODE} · {}", error.code()),
            }));
            return false;
        }
    };
    let _ = crate::app::update::recap::cancel_pending_recap(state, key.pane);
    // mutation 归属切到新 user message；steer 后的文件修改归新轮次。
    if let Some(current) = streams.mutation_users.get(&key)
        && let Ok(mut id) = current.write()
    {
        *id = committed.user_message_id.clone();
    }
    let text = batch.text.clone();
    let model_label = state
        .models
        .model_display_name(&frozen.provider, &frozen.model);
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let take = batch.consumed.min(target.lx.chat.pending_steers.len());
    target.lx.chat.pending_steers.drain(..take);
    if let Some(message) = target
        .lx
        .chat
        .messages
        .iter_mut()
        .find(|message| message.id == assistant_id)
    {
        message.status = "complete".into();
        message.error_code = None;
        // 被 steer 截断的前半段不是轮次终点：不写耗时，避免底部出现模型名与统计行。
        message.duration_ms = None;
    }
    target.lx.chat.session_id = Some(committed.session_id.clone());
    target.lx.chat.revision = u64::try_from(committed.revision).unwrap_or(0);
    target
        .lx
        .chat
        .mark_steered_user_message(&committed.user_message_id);
    target.lx.chat.messages.push(ChatMessageView {
        sequence: committed.user_sequence,
        id: committed.user_message_id.clone(),
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: committed.turn_index,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: text.clone(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: committed.attachments.clone(),
    });
    target.lx.chat.messages.push(ChatMessageView {
        sequence: committed.assistant_sequence,
        id: committed.assistant_message_id.clone(),
        kind: "chat".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: committed.turn_index,
        shell: None,
        steps: Vec::new(),
        model_label: Some(model_label),
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    target
        .lx
        .chat
        .set_message_time(&committed.user_message_id, committed.created_at_ms);
    target
        .lx
        .chat
        .set_message_time(&committed.assistant_message_id, committed.created_at_ms);
    target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
    target.lx.chat.turn_elapsed_ms = Some(0);
    target.lx.chat.stop_armed_at = None;
    target.lx.chat.main_stopped = false;
    target.lx.chat.follow_bottom = true;
    target.lx.chat.reset_step_usage();
    target.lx.chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id: committed.assistant_message_id.clone(),
    };
    if committed.attachments.is_empty() && state.prompt_history.record(&text) {
        state
            .effects
            .push(crate::app::effects::Effect::RecordPromptHistory {
                text: text.trim().to_string(),
            });
    }
    crate::app::update::refresh_lx_status(state, key.pane);
    let mut skill_blocks = super::skills::reference_blocks(&text);
    let workspace_root = state.workspace_cwd_for_pane(key.pane);
    let config_root = state
        .chat_session_key_for_pane(key.pane)
        .or_else(|| workspace_root.clone());
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
    if let Some(pane) = state.pane_anywhere(key.pane) {
        skill_blocks.extend(super::subagent::snapshot::subagent_reference_blocks(
            &text,
            &effective_subagents,
            &pane.lx.agents.runs,
        ));
    }
    let _ = reply.send(Ok(vec![SteerInput {
        text,
        images: batch.model_images,
        skill_blocks,
    }]));
    true
}

/// 子 agent 的轮内 steer：落库新片段、切 handle 的 assistant/user 锚点，并把注入项回执给 loop。
fn deliver_child(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: crate::app::state::RequestKey,
    reply: oneshot::Sender<Result<Vec<SteerInput>, HostError>>,
) -> bool {
    if streams
        .cancels
        .get(&key)
        .is_some_and(|cancel| *cancel.borrow())
    {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    }
    let batch = state
        .pane_anywhere(key.pane)
        .and_then(|pane| pane.lx.agents.by_session(&key.session))
        .and_then(|run| crate::app::state::lx::chat::steer::build_batch(&run.chat.pending_steers));
    let Some(batch) = batch else {
        // 没有 pending 是最常见路径：不要求 frozen/workspace，也不写库。
        let _ = reply.send(Ok(Vec::new()));
        return false;
    };
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Failed {
            code: format!("{STEER_TURN_BEGIN_FAILED_CODE} · workspace missing"),
        }));
        return false;
    };
    let Some(frozen) = streams.frozen.get(&key.session).cloned() else {
        let _ = reply.send(Err(HostError::Failed {
            code: format!("{STEER_TURN_BEGIN_FAILED_CODE} · model missing"),
        }));
        return false;
    };
    if !batch.model_images.is_empty() && !frozen.supports_image_input {
        let _ = reply.send(Err(HostError::Failed {
            code: STEER_IMAGE_UNSUPPORTED_CODE.to_string(),
        }));
        return false;
    }
    // 安全边界保证工具已全部结算；从事件侧 chat 取旧 assistant 的最终 parts。
    let (assistant_id, parts, child_session) = {
        let Some(handle) = streams.subagents.get_mut(&key.session) else {
            let _ = reply.send(Err(HostError::Cancelled));
            return false;
        };
        handle.chat.close_stream(&StopReason::Complete);
        let current_assistant = handle.child_assistant_id.clone();
        let parts = handle
            .chat
            .messages
            .iter()
            .rev()
            .find(|message| message.id == current_assistant)
            .map(chat_message_part_views)
            .unwrap_or_default();
        (
            handle.child_assistant_id.clone(),
            parts,
            handle.child_session_id.clone(),
        )
    };
    let model = crate::db::chat::ModelSnapshot {
        provider: frozen.provider.clone(),
        model: frozen.model.clone(),
        variant: frozen.variant.clone(),
    };
    let committed = match persistence.begin_steer_turn(
        &workspace,
        &child_session,
        &assistant_id,
        &parts,
        &batch.text,
        &batch.attachments,
        &model,
    ) {
        Ok(committed) => committed,
        Err(error) => {
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{STEER_TURN_BEGIN_FAILED_CODE} · {}", error.code()),
            }));
            return false;
        }
    };
    let model_label = state
        .models
        .model_display_name(&frozen.provider, &frozen.model);
    // 事件侧 handle：后续 deltas、结算与 mutation 归属都切到新片段。
    if let Some(handle) = streams.subagents.get_mut(&key.session) {
        handle.child_assistant_id = committed.assistant_message_id.clone();
        handle.child_user_message_id = committed.user_message_id.clone();
        handle
            .turn_user_message_ids
            .push(committed.user_message_id.clone());
        if let Ok(mut id) = handle.mutation_user_id.write() {
            *id = committed.user_message_id.clone();
        }
        apply_steer_segment(
            &mut handle.chat,
            &assistant_id,
            &committed,
            &batch.text,
            Some(model_label.clone()),
            &key,
        );
    }
    // UI 侧 run：同步片段并消费 pending。
    if let Some(pane) = state.pane_mut_anywhere(key.pane)
        && let Some(run) = pane.lx.agents.by_session_mut(&key.session)
    {
        let take = batch.consumed.min(run.chat.pending_steers.len());
        apply_steer_segment(
            &mut run.chat,
            &assistant_id,
            &committed,
            &batch.text,
            Some(model_label),
            &key,
        );
        run.chat.pending_steers.drain(..take);
        run.follow = true;
        run.rows_dirty = true;
    }
    let _ = reply.send(Ok(vec![SteerInput {
        text: batch.text.clone(),
        images: batch.model_images,
        skill_blocks: Vec::new(),
    }]));
    true
}

/// 把已落库的 steer 片段追加到一份 chat：封口旧 assistant、追加 user/assistant 并恢复 streaming。
fn apply_steer_segment(
    chat: &mut crate::app::state::LxChat,
    previous_assistant_id: &str,
    committed: &crate::db::chat::TurnCommitted,
    text: &str,
    model_label: Option<String>,
    key: &crate::app::state::RequestKey,
) {
    // 与主对话路径一致：先收尾旧片段里的开放思考/工具步骤，再标记新 user 为 steer。
    chat.close_stream(&StopReason::Complete);
    if let Some(message) = chat
        .messages
        .iter_mut()
        .find(|message| message.id == previous_assistant_id)
    {
        message.status = "complete".into();
        message.error_code = None;
        // 被 steer 截断的前半段不是轮次终点：不写耗时。
        message.duration_ms = None;
    }
    chat.session_id = Some(committed.session_id.clone());
    chat.revision = u64::try_from(committed.revision).unwrap_or(0);
    chat.messages.push(ChatMessageView {
        sequence: committed.user_sequence,
        id: committed.user_message_id.clone(),
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: committed.turn_index,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: text.to_string(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: committed.attachments.clone(),
    });
    chat.mark_steered_user_message(&committed.user_message_id);
    chat.messages.push(ChatMessageView {
        sequence: committed.assistant_sequence,
        id: committed.assistant_message_id.clone(),
        kind: "chat".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: committed.turn_index,
        shell: None,
        steps: Vec::new(),
        model_label,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    chat.set_message_time(&committed.user_message_id, committed.created_at_ms);
    chat.set_message_time(&committed.assistant_message_id, committed.created_at_ms);
    chat.turn_started_at_ms = Some(committed.created_at_ms);
    chat.turn_elapsed_ms = Some(0);
    chat.stop_armed_at = None;
    chat.main_stopped = false;
    chat.follow_bottom = true;
    chat.reset_step_usage();
    chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id: committed.assistant_message_id.clone(),
    };
    chat.status.streaming = true;
}

/// 只读探测：当前 key 仍有效且窗格有 pending steer。
pub(crate) fn has_pending(state: &AppState, key: &crate::app::state::RequestKey) -> bool {
    if let Some(run) = state
        .pane_anywhere(key.pane)
        .and_then(|pane| pane.lx.agents.by_session(&key.session))
    {
        return !run.chat.pending_steers.is_empty();
    }
    active_assistant_id(state, key).is_some()
        && state.pane_anywhere(key.pane).is_some_and(|pane| {
            pane.lx.chat.session_id.as_deref() == Some(key.session.as_str())
                && !pane.lx.chat.pending_steers.is_empty()
        })
}

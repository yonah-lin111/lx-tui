//! 子轮工具握手：用句柄自证归属，不查窗格。
//!
//! 与主轮的唯一差别在归属校验的锚点（`handle.child_assistant_id`）与时间线来源
//! （子会话自己的 `LxChat`）；落库顺序与主轮完全一致：pending → running → 结果。

use super::*;

/// 子轮 BeginTool：用句柄自证归属，不查窗格；pending → running 同步完成。
pub(crate) fn begin_child_tool(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    call: ReadyToolCall,
    reply: oneshot::Sender<Result<ToolCallTicket, HostError>>,
) -> bool {
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let Some(handle) = streams.subagents.get(&key.session) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let assistant_id = handle.child_assistant_id.clone();
    // 时间线对齐：工具之前的 parts 先落库，DB 分配的 position 才与内存一致。
    if let Some(position) = tool_position(&handle.chat, &assistant_id, &call.call_id) {
        let parts = parts_before(&handle.chat, &assistant_id, position);
        if let Err(error) = persistence.checkpoint_parts(&workspace, &assistant_id, &parts) {
            tracing::warn!(
                target: "lx::agent",
                call_id = %call.call_id,
                code = error.code(),
                "child tool timeline sync failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            return true;
        }
    }
    let ticket = match persistence.begin_tool_call(
        &workspace,
        &key.session,
        &assistant_id,
        &call.call_id,
        &call.name,
        &call.arguments_json,
    ) {
        Ok(ticket) => ticket,
        Err(error) => {
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            return true;
        }
    };
    // 子轮没有「等一帧再执行」的必要：pending 与 running 同步推进后立即回执。
    if let Some(handle) = streams.subagents.get_mut(&key.session) {
        handle.chat.mark_tool_pending(&call.call_id);
    }
    let mut changed = update::on_subagent_tool_pending(state, &key.session, &call.call_id);
    if let Err(error) = persistence.mark_tool_running(&workspace, &ticket) {
        let _ = reply.send(Err(HostError::Failed {
            code: format!("{RUNNING_TOOL_FAILED_CODE} · {}", error.code()),
        }));
        return changed;
    }
    if let Some(handle) = streams.subagents.get_mut(&key.session) {
        handle.chat.mark_tool_running(&call.call_id);
    }
    changed |= update::on_subagent_tool_running(state, &key.session, &call.call_id);
    let _ = reply.send(Ok(ticket));
    changed
}

/// 子轮 FinishTool：真实结果落库 → 子消息模型与 UI 结算 → 回执。
pub(crate) fn finish_child_tool(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    ticket: &ToolCallTicket,
    outcome: ToolOutcome,
    reply: oneshot::Sender<Result<(), HostError>>,
) -> bool {
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let result = super::tool_calls::db_result_of(&outcome);
    let settled_json = result.result_json.clone();
    let arguments = streams
        .subagents
        .get(&key.session)
        .and_then(|handle| tool_arguments(&handle.chat, &ticket.call_id));
    let detail =
        super::tool_calls::detail_of(&outcome, settled_json.as_deref(), arguments.as_deref());
    match persistence.finish_tool_call(&workspace, ticket, &result) {
        Ok(()) => {
            let (status, summary, error) = super::tool_calls::view_settlement(&outcome);
            let mut changed = update::on_subagent_tool_settled(
                state,
                &key.session,
                &ticket.call_id,
                status,
                summary,
                settled_json,
                error,
                detail,
            );
            if let Some(handle) = streams.subagents.get_mut(&key.session) {
                changed |=
                    handle
                        .chat
                        .settle_tool_result(&ticket.call_id, status, None, None, None);
                if let ToolOutcome::Executed(result) = &outcome
                    && let Some(path) = mutation_path(result)
                {
                    handle.changed_paths.insert(path);
                }
            }
            let _ = reply.send(Ok(()));
            changed
        }
        Err(error) => {
            tracing::warn!(
                target: "lx::agent",
                call_id = %ticket.call_id,
                code = error.code(),
                "child tool result persistence failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{FINISH_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            false
        }
    }
}

/// 工具在子消息时间线里的位置（未找到返回 None）。
fn tool_position(
    chat: &crate::app::state::LxChat,
    assistant_id: &str,
    call_id: &str,
) -> Option<i64> {
    chat.messages
        .iter()
        .find(|message| message.id == assistant_id)
        .and_then(|message| message.tool(call_id))
        .map(|call| call.position)
}

/// 工具之前的 parts 快照（position 小于工具位置）。
fn parts_before(
    chat: &crate::app::state::LxChat,
    assistant_id: &str,
    position: i64,
) -> Vec<crate::db::chat::ChatPart> {
    chat.messages
        .iter()
        .find(|message| message.id == assistant_id)
        .map(|message| {
            crate::db::chat::chat_part_views(message.steps.iter().filter_map(|step| match step {
                crate::app::state::AssistantStepView::Part { position: p, part }
                    if *p < position =>
                {
                    Some((*p, part))
                }
                _ => None,
            }))
        })
        .unwrap_or_default()
}

/// 子消息里某次调用的参数 JSON。
fn tool_arguments(chat: &crate::app::state::LxChat, call_id: &str) -> Option<String> {
    chat.messages.iter().rev().find_map(|message| {
        message
            .tool(call_id)
            .map(|call| call.arguments_json.clone())
    })
}

/// 文件工具结果里的主路径（`changed_paths` 的近似来源；精确计数在 DB 结算时查）。
fn mutation_path(result: &ToolExecutionResult) -> Option<PathBuf> {
    match result {
        ToolExecutionResult::Write { path, .. } => Some(path.clone()),
        ToolExecutionResult::Read { .. } | ToolExecutionResult::Search { .. } => None,
        _ => None,
    }
}

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
    let agent_id = handle.agent_id.clone();
    // 时间线对齐：工具之前的 parts 先落库，DB 分配的 position 才与内存一致。
    if let Err(error) = super::super::timeline::checkpoint_before_tool(
        &handle.chat,
        &workspace,
        &assistant_id,
        &call.call_id,
        persistence,
    ) {
        tracing::warn!(
            target: "lx::agent::tool",
            pane = key.pane.raw(),
            session = %key.session,
            agent_id = %agent_id,
            call_id = %call.call_id,
            code = error.code(),
            error = %error,
            "child tool timeline sync failed"
        );
        let _ = reply.send(Err(HostError::Failed {
            code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
        }));
        return true;
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
            tracing::warn!(
                target: "lx::agent::tool",
                pane = key.pane.raw(),
                session = %key.session,
                agent_id = %agent_id,
                call_id = %call.call_id,
                name = %call.name,
                code = error.code(),
                "child tool pending record failed"
            );
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
        tracing::warn!(
            target: "lx::agent::tool",
            pane = key.pane.raw(),
            session = %key.session,
            agent_id = %agent_id,
            call_id = %call.call_id,
            code = error.code(),
            "child tool running record failed"
        );
        let _ = reply.send(Err(HostError::Failed {
            code: format!("{RUNNING_TOOL_FAILED_CODE} · {}", error.code()),
        }));
        return changed;
    }
    tracing::debug!(
        target: "lx::agent::tool",
        pane = key.pane.raw(),
        session = %key.session,
        agent_id = %agent_id,
        call_id = %call.call_id,
        name = %call.name,
        arguments_bytes = call.arguments_json.len(),
        "tool call started"
    );
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
    let agent_id = streams
        .subagents
        .get(&key.session)
        .map(|handle| handle.agent_id.clone())
        .unwrap_or_else(|| "-".into());
    match persistence.finish_tool_call(&workspace, ticket, &result) {
        Ok(()) => {
            let (status, summary, error) = super::tool_calls::view_settlement(&outcome);
            tracing::info!(
                target: "lx::agent::tool",
                pane = key.pane.raw(),
                session = %key.session,
                agent_id = %agent_id,
                call_id = %ticket.call_id,
                status = super::tool_calls::status_text(status),
                duration_ms = crate::db::now_ms().saturating_sub(ticket.started_at_ms),
                is_error = result.error_text.is_some(),
                result_bytes = result.result_json.as_ref().map_or(0, String::len),
                "tool call settled"
            );
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
            }
            // 与主轮同口径：结算后按真实运行实例校正卡片，并登记进程记录供历史恢复。
            if let ToolOutcome::Executed(ToolExecutionResult::Process(process)) = &outcome {
                if let Some(runtime) = streams.runtimes.get(&key.session)
                    && let Some(snapshot) = runtime.manager().snapshot(process.session_id)
                {
                    changed |= update::apply_process_snapshot(state, process.session_id, &snapshot);
                }
                if process.kind == crate::chat::process::ProcessResultKind::ExecCommand {
                    super::tool_calls::register_process_record(
                        streams,
                        persistence,
                        &workspace,
                        &key,
                        &ticket.call_id,
                        process.session_id,
                    );
                }
            }
            let _ = reply.send(Ok(()));
            changed
        }
        Err(error) => {
            tracing::warn!(
                target: "lx::agent::tool",
                pane = key.pane.raw(),
                session = %key.session,
                agent_id = %agent_id,
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

/// 子消息里某次调用的参数 JSON。
fn tool_arguments(chat: &crate::app::state::LxChat, call_id: &str) -> Option<String> {
    chat.messages.iter().rev().find_map(|message| {
        message
            .tool(call_id)
            .map(|call| call.arguments_json.clone())
    })
}

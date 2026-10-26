//! 工具调用握手：BeginTool / FinishTool 的落库门槛、可见状态推进与结算。
//!
//! 顺序不可交换（见 [`super::handle_agent_host_event`] 的文档）：
//!
//! - `BeginTool`：① `begin_tool_call`（pending 事务提交）→ ② 状态转 `Pending` 并安排下一帧
//!   → ③（可见窗格在该帧绘制后）`mark_tool_running` + 状态转 `Running` → 回执 ticket。
//! - `FinishTool`：① `finish_tool_call`（真实结果原子落库）→ ② 状态转终态 + 摘要 + 类型化详情
//!   → ③ 回执 `Ok`。落库失败时回执 `Err`，本轮失败且**不重试**写入。

use super::timeline::{parts_before, tool_step_position};
use super::*;

/// BeginTool：DB pending → UI pending → （帧后）DB running + UI running → ticket。
pub(crate) fn begin_tool(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    call: ReadyToolCall,
    reply: oneshot::Sender<Result<ToolCallTicket, HostError>>,
) -> bool {
    // 子 agent 的握手不查窗格（决策 14）：子会话有自己的句柄与 assistant 锚点。
    if streams.subagents.contains_key(&key.session) {
        return super::subagent::begin_child_tool(state, streams, persistence, key, call, reply);
    }
    // 归属校验：旧 generation / 窗格关闭 / 会话切换一律不执行文件 IO。
    let Some(assistant_id) = active_assistant_id(state, &key) else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    // 时间线对齐：`begin_tool_call` 按该消息内 parts/tools 的 `MAX(position)+1` 分配工具
    // 位置。先把「该工具之前」的 parts 快照提交，使 DB 分配值与 app 时间线一致，
    // 后续 checkpoint/finish 才不会撞 `PositionConflict`。
    if let Some(position) = tool_step_position(state, &key, &call.call_id) {
        let parts = parts_before(state, &key, &assistant_id, position);
        if let Err(error) = persistence.checkpoint_parts(&workspace, &assistant_id, &parts) {
            tracing::warn!(
                target: "lx::agent",
                call_id = %call.call_id,
                code = error.code(),
                "tool call timeline sync failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            return true;
        }
    }
    // ① pending 事务提交：失败即不执行文件 IO，本轮失败。
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
                target: "lx::agent",
                call_id = %call.call_id,
                name = %call.name,
                code = error.code(),
                "tool call pending record failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{BEGIN_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            return true;
        }
    };
    // ② 状态转 Pending（DB 已提交）；调用方随后安排下一帧，用户先看到 loading。
    update::on_tool_pending(state, &key, &call.call_id);
    let pending = PendingBegin {
        key,
        ticket,
        reply: Some(reply),
    };
    // ③ 可见窗格：回执推迟到该帧绘制之后；不可见窗格无需等待不可见的一帧。
    if streams.pane_visible(pending.key.pane) {
        streams.pending_begins.push(pending);
        true
    } else {
        settle_begin(state, streams, persistence, pending)
    }
}

/// 绘制完成后推进 pending → running 并回执 ticket（可见窗格的 loading 门槛）。
///
/// 返回是否有状态变化（需要再画一帧显示 running）。
pub fn flush_pending_tool_begins(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
) -> bool {
    if streams.pending_begins.is_empty() {
        return false;
    }
    let pending = std::mem::take(&mut streams.pending_begins);
    let mut changed = false;
    for item in pending {
        changed |= settle_begin(state, streams, persistence, item);
    }
    changed
}

/// pending 帧已绘制：推进 running 并把 ticket 交给 loop。
pub(crate) fn settle_begin(
    state: &mut AppState,
    streams: &ChatStreams,
    persistence: &Persistence,
    pending: PendingBegin,
) -> bool {
    let Some(reply) = pending.reply else {
        // 回执通道已被丢弃（窗格关闭）：loop 会按取消结束，不再执行。
        return false;
    };
    let Some(workspace) = streams.db_workspaces.get(&pending.key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    // 帧间归属可能变化（会话切换/`/new`）：已提交的 pending 记为 cancelled，不执行。
    if active_assistant_id(state, &pending.key).is_none() {
        cancel_pending(persistence, &workspace, &pending.ticket);
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    }
    match persistence.mark_tool_running(&workspace, &pending.ticket) {
        Ok(()) => {
            let changed = update::on_tool_running(state, &pending.key, &pending.ticket.call_id);
            let _ = reply.send(Ok(pending.ticket));
            changed
        }
        Err(error) => {
            // running 入库失败：不执行文件 IO，本轮失败。
            tracing::warn!(
                target: "lx::agent",
                call_id = %pending.ticket.call_id,
                code = error.code(),
                "tool call running record failed"
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{RUNNING_TOOL_FAILED_CODE} · {}", error.code()),
            }));
            false
        }
    }
}

/// 已提交 pending 但不再允许执行的调用：落为 `cancelled`（恢复时不会显示 running）。
pub(crate) fn cancel_pending(
    persistence: &Persistence,
    workspace: &std::path::Path,
    ticket: &ToolCallTicket,
) {
    let cancelled = ToolCallResult {
        status: ToolTerminalStatus::Cancelled,
        result_json: None,
        error_text: None,
    };
    if let Err(error) = persistence.finish_tool_call(workspace, ticket, &cancelled) {
        tracing::warn!(
            target: "lx::agent",
            call_id = %ticket.call_id,
            code = error.code(),
            "tool call cancel record failed"
        );
    }
}

/// FinishTool：真实结果原子落库 → 状态终态 + 摘要 → 回执。
pub(crate) fn finish_tool(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: RequestKey,
    ticket: &ToolCallTicket,
    outcome: ToolOutcome,
    reply: oneshot::Sender<Result<(), HostError>>,
) -> bool {
    if streams.subagents.contains_key(&key.session) {
        return super::subagent::finish_child_tool(
            state, streams, persistence, key, ticket, outcome, reply,
        );
    }
    let Some(workspace) = streams.db_workspaces.get(&key).cloned() else {
        let _ = reply.send(Err(HostError::Cancelled));
        return false;
    };
    let result = db_result_of(&outcome);
    // 文件详情与实时摘要都来自这份**即将落库**的 JSON（若落库前的序列化失败则为 None）。
    let settled_result_json = result.result_json.clone();
    let started = std::time::Instant::now();
    // ① 真实结果原子落库（DB 是唯一真相）。
    match persistence.finish_tool_call(&workspace, ticket, &result) {
        Ok(()) => {
            let (status, summary, error) = view_settlement(&outcome);
            // ② 状态转终态并写摘要/错误/结果体（不伪造结果：结果体来自刚落库的记录）。
            let mut changed = update::on_tool_settled(
                state,
                &key,
                &ticket.call_id,
                status,
                summary,
                result.result_json.clone(),
                error,
            );
            // ②b 类型化详情（进程/计划/文件/子 agent）：与落库结果体同源。
            let arguments = tool_arguments(state, &key, &ticket.call_id);
            changed |= update::settle_tool_detail(
                state,
                &key,
                &ticket.call_id,
                detail_of(&outcome, settled_result_json.as_deref(), arguments.as_deref()),
            );
            // ②b1 进程工具结算后按**运行实例的真实状态**再校正一次。
            //
            // 结果体里的 `process_state` 是"这次调用读到的那一刻"，而 `write_stdin`
            // 卡片完全可能在进程**早已退出之后**才结算（模型隔了几十秒才发起读写），
            // 于是它记下一个 `running` 快照，之后再没有任何事件会去改它——
            // 卡片就永远显示 `Running`（真实案例见 design §14.8）。
            if let ToolOutcome::Executed(ToolExecutionResult::Process(process)) = &outcome
                && let Some(runtime) = streams.runtimes.get(key.session.as_str())
                && let Some(snapshot) = runtime.manager().snapshot(process.session_id)
            {
                changed |= update::apply_process_snapshot(state, process.session_id, &snapshot);
            }
            // ②b2 新建进程的元数据登记：工具已记账后才落进程记录（供历史恢复）。
            if let ToolOutcome::Executed(ToolExecutionResult::Process(process)) = &outcome
                && process.kind == crate::chat::process::ProcessResultKind::ExecCommand
            {
                register_process_record(
                    streams,
                    persistence,
                    &workspace,
                    &key,
                    &ticket.call_id,
                    process.session_id,
                );
            }
            // ②c 计划提交：只有 update_plan 结果落库成功后才更新“当前计划”。
            if let ToolOutcome::Executed(ToolExecutionResult::Plan(snapshot)) = &outcome {
                changed |= update::commit_plan(state, key.pane, snapshot);
            }
            tracing::debug!(
                target: "lx::agent",
                call_id = %ticket.call_id,
                status = status_text(status),
                duration_ms = started.elapsed().as_millis() as u64,
                "tool call settled"
            );
            let _ = reply.send(Ok(()));
            changed
        }
        Err(db_error) => {
            // ③ 落库失败：不重试写入，本轮失败；UI 明示文件状态需核查。
            tracing::warn!(
                target: "lx::agent",
                call_id = %ticket.call_id,
                code = db_error.code(),
                "tool result persistence failed"
            );
            // lx 的反馈一律显示在输入框上方（错误红），不再用 toast。
            update::show_lx_hint(
                state,
                key.pane,
                crate::app::state::InputHintLevel::Error,
                text::LX_CHAT_TOOL_RESULT_SAVE_FAILED,
            );
            let _ = reply.send(Err(HostError::Failed {
                code: format!("{FINISH_TOOL_FAILED_CODE} · {}", db_error.code()),
            }));
            true
        }
    }
}

/// 把新建进程的起始记录写入 `agent_process`（工具已结算成功后才登记）。
///
/// 进程仍在运行时由后台事件继续合并输出快照；登记失败只记日志，不影响工具结果。
fn register_process_record(
    streams: &ChatStreams,
    persistence: &Persistence,
    workspace: &std::path::Path,
    key: &RequestKey,
    origin_call_id: &str,
    session_id: u64,
) {
    let Some(runtime) = streams.runtimes.get(&key.session) else {
        return;
    };
    let Some(snapshot) = runtime.manager().snapshot(session_id) else {
        return;
    };
    process_events::register_process(
        persistence,
        workspace,
        &runtime.epoch,
        &key.session,
        origin_call_id,
        &snapshot,
    );
}

/// 工具执行结局 → 类型化详情视图（进程 / 计划 / 文件差异）。
///
/// 文件详情必须从**刚落库的 result_json** 解码（不是原始执行结果）：实时与历史
/// 因此读的是同一份经过预算裁切的正文。
pub(crate) fn detail_of(
    outcome: &ToolOutcome,
    result_json: Option<&str>,
    arguments_json: Option<&str>,
) -> Option<crate::app::state::ToolDetailView> {
    let ToolOutcome::Executed(execution) = outcome else {
        return None;
    };
    match execution {
        ToolExecutionResult::Process(result) => {
            Some(crate::app::state::ToolDetailView::from_process(result))
        }
        ToolExecutionResult::Plan(snapshot) => Some(crate::app::state::ToolDetailView::Plan(
            crate::app::state::PlanSnapshotView::from_snapshot(snapshot),
        )),
        ToolExecutionResult::Ask { questions } => Some(crate::app::state::ToolDetailView::Ask(
            crate::app::state::AskDetailView::from_answered(questions),
        )),
        ToolExecutionResult::Edit { .. }
        | ToolExecutionResult::Write { .. }
        | ToolExecutionResult::Failed { .. } => {
            crate::app::state::ToolDetailView::file_detail(result_json)
        }
        // 子 agent 详情由 app 层从结果 JSON 解析（与历史恢复同一路径）。
        _ => None,
    }
}

/// 工具执行结局 → DB 结算记录（取消不带结果体，失败与成功都带规范化 JSON）。
pub(crate) fn db_result_of(outcome: &ToolOutcome) -> ToolCallResult {
    match outcome {
        ToolOutcome::Cancelled => ToolCallResult {
            status: ToolTerminalStatus::Cancelled,
            result_json: None,
            error_text: None,
        },
        ToolOutcome::Executed(execution) => ToolCallResult {
            status: if execution.is_failure() {
                ToolTerminalStatus::Failed
            } else {
                ToolTerminalStatus::Completed
            },
            result_json: crate::chat::tools::to_result_json(execution),
            error_text: failure_text(execution),
        },
    }
}

/// 工具执行结局 → 视图状态 + 摘要 + 错误码。
pub(crate) fn view_settlement(
    outcome: &ToolOutcome,
) -> (
    crate::app::state::ToolStatus,
    Option<crate::app::state::ToolSummary>,
    Option<String>,
) {
    use crate::app::state::ToolStatus;
    match outcome {
        ToolOutcome::Cancelled => (ToolStatus::Cancelled, None, None),
        ToolOutcome::Executed(execution) => {
            let summary = view_summary(&crate::chat::tools::to_summary(execution));
            if execution.is_failure() {
                (ToolStatus::Failed, Some(summary), failure_text(execution))
            } else {
                (ToolStatus::Completed, Some(summary), None)
            }
        }
    }
}

/// `chat::tools::ToolSummary` → `app` 层 `ToolSummary`（一次字段映射，保持单向依赖）。
///
/// 逐字段列出、**不写** `..Default::default()`：`ToolSummary` 新增字段时这里会直接
/// 编译失败，强制两份字段清单同步（漏字段正是 P1-1 那类回归的入口）。
pub(crate) fn view_summary(
    summary: &crate::chat::tools::ToolSummary,
) -> crate::app::state::ToolSummary {
    crate::app::state::ToolSummary {
        path: summary.path.clone(),
        range: summary.range,
        added: summary.added,
        removed: summary.removed,
        matches: summary.matches,
        pattern: summary.pattern.clone(),
        read_lines: summary.read_lines,
        total_lines: summary.total_lines,
        truncated: summary.truncated,
        partial: summary.partial,
        counts_known: summary.counts_known,
        simplified: summary.simplified,
        omitted_files: summary.omitted_files,
    }
}

/// 当前窗格消息里某次调用的参数 JSON（实时结算构造类型化详情用）。
pub(crate) fn tool_arguments(
    state: &AppState,
    key: &RequestKey,
    call_id: &str,
) -> Option<String> {
    state
        .pane_anywhere(key.pane)?
        .lx
        .chat
        .messages
        .iter()
        .rev()
        .find_map(|message| message.tool(call_id).map(|call| call.arguments_json.clone()))
}

/// 失败原因（给模型与 UI 的精简单行；不含文件内容与命令输出）。
///
/// 进程工具的命令失败（非零退出/超时/中断）也算失败，必须留下真实原因
/// （例如 `command exited with code 4`），不能让界面只显示 "Failed"。
fn failure_text(execution: &crate::chat::tools::ToolExecutionResult) -> Option<String> {
    match execution {
        crate::chat::tools::ToolExecutionResult::Failed { message, .. } => {
            Some(message.chars().take(512).collect())
        }
        crate::chat::tools::ToolExecutionResult::Process(result) => result
            .failure_message()
            .map(|message| message.chars().take(512).collect()),
        _ => None,
    }
}

/// 稳定状态字符串（只进 tracing，不当用户文案）。
fn status_text(status: crate::app::state::ToolStatus) -> &'static str {
    use crate::app::state::ToolStatus;
    match status {
        ToolStatus::InputStreaming => "input_streaming",
        ToolStatus::Pending => "pending",
        ToolStatus::Running => "running",
        ToolStatus::Completed => "completed",
        ToolStatus::Failed => "failed",
        ToolStatus::Cancelled => "cancelled",
        ToolStatus::Interrupted => "interrupted",
    }
}

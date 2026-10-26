//! 子 agent 的展示态更新：增量、工具状态、列表回投、懒加载、终态与面板滚动。
//!
//! 只写 `Lx.agents` 与父消息的 `reports`；不执行 IO、不发网络请求。
//! 所有定位都以 `child_session_id` / `agent_id` 为键，找不到即忽略（窗格已关、
//! 会话已切）。

use crate::app::state::{AppState, ChatMessageView, SubagentReportView, SubagentRun};
use crate::app::update::chat::view::view_message_of;
use crate::chat::events::{ChatDelta, StopReason};
use crate::db::chat::{ChatMessage, SubagentRecord, Usage};
use crate::layout::PaneId;

/// 找到持有该子会话的窗格。
fn pane_of_session(state: &AppState, child_session: &str) -> Option<PaneId> {
    state.all_pane_ids().into_iter().find(|pane| {
        state
            .pane_anywhere(*pane)
            .is_some_and(|target| target.lx.agents.by_session(child_session).is_some())
    })
}

/// spawn 回执：把一个运行记录插入窗格。
pub fn on_subagent_started(state: &mut AppState, pane: PaneId, run: SubagentRun) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.agents.runs.push(run);
    true
}

/// 子会话增量 → 对应 run.chat（找不到即忽略）。
pub fn on_subagent_delta(state: &mut AppState, child_session: &str, deltas: &[ChatDelta]) -> bool {
    let Some(pane) = pane_of_session(state, child_session) else {
        return false;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(run) = target.lx.agents.by_session_mut(child_session) else {
        return false;
    };
    let changed = apply_deltas(&mut run.chat, deltas);
    if changed {
        run.rows_dirty = true;
    }
    changed
}

/// 把一批增量应用到子 agent 的消息模型（与主聊天同一套方法）。
pub fn apply_deltas(chat: &mut crate::app::state::LxChat, deltas: &[ChatDelta]) -> bool {
    let mut changed = false;
    for delta in deltas {
        match delta {
            ChatDelta::Part(part) => {
                changed |= chat.append_stream_delta(part.kind, &part.text, part.metadata.clone());
            }
            ChatDelta::ReasoningEnded { item_id } => {
                changed |= chat.reasoning_ended(item_id.as_deref());
            }
            ChatDelta::ToolInputStarted {
                call_id,
                name,
                ordinal,
            } => {
                changed |= chat.start_tool_input(call_id, name, *ordinal);
            }
            ChatDelta::ToolInputDelta { call_id, chunk } => {
                changed |= chat.append_tool_input(call_id, chunk);
            }
            ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            } => {
                changed |= chat.end_tool_input(call_id, arguments_json);
            }
            ChatDelta::ResponseFinished { stop } => {
                changed |= chat.finish_response(*stop);
            }
            ChatDelta::Usage(_) | ChatDelta::Finished { .. } => {}
        }
    }
    changed
}

/// 工具 pending（子会话）。
pub fn on_subagent_tool_pending(state: &mut AppState, child_session: &str, call_id: &str) -> bool {
    let Some(pane) = pane_of_session(state, child_session) else {
        return false;
    };
    let Some(run) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.agents.by_session_mut(child_session))
    else {
        return false;
    };
    run.chat.mark_tool_pending(call_id)
}

/// 工具 running（子会话）。
pub fn on_subagent_tool_running(state: &mut AppState, child_session: &str, call_id: &str) -> bool {
    let Some(pane) = pane_of_session(state, child_session) else {
        return false;
    };
    let Some(run) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.agents.by_session_mut(child_session))
    else {
        return false;
    };
    run.chat.mark_tool_running(call_id)
}

/// 工具结算（子会话）：写终态、摘要、结果体与类型化详情。
pub fn on_subagent_tool_settled(
    state: &mut AppState,
    child_session: &str,
    call_id: &str,
    status: crate::app::state::ToolStatus,
    summary: Option<crate::app::state::ToolSummary>,
    result_json: Option<String>,
    error: Option<String>,
    detail: Option<crate::app::state::ToolDetailView>,
) -> bool {
    let Some(pane) = pane_of_session(state, child_session) else {
        return false;
    };
    let Some(run) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.agents.by_session_mut(child_session))
    else {
        return false;
    };
    let mut changed = run
        .chat
        .settle_tool_result(call_id, status, summary, result_json, error);
    if changed {
        run.rows_dirty = true;
    }
    if let Some(call) = run
        .chat
        .messages
        .iter_mut()
        .rev()
        .find(|message| message.role == "assistant")
        .and_then(|message| message.tool_mut(call_id))
        && call.detail.as_deref() != detail.as_ref()
    {
        call.detail = detail.map(Box::new);
        changed = true;
    }
    changed
}

/// 列表元数据回投（幂等：内容相同返回 false）。
pub fn set_subagent_list(
    state: &mut AppState,
    pane: PaneId,
    _parent_session: &str,
    records: &[SubagentRecord],
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let mut changed = false;
    for record in records {
        match target.lx.agents.by_agent_mut(&record.agent_id) {
            Some(run) => {
                // 只同步元数据；消息模型与滚动状态归本地。
                if run.status != record.status {
                    run.status = record.status.clone();
                    changed = true;
                }
                if run.completed_at_ms != record.completed_at_ms {
                    run.completed_at_ms = record.completed_at_ms;
                    changed = true;
                }
                if let Some(report) = record
                    .report_json
                    .as_deref()
                    .and_then(SubagentReportView::from_json)
                    && run.report.as_ref() != Some(&report)
                {
                    run.report = Some(report);
                    changed = true;
                }
            }
            None => {
                target.lx.agents.runs.push(SubagentRun::from_record(record));
                changed = true;
            }
        }
    }
    if changed {
        let runs = target.lx.agents.runs.len();
        if let Some(page) = target.lx.task.as_mut() {
            page.agents_window.clamp(runs);
        }
    }
    changed
}

/// 懒加载回投：把历史消息流填进对应 run（只在 `!loaded` 时覆盖，保留 live 数据）。
pub fn on_subagent_loaded(
    state: &mut AppState,
    pane: PaneId,
    agent_id: &str,
    messages: &[ChatMessage],
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(run) = target.lx.agents.by_agent_mut(agent_id) else {
        return false;
    };
    if run.loaded {
        return false;
    }
    run.chat.messages = messages
        .iter()
        .map(|message| view_message_of(message, &models))
        .collect();
    run.loaded = true;
    run.rows_dirty = true;
    true
}

/// 子 turn 终态 + 报告 → run 状态与 report。
pub fn on_subagent_finished(
    state: &mut AppState,
    child_session: &str,
    status: &str,
    report: Option<SubagentReportView>,
    usage: Usage,
) -> bool {
    let Some(pane) = pane_of_session(state, child_session) else {
        return false;
    };
    let Some(run) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.agents.by_session_mut(child_session))
    else {
        return false;
    };
    let reason = match status {
        "completed" => StopReason::Complete,
        "cancelled" => StopReason::Cancelled,
        _ => StopReason::Failed {
            code: "subagent failed".into(),
        },
    };
    run.chat.close_stream(&reason);
    run.rows_dirty = true;
    run.status = status.to_string();
    run.completed_at_ms = Some(crate::db::now_ms());
    run.input_tokens = usage.input;
    run.output_tokens = usage.output;
    if let Some(report) = report {
        run.report = Some(report);
    }
    true
}

/// 父消息上的汇报：在对应窗格的消息列表里定位报告行并挂 reports。
pub fn on_subagent_report(
    state: &mut AppState,
    pane: PaneId,
    message_id: &str,
    report: SubagentReportView,
    envelope: String,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if let Some(message) = target
        .lx
        .chat
        .messages
        .iter_mut()
        .find(|message| message.id == message_id)
    {
        if message.reports.contains(&report) {
            return false;
        }
        message.reports.push(report);
        return true;
    }
    // 报告行还不在这条窗格的时间线里（自动投递未必发生）：先补一条消息，
    // 让汇报块立刻可见；后续 `begin_report_turn` 只会在同一条上补 assistant。
    let sequence = target
        .lx
        .chat
        .messages
        .last()
        .map_or(0, |message| message.sequence + 1);
    let turn_index = target
        .lx
        .chat
        .messages
        .last()
        .map_or(0, |message| message.turn_index + 1);
    // 报告行是**内部 user 消息**：正文就是注入模型信封，UI 只画汇报块。
    target.lx.chat.messages.push(ChatMessageView {
        id: message_id.to_string(),
        sequence,
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index,
        shell: None,
        steps: vec![crate::app::state::AssistantStepView::Part {
            position: 0,
            part: crate::app::state::ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: envelope,
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
        reports: vec![report],
    });
    true
}

/// 报告行已提交并起了一轮：追加展示消息并启动网络流（不清草稿）。
pub fn on_report_committed(
    state: &mut AppState,
    pane: PaneId,
    committed: &crate::db::chat::TurnCommitted,
    report: SubagentReportView,
    envelope: String,
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
    target.lx.chat.revision = u64::try_from(committed.revision).unwrap_or(0);
    // 报告行可能已由 `on_subagent_report` 补进时间线：复用同一条，只补 assistant。
    if let Some(message) = target
        .lx
        .chat
        .messages
        .iter_mut()
        .find(|message| message.id == committed.user_message_id)
    {
        message.sequence = committed.user_sequence;
        message.turn_index = committed.turn_index;
        if !message.reports.contains(&report) {
            message.reports.push(report);
        }
        if message.text().is_empty() {
            message.steps = vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: crate::app::state::ChatPartView {
                    kind: crate::chat::events::PartKind::Text,
                    content: envelope,
                    provider_metadata_json: None,
                    started_at_ms: None,
                    ended_at_ms: None,
                    completed_at_ms: None,
                },
            }];
        }
    } else {
        target.lx.chat.messages.push(ChatMessageView {
            id: committed.user_message_id.clone(),
            sequence: committed.user_sequence,
            kind: "chat".into(),
            role: "user".into(),
            status: "complete".into(),
            turn_index: committed.turn_index,
            shell: None,
            steps: vec![crate::app::state::AssistantStepView::Part {
                position: 0,
                part: crate::app::state::ChatPartView {
                    kind: crate::chat::events::PartKind::Text,
                    content: envelope,
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
            reports: vec![report],
        });
    }
    let model_label = target
        .lx
        .chat
        .effective_selection(&models)
        .map(|selection| models.model_display_name(&selection.provider, &selection.model));
    target.lx.chat.messages.push(ChatMessageView {
        id: committed.assistant_message_id.clone(),
        sequence: committed.assistant_sequence,
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
    });
    target.lx.chat.follow_bottom = true;
    // 报告轮也需要独立代次：迟到增量/取消按 key 精确匹配。
    target.lx.chat.generation_counter = target.lx.chat.generation_counter.wrapping_add(1);
    let key = crate::app::state::RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation: target.lx.chat.generation_counter,
    };
    target.lx.chat.request = crate::app::state::RequestState::Streaming {
        key: key.clone(),
        assistant_id: committed.assistant_message_id.clone(),
    };
    state
        .effects
        .push(crate::app::effects::Effect::StartChatStream {
            key,
            assistant_id: committed.assistant_message_id.clone(),
        });
    // 报告轮不消费用户草稿。
    true
}

/// 详情区滚动（滚轮/PageUp/PageDown），follow 语义对齐主历史。
pub fn scroll_agents_detail(state: &mut AppState, pane: PaneId, delta: isize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    if page.tab != crate::app::state::TaskTab::Agents {
        return false;
    }
    let visible = page.agents_visible.max(1);
    let active = page.agents_window.active;
    let Some(run) = target.lx.agents.runs.get_mut(active) else {
        return false;
    };
    let max = run.total_rows.saturating_sub(visible);
    let current = run.scroll.min(max);
    let next = (current as isize)
        .saturating_add(delta)
        .clamp(0, max as isize) as usize;
    if next == run.scroll {
        return false;
    }
    run.scroll = next;
    run.follow = next >= max;
    true
}

/// `^d`：停止高亮项 → `Effect::StopSubagent`。
pub fn stop_selected_subagent(state: &mut AppState, pane: PaneId) -> bool {
    let agent_id = state
        .pane_anywhere(pane)
        .and_then(|target| {
            let page = target.lx.task.as_ref()?;
            page.selected_agent(&target.lx.agents)
        })
        .filter(|run| run.is_running())
        .map(|run| run.agent_id.clone());
    let Some(agent_id) = agent_id else {
        return false;
    };
    state
        .effects
        .push(crate::app::effects::Effect::StopSubagent { pane, agent_id });
    true
}

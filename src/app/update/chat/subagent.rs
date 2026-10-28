//! 子 agent 的展示态更新：增量、工具状态、列表回投、懒加载、终态与面板滚动。
//!
//! 只写 `Lx.agents` 与父消息的 `reports`；不执行 IO、不发网络请求。
//! 所有定位都以 `child_session_id` / `agent_id` 为键，找不到即忽略（窗格已关、
//! 会话已切）。

use crate::app::state::{
    AppState, AssistantStepView, CardTarget, ChatMessageView, CompactNoticeView,
    SubagentReportView, SubagentRun, ToolDetailView, ToolStatus,
};
use crate::app::update::chat::view::view_message_of;
use crate::chat::agent::{AgentRole, AgentStatusView};
use crate::chat::compact::AutoCompactNotice;
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
    // 迟到的 spawn 回执不得把旧会话的 run 塞进已经切走的新会话。
    if target
        .lx
        .chat
        .session_id
        .as_deref()
        .is_some_and(|current| current != run.parent_session_id)
    {
        return false;
    }
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

/// 子 agent 卡片状态回填：
/// - `spawn_agent` 标题状态跟对应 run 的 running/completed 走；
/// - 运行中的 `wait_agent` 目标状态用当前 run 快照实时填充。
///
/// spawn 的工具调用在子 agent 启动时就已经返回，不能拿工具状态冒充运行状态。
pub fn refresh_subagent_cards(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let runs: Vec<(
        String,
        AgentRole,
        String,
        String,
        Option<SubagentReportView>,
    )> = target
        .lx
        .agents
        .runs
        .iter()
        .map(|run| {
            (
                run.agent_id.clone(),
                run.role,
                run.title.clone(),
                run.status.clone(),
                run.report.clone(),
            )
        })
        .collect();
    // 历史会话不一定回投了 Agents 列表；父消息里的汇报同样带终态。
    let reports: Vec<SubagentReportView> = target
        .lx
        .chat
        .messages
        .iter()
        .flat_map(|message| &message.reports)
        .cloned()
        .collect();
    let mut changed = false;
    for message in &mut target.lx.chat.messages {
        for step in &mut message.steps {
            let AssistantStepView::Tool { call, .. } = step else {
                continue;
            };
            let Some(ToolDetailView::Subagent(view)) = call.detail.as_deref_mut() else {
                continue;
            };
            if call.name == "spawn_agent"
                && let Some(agent_id) = view.agent_id.as_deref()
                && let Some((title, status, report)) = runs
                    .iter()
                    .find(|(id, _, _, _, _)| id == agent_id)
                    .map(|(_, _, title, status, report)| {
                        (title.clone(), status.clone(), report.clone())
                    })
                    .or_else(|| {
                        reports
                            .iter()
                            .find(|report| report.agent_id == agent_id)
                            .map(|report| {
                                (
                                    report.title.clone(),
                                    report.status.clone(),
                                    Some(report.clone()),
                                )
                            })
                    })
            {
                if view.agent_status.as_deref() != Some(status.as_str()) {
                    view.agent_status = Some(status);
                    changed = true;
                }
                if view.title.as_deref().unwrap_or("").is_empty() && !title.is_empty() {
                    view.title = Some(title);
                    changed = true;
                }
                if view.report != report {
                    view.report = report;
                    changed = true;
                }
            }
            if call.name != "wait_agent"
                || !matches!(call.status, ToolStatus::Pending | ToolStatus::Running)
            {
                continue;
            }
            if view.all || view.targets.is_empty() {
                continue;
            }
            let agents: Vec<AgentStatusView> = view
                .targets
                .iter()
                .filter_map(|agent_id| {
                    runs.iter().find(|(id, _, _, _, _)| id == agent_id).map(
                        |(agent_id, role, title, status, report)| AgentStatusView {
                            agent_id: agent_id.clone(),
                            title: title.clone(),
                            role: *role,
                            status: status.clone(),
                            report: report.as_ref().map(|report| report.body.clone()).or_else(
                                || {
                                    reports
                                        .iter()
                                        .find(|report| report.agent_id == *agent_id)
                                        .map(|report| report.body.clone())
                                },
                            ),
                        },
                    )
                })
                .collect();
            if view.agents != agents {
                view.agents = agents;
                changed = true;
            }
        }
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
    parent_session: &str,
    records: &[SubagentRecord],
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 列表请求在途时用户可能已经 /resume 到别的会话；旧列表不能污染新会话。
    if target
        .lx
        .chat
        .session_id
        .as_deref()
        .is_some_and(|current| current != parent_session)
    {
        return false;
    }
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
    let focused = consume_agent_focus(&mut target.lx);
    let changed = changed || focused;
    // 历史主消息里的 spawn 卡片也要拿 run 状态。
    changed || refresh_subagent_cards(state, pane)
}

/// 打开 Agents 面板并定位到指定 agent；列表尚未回投时留下一次性定位请求。
pub fn open_agent_page(state: &mut AppState, pane: PaneId, agent_id: &str) -> bool {
    let (opened, session) = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        let Some(session) = target.lx.chat.session_id.clone() else {
            return false;
        };
        let lx = &mut target.lx;
        let page = lx
            .task
            .get_or_insert_with(crate::app::state::TaskPage::default);
        page.select_tab(crate::app::state::TaskTab::Agents);
        page.agents_focus = Some(agent_id.to_string());
        let focused = consume_agent_focus(lx);
        (focused || lx.task.is_some(), session)
    };
    // Agents 列表按需从 DB 回投；请求是幂等的。
    state
        .effects
        .push(crate::app::effects::Effect::ListSubagents {
            pane,
            parent_session: session,
        });
    if opened {
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    opened
}

/// 消费 `TaskPage.agents_focus`：找到对应 run 就切高亮并贴尾。
fn consume_agent_focus(lx: &mut crate::app::state::Lx) -> bool {
    let Some(agent_id) = lx.task.as_ref().and_then(|page| page.agents_focus.clone()) else {
        return false;
    };
    let Some(index) = lx
        .agents
        .runs
        .iter()
        .position(|run| run.agent_id == agent_id)
    else {
        return false;
    };
    let Some(page) = lx.task.as_mut() else {
        return false;
    };
    page.agents_focus = None;
    page.agents_window.active = index;
    page.agents_window.anchor = index;
    page.agents_window.viewport = None;
    if let Some(run) = lx.agents.runs.get_mut(index) {
        run.follow = true;
        run.scroll = usize::MAX;
    }
    true
}

/// 点击 Agents 列表某一行：切换高亮并把详情贴回最新消息。
pub fn select_task_agent(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    if page.tab != crate::app::state::TaskTab::Agents || index >= target.lx.agents.runs.len() {
        return false;
    }
    page.agents_window.active = index;
    page.agents_window.anchor = index;
    page.agents_window.viewport = None;
    page.agents_focus = None;
    if let Some(run) = target.lx.agents.runs.get_mut(index) {
        run.follow = true;
        run.scroll = usize::MAX;
    }
    crate::app::update::clear_lx_selection_for(state, pane);
    true
}

/// 切换 Agents 详情区某个可折叠块（工具 / 思考 / compact / 汇报）。
///
/// 详情区不参与主历史的视口锚定（它自己的 `run.scroll`/`follow` 由滚动逻辑维护），
/// 因此这里只改块状态并置 `rows_dirty`，下一帧重新量行数。
pub fn toggle_agent_card(state: &mut AppState, pane: PaneId, target: CardTarget) -> bool {
    let Some(target_pane) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target_pane.lx.task.as_ref() else {
        return false;
    };
    if page.tab != crate::app::state::TaskTab::Agents {
        return false;
    }
    if let CardTarget::OpenAgent { agent_id } = &target {
        let agent_id = agent_id.clone();
        return open_agent_page(state, pane, &agent_id);
    }
    let active = page.agents_window.active;
    let Some(run) = target_pane.lx.agents.runs.get_mut(active) else {
        return false;
    };
    let changed = match target {
        CardTarget::Tool {
            message_id,
            call_id,
            section,
        } => {
            if !run
                .chat
                .messages
                .iter()
                .any(|message| message.id == message_id && message.tool(&call_id).is_some())
            {
                return false;
            }
            run.chat.toggle_section(&message_id, &call_id, section);
            true
        }
        CardTarget::Thinking {
            message_index,
            step_index,
        } => {
            if run.chat.messages.get(message_index).is_none() {
                return false;
            }
            run.chat.toggle_reasoning(message_index, step_index);
            true
        }
        CardTarget::Shell {
            message_id,
            section,
        } => run
            .chat
            .toggle_section(&message_id, crate::app::state::SHELL_SLOT, section),
        CardTarget::Compact { checkpoint_id } => run.compact.toggle(&checkpoint_id),
        CardTarget::Report {
            message_id,
            agent_id,
            more,
        } => {
            let slot = format!("report:{agent_id}");
            if more {
                run.chat.expand_report_more(&message_id, &slot);
                true
            } else {
                let expanded = run.chat.toggle_detail(&message_id, &slot);
                if !expanded {
                    run.chat.clear_report_expand(&message_id, &slot);
                }
                true
            }
        }
        // 已在函数入口处理；此处只为穷尽匹配。
        CardTarget::OpenAgent { .. } => false,
    };
    if changed {
        run.rows_dirty = true;
        crate::app::update::clear_lx_selection_for(state, pane);
    }
    changed
}

/// 子 agent 运行时自动压缩：把摘要挂成 compact 检查点，详情区复用现有组件。
///
/// 自动压缩不写数据库，因此没有真实的消息 sequence 范围；`from_auto` 用
/// `auto · this run` 替代范围行。检查点插在当前时间线末尾，后续消息继续排在后面。
pub fn on_subagent_auto_compact(
    state: &mut AppState,
    child_session: &str,
    notice: &AutoCompactNotice,
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
    let sequence = run
        .chat
        .messages
        .iter()
        .map(|message| message.sequence)
        .chain(run.compact.checkpoints.iter().map(|item| item.sequence))
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let id = format!("auto-{}-{}", run.agent_id, run.compact.checkpoints.len());
    run.compact.checkpoints.push(CompactNoticeView::from_auto(
        id,
        sequence,
        notice.model_label.clone(),
        notice.summary.clone(),
        crate::db::now_ms(),
    ));
    run.rows_dirty = true;
    true
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
    run.chat.status.streaming = false;
    run.rows_dirty = true;
    run.status = status.to_string();
    run.completed_at_ms = Some(crate::db::now_ms());
    run.input_tokens = usage.input;
    run.output_tokens = usage.output;
    // 详情的统计行只在终态消息上出现；usage 从事件层回投到这里，补进消息本身。
    let message_status = match status {
        "completed" => "complete",
        "cancelled" => "cancelled",
        _ => "failed",
    };
    let duration_ms = run
        .completed_at_ms
        .map(|completed| completed.saturating_sub(run.started_at_ms).max(0));
    if let Some(message) = run
        .chat
        .messages
        .iter_mut()
        .rev()
        .find(|message| message.role == "assistant")
    {
        message.status = message_status.to_string();
        message.input_tokens = usage.input;
        message.output_tokens = usage.output;
        message.cache_read_tokens = usage.cache_read;
        message.duration_ms = duration_ms;
    }
    if let Some(report) = report {
        run.report = Some(report);
    }
    true
}

/// 报告批次已提交：加入报告消息 + assistant 流式占位，并启动网络流。
///
/// 报告消息只在后台自动轮出现；`wait_agent` 已消费的报告不会走到这里。
pub fn on_report_batch_committed(
    state: &mut AppState,
    pane: PaneId,
    committed: &crate::db::chat::TurnCommitted,
    reports: Vec<SubagentReportView>,
) -> bool {
    if reports.is_empty() {
        return false;
    }
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let envelope = format!(
        "[local runtime] subagent reports:\n{}",
        reports
            .iter()
            .map(crate::chat::agent::format_report_envelope)
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
    target.lx.chat.revision = u64::try_from(committed.revision).unwrap_or(0);
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
        reports,
    });
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
    target.lx.chat.main_stopped = false;
    target.lx.chat.request = crate::app::state::RequestState::Streaming {
        key: key.clone(),
        assistant_id: committed.assistant_message_id.clone(),
    };
    // 自动报告轮没有用户 Enter 的 `on_turn_committed`，这里主动点亮 loading；
    // 后续 delta/finish 的 refresh_lx_status 会继续维护。
    target.lx.chat.status.streaming = true;
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
    let active = page.agents_window.active;
    let Some(run) = target.lx.agents.runs.get_mut(active) else {
        return false;
    };
    let visible = run.visible.max(1);
    let max = run.total_rows.saturating_sub(visible);
    let current = if run.follow { max } else { run.scroll.min(max) };
    let next = (current as isize)
        .saturating_add(delta)
        .clamp(0, max as isize) as usize;
    let moved = next != current || (run.follow && next < max);
    if !moved {
        return false;
    }
    run.scroll = next;
    run.follow = next >= max;
    true
}

/// 直接设置 Agents 详情区顶部行（滚动条点击/拖拽）；语义与
/// [`super::set_lx_history_scroll`] 一致：贴底即恢复跟随。
pub fn set_task_agents_scroll(state: &mut AppState, pane: PaneId, offset: usize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    if page.tab != crate::app::state::TaskTab::Agents {
        return false;
    }
    let Some(run) = target.lx.agents.runs.get_mut(page.agents_window.active) else {
        return false;
    };
    let max = run.total_rows.saturating_sub(run.visible.max(1));
    let clamped = offset.min(max);
    let before = run.scroll;
    let was_following = run.follow;
    run.scroll = clamped;
    run.follow = clamped >= max;
    before != clamped || was_following != run.follow
}

/// Agents 面板 `^d`：停止当前高亮的 running 子 agent。
///
/// Esc 只停主轮与正在等待的 wait 回执，所有 running 子 agent 继续跑；
/// 这里按选中项单独停止，partial transcript 留在 Agents 面板，不自动重跑、不起报告轮。
pub fn stop_selected_subagent(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let Some(page) = target.lx.task.as_ref() else {
        return false;
    };
    let Some(run) = page.selected_agent(&target.lx.agents) else {
        return false;
    };
    if !run.is_running() {
        crate::app::update::show_lx_toast(
            state,
            pane,
            crate::app::toast::Toast::new(
                crate::app::toast::ToastKind::Default,
                crate::ui::text::LX_TASK_SELECTED_AGENT_NOT_RUNNING,
                Some(pane),
                std::time::Instant::now(),
            ),
        );
        return false;
    }
    let agent_id = run.agent_id.clone();
    let Some(session_id) = target.lx.chat.session_id.clone() else {
        return false;
    };
    state
        .effects
        .push(crate::app::effects::Effect::CancelSubagents {
            session_id,
            agent_id: Some(agent_id),
        });
    crate::app::update::show_lx_toast(
        state,
        pane,
        crate::app::toast::Toast::new(
            crate::app::toast::ToastKind::Default,
            crate::ui::text::stopping_subagents(1),
            Some(pane),
            std::time::Instant::now(),
        ),
    );
    true
}

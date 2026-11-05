//! 子 agent 的展示态更新：增量、工具状态、列表回投、懒加载、终态与面板滚动。
//!
//! 只写 `Lx.agents` 与父消息的 `reports`；不执行 IO、不发网络请求。
//! 所有定位都以 `child_session_id` / `agent_id` 为键，找不到即忽略（窗格已关、
//! 会话已切）。

use crate::app::state::{
    AppState, CardTarget, ChatMessageView, CompactNoticeView, SubagentReportView, SubagentRun,
};
use crate::app::update::chat::view::view_message_of;
use crate::chat::compact::AutoCompactNotice;
use crate::chat::events::{ChatDelta, StopReason};
use crate::db::chat::{ChatMessage, SubagentRecord, Usage};
use crate::layout::PaneId;

pub use super::subagent_cards::refresh_subagent_cards;

/// 找到持有该子会话的窗格。
fn pane_of_session(state: &AppState, child_session: &str) -> Option<PaneId> {
    state.all_pane_ids().into_iter().find(|pane| {
        state
            .pane_anywhere(*pane)
            .is_some_and(|target| target.lx.agents.by_session(child_session).is_some())
    })
}

/// spawn 回执：把一个运行记录插入窗格。
///
/// 孙 agent 的 `parent_session_id` 指向 depth-1 run 的子会话，而不是根窗格的
/// `chat.session_id`；窗格持有该父 run 时同样视为同一棵会话树。
pub fn on_subagent_started(state: &mut AppState, pane: PaneId, mut run: SubagentRun) -> bool {
    let project_root = state.project_root_for_pane(pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    // 迟到的 spawn 回执不得把旧会话的 run 塞进已经切走的新会话。
    let switched_away = target
        .lx
        .chat
        .session_id
        .as_deref()
        .is_some_and(|current| current != run.parent_session_id);
    let owns_parent = target
        .lx
        .agents
        .by_session(&run.parent_session_id)
        .is_some();
    if switched_away && !owns_parent {
        return false;
    }
    let selected = current_selected_agent_id(&target.lx);
    run.chat.set_project_root(project_root);
    target.lx.agents.runs.push(run);
    restore_selected_task_agent(&mut target.lx, selected.as_deref());
    refresh_status(state, pane);
    true
}

fn current_selected_agent_id(lx: &crate::app::state::Lx) -> Option<String> {
    lx.task
        .as_ref()
        .and_then(|page| page.selected_agent_index(&lx.agents))
        .and_then(|index| lx.agents.runs.get(index))
        .map(|run| run.agent_id.clone())
}

/// 按树形前序重排 run，并让筛选后的列表高亮继续指向修改前捕获的 `selected_agent_id`；
/// 若目标已被当前筛选隐藏则钳制到合法范围。
pub fn restore_selected_task_agent(
    lx: &mut crate::app::state::Lx,
    selected_agent_id: Option<&str>,
) {
    lx.agents.sort_runs_tree_order();
    let Some(page) = lx.task.as_mut() else {
        return;
    };
    let filtered = lx.agents.filtered_indices(page.agent_filter);
    if let Some(selected) = selected_agent_id
        && let Some(index) = lx
            .agents
            .runs
            .iter()
            .position(|run| run.agent_id == selected)
        && let Some(position) = filtered.iter().position(|real| *real == index)
    {
        page.agents_window.active = position;
        page.agents_window.anchor = position;
        page.agents_window.viewport = None;
    } else {
        page.agents_window.clamp(filtered.len());
    }
}

/// 同步底边框的 blocked 计数（子 agent 状态变化后调用）。
fn refresh_status(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let blocked = target.lx.agents.blocked_count();
    if target.lx.chat.status.blocked_subagents == blocked {
        return false;
    }
    target.lx.chat.status.blocked_subagents = blocked;
    true
}

/// 子会话增量 → 对应 run.chat（找不到即忽略）。
pub fn on_subagent_delta(state: &mut AppState, child_session: &str, deltas: &[ChatDelta]) -> bool {
    let Some(pane) = pane_of_session(state, child_session) else {
        return false;
    };
    let project_root = state.project_root_for_pane(pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(run) = target.lx.agents.by_session_mut(child_session) else {
        return false;
    };
    if project_root.is_some() {
        run.chat.project_root.clone_from(&project_root);
    }
    let changed = apply_deltas(&mut run.chat, deltas);
    if changed {
        run.rows_dirty = true;
    }
    // 子 agent 的实时动作（在途工具 / 思考 / 正文）挂在父消息的 spawn 卡片上：
    // 每次增量都要同步，否则要等下一次主轮事件才会刷新。
    changed | refresh_subagent_cards(state, pane)
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
    let changed = run.chat.mark_tool_pending(call_id);
    changed | refresh_subagent_cards(state, pane)
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
    let changed = run.chat.mark_tool_running(call_id);
    changed | refresh_subagent_cards(state, pane)
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
    let project_root = state.project_root_for_pane(pane);
    let Some(run) = state
        .pane_mut_anywhere(pane)
        .and_then(|target| target.lx.agents.by_session_mut(child_session))
    else {
        return false;
    };
    if project_root.is_some() {
        run.chat.project_root.clone_from(&project_root);
    }
    let root = run.chat.project_root.clone();
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
        && !(detail.is_none()
            && call
                .detail
                .as_deref()
                .is_some_and(crate::app::state::ToolDetailView::preserves_on_empty_settle))
        && call.detail.as_deref() != detail.as_ref()
    {
        call.detail = detail.map(Box::new);
        if let Some(root) = root.as_deref() {
            call.relativize_paths(root);
        }
        changed = true;
    }
    changed | refresh_subagent_cards(state, pane)
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
    let selected = current_selected_agent_id(&target.lx);
    let mut changed = false;
    for record in records {
        match target.lx.agents.by_agent_mut(&record.agent_id) {
            Some(run) => {
                // 只同步元数据；消息模型与滚动状态归本地。
                if run.nickname != record.nickname && record.nickname.is_some() {
                    run.nickname = record.nickname.clone();
                    changed = true;
                }
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
        restore_selected_task_agent(&mut target.lx, selected.as_deref());
    }
    let focused = consume_agent_focus(&mut target.lx);
    let changed = changed || focused;
    // 历史主消息里的 spawn 卡片也要拿 run 状态。
    let cards = refresh_subagent_cards(state, pane);
    refresh_status(state, pane);
    changed || cards
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
    let Some(index) = lx.agents.runs.iter().position(|run| {
        crate::chat::agent::matches_agent_target(
            &run.agent_id,
            run.nickname.as_deref(),
            Some(run.agent_path.as_str()),
            crate::chat::agent::ROOT_AGENT_PATH,
            &agent_id,
        )
    }) else {
        return false;
    };
    let Some(page) = lx.task.as_ref() else {
        return false;
    };
    // 目标被当前筛选隐藏时回退到 all，保证定位后真的可见。
    let mut filter = page.agent_filter;
    let mut position = lx
        .agents
        .filtered_indices(filter)
        .iter()
        .position(|real| *real == index);
    if position.is_none() && filter != crate::app::state::AgentFilter::All {
        filter = crate::app::state::AgentFilter::All;
        position = lx
            .agents
            .filtered_indices(filter)
            .iter()
            .position(|real| *real == index);
    }
    let Some(position) = position else {
        return false;
    };
    let agent_id = lx.agents.runs[index].agent_id.clone();
    let Some(page) = lx.task.as_mut() else {
        return false;
    };
    page.agents_focus = None;
    page.agent_filter = filter;
    if page
        .agent_reply_target
        .as_ref()
        .is_some_and(|(target_id, _)| target_id != &agent_id)
    {
        page.agent_reply = None;
        page.agent_reply_target = None;
    }
    page.agents_window.active = position;
    page.agents_window.anchor = position;
    page.agents_window.viewport = None;
    if let Some(run) = lx.agents.runs.get_mut(index) {
        run.follow = true;
        run.scroll = usize::MAX;
        run.touch_access();
    }
    lx.agents.evict_excess_idle_chats(Some(&agent_id));
    true
}

/// 点击 Agents 列表某一行：切换高亮并把详情贴回最新消息。
///
/// `index` 是 `agents.runs` 的真实下标；筛选视图下先映射到可见位置。
pub fn select_task_agent(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let position = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        let Some(page) = target.lx.task.as_ref() else {
            return false;
        };
        if page.tab != crate::app::state::TaskTab::Agents {
            return false;
        }
        let Some(position) = target
            .lx
            .agents
            .filtered_indices(page.agent_filter)
            .iter()
            .position(|real| *real == index)
        else {
            return false;
        };
        position
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let clicked_agent_id = target.lx.agents.runs.get(index).map(|r| r.agent_id.clone());
    let Some(page) = target.lx.task.as_mut() else {
        return false;
    };
    if page
        .agent_reply_target
        .as_ref()
        .is_some_and(|(target_id, _)| Some(target_id) != clicked_agent_id.as_ref())
    {
        page.agent_reply = None;
        page.agent_reply_target = None;
    }
    page.agents_window.active = position;
    page.agents_window.anchor = position;
    page.agents_window.viewport = None;
    page.agents_focus = None;
    if let Some(run) = target.lx.agents.runs.get_mut(index) {
        run.follow = true;
        run.scroll = usize::MAX;
    }
    ensure_agent_loaded(state, pane, index);
    crate::app::update::clear_lx_selection_for(state, pane);
    true
}

/// 选中 run 后的驻留维护：已加载则刷新访问序，未加载则请求从 DB 懒加载。
fn ensure_agent_loaded(state: &mut AppState, pane: PaneId, index: usize) {
    let pending = {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return;
        };
        let Some(run) = target.lx.agents.runs.get_mut(index) else {
            return;
        };
        let agent_id = run.agent_id.clone();
        let child_session = run.child_session_id.clone();
        let loaded = run.loaded;
        if loaded {
            run.touch_access();
        }
        target.lx.agents.evict_excess_idle_chats(Some(&agent_id));
        (!loaded).then_some((agent_id, child_session))
    };
    if let Some((agent_id, child_session)) = pending {
        state
            .effects
            .push(crate::app::effects::Effect::LoadSubagentSession {
                pane,
                agent_id,
                child_session,
            });
    }
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
    let Some(active) = page.selected_agent_index(&target_pane.lx.agents) else {
        return false;
    };
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
        CardTarget::ModelSwitch { .. } => false,
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
        CardTarget::UserMessage { message_id } => {
            run.chat.toggle_user_message_expanded(&message_id)
        }
        CardTarget::UserPaste {
            message_id,
            paste_id,
            occurrence,
        } => {
            run.chat
                .toggle_user_paste(&message_id, paste_id, occurrence);
            true
        }
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
    let project_root = state.project_root_for_pane(pane);
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
    run.chat.set_project_root(project_root);
    run.loaded = true;
    run.rows_dirty = true;
    run.touch_access();
    target.lx.agents.evict_excess_idle_chats(Some(agent_id));
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
    let protected = state
        .pane_anywhere(pane)
        .and_then(|target| {
            target.lx.task.as_ref().and_then(|page| {
                page.selected_agent_index(&target.lx.agents)
                    .and_then(|index| target.lx.agents.runs.get(index))
            })
        })
        .map(|run| run.agent_id.clone());
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
    run.touch_access();
    let _ = run;
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target
            .lx
            .agents
            .evict_excess_idle_chats(protected.as_deref());
        restore_selected_task_agent(&mut target.lx, protected.as_deref());
    }
    refresh_status(state, pane);
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
        attachments: Vec::new(),
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
        attachments: Vec::new(),
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
    let Some(active) = page.selected_agent_index(&target.lx.agents) else {
        return false;
    };
    let Some(run) = target.lx.agents.runs.get_mut(active) else {
        return false;
    };
    let visible = run.visible.max(1);
    let max = run.total_rows.saturating_sub(visible);
    let current = if run.follow { max } else { run.scroll.min(max) };
    let mut next = (current as isize)
        .saturating_add(delta)
        .clamp(0, max as isize) as usize;
    if delta > 0 && next.saturating_add(1) >= max {
        next = max;
    }
    let next_follow = next >= max;
    let moved = next != current || run.follow != next_follow;
    if !moved {
        return false;
    }
    run.scroll = next;
    run.follow = next_follow;
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
    let Some(active) = page.selected_agent_index(&target.lx.agents) else {
        return false;
    };
    let Some(run) = target.lx.agents.runs.get_mut(active) else {
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

/// Agents 面板 `^d`：停止当前高亮的 running 子 agent（事件层沿 `agent_path` 级联停止后代）。
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
    // 选中的父 run 已结束但仍有后代在跑时，停止操作要沿路径级联下去。
    let has_running_descendant = target.lx.agents.runs.iter().any(|other| {
        other.is_running()
            && crate::chat::agent::is_strict_descendant_path(&run.agent_path, &other.agent_path)
    });
    if !run.is_running() && !has_running_descendant {
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

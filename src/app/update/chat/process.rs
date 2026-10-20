//! 进程事件与工具卡片交互的纯状态转换（无 IO、无时间戳重排）。
//!
//! 三条路径共用同一份类型化视图：
//!
//! 1. **结算**：`FinishTool` 落库成功后由事件层调用 [`settle_tool_detail`]；
//! 2. **后台更新**：进程输出/退出事件经 [`apply_process_output`] / [`apply_process_exit`]
//!    只改数据、不新增时间线步骤，也不触发新的模型请求；
//! 3. **历史重载**：`view::tool_view_of` 从结果 JSON 生成同一份 `ToolDetailView`。
//!
//! 渲染层只读这些视图：不读数据库、不读进程、不解析巨型 JSON。

use crate::app::state::{AppState, RequestKey, ToolStatus, ToolSummary};
use crate::chat::process::ProcessUiEvent;
use crate::layout::PaneId;
use crate::ui::text;

use super::refresh_lx_status;

/// 工具结算后的类型化详情写入（`FinishTool` ack 第 ② 步）。
///
/// `detail` 来自刚落库的规范化结果；进程工具与计划工具各走自己的视图。
pub fn settle_tool_detail(
    state: &mut AppState,
    key: &RequestKey,
    call_id: &str,
    detail: Option<crate::app::state::ToolDetailView>,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    let Some(message) = target.lx.chat.streaming_message_mut() else {
        return false;
    };
    let Some(call) = message.tool_mut(call_id) else {
        return false;
    };
    if call.detail.as_deref() == detail.as_ref() {
        return false;
    }
    call.detail = detail.map(Box::new);
    true
}

/// 后台输出到达：刷新原命令卡片的尾部输出与状态（**不**新增时间线步骤）。
///
/// 只更新 `exec_command` 对应的卡片；`write_stdin` 的历史卡片保留当时快照。
/// 返回是否发生变化（供事件循环安排一次重绘）。
pub fn apply_process_output(
    state: &mut AppState,
    session_id: u64,
    origin_call_id: &str,
    snapshot: crate::chat::process::ProcessSnapshot,
) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(message) = target.lx.chat.streaming_message_mut() else {
            continue;
        };
        let Some(call) = message.tool_mut(origin_call_id) else {
            continue;
        };
        let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref_mut()
        else {
            continue;
        };
        if view.process_session_id != Some(session_id) {
            continue;
        }
        // 逐字段比较，避免用「无变化」的输出触发额外重绘。
        let preview = crate::app::state::lx::chat::tool_view::preview_lines(&snapshot.tail);
        let next_state = crate::app::state::ProcessStateView::from(snapshot.state.clone());
        if view.output_preview == preview
            && view.state == next_state
            && view.dropped_bytes == snapshot.dropped_bytes
            && view.output_tail == snapshot.tail
        {
            continue;
        }
        view.output_preview = preview;
        view.state = next_state;
        view.dropped_bytes = snapshot.dropped_bytes;
        view.output_tail.clone_from(&snapshot.tail);
        view.ended_at_ms = snapshot.ended_at_ms;
        if view.started_at_ms.is_none() {
            view.started_at_ms = Some(snapshot.started_at_ms);
        }
        changed = true;
    }
    if changed {
        for pane in state.all_pane_ids() {
            refresh_lx_status(state, pane);
        }
    }
    changed
}

/// 进程终态：更新卡片状态，并把 FAILED / STOPPED 的语义写进工具步骤。
///
/// 工具调用的**持久化状态**不改写：只有进程记录变化，历史回执保持原样。
pub fn apply_process_exit(
    state: &mut AppState,
    event: &ProcessUiEvent,
    snapshot: Option<crate::chat::process::ProcessSnapshot>,
) -> bool {
    let ProcessUiEvent::Exited {
        session_id,
        origin_call_id,
        state: terminal,
        exit_code,
    } = event
    else {
        return false;
    };
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        let Some(message) = target.lx.chat.streaming_message_mut() else {
            continue;
        };
        let Some(call) = message.tool_mut(origin_call_id) else {
            continue;
        };
        let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref_mut()
        else {
            continue;
        };
        if view.process_session_id != Some(*session_id) {
            continue;
        }
        let next = crate::app::state::ProcessStateView::from(terminal.clone());
        if view.state == next {
            continue;
        }
        view.state = next;
        if let Some(snapshot) = &snapshot {
            view.output_preview =
                crate::app::state::lx::chat::tool_view::preview_lines(&snapshot.tail);
            view.output_tail.clone_from(&snapshot.tail);
            view.dropped_bytes = snapshot.dropped_bytes;
            view.ended_at_ms = snapshot.ended_at_ms;
        }
        // 只有**进程级失败**（超时 / 中断 / 启动失败）才在步骤上留原因。
        //
        // 非零退出不在此列：它是命令跑完后的正常结果（`grep` 无匹配 = 1、
        // `diff` 有差异 = 1），写进 `error` 会被渲染成红色失败标记。
        // 退出码仍然显示在卡片标题上（`exit 1`），信息不丢。
        if next.is_process_failure() && call.status == ToolStatus::Completed {
            call.error = Some(text::agent::proc_state_label(next).to_string());
        }
        let _ = exit_code;
        changed = true;
    }
    if changed {
        for pane in state.all_pane_ids() {
            refresh_lx_status(state, pane);
        }
    }
    changed
}

/// 切换某个工具卡片的详情展开状态；`call_id` 未知时返回 false。
///
/// 与 [`super::toggle_thinking_at`] 同一口径：先锚定视口再切换，否则跟随底部时
/// 展开后界面会跳回历史最顶部。
pub fn toggle_tool_detail(
    state: &mut AppState,
    pane: PaneId,
    message_id: &str,
    call_id: &str,
) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if !target
        .lx
        .chat
        .messages
        .iter()
        .any(|message| message.id == message_id && message.tool(call_id).is_some())
    {
        return false;
    }
    super::pin_lx_viewport(state, pane);
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.toggle_detail(message_id, call_id);
    true
}

/// `update_plan` 落库成功后的计划提交：**只有这一步成功**才更新当前计划。
///
/// 失败、中断或落库失败的更新都不覆盖最后成功的快照；历史计划卡片各自保留
/// 自己那次提交的视图（倒改历史会让用户以为旧步骤也被改了）。
pub fn commit_plan(
    state: &mut AppState,
    pane: PaneId,
    snapshot: &crate::chat::plan::PlanSnapshot,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let next = crate::app::state::PlanSnapshotView::from_snapshot(snapshot);
    if target.lx.chat.plan.as_ref() == Some(&next) {
        return false;
    }
    target.lx.chat.plan = Some(next);
    true
}

/// 从最后一条**成功**的 `update_plan` 结果恢复当前计划（会话重载路径）。
///
/// 失败、pending、interrupted 的更新都不覆盖最后成功的计划：只接受 `completed`
/// 且有合法结果体的调用，并按 revision 取最新。返回是否有计划写入。
pub fn restore_plan(
    state: &mut AppState,
    pane: PaneId,
    messages: &[crate::app::state::ChatMessageView],
) -> bool {
    let mut latest: Option<crate::app::state::PlanSnapshotView> = None;
    for message in messages {
        for call in message.tools() {
            if call.name != "update_plan" || call.status != ToolStatus::Completed {
                continue;
            }
            let Some(snapshot) = call
                .result_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
                .and_then(|value| crate::chat::plan::PlanSnapshot::from_json(&value))
            else {
                continue;
            };
            let next = crate::app::state::PlanSnapshotView::from_snapshot(&snapshot);
            latest = Some(match latest {
                Some(previous) if previous.revision > next.revision => previous,
                _ => next,
            });
        }
    }
    let Some(next) = latest else {
        return false;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.plan.as_ref() == Some(&next) {
        return false;
    }
    target.lx.chat.plan = Some(next);
    true
}

/// 键盘 `^o`：切换当前选中工具详情；未选中时选最近一张可展开卡片。
pub fn toggle_selected_tool_detail(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let selected = target.lx.chat.selected_tool.clone();
    let target_key = selected.or_else(|| latest_expandable(&target.lx.chat));
    let Some((message_id, call_id)) = target_key else {
        return false;
    };
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.selected_tool = Some((message_id.clone(), call_id.clone()));
    }
    toggle_tool_detail(state, pane, &message_id, &call_id)
}

/// Alt+↑/↓：按时间线选择上一/下一张可展开卡片（保留草稿与光标）。
pub fn select_adjacent_tool(state: &mut AppState, pane: PaneId, forward: bool) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let cards = expandable_cards(&target.lx.chat);
    if cards.is_empty() {
        return false;
    }
    let current = target.lx.chat.selected_tool.clone();
    let index = current
        .as_ref()
        .and_then(|selected| cards.iter().position(|card| card == selected));
    let next = match (index, forward) {
        (Some(index), true) => (index + 1).min(cards.len() - 1),
        (Some(index), false) => index.saturating_sub(1),
        (None, true) => 0,
        (None, false) => cards.len() - 1,
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.selected_tool = Some(cards[next].clone());
    true
}

/// 全部可展开的工具卡片（按时间线顺序）。
pub fn expandable_cards(chat: &crate::app::state::LxChat) -> Vec<(String, String)> {
    chat.messages
        .iter()
        .filter(|message| message.role == "assistant")
        .flat_map(|message| {
            message
                .tools()
                .filter(|call| is_expandable(call))
                .map(move |call| (message.id.clone(), call.call_id.clone()))
        })
        .collect()
}

/// 最近一张可展开卡片。
fn latest_expandable(chat: &crate::app::state::LxChat) -> Option<(String, String)> {
    expandable_cards(chat).pop()
}

/// 是否有可展开的详情（进程/计划工具有类型化视图；文件工具有摘要或结果体）。
pub fn is_expandable(call: &crate::app::state::ToolCallView) -> bool {
    if call.detail.is_some() {
        return true;
    }
    let has_summary = call
        .summary
        .as_ref()
        .is_some_and(|summary: &ToolSummary| !summary.is_empty());
    has_summary || call.result_json.is_some()
}

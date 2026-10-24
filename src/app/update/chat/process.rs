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
//!
//! **卡片定位不按「第几条消息」猜**：进程事件带 `origin_call_id`，而 `call_id` 在整个
//! 会话内唯一（DB 唯一约束）。早先的实现只在**最后一条 streaming 助手消息**里找卡片，
//! 一旦本轮结束（消息转 `complete`）后台事件就再也找不到它，标题上的 `Running · 4m 44s`
//! 会一直走到进程被杀——这就是用户看到的「关掉了还在计时」。

use crate::app::state::{AppState, ChatMessageView, RequestKey, ToolStatus};
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

/// 遍历所有窗格里**全部**消息的工具卡片。
///
/// 进程卡片可以在本轮结束后继续被后台事件更新，所以这里不能用
/// [`crate::app::state::lx::chat::LxChat::streaming_message_mut`]——那条捷径只在流式
/// 期间的“最后一条”里找，轮次一结束就失效（历史 bug，见模块文档）。
fn for_each_tool_mut(
    state: &mut AppState,
    call_id: &str,
    mut visit: impl FnMut(&mut crate::app::state::ToolCallView) -> bool,
) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        for message in &mut target.lx.chat.messages {
            let Some(call) = message.tool_mut(call_id) else {
                continue;
            };
            changed |= visit(call);
        }
    }
    changed
}

/// 后台输出到达：刷新原命令卡片的尾部输出与状态（**不**新增时间线步骤）。
///
/// 只更新 `exec_command` 的 `Command` 视图；`write_stdin` 的历史卡片保留当时快照
/// （那是一次调用的事实记录，不该被后续输出追改）。
///
/// 返回是否发生变化（供事件循环安排一次重绘）。
pub fn apply_process_output(
    state: &mut AppState,
    session_id: u64,
    origin_call_id: &str,
    snapshot: crate::chat::process::ProcessSnapshot,
) -> bool {
    let changed = for_each_tool_mut(state, origin_call_id, |call| {
        let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref_mut()
        else {
            return false;
        };
        if view.process_session_id != Some(session_id) {
            return false;
        }
        // 逐字段比较，避免用「无变化」的输出触发额外重绘。
        let preview = crate::app::state::lx::chat::tool_view::preview_lines(&snapshot.tail);
        let next_state = crate::app::state::ProcessStateView::from(snapshot.state.clone());
        if view.output_preview == preview
            && view.state == next_state
            && view.dropped_bytes == snapshot.dropped_bytes
            && view.output_tail == snapshot.tail
        {
            return false;
        }
        view.output_preview = preview;
        view.state = next_state;
        view.dropped_bytes = snapshot.dropped_bytes;
        view.output_tail.clone_from(&snapshot.tail);
        view.ended_at_ms = snapshot.ended_at_ms;
        if view.started_at_ms.is_none() {
            view.started_at_ms = Some(snapshot.started_at_ms);
        }
        true
    });
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
///
/// 与 [`apply_process_output`] 一样按 `call_id` 扫全部消息：进程经常在本轮结束之后
/// 才退出，只认流式末条消息会让卡片永远停在 `Running`。
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
    let changed = for_each_tool_mut(state, origin_call_id, |call| {
        let Some(crate::app::state::ToolDetailView::Command(view)) = call.detail.as_deref_mut()
        else {
            return false;
        };
        if view.process_session_id != Some(*session_id) {
            return false;
        }
        let next = crate::app::state::ProcessStateView::from(terminal.clone());
        if view.state == next {
            return false;
        }
        view.state = next;
        if let Some(snapshot) = &snapshot {
            view.output_preview =
                crate::app::state::lx::chat::tool_view::preview_lines(&snapshot.tail);
            view.output_tail.clone_from(&snapshot.tail);
            view.dropped_bytes = snapshot.dropped_bytes;
            view.ended_at_ms = snapshot.ended_at_ms;
            if view.started_at_ms.is_none() {
                view.started_at_ms = Some(snapshot.started_at_ms);
            }
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
        true
    });
    if changed {
        for pane in state.all_pane_ids() {
            refresh_lx_status(state, pane);
        }
    }
    changed
}

/// `write_stdin` 卡片的本轮快照随进程终态收敛。
///
/// `write_stdin` 的历史卡片保留**当次调用**的输出（那是一次事实记录），但它记录的
/// `state` 是「这一刻进程在跑」，进程结束后必须改成终态——否则 `Send input` 卡片会
/// 永远显示 `Running`，跟第一张卡片一样停不下来。
///
/// 与 [`apply_process_exit`] 分开：这里只改状态、不动输出正文。
pub fn apply_interaction_exit(
    state: &mut AppState,
    session_id: u64,
    terminal: &crate::chat::process::ProcessState,
) -> bool {
    let next = crate::app::state::ProcessStateView::from(terminal.clone());
    if !next.is_terminal() {
        return false;
    }
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        for message in &mut target.lx.chat.messages {
            for step in &mut message.steps {
                let crate::app::state::AssistantStepView::Tool { call, .. } = step else {
                    continue;
                };
                let Some(crate::app::state::ToolDetailView::Interaction(view)) =
                    call.detail.as_deref_mut()
                else {
                    continue;
                };
                if view.process_session_id != session_id || view.state.is_terminal() {
                    continue;
                }
                view.state = next;
                changed = true;
            }
        }
    }
    if changed {
        for pane in state.all_pane_ids() {
            refresh_lx_status(state, pane);
        }
    }
    changed
}

/// 按**运行实例的真实快照**校正某个进程的全部卡片（结算后立即调用）。
///
/// 与 [`apply_interaction_exit`] 的区别：那个只在**终态事件**到达时生效，而
/// `write_stdin` 卡片完全可能在进程早已退出之后才结算——它记下一个 `running`
/// 快照，此后不会再有任何事件去改它，卡片就永远显示 `Running`。
///
/// 这里在结算当下按会话内运行实例的真实状态再校正一次：终态覆盖，运行态也覆盖
/// （快照比结果体更新）。只改 `state`/`exit_code`，不动输出正文——输出是那次调用
/// 的事实记录。
pub fn apply_process_snapshot(
    state: &mut AppState,
    process_session_id: u64,
    snapshot: &crate::chat::process::ProcessSnapshot,
) -> bool {
    let next = crate::app::state::ProcessStateView::from(snapshot.state.clone());
    let mut changed = false;
    for pane in state.all_pane_ids() {
        let Some(target) = state.pane_mut_anywhere(pane) else {
            continue;
        };
        for message in &mut target.lx.chat.messages {
            for step in &mut message.steps {
                let crate::app::state::AssistantStepView::Tool { call, .. } = step else {
                    continue;
                };
                match call.detail.as_deref_mut() {
                    Some(crate::app::state::ToolDetailView::Command(view)) => {
                        if view.process_session_id != Some(process_session_id) || view.state == next
                        {
                            continue;
                        }
                        view.state = next;
                        changed = true;
                    }
                    Some(crate::app::state::ToolDetailView::Interaction(view)) => {
                        if view.process_session_id != process_session_id || view.state == next {
                            continue;
                        }
                        view.state = next;
                        if let Some(code) = snapshot.exit_code
                            && view.exit_code.is_none()
                        {
                            view.exit_code = Some(code);
                        }
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
    }
    if changed {
        for pane in state.all_pane_ids() {
            refresh_lx_status(state, pane);
        }
    }
    changed
}

/// 历史重载：用 `agent_process` 的真实记录校正卡片状态。
///
/// 结果 JSON 记的是**当次调用结算那一刻**的进程状态（`running` 是常态），
/// 而进程之后可能早已退出、被停止，或在 lx 退出时被标成 `interrupted`。
/// 只按结果 JSON 回放会让历史卡片永远停在 `Running`——正是用户看到的
/// 「关掉了还在计时」。这里用进程表的终态覆盖它。
///
/// 返回是否有卡片被改动。
pub fn reconcile_process_records(
    state: &mut AppState,
    pane: PaneId,
    records: &[crate::db::chat::ProcessRecord],
) -> bool {
    if records.is_empty() {
        return false;
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let mut changed = false;
    for message in &mut target.lx.chat.messages {
        for step in &mut message.steps {
            let crate::app::state::AssistantStepView::Tool { call, .. } = step else {
                continue;
            };
            // 只认本会话确有记录的进程；没有记录（例如库不可用）时保持结果 JSON 的原样。
            let session_id = match call.detail.as_deref() {
                Some(crate::app::state::ToolDetailView::Command(view)) => view.process_session_id,
                Some(crate::app::state::ToolDetailView::Interaction(view)) => {
                    Some(view.process_session_id)
                }
                _ => None,
            };
            let Some(session_id) = session_id else {
                continue;
            };
            let Some(record) = records
                .iter()
                .find(|record| i64::try_from(session_id).ok() == Some(record.process_session_id))
            else {
                continue;
            };
            // `origin_call_id` 是进程的发起卡片；只有它承载时长，`write_stdin` 卡片
            // 只是一次交互快照（时长会误导成"交互持续了多久"）。
            let owns_process = call.call_id == record.origin_call_id;
            let next = crate::app::state::ProcessStateView::from_stored(
                &record.state,
                record.exit_code.and_then(|code| i32::try_from(code).ok()),
            );
            match call.detail.as_deref_mut() {
                Some(crate::app::state::ToolDetailView::Command(view)) => {
                    if view.state == next
                        && (!owns_process
                            || (view.started_at_ms == Some(record.started_at_ms)
                                && view.ended_at_ms == record.ended_at_ms))
                    {
                        continue;
                    }
                    view.state = next;
                    if owns_process {
                        view.started_at_ms = Some(record.started_at_ms);
                        view.ended_at_ms = record.ended_at_ms;
                    }
                    changed = true;
                }
                Some(crate::app::state::ToolDetailView::Interaction(view)) => {
                    if view.state == next {
                        continue;
                    }
                    view.state = next;
                    if view.exit_code.is_none() {
                        view.exit_code = record.exit_code.and_then(|code| i32::try_from(code).ok());
                    }
                    changed = true;
                }
                _ => {}
            }
        }
    }
    changed
}

/// 点击工具卡片：**行为随进程状态切换**。
///
/// - 命令仍在跑（长任务）：打开后台任务面板并定位该进程——卡片自己不给输出，
///   完整实时输出在面板里；
/// - 其余卡片：切换**被点中的那个区域**的折叠（参数与输出各自独立）。
pub fn activate_tool_card(
    state: &mut AppState,
    pane: PaneId,
    message_id: &str,
    call_id: &str,
    section: crate::app::state::CardSection,
) -> bool {
    let running = state
        .pane_anywhere(pane)
        .and_then(|target| {
            target
                .lx
                .chat
                .messages
                .iter()
                .find(|message| message.id == message_id)
                .and_then(|message| message.tool(call_id))
        })
        .and_then(|call| call.command_view())
        .filter(|view| view.state.is_running())
        .and_then(|view| view.process_session_id);
    let Some(session_id) = running else {
        return toggle_tool_detail(state, pane, message_id, call_id, section);
    };
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.selected_tool = Some((message_id.to_string(), call_id.to_string()));
    }
    super::open_task_page_at(state, pane, Some(session_id))
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
    section: crate::app::state::CardSection,
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
    target.lx.chat.toggle_section(message_id, call_id, section);
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

/// 历史重载时恢复「当前计划」：取最后一条成功的 `update_plan`。
pub fn restore_plan(state: &mut AppState, pane: PaneId, messages: &[ChatMessageView]) -> bool {
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
            // 按**消息时间线**取最后一条成功的 `update_plan`。
            //
            // 不能按 `revision` 取最大：`revision` 由每轮新建的 `AgentTools` 分配，
            // 是「本轮内单调」而非「跨轮单调」。第 1 轮可能已用到 revision 3、
            // 第 2 轮才从 1 重新开始，按最大值恢复会把上一轮已作废的计划捞回来。
            latest = Some(crate::app::state::PlanSnapshotView::from_snapshot(
                &snapshot,
            ));
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

/// 键盘 `^o`：对当前（或最近一张）可展开卡片执行它的**唯一动作**。
///
/// - 文件差异卡片：切换整张卡片的展开/收起（与点击标题同一条路径）；
/// - 进程卡片：打开后台任务面板并把高亮定位到该进程（与 `[open]` 同一条路径）。
///
/// 没有可定位的卡片时退化为普通打开（高亮保持当前值）。
pub fn open_task_for_selected_tool(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let selected = target.lx.chat.selected_tool.clone();
    let target_key = selected.or_else(|| latest_expandable(&target.lx.chat));
    let Some((message_id, call_id)) = target_key else {
        // 没有可展开的工具卡片：`^o` 改为切换最近的 compact 检查点（同一语义：
        // “当前或最近一张可展开卡片”）。都没有才退化为打开后台任务页。
        if let Some(checkpoint_id) = latest_compact_checkpoint(&target.lx) {
            return super::compact::toggle_checkpoint(state, pane, &checkpoint_id);
        }
        return super::open_task_page_at(state, pane, None);
    };
    let call = target
        .lx
        .chat
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .and_then(|message| message.tool(&call_id));
    // 文件卡片：`^o` 展开/收起，绝不打开后台任务面板。
    // 判据用 app 层的视图本身（`app` 不得反向依赖 `ui`）。
    if call.is_some_and(|call| call.file_view().is_some_and(|view| !view.is_empty())) {
        return toggle_tool_detail(
            state,
            pane,
            &message_id,
            &call_id,
            crate::app::state::CardSection::Body,
        );
    }
    let session_id = call.and_then(|call| call.process_session_id());
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.selected_tool = Some((message_id, call_id));
    }
    super::open_task_page_at(state, pane, session_id)
}

/// Alt+↑/↓：按时间线选择上一/下一张可展开卡片（保留草稿与光标）。
pub fn select_adjacent_tool(state: &mut AppState, pane: PaneId, forward: bool) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    let cards = expandable_cards(&target.lx.chat);
    if cards.is_empty() {
        // 没有工具卡片：在 compact 检查点之间切换（同样按时间线顺序）。
        let checkpoints: Vec<String> = target
            .lx
            .compact
            .checkpoints
            .iter()
            .map(|checkpoint| checkpoint.id.clone())
            .collect();
        if checkpoints.is_empty() {
            return false;
        }
        let current = target.lx.chat.selected_compact.clone();
        let index = current
            .as_ref()
            .and_then(|selected| checkpoints.iter().position(|id| id == selected));
        let next = match (index, forward) {
            (Some(index), true) => (index + 1).min(checkpoints.len() - 1),
            (Some(index), false) => index.saturating_sub(1),
            (None, true) => 0,
            (None, false) => checkpoints.len() - 1,
        };
        let Some(target) = state.pane_mut_anywhere(pane) else {
            return false;
        };
        target.lx.chat.selected_compact = Some(checkpoints[next].clone());
        return true;
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
    target.lx.chat.selected_compact = None;
    true
}

/// 最近的 compact 检查点 ID（`^o` 在没有工具卡片时的回退目标）。
fn latest_compact_checkpoint(lx: &crate::app::state::Lx) -> Option<String> {
    lx.compact
        .checkpoints
        .last()
        .map(|checkpoint| checkpoint.id.clone())
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
        .is_some_and(|summary| !summary.is_empty());
    has_summary || call.result_json.is_some()
}

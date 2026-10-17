//! lx 聊天状态机更新：发送、流事件收敛、终态与 `/` 命令。
//!
//! 只产生效应/更新纯状态；IO（DB/HTTP）由事件层执行，结果经 `AppEvent` 回投。
//! 按 `(pane, session, generation)` 过滤过期结果；`Finishing` 期间不允许新一轮。

use std::time::{Duration, Instant};

use crate::app::state::{
    AppState, AssistantStepView, ChatMessageView, ChatPartView, FrozenModel, RequestKey,
    RequestState,
};
use crate::app::toast::{Toast, ToastKind};
use crate::chat::events::{ChatDelta, PartKind, StopReason};
use crate::db::chat::{ModelSnapshot, TurnOutcome, Usage};
use crate::layout::PaneId;
use crate::ui::text;

mod session;
mod view;

pub use session::{
    apply_models_changed, apply_slash_command, apply_slash_enter, delete_session, new_session,
    on_history_listed, on_session_deleted, on_session_loaded, open_history, open_model_picker,
    persist_default_model, rename_session, resume_session,
};
pub(crate) use view::view_message_of;

/// 尝试发送：校验 Native 入口、非空草稿、请求空闲与有效模型后冻结快照并入队效应。
///
/// 返回是否产生状态变化。DB ack 成功后才清草稿；失败保留草稿可重试。
pub fn try_send(state: &mut AppState, pane: PaneId) -> bool {
    // 先收集纯数据（避免长持窗格可变借用）。
    let context = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        if target.view != crate::app::state::PaneView::Lx {
            return false;
        }
        let entry = target.lx.entry;
        let draft = target.lx.input.trim().to_string();
        let busy = !target.lx.chat.request.can_send();
        let selection = target.lx.chat.effective_selection(&state.models);
        let session_id = target.lx.chat.session_id.clone();
        let generation = next_generation(state, pane);
        (entry, draft, busy, selection, session_id, generation)
    };
    let (entry, draft, busy, selection, session_id, generation) = context;
    // 只有 Native Agent 发送；Hybrid/Automation 显示不可用提示。
    if entry != crate::app::state::LxEntry::Native {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_ENTRY_UNAVAILABLE);
        return true;
    }
    if draft.is_empty() || busy {
        // 流中 Enter 不入队、不造消息。
        return false;
    }
    // 解析有效模型（窗格选择或配置默认）。
    let Some(selection) = selection else {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    let Some(frozen) = FrozenModel::resolve(&state.models, &selection) else {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_NO_MODEL);
        return true;
    };
    // 冻结草稿并进入 Committing：清空输入由 DB ack 成功后执行。
    let workspace = state
        .workspace_cwd_for_pane(pane)
        .or_else(|| state.active_workspace().cwd.clone());
    let Some(workspace) = workspace else {
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_NO_WORKSPACE);
        return true;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.generation_counter = generation;
    target.lx.chat.request = RequestState::Committing {
        generation,
        draft: draft.clone(),
    };
    state
        .effects
        .push(crate::app::effects::Effect::BeginChatTurn {
            pane,
            generation,
            workspace,
            session_id,
            draft,
            model: ModelSnapshot {
                provider: frozen.provider.clone(),
                model: frozen.model.clone(),
                variant: frozen.variant.clone(),
            },
        });
    // 立即进入流中态：loading 行（转轮 + Esc 待命提示）从按下 Enter 起就可见，
    // 不必等首个 delta。
    refresh_lx_status(state, pane);
    true
}

/// DB begin_turn 成功回执：绑定 session/message、清冻结草稿、启动网络流。
///
/// 若期间用户继续编辑，新草稿不被清掉（只清与冻结草稿相同的输入）。
pub fn on_turn_committed(
    state: &mut AppState,
    pane: PaneId,
    generation: u64,
    committed: &crate::db::chat::TurnCommitted,
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Committing { generation: g, .. } if *g == generation
    ) {
        return false;
    }
    let frozen_draft = match &target.lx.chat.request {
        RequestState::Committing { draft, .. } => draft.clone(),
        _ => return false,
    };
    // 只清仍与冻结草稿相同的输入（期间用户编辑则保留新草稿）。
    if target.lx.input.trim() == frozen_draft.trim() {
        target.lx.clear();
    }
    // 展示：user 消息 + streaming assistant 占位。
    target.lx.chat.session_id = Some(committed.session_id.clone());
    target.lx.chat.turn_started_at_ms = Some(committed.created_at_ms);
    // 本轮模型展示名：发送时冻结的选择（设置页缺失时回退 provider/model）。
    let model_label = target
        .lx
        .chat
        .effective_selection(&models)
        .map(|selection| models.model_display_name(&selection.provider, &selection.model));
    target.lx.chat.messages.push(ChatMessageView {
        id: committed.user_message_id.clone(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: committed.turn_index,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: frozen_draft,
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
    });
    target.lx.chat.messages.push(ChatMessageView {
        id: committed.assistant_message_id.clone(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: committed.turn_index,
        steps: Vec::new(),
        model_label,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
    });
    target.lx.chat.follow_bottom = true;
    let key = RequestKey {
        pane,
        session: committed.session_id.clone(),
        generation,
    };
    target.lx.chat.request = RequestState::Streaming {
        key: key.clone(),
        assistant_id: committed.assistant_message_id.clone(),
    };
    // 启动网络流：由事件层按 key 执行；先入队效应。
    state
        .effects
        .push(crate::app::effects::Effect::StartChatStream {
            key,
            assistant_id: committed.assistant_message_id.clone(),
        });
    true
}

/// DB begin_turn 失败：回 Idle 保留草稿并提示可重试。
pub fn on_turn_failed(state: &mut AppState, pane: PaneId, generation: u64) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Committing { generation: g, .. } if *g == generation
    ) {
        return false;
    }
    target.lx.chat.request = RequestState::Idle;
    show_chat_toast(state, ToastKind::Error, text::LX_CHAT_COMMIT_FAILED);
    refresh_lx_status(state, pane);
    true
}

/// 流 delta 回投：只更新匹配 `(pane, session, generation)` 的视图。
///
/// 整轮终态 `Finished` 由 `on_stream_finished` 处理，这里只做增量与响应收尾；
/// 旧 generation / 其它窗格的事件在状态机校验处被丢弃（不回写任何状态）。
pub fn on_stream_delta(state: &mut AppState, key: &RequestKey, deltas: &[ChatDelta]) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Streaming {
            key: active,
            ..
        } if active == key
    ) {
        return false;
    }
    let mut changed = false;
    for delta in deltas {
        match delta {
            ChatDelta::Part(part) => {
                changed |= target.lx.chat.append_stream_delta(
                    part.kind,
                    &part.text,
                    part.metadata.clone(),
                );
            }
            ChatDelta::Usage(usage) => {
                // 上下文占用按 codex 口径：上一轮请求的输入 + 输出（下一轮请求的输入近似
                // 为其上一轮输入加上该轮输出）。切模型时分子清空（键由 refresh 维护）。
                if let Some(input) = usage.input {
                    let context = input.saturating_add(usage.output.unwrap_or(0));
                    target.lx.chat.context_tokens = Some(context.max(0) as u64);
                }
                changed = true;
            }
            ChatDelta::ReasoningEnded { item_id } => {
                changed |= target.lx.chat.reasoning_ended(item_id.as_deref());
            }
            ChatDelta::ToolInputStarted {
                call_id,
                name,
                ordinal,
            } => {
                changed |= target.lx.chat.start_tool_input(call_id, name, *ordinal);
            }
            ChatDelta::ToolInputDelta { call_id, chunk } => {
                changed |= target.lx.chat.append_tool_input(call_id, chunk);
            }
            ChatDelta::ToolInputEnded {
                call_id,
                arguments_json,
            } => {
                changed |= target.lx.chat.end_tool_input(call_id, arguments_json);
            }
            ChatDelta::ResponseFinished { stop } => {
                // 一次模型响应结束（可能是工具阶段）：不结束消息、不产生工具结果。
                changed |= target.lx.chat.finish_response(*stop);
            }
            ChatDelta::Finished { .. } => {
                // 终态事件由 on_stream_finished 处理；此处只标记。
                changed = true;
            }
        }
    }
    if changed {
        refresh_lx_status(state, key.pane);
    }
    changed
}

/// 流终态回投（完整 deltas 先经 on_stream_delta 消费；本函数只处理结束）。
///
/// 取消/失败/完成均先转 Finishing，再入队最终 DB 写入。
pub fn on_stream_finished(
    state: &mut AppState,
    key: &RequestKey,
    reason: &StopReason,
    usage: Usage,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    let assistant_id = match &target.lx.chat.request {
        RequestState::Streaming {
            key: active,
            assistant_id,
        } if active == key => assistant_id.clone(),
        _ => return false,
    };
    // 先按终态收尾步骤时间线：关闭仍开放的思考块、未结算工具标取消/中断（不伪造结果），
    // 再从时间线快照 parts（position 用真实分配值，不能按 enumerate 重新压平）。
    target.lx.chat.close_stream(reason);
    let parts: Vec<crate::db::chat::ChatPart> = target
        .lx
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.id == assistant_id)
        .map(crate::db::chat::chat_message_part_views)
        .unwrap_or_default();
    // 视图先行进入终态（DB ack 后确认；失败时保留内存 partial）。
    target.lx.chat.stop_armed_at = None;
    let duration_ms = target
        .lx
        .chat
        .turn_started_at_ms
        .map(|started| crate::db::now_ms().saturating_sub(started).max(0));
    if let Some(message) = target
        .lx
        .chat
        .messages
        .last_mut()
        .filter(|message| message.id == assistant_id)
    {
        message.status = match reason {
            StopReason::Complete => "complete".into(),
            StopReason::Cancelled => "interrupted".into(),
            StopReason::Failed { .. } => "failed".into(),
        };
        // 失败原因（脱敏码）随消息保留，供历史行与提示显示。
        message.error_code = match reason {
            StopReason::Failed { code } => Some(code.clone()),
            _ => None,
        };
        // 本轮统计：供应商用量 + 耗时（tok/s 由渲染层按输出/耗时派生）。
        message.input_tokens = usage.input;
        message.output_tokens = usage.output;
        message.cache_read_tokens = usage.cache_read;
        message.duration_ms = duration_ms;
    }
    target.lx.chat.request = RequestState::Finishing {
        key: key.clone(),
        reason: reason.clone(),
    };
    let outcome = match reason {
        StopReason::Complete => TurnOutcome::Complete {
            stop_reason: Some("complete".into()),
        },
        StopReason::Cancelled => TurnOutcome::Interrupted,
        StopReason::Failed { code } => TurnOutcome::Failed {
            error_code: code.clone(),
        },
    };
    state
        .effects
        .push(crate::app::effects::Effect::FinishChatTurn {
            key: key.clone(),
            assistant_id,
            parts,
            outcome,
            usage,
        });
    true
}

/// 最终 DB 写入成功：回 Idle；失败保留内存 partial 并提示保存失败。
pub fn on_finish_ack(state: &mut AppState, key: &RequestKey, success: bool) -> bool {
    let Some(target) = state.pane_mut_anywhere(key.pane) else {
        return false;
    };
    if !matches!(
        &target.lx.chat.request,
        RequestState::Finishing {
            key: active,
            ..
        } if active == key
    ) {
        return false;
    }
    if success {
        target.lx.chat.request = RequestState::Idle;
    } else {
        // Finishing（保存失败）：保留内存 partial，不能伪装成可恢复历史。
        show_chat_toast(state, ToastKind::Error, text::LX_CHAT_SAVE_FAILED);
    }
    refresh_lx_status(state, key.pane);
    true
}

/// Esc 停止的待命窗口：窗口内再按一次才真正停止（对齐 opencode 的 5 秒）。
pub const STOP_ARM_WINDOW: Duration = Duration::from_secs(5);

/// 流中按 `Esc`（对齐 opencode 的双击停止）：首次只**待命**并显示提示，
/// 窗口内再按一次才停止；非流中吞掉（返回 false 不置脏）。
pub fn escape_stop(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if !target.lx.chat.request.is_busy() {
        return false;
    }
    let now = Instant::now();
    let armed = target
        .lx
        .chat
        .stop_armed_at
        .is_some_and(|at| now.saturating_duration_since(at) <= STOP_ARM_WINDOW);
    if !armed {
        target.lx.chat.stop_armed_at = Some(now);
        return true;
    }
    target.lx.chat.stop_armed_at = None;
    stop_stream(state, pane);
    true
}

/// 停止生成：Esc 双击确认后触发；流取消后走正常 Finishing 收尾。
pub fn stop_stream(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.stop_armed_at = None;
    match target.lx.chat.request.clone() {
        RequestState::Streaming { key, .. } => {
            state
                .effects
                .push(crate::app::effects::Effect::CancelChatStream { key });
            true
        }
        RequestState::Committing { generation, .. } => {
            // 等 DB ack 后直接落 interrupted（不发 HTTP）：标记待取消。
            state
                .effects
                .push(crate::app::effects::Effect::CancelChatCommit { pane, generation });
            true
        }
        _ => false,
    }
}

/// 刷新单窗格状态行展示数据（设置页保存、选择变更、usage 到达后调用）。
pub fn refresh_lx_status(state: &mut AppState, pane: PaneId) {
    let models = state.models.clone();
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.chat.refresh_status(&models);
    }
}

/// 启动/配置加载后刷新所有窗格状态行。
pub fn refresh_all_lx_status(state: &mut AppState) {
    for pane in state.all_pane_ids() {
        refresh_lx_status(state, pane);
    }
}

/// 窗格内下一个 generation（单调递增；从窗格计数器取）。
fn next_generation(state: &AppState, pane: PaneId) -> u64 {
    state
        .pane_anywhere(pane)
        .map(|target| target.lx.chat.generation_counter)
        .map(|counter| counter.wrapping_add(1))
        .unwrap_or(1)
        .max(1)
}

/// 显示聊天相关 toast。
fn show_chat_toast(state: &mut AppState, kind: ToastKind, message: &'static str) {
    crate::app::update::show_toast(state, Toast::new(kind, message, None, Instant::now()));
}

/// 测试钩子：直接完成模型选择（绕过两级面板交互）。
#[cfg(test)]
pub(crate) mod test_hooks {
    use crate::app::state::AppState;
    use crate::config::ai::ModelSelection;
    use crate::layout::PaneId;

    pub(crate) fn finish_model_selection(
        state: &mut AppState,
        pane: PaneId,
        selection: ModelSelection,
    ) {
        super::session::finish_model_selection(state, pane, selection)
    }
}

/// 切换指定 thinking 块展开状态。
pub fn toggle_thinking_at(
    state: &mut AppState,
    pane: PaneId,
    message_index: usize,
    step_index: usize,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.chat.toggle_reasoning(message_index, step_index);
    target.lx.chat.follow_bottom = false;
    true
}

/// 切换最近一条 assistant 消息里的最后一个 thinking 块（Ctrl+T）。
pub fn toggle_latest_thinking(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let mut found = None;
    'outer: for (message_index, message) in target.lx.chat.messages.iter().enumerate().rev() {
        for (step_index, step) in message.steps.iter().enumerate().rev() {
            if step
                .part()
                .is_some_and(|part| part.kind == PartKind::Reasoning)
            {
                found = Some((message_index, step_index));
                break 'outer;
            }
        }
    }
    let Some((message_index, step_index)) = found else {
        return false;
    };
    target.lx.chat.toggle_reasoning(message_index, step_index);
    true
}

/// 滚动历史视图（正数向下）；`total`/`visible` 为渲染行数与视口行数；返回是否移动。
///
/// 状态里的 `history_scroll` 是**顶部行索引**（0 = 最早）；`follow_bottom` 表示
/// 视口钉在最新一屏（顶部行 = `total - visible`）。
pub fn scroll_lx_history(
    state: &mut AppState,
    pane: PaneId,
    delta: isize,
    total: usize,
    visible: usize,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let max_top = total.saturating_sub(visible);
    let current = if target.lx.chat.follow_bottom {
        max_top
    } else {
        target.lx.chat.history_scroll.min(max_top)
    };
    let next = (current as isize + delta).clamp(0, max_top as isize) as usize;
    let moved = next != current || target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = next;
    target.lx.chat.follow_bottom = next >= max_top;
    moved
}

/// 直接设置历史视口顶部行偏移（滚动条点击/拖拽）；`total`/`visible` 为渲染行数与视口行数。
/// 顶部行 == `total - visible` 时视为钉底跟随；返回视口是否移动。
pub fn set_lx_history_scroll(
    state: &mut AppState,
    pane: PaneId,
    offset: usize,
    total: usize,
    visible: usize,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let max_top = total.saturating_sub(visible);
    let clamped = offset.min(max_top);
    let before = target.lx.chat.history_scroll;
    let was_following = target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = clamped;
    target.lx.chat.follow_bottom = clamped >= max_top;
    before != clamped || was_following != target.lx.chat.follow_bottom
}

/// 每轮把各 lx 窗格的置顶行**抑制记忆**落状态。
///
/// 渲染层只读、不写状态（AGENTS：渲染只读），而记忆是唯一的跨帧状态，故放在
/// 事件循环里、`tui.draw` 之前由这里写；判定本身是
/// `ui::main_content::lx::history_view` 的纯函数（渲染/命中各自现算，不存第二份真相）。
/// 返回是否有窗格记忆变化。
pub fn resolve_lx_sticky_prompt(
    state: &mut AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
) -> bool {
    let mut changed = false;
    for (pane, rect) in rects {
        let inner = crate::layout::pane_inner_rect(*rect);
        // 先只读地算出结论（借用结束后再写状态）。
        let resolved = state
            .pane_anywhere(*pane)
            .filter(|target| target.view == crate::app::state::PaneView::Lx)
            .and_then(|target| crate::ui::main_content::lx::history_view(inner, &target.lx))
            .map(|view| view.suppressed);
        let Some(suppressed) = resolved else {
            continue;
        };
        let Some(target) = state.pane_mut_anywhere(*pane) else {
            continue;
        };
        if target.lx.chat.suppressed_prompt_header == suppressed {
            continue;
        }
        target.lx.chat.suppressed_prompt_header = suppressed;
        changed = true;
    }
    changed
}

/// 点击置顶行：跳到该 prompt（顶部对齐该消息起始行），并退出跟随底部。
/// `target_row` 由 `ui::main_content::lx::sticky_prompt_jump` 按当前几何给出。
pub fn jump_lx_history_to_prompt(state: &mut AppState, pane: PaneId, target_row: usize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let before = target.lx.chat.history_scroll;
    let was_following = target.lx.chat.follow_bottom;
    target.lx.chat.history_scroll = target_row;
    // 明确进入阅读态：置顶行随之按规则让位（顶行成了那条用户消息自己）。
    target.lx.chat.follow_bottom = false;
    before != target_row || was_following
}

/// 回到底部：把历史视口钉到最新一屏（跟随底部）并清除悬停态；返回是否发生变化。
pub fn jump_lx_history_bottom(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.view != crate::app::state::PaneView::Lx || target.lx.chat.messages.is_empty() {
        return false;
    }
    let changed = !target.lx.chat.follow_bottom || target.lx.chat.jump_bottom_hover;
    target.lx.chat.follow_bottom = true;
    target.lx.chat.jump_bottom_hover = false;
    changed
}

/// 设置「回到底部」按钮悬停态；返回是否变化。
pub fn hover_lx_jump_bottom(state: &mut AppState, pane: PaneId, hovered: bool) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.jump_bottom_hover == hovered {
        return false;
    }
    target.lx.chat.jump_bottom_hover = hovered;
    true
}

/// 推进流中窗格的 loading 转轮帧；返回是否需要重绘（仅在有流中的 lx 窗格时）。
pub fn advance_lx_loading(state: &mut AppState) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        if let Some(target) = state.pane_mut_anywhere(pane)
            && target.view == crate::app::state::PaneView::Lx
            && target.lx.chat.status.streaming
        {
            // 待命超时：提示消失（重新回到普通 `Working`）。
            if target
                .lx
                .chat
                .stop_armed_at
                .is_some_and(|at| at.elapsed() > STOP_ARM_WINDOW)
            {
                target.lx.chat.stop_armed_at = None;
            }
            target.lx.chat.loading_frame = target.lx.chat.loading_frame.wrapping_add(1);
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests;

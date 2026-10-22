//! 最新轮次撤销的状态转换：双 Esc 待命、最新目标、四项面板、执行与回执。
//!
//! 这里只改纯状态并入队 `Effect`；文件/DB IO 全部由事件层执行。成功回执到达后才
//! 刷新历史与回填，绝不做 UI 乐观删除。

use std::time::Instant;

use crate::app::actions::LxInput;
use crate::app::effects::{Effect, RewindRequest};
use crate::app::state::{
    AppState, InputHintLevel, RequestState, RewindAction, RewindExecution, RewindPanel,
    RewindTarget, SnapshotState, lx::SlashPanel, lx::rewind::REWIND_ARM_WINDOW,
    lx::slash::PanelWindow,
};
use crate::db::chat::FileAvailability;
use crate::layout::PaneId;
use crate::ui::text;

use super::{refresh_lx_status, show_lx_hint};

/// 面板之前的输入路由结果。
pub enum Route {
    /// 已消费该键，不再进入普通编辑/发送。
    Consumed,
    /// 不属于撤销；按原语义继续。
    Pass,
}

/// 处理可能有撤销语义的按键（在任务页之后、历史/面板之前调用）。
///
/// - 执行中吞掉全部输入；
/// - 动作面板由 `/` 面板路由接管（本函数 Pass，Esc 在面板分支单独处理）；
/// - 选中最新轮次时 Enter 开面板、Esc 取消、上下键不改目标；
/// - 空闲空输入的双 Esc：第一次待命、第二次选中最新轮次。
pub fn apply_input(state: &mut AppState, pane: PaneId, command: LxInput) -> Route {
    let Some(view_ok) = state
        .pane_anywhere(pane)
        .map(|target| target.view == crate::app::state::PaneView::Lx)
    else {
        return Route::Pass;
    };
    if !view_ok {
        return Route::Pass;
    }
    // 即便事件循环节拍尚未清除，超过 3 秒的待命也必须作废：本次 Esc 重新开始计时。
    let expired = state.pane_anywhere(pane).is_some_and(|target| {
        target
            .lx
            .rewind
            .armed_at
            .is_some_and(|at| at.elapsed() > REWIND_ARM_WINDOW)
    });
    if expired && let Some(target) = state.pane_mut_anywhere(pane) {
        target.lx.rewind.armed_at = None;
    }
    let Some(target) = state.pane_anywhere(pane) else {
        return Route::Pass;
    };
    if target.lx.rewind.is_executing() {
        return Route::Consumed;
    }
    if matches!(target.lx.slash, Some(SlashPanel::Rewind(_))) {
        // 面板上下/Enter/Esc 由 `/` 面板路由处理（见 `apply_slash_enter` 与 pane 分支）。
        return Route::Pass;
    }
    // 其它覆盖层同样优先消费 Esc：`/model`、`/resume` 打开时输入框为空，若放行到这里，
    // 关闭面板的 Esc 会被误算成撤销待命的第一下。
    if target.lx.slash.is_some()
        || target.lx.panel.is_some()
        || target.lx.mention_open()
        || target.lx.paste_panel().is_some()
    {
        return Route::Pass;
    }
    if target.lx.rewind.selected.is_some() {
        match command {
            LxInput::Enter => {
                open_panel(state, pane);
                return Route::Consumed;
            }
            LxInput::Escape => {
                cancel_selection(state, pane);
                return Route::Consumed;
            }
            // 上下键不能选择更早轮次；滚轮仍可浏览历史。
            LxInput::Up | LxInput::Down => return Route::Consumed,
            _ => {
                cancel_selection(state, pane);
                return Route::Pass;
            }
        }
    }
    if target.lx.rewind.armed_at.is_some() {
        if command == LxInput::Escape {
            if select_latest(state, pane).is_some() {
                return Route::Consumed;
            }
            // 目标已不可选（忙/会话变化）：作废旧待命，避免结束后误触发。
            if let Some(target) = state.pane_mut_anywhere(pane) {
                target.lx.rewind.armed_at = None;
            }
            return Route::Pass;
        }
        // 任何其它输入都使待命失效，再按普通输入处理。
        if let Some(target) = state.pane_mut_anywhere(pane) {
            target.lx.rewind.armed_at = None;
        }
        return Route::Pass;
    }
    if command == LxInput::Escape {
        return arm(state, pane).map_or(Route::Pass, |_| Route::Consumed);
    }
    Route::Pass
}

/// 空闲空输入第一次 Esc：只在可撤销时进入 3 秒待命。
fn arm(state: &mut AppState, pane: PaneId) -> Option<()> {
    let target = state.pane_anywhere(pane)?;
    if target.view != crate::app::state::PaneView::Lx {
        return None;
    }
    // 忙时保留现有双 Esc 停止语义；草稿非空或根本没有用户轮次时不进入撤销。
    if !matches!(target.lx.chat.request, RequestState::Idle)
        || !target.lx.input.trim().is_empty()
        || target.lx.chat.session_id.is_none()
        || !target
            .lx
            .chat
            .messages
            .iter()
            .any(|message| message.role == "user")
    {
        return None;
    }
    let target = state.pane_mut_anywhere(pane)?;
    target.lx.rewind.armed_at = Some(Instant::now());
    target.lx.chat.stop_armed_at = None;
    show_lx_hint(
        state,
        pane,
        InputHintLevel::Warning,
        text::LX_REWIND_ARMED_HINT,
    );
    Some(())
}

/// 选中当前会话最新一条用户消息（只允许最新轮次）。
pub fn select_latest(state: &mut AppState, pane: PaneId) -> Option<()> {
    let target = state.pane_anywhere(pane)?;
    if !matches!(target.lx.chat.request, RequestState::Idle) {
        return None;
    }
    // 待命后可能经粘贴等非键盘路径写入草稿：选中时再复核一次空输入。
    if !target.lx.input.trim().is_empty() {
        return None;
    }
    let session_id = target.lx.chat.session_id.clone()?;
    let message = target
        .lx
        .chat
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")?;
    let user_text = message
        .text_parts()
        .map(|part| part.content.as_str())
        .collect::<String>();
    let revision = target.lx.chat.revision;
    let input_at_open = target.lx.input.clone();
    let user_message_id = message.id.clone();
    let target = state.pane_mut_anywhere(pane)?;
    target.lx.rewind.armed_at = None;
    target.lx.rewind.snapshot = SnapshotState::Loading;
    target.lx.rewind.selected = Some(RewindTarget {
        session_id: session_id.clone(),
        user_message_id: user_message_id.clone(),
        expected_revision: revision,
        pane,
        input_at_open,
        user_text,
    });
    target.lx.rewind.panel = None;
    // 最新轮次在列表底部；先跟随底部，取得几何后再把目标滚到视口居中。
    target.lx.chat.follow_bottom = true;
    target.lx.rewind.center_target = Some(user_message_id.clone());
    state.effects.push(Effect::PrepareRewind {
        pane,
        session_id,
        user_message_id,
    });
    Some(())
}

/// `/rewind` 命令采纳：选中最新轮次并直接打开动作面板（采纳 Enter 不执行）。
pub fn open_from_command(state: &mut AppState, pane: PaneId) -> bool {
    if select_latest(state, pane).is_none() {
        show_lx_hint(state, pane, InputHintLevel::Warning, text::LX_REWIND_FAILED);
        return false;
    }
    open_panel(state, pane)
}

/// 选中态 Enter：打开四项面板（默认第一项）。
fn open_panel(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.rewind.selected.is_none() {
        return false;
    }
    let panel = RewindPanel::new(target.lx.rewind.snapshot);
    target.lx.rewind.panel = Some(panel.clone());
    target.lx.slash = Some(SlashPanel::Rewind(Box::new(panel)));
    true
}

/// 面板 Esc：退回选中态（不取消目标）。
pub fn close_panel(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.rewind.executing.is_some() {
        return true;
    }
    target.lx.rewind.panel = None;
    if matches!(target.lx.slash, Some(SlashPanel::Rewind(_))) {
        target.lx.slash = None;
    }
    target.lx.rewind.selected.is_some()
}

/// 鼠标滚动历史时退出撤销高亮/面板（执行中不打断）。
///
/// 返回是否清除了撤销状态（调用方据此置脏并吞掉该滚轮事件）。
pub fn cancel_on_scroll(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if target.lx.rewind.is_executing() {
        return false;
    }
    let active = target.lx.rewind.selected.is_some()
        || target.lx.rewind.panel.is_some()
        || matches!(target.lx.slash, Some(SlashPanel::Rewind(_)));
    if !active {
        return false;
    }
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.rewind.clear();
    if matches!(target.lx.slash, Some(SlashPanel::Rewind(_))) {
        target.lx.slash = None;
    }
    true
}

/// 取消选择：清空 arm/selected/panel，恢复普通编辑。
pub fn cancel_selection(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.rewind.executing.is_some() {
        return true;
    }
    target.lx.rewind.clear();
    if matches!(target.lx.slash, Some(SlashPanel::Rewind(_))) {
        target.lx.slash = None;
    }
    true
}

/// 面板 Enter：执行高亮动作；Cancel 只关闭面板。
pub fn apply_enter(state: &mut AppState, pane: PaneId) -> bool {
    let (action, selected) = {
        let Some(target) = state.pane_anywhere(pane) else {
            return false;
        };
        if target.lx.rewind.executing.is_some() {
            return true;
        }
        // 高亮/可用性以 `/` 面板为准（上下键与鼠标都只改那一份窗口状态）。
        let Some(SlashPanel::Rewind(panel)) = target.lx.slash.as_ref() else {
            return false;
        };
        let action = panel.active_action();
        if !panel.enabled(action) {
            // 含文件动作在快照不可用/未就绪时禁用：吞掉 Enter，不降级成删对话。
            return true;
        }
        if action == RewindAction::Cancel {
            cancel_selection(state, pane);
            return true;
        }
        let Some(selected) = target.lx.rewind.selected.clone() else {
            return true;
        };
        (action, selected)
    };
    // 执行前最后一道本地预检：忙、后台任务、其他窗格在途都拒绝。
    if let Some(reason) = execute_blocker(state, pane, &selected) {
        show_lx_hint(state, pane, InputHintLevel::Warning, reason);
        return true;
    }
    let operation_id = crate::db::new_external_id("rewind-op");
    let Some(workspace) = state.chat_session_key_for_pane(pane) else {
        show_lx_hint(
            state,
            pane,
            InputHintLevel::Error,
            text::LX_CHAT_NO_WORKSPACE,
        );
        return true;
    };
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    target.lx.rewind.executing = Some(RewindExecution {
        operation_id: operation_id.clone(),
        action,
        target: selected.clone(),
    });
    target.lx.rewind.panel = None;
    target.lx.slash = None;
    state.effects.push(Effect::ExecuteRewind(RewindRequest {
        pane,
        session_id: selected.session_id.clone(),
        target_user_message_id: selected.user_message_id.clone(),
        expected_revision: selected.expected_revision,
        action,
        workspace,
        input_at_open: selected.input_at_open.clone(),
        user_text: selected.user_text.clone(),
        operation_id,
    }));
    true
}

/// 执行前的本地预检：本会话无在途请求、无存活后台任务、其他窗格不忙。
fn execute_blocker(
    state: &AppState,
    pane: PaneId,
    selected: &RewindTarget,
) -> Option<&'static str> {
    let target = state.pane_anywhere(pane)?;
    if target.lx.background_tasks > 0 {
        return Some(text::LX_REWIND_BUSY);
    }
    if !matches!(target.lx.chat.request, RequestState::Idle) {
        return Some(text::LX_REWIND_SESSION_BUSY);
    }
    let other_busy = state.all_pane_ids().into_iter().any(|other| {
        other != pane
            && state.pane_anywhere(other).is_some_and(|other_pane| {
                other_pane.lx.chat.session_id.as_deref() == Some(selected.session_id.as_str())
                    && other_pane.lx.chat.request.is_busy()
            })
    });
    other_busy.then_some(text::LX_REWIND_SESSION_BUSY)
}

/// PrepareRewind 回执：更新快照可用性与目标原文/revision。
pub fn on_prepared(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    user_message_id: &str,
    availability: &FileAvailability,
    revision: i64,
    user_text: &str,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(selected) = target.lx.rewind.selected.as_mut() else {
        return false;
    };
    if selected.session_id != session_id || selected.user_message_id != user_message_id {
        return false;
    }
    selected.user_text = user_text.to_string();
    selected.expected_revision = u64::try_from(revision).unwrap_or(0);
    let snapshot = SnapshotState::from_db(availability);
    target.lx.rewind.snapshot = snapshot;
    if let Some(panel) = target.lx.rewind.panel.as_mut() {
        panel.snapshot = snapshot;
    }
    if let Some(SlashPanel::Rewind(panel)) = target.lx.slash.as_mut() {
        panel.snapshot = snapshot;
    }
    true
}

/// 撤销成功后的历史重载：替换消息与独立提示，保留草稿；清掉被删轮次的交互状态。
pub fn on_session_reloaded(
    state: &mut AppState,
    pane: PaneId,
    session_id: &str,
    messages: &[crate::db::chat::ChatMessage],
    notices: &[crate::db::chat::NoticeRecord],
    revision: i64,
) -> bool {
    let models = state.models.clone();
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    if target.lx.chat.session_id.as_deref() != Some(session_id) {
        return false;
    }
    target.lx.chat.messages = messages
        .iter()
        .map(|message| super::view::view_message_of(message, &models))
        .collect();
    target.lx.chat.notices = notices
        .iter()
        .map(crate::app::state::RewindNoticeView::from_record)
        .collect();
    target.lx.chat.revision = u64::try_from(revision).unwrap_or(0);
    target.lx.chat.thinking_toggled.clear();
    target.lx.chat.detail_expanded.clear();
    target.lx.chat.selected_tool = None;
    target.lx.chat.hover = None;
    target.lx.chat.plan = None;
    target.lx.chat.follow_bottom = true;
    target.lx.chat.viewport_top = None;
    target.lx.chat.suppressed_prompt_header = None;
    target.lx.chat.turn_started_at_ms = None;
    target.lx.chat.turn_elapsed_ms = None;
    // 其他 pane 的旧选择/面板随被删轮次作废；发起 pane 的 executing 必须保留到回填。
    if target.lx.rewind.executing.is_none() {
        target.lx.clear_rewind();
    }
    // 用量按剩余消息重算，避免残留被删轮次。
    let recent = messages.iter().rev().find(|message| {
        message.role == "assistant"
            && (message.input_tokens.is_some() || message.output_tokens.is_some())
    });
    target.lx.chat.context_tokens = recent.map(|message| {
        let input = message.input_tokens.unwrap_or(0).max(0);
        let output = message.output_tokens.unwrap_or(0).max(0);
        u64::try_from(input.saturating_add(output)).unwrap_or(u64::MAX)
    });
    // 计划从剩余消息的最后一条成功 update_plan 重建：revert_only 时对话未变，
    // 计划卡片与下一轮运行上下文都不能因重载而丢。
    let restored_messages = target.lx.chat.messages.clone();
    refresh_lx_status(state, pane);
    super::restore_plan(state, pane, &restored_messages);
    true
}

/// 执行失败/冲突：保留对话与草稿、清执行态、提示原因。
pub fn on_failed(state: &mut AppState, pane: PaneId, message: &str) {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return;
    };
    target.lx.rewind.clear();
    if matches!(target.lx.slash, Some(SlashPanel::Rewind(_))) {
        target.lx.slash = None;
    }
    show_lx_hint(state, pane, InputHintLevel::Error, message);
    refresh_lx_status(state, pane);
}

/// 执行成功：清除撤销状态；仅删对话时回填原文（草稿未被用户改过才回填）。
///
/// 历史与提示由事件层在回执前重载；这里只处理发起窗格的草稿。
pub fn on_succeeded(
    state: &mut AppState,
    pane: PaneId,
    action: RewindAction,
    user_text: &str,
) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let bound = target
        .lx
        .rewind
        .executing
        .as_ref()
        .is_some_and(|execution| {
            target.lx.chat.session_id.as_deref() == Some(execution.target.session_id.as_str())
        });
    let input_at_open = target
        .lx
        .rewind
        .executing
        .as_ref()
        .map(|execution| execution.target.input_at_open.clone());
    let unchanged = input_at_open.as_deref() == Some(target.lx.input.as_str());
    target.lx.rewind.clear();
    if matches!(target.lx.slash, Some(SlashPanel::Rewind(_))) {
        target.lx.slash = None;
    }
    if action.fills_draft() && unchanged && bound {
        target.lx.replace_input(user_text);
        target.lx.slash = None;
        target.lx.rewind.clear();
        target.lx.chat.stop_armed_at = None;
        return true;
    }
    refresh_lx_status(state, pane);
    true
}

/// 向上/向下移动面板高亮（循环）。
pub fn move_active(state: &mut AppState, pane: PaneId, delta: isize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(SlashPanel::Rewind(panel)) = target.lx.slash.as_mut() else {
        return false;
    };
    let len = RewindAction::ALL.len();
    panel.window.move_active(delta, len);
    let window = panel.window;
    if let Some(stored) = target.lx.rewind.panel.as_mut() {
        stored.window = window;
    }
    true
}

/// 鼠标悬停面板条目。
pub fn set_active(state: &mut AppState, pane: PaneId, index: usize) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let Some(SlashPanel::Rewind(panel)) = target.lx.slash.as_mut() else {
        return false;
    };
    let len = RewindAction::ALL.len();
    if index >= len || panel.window.active == index {
        return false;
    }
    panel.window.set_active(index);
    let window = panel.window;
    if let Some(stored) = target.lx.rewind.panel.as_mut() {
        stored.window = window;
    }
    true
}

/// 面板是否需要同时被画在 `/` 面板位置（状态侧只看 `lx.slash`）。
pub fn panel_open(state: &AppState, pane: PaneId) -> bool {
    state
        .pane_anywhere(pane)
        .is_some_and(|target| matches!(target.lx.slash, Some(SlashPanel::Rewind(_))))
}

/// 面板窗口（测试与渲染共用）。
pub fn window(state: &AppState, pane: PaneId) -> Option<PanelWindow> {
    match state.pane_anywhere(pane)?.lx.slash.as_ref()? {
        SlashPanel::Rewind(panel) => Some(panel.window),
        _ => None,
    }
}

/// 待命窗口是否已过期（显示层据此清提示）。
pub fn armed_expired(state: &AppState, pane: PaneId) -> bool {
    state.pane_anywhere(pane).is_some_and(|target| {
        target
            .lx
            .rewind
            .armed_at
            .is_some_and(|at| at.elapsed() > REWIND_ARM_WINDOW)
    })
}

/// 清空已过期的待命（事件循环节拍调用）。
pub fn clear_expired_arms(state: &mut AppState) -> bool {
    let mut changed = false;
    for pane in state.all_pane_ids() {
        if armed_expired(state, pane)
            && let Some(target) = state.pane_mut_anywhere(pane)
        {
            target.lx.rewind.armed_at = None;
            changed = true;
        }
    }
    changed
}

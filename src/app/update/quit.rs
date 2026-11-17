//! 退出二次确认：首次请求展示全局 toast，确认窗口内再次请求才退出。
//!
//! 不使用模态层：底层浮层、异步回执、焦点与选区全程照常，toast 只是提示。

use std::time::Instant;

use crate::app::overlay::Overlay;
use crate::app::state::{AppState, QUIT_CONFIRM_REPEAT_GUARD, QUIT_CONFIRM_WINDOW};
use crate::app::toast::{Toast, ToastKind};
use crate::ui::text;

/// 第一次退出请求：展示全局 toast，不动底层浮层、焦点与文本选区。
///
/// 后续请求落在 `QUIT_CONFIRM_REPEAT_GUARD` 内视为长按自动重复，仅刷新窗口；
/// 落在确认窗口内才退出；超过确认窗口则重新开始，绝不直接退出。
pub fn request_quit(state: &mut AppState) {
    let now = Instant::now();
    let Some(requested_at) = state.quit_requested_at else {
        arm_quit(state, now);
        return;
    };
    let elapsed = now.duration_since(requested_at);
    let toast_visible = quit_toast_visible(state, now);
    if toast_visible && elapsed >= QUIT_CONFIRM_REPEAT_GUARD && elapsed < QUIT_CONFIRM_WINDOW {
        confirm_quit(state);
        return;
    }
    state.quit_requested_at = Some(now);
    if toast_visible && elapsed < QUIT_CONFIRM_REPEAT_GUARD {
        return;
    }
    // 窗口过期或提示已被关闭/替换：重新开始，不能凭旧请求直接退出。
    clear_active_gestures(state);
    show_quit_toast(state, now);
}

/// 当前全局 toast 是否仍是退出提示（未被关闭、替换或过期）。
fn quit_toast_visible(state: &AppState, now: Instant) -> bool {
    state
        .toast
        .as_ref()
        .is_some_and(|toast| toast.message == text::EXIT_CONFIRM_TOAST && !toast.is_expired(now))
}

/// 确认退出：只有此时才提交编辑器并置 `should_quit`。
pub fn confirm_quit(state: &mut AppState) {
    if state.quit_requested_at.take().is_none() {
        return;
    }
    super::commit_editing_prompt(state);
    state.prompt_committed_for_quit = true;
    state.should_quit = true;
}

/// 首次请求或窗口过期后的重新请求：记录时刻并展示全局 toast。
fn arm_quit(state: &mut AppState, now: Instant) {
    state.quit_requested_at = Some(now);
    clear_active_gestures(state);
    show_quit_toast(state, now);
}

/// 全局 toast（无窗格锚点），避免落在 lx 窗格内或遮挡确认语义。
fn show_quit_toast(state: &mut AppState, now: Instant) {
    super::show_toast(
        state,
        Toast::new(ToastKind::Warning, text::EXIT_CONFIRM_TOAST, None, now)
            .with_title(text::EXIT_CONFIRM_TITLE),
    );
}

/// 清理按下中/拖拽中的手势状态；不触碰任何文本选区。
///
/// 请求退出时不打断底层交互，但按下中的拖拽不应跨过这次请求继续生效，
/// 否则松手时仍会改掉分割线尺寸、滚动条或排序。
fn clear_active_gestures(state: &mut AppState) {
    state.resizing_sidebar = false;
    state.resizing_pane = None;
    state.resizing_prompt = false;
    state.prompt_drag = None;
    state.workspace_drag = None;
    state.workspace_dragging = false;
    state.workspace_drag_moved = false;
    state.workspace_history.drag = None;
    state.workspace_history.dragging = false;
    state.workspace_history.drag_moved = false;
    state.workspace_history.search_drag = false;
    state.overlay_input_drag = false;
    state.new_agent_scroll_drag = None;
    state.lx_input_drag = None;
    state.todo_input_drag = false;
    state.prompt_search_drag = false;
    state.ask_input_drag = None;
    state.lx_card_press = None;
    state.selection_autoscroll = None;
    state.lx_selection_autoscroll = None;
    state.drag_autoscroll = None;
    state.workspace_scroll_drag = None;
    state.prompt_scroll_drag = None;
    state.prompt_recent_scroll_drag = None;
    state.task_scroll_drag = None;
    state.task_list_scroll_drag = None;
    state.task_agents_scroll_drag = None;
    state.task_btw_scroll_drag = None;
    state.referenced_search_scroll_drag = None;
    state.referenced_menu_scroll_drag = None;
    state.connect_scroll_drag = None;
    state.terminal_scroll_drag = None;
    state.lx_scroll_drag = None;
    state.lx_history_scroll_drag = None;
    state.agent_scroll_drag = None;
    state.todo_scroll_drag = None;
    state.todo_drag = None;
    if let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_mut() {
        confirm.scroll_drag = None;
    }
}

#[cfg(test)]
mod tests;

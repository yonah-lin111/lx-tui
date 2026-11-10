//! 退出二次确认：请求、确认与取消。
//!
//! 确认层独立于 `state.overlay`：打开期间底层浮层、异步回执、焦点与选区全部照常，
//! 渲染与输入只在最上层优先处理确认层。

use std::time::Instant;

use crate::app::overlay::Overlay;
use crate::app::state::{AppState, QuitConfirmState};

/// 第一次退出请求：只打开确认层，不动底层浮层、焦点与文本选区。
pub fn request_quit(state: &mut AppState) {
    if state.quit_confirm.is_some() {
        return;
    }
    clear_active_gestures(state);
    state.quit_confirm = Some(QuitConfirmState {
        last_shortcut_press: Instant::now(),
    });
    state.quit_cancel_latch = None;
}

/// 确认退出：只有此时才提交编辑器并置 `should_quit`。
pub fn confirm_quit(state: &mut AppState) {
    if state.quit_confirm.take().is_none() {
        return;
    }
    super::commit_editing_prompt(state);
    state.prompt_committed_for_quit = true;
    state.should_quit = true;
}

/// 取消退出：关闭确认层并登记短窗口 latch，底层浮层原样保留。
pub fn cancel_quit(state: &mut AppState) {
    if state.quit_confirm.take().is_none() {
        return;
    }
    state.quit_cancel_latch = Some(Instant::now());
}

/// 清理按下中/拖拽中的手势状态；不触碰任何文本选区。
///
/// 确认层打开后拖拽与松开都会被拦截，残留的手势标记必须在请求时丢弃，
/// 否则取消退出后继续移动鼠标仍会改掉分割线尺寸、滚动条或排序。
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
    state.lx_input_drag = None;
    state.todo_input_drag = false;
    state.prompt_search_drag = false;
    state.ask_input_drag = None;
    state.lx_card_press = None;
    state.selection_autoscroll = None;
    state.workspace_scroll_drag = None;
    state.prompt_scroll_drag = None;
    state.prompt_recent_scroll_drag = None;
    state.task_scroll_drag = None;
    state.task_list_scroll_drag = None;
    state.task_agents_scroll_drag = None;
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

//! 退出二次确认：请求、确认与取消。

use crate::app::overlay::Overlay;
use crate::app::state::AppState;

/// 第一次退出请求：只打开确认浮层，暂存底层浮层。
pub fn request_quit(state: &mut AppState) {
    if matches!(state.overlay, Some(Overlay::ConfirmQuit)) {
        return;
    }
    if state.overlay.is_some() {
        state.suspended_overlay = state.overlay.take();
    }
    super::clear_selection(state);
    state.overlay = Some(Overlay::ConfirmQuit);
}

/// 确认退出：只有此时才提交编辑器并置 `should_quit`。
pub fn confirm_quit(state: &mut AppState) {
    if !matches!(state.overlay, Some(Overlay::ConfirmQuit)) {
        return;
    }
    state.overlay = None;
    state.suspended_overlay = None;
    super::commit_editing_prompt(state);
    state.should_quit = true;
}

/// 取消退出：恢复打开确认前的浮层（没有则回到无浮层）。
pub fn cancel_quit(state: &mut AppState) {
    if !matches!(state.overlay, Some(Overlay::ConfirmQuit)) {
        return;
    }
    state.overlay = state.suspended_overlay.take();
}

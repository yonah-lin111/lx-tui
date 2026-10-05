//! 行为到状态的转换；几何信息由事件循环传入，保证逻辑纯且可测。

use std::time::Instant;

use ratatui::layout::Rect;

use crate::layout::{self, PaneId};

use super::actions::Action;
use super::selection::Selection;
use super::state::{AppState, PaneKind};
use super::toast::Toast;

/// 应用行为。
pub fn apply(action: Action, state: &mut AppState) {
    match action {
        Action::Quit => state.should_quit = true,
        Action::ToggleSidebar => state.sidebar_collapsed = !state.sidebar_collapsed,
        Action::TogglePrompt => state.prompt_collapsed = !state.prompt_collapsed,
        Action::ToggleAgents => state.agents_collapsed = !state.agents_collapsed,
    }
}

/// 按几何同步各窗格与 prompt 右栏的仿真尺寸。
pub fn resize_panes(state: &mut AppState, pane_rects: &[(PaneId, Rect)]) {
    for (id, rect) in pane_rects {
        let (cols, rows) = layout::pane_inner_size(*rect);
        if let Some(pane) = state.pane_mut_anywhere(*id) {
            pane.terminal.resize(cols, rows);
        }
    }
}

/// 喂入窗格输出；返回需要写回 PTY 的响应。
pub fn feed_pane(state: &mut AppState, id: PaneId, bytes: &[u8]) -> Vec<u8> {
    state
        .pane_mut_anywhere(id)
        .map(|pane| pane.terminal.feed(bytes))
        .unwrap_or_default()
}

/// 标记窗格进程已退出；保留最后一屏。
pub fn mark_pane_exited(state: &mut AppState, id: PaneId) {
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.exited = true;
    }
}

/// 在区域内容区开始一次文本选择；与右栏拖拽互斥。
pub fn begin_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    state.resizing_prompt = false;
    state.selection = Some(Selection::begin(pane, row, col));
}

/// 扩展当前选区；窗格不一致时忽略。
pub fn drag_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    if let Some(selection) = state.selection.as_mut()
        && selection.pane() == pane
    {
        selection.drag(row, col);
    }
}

/// 清除选区。
pub fn clear_selection(state: &mut AppState) {
    state.selection = None;
}

/// 结束选区并提取文本（选区保留高亮）；未拖动或空占位窗格返回 None。
pub fn finish_selection(state: &mut AppState) -> Option<String> {
    let selection = state.selection?;
    let (start, end) = selection.range()?;
    let pane = state.pane_mut_anywhere(selection.pane())?;
    if !matches!(pane.kind, PaneKind::Terminal | PaneKind::Prompt) {
        return None;
    }
    pane.terminal
        .text_in_range(start, end)
        .filter(|text| !text.is_empty())
}

/// 在 prompt 右栏分割线上开始拖拽；清除已有选区。
pub fn begin_prompt_resize(state: &mut AppState) {
    state.selection = None;
    state.resizing_prompt = true;
}

/// 拖拽中更新右栏宽度；未处于拖拽时忽略。
pub fn drag_prompt(state: &mut AppState, width: u16) {
    if state.resizing_prompt {
        state.prompt_width = width;
    }
}

/// 结束右栏拖拽。
pub fn end_prompt_resize(state: &mut AppState) {
    state.resizing_prompt = false;
}

/// 更新右栏分割线悬停状态；返回是否发生变化。
pub fn set_prompt_hover(state: &mut AppState, hover: bool) -> bool {
    if state.prompt_hover == hover {
        return false;
    }
    state.prompt_hover = hover;
    true
}

/// 展示 toast；单条替换并重置倒计时。
pub fn show_toast(state: &mut AppState, toast: Toast) {
    state.toast = Some(toast);
}

/// 关闭当前 toast。
pub fn dismiss_toast(state: &mut AppState) {
    state.toast = None;
}

/// 更新 toast 悬停状态；返回是否发生变化。
pub fn set_toast_hover(state: &mut AppState, hovered: bool) -> bool {
    let Some(toast) = state.toast.as_mut() else {
        return false;
    };
    if toast.hovered() == hovered {
        return false;
    }
    toast.set_hovered(hovered);
    true
}

/// 清除已过期的 toast；返回是否发生变化（用于置脏重绘）。
pub fn tick(state: &mut AppState, now: Instant) -> bool {
    let expired = state
        .toast
        .as_ref()
        .is_some_and(|toast| toast.is_expired(now));
    if expired {
        state.toast = None;
    }
    expired
}

/// 最近一次 toast 到期时间；事件循环据此安排唤醒。
pub fn next_deadline(state: &AppState) -> Option<Instant> {
    state.toast.as_ref().and_then(Toast::next_deadline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::toast::{TOAST_DURATION, ToastKind};
    use crate::terminal::GridSize;
    use std::time::Duration;

    #[test]
    fn quit_sets_flag() {
        let mut state = AppState::demo();
        apply(Action::Quit, &mut state);
        assert!(state.should_quit);
    }

    #[test]
    fn toggle_sidebar_flips_flag() {
        let mut state = AppState::demo();
        apply(Action::ToggleSidebar, &mut state);
        assert!(state.sidebar_collapsed);
        apply(Action::ToggleSidebar, &mut state);
        assert!(!state.sidebar_collapsed);
    }

    #[test]
    fn toggle_prompt_flips_flag() {
        let mut state = AppState::demo();
        apply(Action::TogglePrompt, &mut state);
        assert!(state.prompt_collapsed);
        apply(Action::TogglePrompt, &mut state);
        assert!(!state.prompt_collapsed);
    }

    #[test]
    fn toggle_agents_flips_flag() {
        let mut state = AppState::demo();
        apply(Action::ToggleAgents, &mut state);
        assert!(state.agents_collapsed);
        apply(Action::ToggleAgents, &mut state);
        assert!(!state.agents_collapsed);
    }

    #[test]
    fn resize_syncs_terminal_size() {
        let mut state = AppState::demo();
        let id = state.active_tab().layout.focus();
        resize_panes(&mut state, &[(id, Rect::new(0, 1, 12, 6))]);
        let pane = state.active_tab().pane(id);
        assert_eq!(
            pane.map(|pane| pane.terminal.size()),
            Some(GridSize { cols: 10, rows: 4 })
        );
    }

    #[test]
    fn resize_syncs_prompt_terminal_size() {
        let mut state = AppState::demo();
        let id = state.prompt.id();
        resize_panes(&mut state, &[(id, Rect::new(70, 0, 30, 20))]);
        let size = state.prompt.pane().terminal.size();
        assert_eq!(size, GridSize { cols: 28, rows: 18 });
    }

    #[test]
    fn feed_and_exit_mark_pane() {
        let mut state = AppState::demo();
        let id = state.active_tab().layout.focus();
        let responses = feed_pane(&mut state, id, b"\x1b[6n");
        assert_eq!(String::from_utf8_lossy(&responses), "\x1b[1;1R");
        mark_pane_exited(&mut state, id);
        assert_eq!(
            state.active_tab().pane(id).map(|pane| pane.exited),
            Some(true)
        );
    }

    #[test]
    fn selection_finish_extracts_terminal_text() {
        let mut state = AppState::demo();
        let logs = state.workspaces[0].tabs[1].layout.pane_ids()[0];
        feed_pane(&mut state, logs, b"hello");
        begin_selection(&mut state, logs, 0, 0);
        drag_selection(&mut state, logs, 0, 4);
        assert_eq!(finish_selection(&mut state).as_deref(), Some("hello"));
        assert!(state.selection.is_some());
    }

    #[test]
    fn selection_ignores_non_terminal_pane() {
        let mut state = AppState::demo();
        let focus = state.active_tab().layout.focus();
        if let Some(pane) = state.active_tab_mut().pane_mut(focus) {
            pane.kind = PaneKind::Placeholder;
        }
        begin_selection(&mut state, focus, 0, 0);
        drag_selection(&mut state, focus, 0, 3);
        assert_eq!(finish_selection(&mut state), None);
        assert!(state.selection.is_some());
    }

    #[test]
    fn selection_extracts_prompt_text() {
        let mut state = AppState::demo();
        let prompt = state.prompt.id();
        begin_selection(&mut state, prompt, 0, 0);
        drag_selection(&mut state, prompt, 0, 3);
        assert_eq!(finish_selection(&mut state).as_deref(), Some("Drag"));
    }

    #[test]
    fn selection_drag_ignores_other_pane() {
        let mut state = AppState::demo();
        let focus = state.active_tab().layout.focus();
        let logs = state.workspaces[0].tabs[1].layout.pane_ids()[0];
        begin_selection(&mut state, logs, 0, 0);
        drag_selection(&mut state, focus, 2, 2);
        assert_eq!(finish_selection(&mut state), None);
    }

    #[test]
    fn begin_prompt_resize_clears_selection() {
        let mut state = AppState::demo();
        let focus = state.active_tab().layout.focus();
        begin_selection(&mut state, focus, 0, 0);
        begin_prompt_resize(&mut state);
        assert!(state.selection.is_none());
        assert!(state.resizing_prompt);
    }

    #[test]
    fn begin_selection_clears_prompt_resize() {
        let mut state = AppState::demo();
        let focus = state.active_tab().layout.focus();
        begin_prompt_resize(&mut state);
        begin_selection(&mut state, focus, 0, 0);
        assert!(!state.resizing_prompt);
        assert!(state.selection.is_some());
    }

    #[test]
    fn drag_prompt_only_applies_while_resizing() {
        let mut state = AppState::demo();
        let before = state.prompt_width;
        drag_prompt(&mut state, 40);
        assert_eq!(state.prompt_width, before);
        begin_prompt_resize(&mut state);
        drag_prompt(&mut state, 40);
        assert_eq!(state.prompt_width, 40);
        end_prompt_resize(&mut state);
        assert!(!state.resizing_prompt);
    }

    #[test]
    fn set_prompt_hover_reports_changes() {
        let mut state = AppState::demo();
        assert!(set_prompt_hover(&mut state, true));
        assert!(!set_prompt_hover(&mut state, true));
        assert!(set_prompt_hover(&mut state, false));
        assert!(!state.prompt_hover);
    }

    #[test]
    fn show_toast_replaces_previous_and_resets_deadline() {
        let mut state = AppState::demo();
        let now = Instant::now();
        show_toast(&mut state, Toast::new(ToastKind::Info, "first", None, now));
        show_toast(&mut state, Toast::new(ToastKind::Info, "second", None, now));
        let toast = state.toast.as_ref().expect("toast is shown");
        assert_eq!(toast.message, "second");
        assert_eq!(toast.next_deadline(), Some(now + TOAST_DURATION));
    }

    #[test]
    fn tick_clears_expired_toast_only() {
        let mut state = AppState::demo();
        let now = Instant::now();
        show_toast(&mut state, Toast::new(ToastKind::Info, "hello", None, now));
        assert!(!tick(
            &mut state,
            now + TOAST_DURATION - Duration::from_millis(1)
        ));
        assert!(state.toast.is_some());
        assert!(tick(&mut state, now + TOAST_DURATION));
        assert!(state.toast.is_none());
        assert!(!tick(&mut state, now + TOAST_DURATION));
    }

    #[test]
    fn toast_hover_suspends_expiry_until_pointer_leaves() {
        let mut state = AppState::demo();
        let now = Instant::now();
        show_toast(&mut state, Toast::new(ToastKind::Info, "hello", None, now));
        assert!(set_toast_hover(&mut state, true));
        assert!(!set_toast_hover(&mut state, true));
        assert!(!tick(&mut state, now + Duration::from_secs(60)));
        assert!(state.toast.is_some());
        assert_eq!(next_deadline(&state), None);
        assert!(set_toast_hover(&mut state, false));
        assert!(tick(&mut state, now + Duration::from_secs(60)));
        assert!(state.toast.is_none());
    }

    #[test]
    fn hover_without_toast_is_noop() {
        let mut state = AppState::demo();
        assert!(!set_toast_hover(&mut state, true));
        assert_eq!(next_deadline(&state), None);
        dismiss_toast(&mut state);
        assert!(state.toast.is_none());
    }
}

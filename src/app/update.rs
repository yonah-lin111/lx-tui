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
mod tests;

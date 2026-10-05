//! 行为到状态的转换；几何信息由事件循环传入，保证逻辑纯且可测。

use std::time::Instant;

use ratatui::layout::Rect;

use crate::layout::{self, PaneId};

use super::actions::{Action, EditorCommand};
use super::selection::Selection;
use super::state::{AppState, PaneKind};
use super::toast::Toast;

/// 应用行为。
pub fn apply(action: Action, state: &mut AppState) {
    match action {
        Action::Quit => state.should_quit = true,
        Action::ToggleSidebar => state.sidebar_collapsed = !state.sidebar_collapsed,
        Action::TogglePrompt => {
            state.prompt_collapsed = !state.prompt_collapsed;
            if state.prompt_collapsed {
                state.prompt_focused = false;
            }
        }
        Action::ToggleAgents => state.agents_collapsed = !state.agents_collapsed,
    }
}

/// 按键产生的编辑命令；仅当 prompt 聚焦时由事件循环调用。
pub fn apply_editor(state: &mut AppState, command: EditorCommand) {
    match command {
        EditorCommand::InsertChar(ch) => state.prompt.insert_char(ch),
        EditorCommand::InsertText(text) => state.prompt.insert_str(&text),
        EditorCommand::Newline => state.prompt.newline(),
        EditorCommand::Backspace => state.prompt.backspace(),
        EditorCommand::Delete => state.prompt.delete(),
        EditorCommand::Left => state.prompt.move_left(),
        EditorCommand::Right => state.prompt.move_right(),
        EditorCommand::Up => state.prompt.move_up(),
        EditorCommand::Down => state.prompt.move_down(),
        EditorCommand::Home => state.prompt.move_home(),
        EditorCommand::End => state.prompt.move_end(),
        EditorCommand::LineStart => state.prompt.move_line_start(),
        EditorCommand::LineEnd => state.prompt.move_line_end(),
        EditorCommand::WordLeft => state.prompt.move_word_backward(),
        EditorCommand::WordRight => state.prompt.move_word_forward(),
        EditorCommand::DeleteToLineStart => state.prompt.delete_to_line_start(),
        EditorCommand::DeleteToLineEnd => state.prompt.delete_to_line_end(),
        EditorCommand::DeleteWordBackward => state.prompt.delete_word_backward(),
        EditorCommand::DeleteWordForward => state.prompt.delete_word_forward(),
        EditorCommand::Indent => state.prompt.indent(),
        EditorCommand::Outdent => state.prompt.outdent(),
        EditorCommand::Undo => state.prompt.undo(),
        EditorCommand::Redo => state.prompt.redo(),
    }
}

/// 滚轮一格滚动的视觉行数；对齐 opencode 默认步长。
const WHEEL_LINES: isize = 3;

/// 按方向滚动 prompt 视口（负数向上、正数向下）；光标不动，编辑后自动吸回。
pub fn scroll_prompt(state: &mut AppState, direction: isize) {
    state.prompt.scroll_by(direction.signum() * WHEEL_LINES);
}

/// 鼠标点击 prompt：把视口单元格映射为光标位置。
pub fn place_prompt_cursor(state: &mut AppState, row: u16, col: u16) {
    state.prompt.set_cursor_from_cell(row, col);
}

/// 点击 prompt：键盘焦点交给编辑器。
pub fn focus_prompt(state: &mut AppState) {
    state.prompt_focused = true;
}

/// 点击终端窗格：焦点回到窗格，prompt 失焦。
pub fn focus_pane(state: &mut AppState, id: PaneId) {
    state.prompt_focused = false;
    state.active_tab_mut().layout.focus_pane(id);
}

/// 按几何同步各窗格终端与 prompt 编辑器的尺寸。
pub fn resize_panes(state: &mut AppState, pane_rects: &[(PaneId, Rect)]) {
    for (id, rect) in pane_rects {
        let (cols, rows) = layout::pane_inner_size(*rect);
        if *id == state.prompt.id() {
            state.prompt.resize(cols, rows);
        } else if let Some(pane) = state.pane_mut_anywhere(*id) {
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

/// 结束选区并提取文本（选区保留高亮）；未拖动、prompt 空选区或空占位窗格返回 None。
pub fn finish_selection(state: &mut AppState) -> Option<String> {
    let selection = state.selection?;
    let (start, end) = selection.range()?;
    if selection.pane() == state.prompt.id() {
        return state.prompt.selection_text(start, end);
    }
    let pane = state.pane_mut_anywhere(selection.pane())?;
    if pane.kind != PaneKind::Terminal {
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

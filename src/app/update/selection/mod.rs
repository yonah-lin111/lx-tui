//! 选区操作：prompt、终端仿真器与 lx 页选区拖拽、自动滚动与复制。

mod autoscroll;
mod lx;

#[cfg(test)]
mod tests;

pub use autoscroll::{
    AUTOSCROLL_BAND_ROWS, AUTOSCROLL_INTERVAL, arm_selection_autoscroll, autoscroll_edge,
    autoscroll_tick_lines, edge_scroll_lines, stop_lx_selection_autoscroll,
    stop_selection_autoscroll, wheel_prompt_selection,
};
pub(crate) use autoscroll::{scroll_prompt_selection, tick_selection_autoscroll};
pub use lx::{
    begin_lx_input_selection, begin_lx_input_selection_with_clicks, begin_lx_selection,
    begin_lx_selection_range, clear_lx_selection, clear_lx_selection_for, drag_lx_input_selection,
    drag_lx_selection, drag_lx_selection_range, end_lx_input_selection_drag, end_lx_selection_drag,
    lx_selection_dragging_on, take_lx_input_selection, take_lx_selection,
};

use crate::app::overlay::Overlay;
use crate::app::selection::{Selection, SelectionUnit};
use crate::app::state::{AppState, PaneKind};
use crate::tiling::PaneId;

/// 全局滚轮一格滚动的视觉行数/条目数；对齐 opencode 默认步长。
pub const WHEEL_LINES: isize = 3;

/// 在 prompt 开始一次文本选择；与右栏拖拽、终端选区互斥。
pub fn begin_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    begin_selection_with_clicks(state, pane, row, col, 1);
}

/// 按连续点击次数（1=单字、2=选词、3=选逻辑行）在 prompt 开始文本选择。
pub fn begin_selection_with_clicks(
    state: &mut AppState,
    pane: PaneId,
    row: u16,
    col: u16,
    clicks: u8,
) {
    state.resizing_prompt = false;
    clear_selection(state);
    let content_row = selection_content_row(state, pane, row);
    let unit = SelectionUnit::from_clicks(clicks);
    if pane == state.prompt.id() && unit != SelectionUnit::Character {
        let ((start, end), (_start_byte, end_byte)) = state.prompt.unit_cell_range(
            usize::try_from(content_row.max(0)).unwrap_or(0),
            col,
            unit,
        );
        state.selection = Some(Selection::begin_range(pane, start, end, unit));
        state.prompt.set_cursor_offset(end_byte);
    } else {
        state.selection = Some(Selection::begin(pane, content_row, col));
    }
}

/// 扩展当前 prompt 选区；窗格不一致时忽略。
pub fn drag_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    let content_row = selection_content_row(state, pane, row);
    let Some(selection) = state.selection.filter(|sel| sel.pane() == pane) else {
        return;
    };
    let unit = selection.unit();
    if pane == state.prompt.id() && unit != SelectionUnit::Character {
        let ((start, end), (start_byte, end_byte)) = state.prompt.unit_cell_range(
            usize::try_from(content_row.max(0)).unwrap_or(0),
            col,
            unit,
        );
        let origin_start = selection.origin().0;
        if let Some(sel) = state.selection.as_mut() {
            sel.drag_range(start, end);
        }
        state.prompt.set_cursor_offset(if start < origin_start {
            start_byte
        } else {
            end_byte
        });
    } else if let Some(sel) = state.selection.as_mut() {
        sel.drag(content_row, col);
    }
}

/// 视口行换算成内容行：只有 prompt 视口会滚动，加回滚动量即可。
fn selection_content_row(state: &AppState, pane: PaneId, row: u16) -> i32 {
    if pane == state.prompt.id() {
        i32::from(row) + state.prompt.scroll() as i32
    } else {
        i32::from(row)
    }
}

/// 在终端窗格开始拖拽选择；选区存于仿真器，随输出滚动钉在内容上。
pub fn begin_terminal_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    begin_terminal_selection_with_clicks(state, pane, row, col, 1);
}

/// 按连续点击次数（1=单字、2=选词、3=选行）在终端窗格开始选择。
pub fn begin_terminal_selection_with_clicks(
    state: &mut AppState,
    pane: PaneId,
    row: u16,
    col: u16,
    clicks: u8,
) {
    clear_selection(state);
    let started = state.pane_mut_anywhere(pane).is_some_and(|target| {
        if target.kind != PaneKind::Terminal {
            return false;
        }
        target
            .terminal
            .start_selection_with_clicks(row, col, clicks);
        true
    });
    state.terminal_selection = started.then_some(pane);
}

/// 扩展终端选区到新的视口坐标；目标窗格不一致时忽略。
pub fn drag_terminal_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    if state.terminal_selection != Some(pane) {
        return;
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.terminal.update_selection(row, col);
    }
}

/// 松开鼠标：结束终端选区拖拽；非空选区保留在仿真器中供 Ctrl/Cmd+C 复制，
/// 空选区或全空白选区直接清除；返回原本是否处于终端选区拖拽中。
pub fn end_terminal_selection_drag(state: &mut AppState) -> bool {
    let Some(pane) = state.terminal_selection.take() else {
        return false;
    };
    state.selection_autoscroll = None;
    if let Some(target) = state.pane_mut_anywhere(pane)
        && target.terminal.selection_text().is_none()
    {
        target.terminal.clear_selection();
    }
    true
}

/// 终端窗格当前是否存在可见选区范围（与 `renderable_content().selection` 同源，不分配字符串）。
pub fn terminal_has_selection(state: &AppState, pane: PaneId) -> bool {
    state
        .pane_anywhere(pane)
        .is_some_and(|target| target.kind == PaneKind::Terminal && target.terminal.has_selection())
}

/// 读取终端窗格当前保留的选区文本（保留高亮，供 Ctrl/Cmd+C 复制）；无有效选区返回 None。
pub fn terminal_selection_text(state: &AppState, pane: PaneId) -> Option<String> {
    let target = state.pane_anywhere(pane)?;
    if target.kind != PaneKind::Terminal {
        return None;
    }
    target.terminal.selection_text()
}

/// 结束终端选区并提取文本；空选区返回 None。
pub fn finish_terminal_selection(state: &mut AppState, pane: PaneId) -> Option<String> {
    state.terminal_selection = None;
    state.selection_autoscroll = None;
    let target = state.pane_mut_anywhere(pane)?;
    if target.kind != PaneKind::Terminal {
        return None;
    }
    // 全空白选区没有可复制的内容，与空选区一样不产生文本。
    target
        .terminal
        .take_selection_text()
        .filter(|text| !text.trim().is_empty())
}

/// 清除终端选区（含拖拽中与松开后保留的仿真器高亮）。
pub fn clear_terminal_selection(state: &mut AppState) {
    state.terminal_selection = None;
    for workspace in &mut state.workspaces {
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(target) = tab.pane_mut(id) {
                    target.terminal.clear_selection();
                }
            }
        }
    }
}

/// 终止所有正在进行的鼠标选区拖拽、自动滚动与空选区锚点，但保留已成立的静态选区。
pub fn stop_active_selection_drags(state: &mut AppState) {
    state.selection_autoscroll = None;
    state.lx_selection_autoscroll = None;
    state.drag_autoscroll = None;
    state.lx_card_press = None;
    state.lx_scroll_drag = None;
    state.lx_history_scroll_drag = None;
    state.task_scroll_drag = None;
    state.task_list_scroll_drag = None;
    state.task_agents_scroll_drag = None;
    state.task_btw_scroll_drag = None;
    state.lx_input_drag = None;
    state.overlay_input_drag = false;
    state.new_agent_scroll_drag = None;
    if let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_mut() {
        confirm.scroll_drag = None;
    }
    state.ask_input_drag = None;
    state.todo_input_drag = false;
    state.prompt_search_drag = false;
    state.workspace_history.search_drag = false;
    end_selection_drag(state);
    let _ = end_lx_selection_drag(state);
    let _ = end_terminal_selection_drag(state);
    if state.prompt.is_preview_dragging() {
        state.prompt.finish_preview_selection();
    }
    if !state.workspace_history.search.has_selection() {
        state.workspace_history.search.clear_selection();
    }
    if let Some(overlay) = state.overlay.as_mut()
        && !overlay.has_selection()
    {
        overlay.clear_selection();
    }
    if state
        .todo
        .edit_input()
        .is_some_and(|inp| !inp.has_selection())
    {
        state.todo.clear_selection();
    }
    for workspace in &mut state.workspaces {
        if !workspace.prompt_panel.search.has_selection() {
            workspace.prompt_panel.search.clear_selection();
        }
        if !workspace.prompt_panel.pages_view.search.has_selection() {
            workspace.prompt_panel.pages_view.search.clear_selection();
        }
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(target) = tab.pane_mut(id) {
                    target.lx.with_active_editor(|editor| {
                        if editor.is_preview_dragging() {
                            editor.finish_preview_selection();
                        }
                        if editor.text_selection_range().is_none() {
                            editor.clear_text_selection();
                        }
                    });
                    if let Some(ask) = target.lx.ask.as_mut()
                        && let Some(editor) = ask.editor_mut()
                        && editor.text_selection_range().is_none()
                    {
                        editor.clear_text_selection();
                    }
                    if let Some(task) = target.lx.task.as_mut()
                        && !task.agents_search.has_selection()
                    {
                        task.agents_search.clear_selection();
                    }
                    if !target.lx.btw.search.has_selection() {
                        target.lx.btw.search.clear_selection();
                    }
                }
            }
        }
    }
}

/// 清除全应用所有容器的文本选区以及正在进行的鼠标拖选/自动滚动状态。
pub fn clear_selection(state: &mut AppState) {
    stop_active_selection_drags(state);
    state.selection = None;
    state.lx_selection = None;
    state.workspace_history.search.clear_selection();
    if let Some(overlay) = state.overlay.as_mut() {
        overlay.clear_selection();
    }
    state.prompt.clear_preview_selection();
    state.todo.clear_selection();
    for workspace in &mut state.workspaces {
        workspace.prompt_panel.search.clear_selection();
        workspace.prompt_panel.pages_view.search.clear_selection();
        for tab in &mut workspace.tabs {
            for id in tab.layout.pane_ids() {
                if let Some(target) = tab.pane_mut(id) {
                    target.lx.with_active_editor(|editor| {
                        editor.clear_text_selection();
                        editor.clear_preview_selection();
                    });
                    if let Some(ask) = target.lx.ask.as_mut() {
                        ask.clear_all_text_selections();
                    }
                    if let Some(task) = target.lx.task.as_mut() {
                        task.agents_search.clear_selection();
                    }
                    target.lx.btw.search.clear_selection();
                }
            }
        }
    }
    clear_terminal_selection(state);
}

/// 当前界面是否存在对用户可见的非空文本选区。
pub fn has_visible_selection(state: &AppState) -> bool {
    if let Some(overlay) = state.overlay.as_ref() {
        return overlay.has_selection();
    }

    if !state.sidebar_collapsed
        && super::workspace_history_mode(state)
        && state.workspace_history.search.has_selection()
    {
        return true;
    }

    if !state.prompt_collapsed {
        match state.right_tab {
            crate::app::state::RightTab::Prompt => {
                if super::prompt_tree_mode(state) {
                    if state.prompt_owner().prompt_panel.search.has_selection() {
                        return true;
                    }
                } else if super::prompt_pages_mode(state) {
                    if state
                        .prompt_owner()
                        .prompt_panel
                        .pages_view
                        .search
                        .has_selection()
                    {
                        return true;
                    }
                } else if state.prompt.preview_selection().is_some()
                    || prompt_selection_range(state).is_some()
                {
                    return true;
                }
            }
            crate::app::state::RightTab::Todo => {
                if state
                    .todo
                    .edit_input()
                    .is_some_and(|input| input.has_selection())
                {
                    return true;
                }
            }
        }
    }

    if !state.panes_collapsed {
        for id in state.active_tab().layout.pane_ids() {
            let Some(pane) = state.pane_anywhere(id) else {
                continue;
            };
            match pane.view {
                crate::app::state::PaneView::Lx => {
                    if state
                        .lx_selection
                        .is_some_and(|sel| sel.pane() == id && sel.range().is_some())
                    {
                        return true;
                    }
                    if pane.lx.btw_editing() || pane.lx.agents_editing() {
                        if pane.lx.active_editor().text_selection_range().is_some()
                            || pane.lx.active_editor().preview_selection().is_some()
                        {
                            return true;
                        }
                    } else if let Some(task) = pane.lx.task.as_ref() {
                        if task.agents_search.has_selection() || pane.lx.btw.search.has_selection()
                        {
                            return true;
                        }
                    } else if let Some(ask) = pane.lx.ask.as_ref() {
                        if ask.editing
                            && ask
                                .editor()
                                .is_some_and(|ed| ed.text_selection_range().is_some())
                        {
                            return true;
                        }
                    } else if pane.lx.preview_selection().is_some()
                        || pane.lx.text_selection_range().is_some()
                    {
                        return true;
                    }
                }
                crate::app::state::PaneView::Terminal => {
                    if terminal_has_selection(state, id) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// 按下 `Esc` 时尝试取消当前可见的文字选区（对齐 OpenCode `handleSelectionKey` 优先级 101）。
///
/// - 若存在可见的非空文本选区：清除选区并终止所有拖选/自动滚动状态，返回 `true`（消费该次 `Esc`）。
/// - 若不存在可见的非空文本选区：仅终止进行中的拖选、自动滚动与空选区锚点，返回 `false`（不消费 `Esc`）。
pub fn escape_clear_selection(state: &mut AppState) -> bool {
    let had_selection = has_visible_selection(state);
    state.selection_click = None;
    stop_active_selection_drags(state);
    if state.overlay.is_some() {
        if had_selection && let Some(overlay) = state.overlay.as_mut() {
            overlay.clear_selection();
        }
        return had_selection;
    }
    if had_selection {
        clear_selection(state);
    }
    had_selection
}

/// 松开鼠标：结束拖动；空选区（未拖动）直接清除，非空选区保留供复制或删除。
pub fn end_selection_drag(state: &mut AppState) {
    state.selection_autoscroll = None;
    let keep = state
        .selection
        .as_ref()
        .is_some_and(|selection| selection.range().is_some());
    if keep {
        if let Some(selection) = state.selection.as_mut() {
            selection.finish();
        }
    } else {
        state.selection = None;
    }
}

/// prompt 选区文本（保留选区，供 Ctrl/Cmd+C 复制）；空选区返回 None。
pub fn prompt_selection_text(state: &AppState) -> Option<String> {
    let (start, end) = prompt_selection_range(state)?;
    let allow_single_cell = state
        .selection
        .is_some_and(|sel| sel.unit() != SelectionUnit::Character);
    state
        .prompt
        .selection_text_with(start, end, allow_single_cell)
}

/// prompt 复制文本：优先展开区选区、普通选区；无选区时复制全文；无内容返回 None。
pub fn prompt_copy_text(state: &AppState) -> Option<String> {
    if let Some((start, end)) = state.prompt.preview_selection() {
        let text = state.prompt.projection().copy_display_range(start..end);
        if !text.is_empty() {
            return Some(text);
        }
    }
    if let Some(text) = prompt_selection_text(state) {
        return Some(text);
    }
    (!state.prompt.text().is_empty()).then(|| state.prompt.text().to_string())
}

/// prompt 选区对应的内容行列范围；选区不在 prompt 或为空时返回 None。
pub(crate) fn prompt_selection_range(state: &AppState) -> Option<((u16, u16), (u16, u16))> {
    let selection = state.selection?;
    if selection.pane() != state.prompt.id() {
        return None;
    }
    let (start, end) = selection.range()?;
    let content_row = |row: i32| u16::try_from(row).ok();
    Some((
        (content_row(start.0)?, start.1),
        (content_row(end.0)?, end.1),
    ))
}

/// prompt 选区对应的字节范围；选区不在 prompt 或为空时返回 None。
pub(crate) fn prompt_selection_bounds(state: &AppState) -> Option<(usize, usize)> {
    let (start, end) = prompt_selection_range(state)?;
    let allow_single_cell = state
        .selection
        .is_some_and(|sel| sel.unit() != SelectionUnit::Character);
    state
        .prompt
        .selection_bounds_with(start, end, allow_single_cell)
}

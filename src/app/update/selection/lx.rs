//! lx 页与 lx 输入框的选区拖拽操作。

use crate::app::selection::{Selection, SelectionUnit};
use crate::app::state::AppState;
use crate::tiling::PaneId;

use super::clear_selection;

/// 在 lx 主内容区开始一次文本选择（内容行、正文列坐标）。
pub fn begin_lx_selection(state: &mut AppState, id: PaneId, row: i32, col: u16) {
    begin_lx_selection_range(state, id, (row, col), (row, col), SelectionUnit::Character);
}

/// 按选区粒度（单字 / 双击选词 / 三击选行）在 lx 主内容区开始一次文本选择。
pub fn begin_lx_selection_range(
    state: &mut AppState,
    id: PaneId,
    start: (i32, u16),
    end: (i32, u16),
    unit: SelectionUnit,
) {
    clear_selection(state);
    state.lx_selection = Some(Selection::begin_range(id, start, end, unit));
}

/// 扩展 lx 主内容区选区（内容行、正文列坐标）；窗格不一致时忽略。
pub fn drag_lx_selection(state: &mut AppState, id: PaneId, row: i32, col: u16) {
    drag_lx_selection_range(state, id, (row, col), (row, col));
}

/// 按粒度区间扩展 lx 主内容区选区；窗格不一致时忽略。
pub fn drag_lx_selection_range(
    state: &mut AppState,
    id: PaneId,
    start: (i32, u16),
    end: (i32, u16),
) {
    if let Some(selection) = state.lx_selection.as_mut()
        && selection.pane() == id
    {
        selection.drag_range(start, end);
    }
}

/// 松开鼠标：结束 lx 主内容区选区拖动；非空选区保留供 Ctrl/Cmd+C 复制，空选区直接清除。
/// 返回是否存在保留下来的非空选区。
pub fn end_lx_selection_drag(state: &mut AppState) -> bool {
    state.lx_selection_autoscroll = None;
    let keep = state
        .lx_selection
        .as_ref()
        .is_some_and(|selection| selection.range().is_some());
    if keep {
        if let Some(selection) = state.lx_selection.as_mut() {
            selection.finish();
        }
        true
    } else {
        state.lx_selection = None;
        false
    }
}

/// 取走 lx 页选区；无选区返回 None。
pub fn take_lx_selection(state: &mut AppState) -> Option<Selection> {
    state.lx_selection.take()
}

/// 清除 lx 页选区。
pub fn clear_lx_selection(state: &mut AppState) {
    state.lx_selection = None;
    state.lx_selection_autoscroll = None;
}

/// 清除指定窗格上的 lx 选区与卡片按下记录。
pub fn clear_lx_selection_for(state: &mut AppState, id: PaneId) {
    if state
        .lx_selection
        .is_some_and(|selection| selection.pane() == id)
    {
        state.lx_selection = None;
        state.lx_selection_autoscroll = None;
    }
    if state
        .lx_card_press
        .as_ref()
        .is_some_and(|(pane, _)| *pane == id)
    {
        state.lx_card_press = None;
    }
}

/// 该窗格上是否存在进行中的 lx 主内容区选择拖拽。
pub fn lx_selection_dragging_on(state: &AppState, pane: PaneId) -> bool {
    state
        .lx_selection
        .is_some_and(|selection| selection.is_dragging() && selection.pane() == pane)
}

/// 在 lx 输入框内开始文本选择（视觉行、内容列）；光标落到该单元格。
pub fn begin_lx_input_selection(state: &mut AppState, id: PaneId, row: usize, col: u16) {
    begin_lx_input_selection_with_clicks(state, id, row, col, 1);
}

/// 按连续点击次数（1=单字、2=选词、3=选逻辑行）在 lx 输入框开始文本选择。
pub fn begin_lx_input_selection_with_clicks(
    state: &mut AppState,
    id: PaneId,
    row: usize,
    col: u16,
    clicks: u8,
) {
    clear_selection(state);
    if let Some(pane) = state.pane_mut_anywhere(id) {
        // BTW composer 是独立编辑器；命中几何已按 active_editor() 计算，状态也必须
        // 写回同一个编辑器，不能落到隐藏的主输入框。
        let hit = pane.lx.active_editor().paste_hit_at(row, col);
        if let Some(hit) = hit {
            if !hit.expanded {
                pane.lx
                    .with_active_editor(|editor| editor.toggle_paste(hit.id, hit.occurrence));
            } else {
                pane.lx
                    .with_active_editor(|editor| editor.begin_preview_selection(row, col, clicks));
                state.lx_input_drag = Some(id);
            }
            return;
        }
        pane.lx
            .with_active_editor(|editor| editor.begin_text_selection_with_clicks(row, col, clicks));
        state.lx_input_drag = Some(id);
    }
}

/// 拖拽扩展 lx 输入框选区；窗格不一致时忽略。
pub fn drag_lx_input_selection(state: &mut AppState, id: PaneId, row: usize, col: u16) {
    if state.lx_input_drag != Some(id) {
        return;
    }
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.with_active_editor(|editor| {
            if editor.is_preview_dragging() {
                editor.drag_preview_selection(row, col);
            } else {
                editor.drag_text_selection(row, col);
            }
        });
    }
}

/// 松开鼠标：结束 lx 输入框选区拖动；非空选区保留在输入框中供 Ctrl/Cmd+C 复制，
/// 空选区直接清除；返回原本是否处于 lx 输入框选区拖动中。
pub fn end_lx_input_selection_drag(state: &mut AppState) -> bool {
    let Some(id) = state.lx_input_drag.take() else {
        return false;
    };
    if let Some(pane) = state.pane_mut_anywhere(id) {
        if pane.lx.active_editor().is_preview_dragging() {
            pane.lx
                .with_active_editor(|editor| editor.finish_preview_selection());
            return true;
        }
        if pane.lx.active_editor().text_selection_range().is_none() {
            pane.lx
                .with_active_editor(|editor| editor.clear_text_selection());
        }
    }
    true
}

/// 结束 lx 输入框选区并取走选中文本；空选区或未拖拽返回 None，锚点总是清除。
pub fn take_lx_input_selection(state: &mut AppState) -> Option<(PaneId, String)> {
    let id = state.lx_input_drag.take()?;
    let pane = state.pane_mut_anywhere(id)?;
    pane.lx.with_active_editor(|editor| {
        let text = editor.selected_text();
        editor.clear_text_selection();
        text.map(|text| (id, text))
    })
}

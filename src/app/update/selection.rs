//! 选区操作：prompt、终端仿真器与 lx 页选区拖拽、自动滚动与复制。

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::app::selection::Selection;
use crate::app::state::{AppState, AutoscrollDirection, PaneKind, SelectionAutoscroll};
use crate::layout::PaneId;

use super::editor::place_prompt_cursor;
use super::pane::scroll_pane_lines;

/// 滚轮一格滚动的视觉行数；对齐 opencode 默认步长。
pub(crate) const WHEEL_LINES: isize = 3;

/// 边缘自动滚动每步的间隔。
pub(crate) const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(30);

/// 在 lx 主内容区开始一次文本选择（内容行、正文列坐标）。
pub fn begin_lx_selection(state: &mut AppState, id: PaneId, row: i32, col: u16) {
    clear_selection(state);
    state.lx_selection = Some(Selection::begin(id, row, col));
}

/// 扩展 lx 主内容区选区（内容行、正文列坐标）；窗格不一致时忽略。
pub fn drag_lx_selection(state: &mut AppState, id: PaneId, row: i32, col: u16) {
    if let Some(selection) = state.lx_selection.as_mut()
        && selection.pane() == id
    {
        selection.drag(row, col);
    }
}

/// 松开鼠标：结束 lx 主内容区选区拖动；非空选区保留供 Ctrl/Cmd+C 复制，空选区直接清除。
/// 返回是否存在保留下来的非空选区。
pub fn end_lx_selection_drag(state: &mut AppState) -> bool {
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
}

/// 清除指定窗格上的 lx 选区与卡片按下记录。
pub fn clear_lx_selection_for(state: &mut AppState, id: PaneId) {
    if state
        .lx_selection
        .is_some_and(|selection| selection.pane() == id)
    {
        state.lx_selection = None;
    }
    if state
        .lx_card_press
        .as_ref()
        .is_some_and(|(pane, _)| *pane == id)
    {
        state.lx_card_press = None;
    }
}

/// 在 lx 输入框内开始文本选择（视觉行、内容列）；光标落到该单元格。
pub fn begin_lx_input_selection(state: &mut AppState, id: PaneId, row: usize, col: u16) {
    clear_selection(state);
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.begin_text_selection(row, col);
        state.lx_input_drag = Some(id);
    }
}

/// 拖拽扩展 lx 输入框选区；窗格不一致时忽略。
pub fn drag_lx_input_selection(state: &mut AppState, id: PaneId, row: usize, col: u16) {
    if state.lx_input_drag != Some(id) {
        return;
    }
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.drag_text_selection(row, col);
    }
}

/// 结束 lx 输入框选区并取走选中文本；空选区或未拖拽返回 None，锚点总是清除。
pub fn take_lx_input_selection(state: &mut AppState) -> Option<(PaneId, String)> {
    let id = state.lx_input_drag.take()?;
    let pane = state.pane_mut_anywhere(id)?;
    let text = pane.lx.selected_text();
    pane.lx.clear_text_selection();
    text.map(|text| (id, text))
}

/// 在 prompt 开始一次文本选择；与右栏拖拽、终端选区互斥。
pub fn begin_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    state.resizing_prompt = false;
    clear_terminal_selection(state);
    state.selection_autoscroll = None;
    let row = selection_content_row(state, pane, row);
    state.selection = Some(Selection::begin(pane, row, col));
}

/// 扩展当前 prompt 选区；窗格不一致时忽略。
pub fn drag_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    let row = selection_content_row(state, pane, row);
    if let Some(selection) = state.selection.as_mut()
        && selection.pane() == pane
    {
        selection.drag(row, col);
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
    clear_selection(state);
    let started = state.pane_mut_anywhere(pane).is_some_and(|target| {
        if target.kind != PaneKind::Terminal {
            return false;
        }
        target.terminal.start_selection(row, col);
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

/// 清除 prompt、终端、lx 页与 lx 输入框选区以及输入框滚动条拖拽。
pub fn clear_selection(state: &mut AppState) {
    state.selection = None;
    state.selection_autoscroll = None;
    state.lx_selection = None;
    state.lx_card_press = None;
    state.lx_scroll_drag = None;
    state.lx_history_scroll_drag = None;
    state.task_agents_scroll_drag = None;
    if let Some(pane) = state.lx_input_drag.take()
        && let Some(target) = state.pane_mut_anywhere(pane)
    {
        target.lx.clear_text_selection();
    }
    clear_terminal_selection(state);
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

/// 拖拽选择时按鼠标相对窗格内容区的位置安排边缘自动滚动。
///
/// 鼠标越出上下边缘：立即按距离滚动并延伸选区，随后进入定时自动滚动；
/// 停在边缘行：只登记计划；回到内部：清除计划。
pub fn arm_selection_autoscroll(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    mouse: (u16, u16),
    now: Instant,
) {
    if !selection_dragging_on(state, pane) {
        state.selection_autoscroll = None;
        return;
    }
    let top = inner.y;
    let bottom = inner.bottom().saturating_sub(1);
    let (direction, distance) = if mouse.1 < top {
        (AutoscrollDirection::Up, top - mouse.1)
    } else if mouse.1 > bottom {
        (AutoscrollDirection::Down, mouse.1 - bottom)
    } else if mouse.1 == top {
        (AutoscrollDirection::Up, 0)
    } else if mouse.1 == bottom {
        (AutoscrollDirection::Down, 0)
    } else {
        state.selection_autoscroll = None;
        return;
    };
    if distance > 0 {
        autoscroll_step(
            state,
            pane,
            direction,
            edge_scroll_lines(distance),
            mouse,
            inner,
        );
    }
    state.selection_autoscroll = Some(SelectionAutoscroll {
        pane,
        direction,
        mouse,
        inner,
        next_at: now + AUTOSCROLL_INTERVAL,
    });
}

/// 停止边缘自动滚动。
pub fn stop_selection_autoscroll(state: &mut AppState) {
    state.selection_autoscroll = None;
}

/// 自动滚动到点后补一步；返回是否产生变化。
pub(crate) fn tick_selection_autoscroll(state: &mut AppState, now: Instant) -> bool {
    let Some(autoscroll) = state.selection_autoscroll else {
        return false;
    };
    if now < autoscroll.next_at {
        return false;
    }
    if !selection_dragging_on(state, autoscroll.pane) {
        state.selection_autoscroll = None;
        return false;
    }
    if !autoscroll_step(
        state,
        autoscroll.pane,
        autoscroll.direction,
        1,
        autoscroll.mouse,
        autoscroll.inner,
    ) {
        // 到达回滚边界或视口尽头，停止。
        state.selection_autoscroll = None;
        return false;
    }
    if let Some(autoscroll) = state.selection_autoscroll.as_mut() {
        autoscroll.next_at = now + AUTOSCROLL_INTERVAL;
    }
    true
}

/// 自动滚动一步：滚动若干行并把选区终点推进到鼠标位置；返回视口是否移动。
fn autoscroll_step(
    state: &mut AppState,
    pane: PaneId,
    direction: AutoscrollDirection,
    lines: isize,
    mouse: (u16, u16),
    inner: Rect,
) -> bool {
    let step = match direction {
        AutoscrollDirection::Up => -1,
        AutoscrollDirection::Down => 1,
    };
    let terminal = state.terminal_selection == Some(pane);
    let moved = if terminal {
        scroll_pane_lines(state, pane, step, lines)
    } else {
        // 拖选自动滚动到真实正文边界即停，不进入底部虚拟留白。
        let cap = state.prompt.text_max_scroll();
        if state.prompt.scroll() >= cap {
            false
        } else {
            scroll_prompt_selection(state, step, lines);
            if state.prompt.scroll() > cap {
                state.prompt.scroll_to(cap);
            }
            true
        }
    };
    if !moved {
        return false;
    }
    let row = mouse.1.clamp(inner.y, inner.bottom().saturating_sub(1)) - inner.y;
    let col = mouse.0.clamp(inner.x, inner.right().saturating_sub(1)) - inner.x;
    if terminal {
        drag_terminal_selection(state, pane, row, col);
    } else {
        drag_selection(state, pane, row, col);
        place_prompt_cursor(state, row, col);
    }
    true
}

/// 该窗格上是否存在进行中的选择拖拽。
fn selection_dragging_on(state: &AppState, pane: PaneId) -> bool {
    if state.terminal_selection == Some(pane) {
        return true;
    }
    state
        .selection
        .is_some_and(|selection| selection.is_dragging() && selection.pane() == pane)
}

/// 边缘距离对应的立即滚动行数（对齐 herdr：距离 × 3，夹在 3..=15）。
fn edge_scroll_lines(distance: u16) -> isize {
    usize::from(distance).saturating_mul(3).clamp(3, 15) as isize
}

/// 滚动 prompt 视口；选区是内容行坐标，视口滚动天然不影响选区；返回视口是否移动。
pub(crate) fn scroll_prompt_selection(
    state: &mut AppState,
    direction: isize,
    lines: isize,
) -> bool {
    let before = state.prompt.scroll();
    state.prompt.scroll_by(direction.signum() * lines);
    state.prompt.scroll() != before
}

/// prompt 拖拽选区时滚轮：滚动视口并保持选区锚点，终点跟到鼠标单元格；返回视口是否移动。
pub fn wheel_prompt_selection(state: &mut AppState, direction: isize, row: u16, col: u16) -> bool {
    if !scroll_prompt_selection(state, direction, WHEEL_LINES) {
        return false;
    }
    let prompt = state.prompt.id();
    drag_selection(state, prompt, row, col);
    place_prompt_cursor(state, row, col);
    true
}

/// prompt 选区文本（保留选区，供 Ctrl/Cmd+C 复制）；空选区返回 None。
pub fn prompt_selection_text(state: &AppState) -> Option<String> {
    let (start, end) = prompt_selection_range(state)?;
    state.prompt.selection_text(start, end)
}

/// prompt 复制文本：优先选区；无选区时复制全文；无内容返回 None。
pub fn prompt_copy_text(state: &AppState) -> Option<String> {
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
    state.prompt.selection_bounds(start, end)
}

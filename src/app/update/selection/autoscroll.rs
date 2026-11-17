//! 拖拽边缘自动滚动通用计算与 prompt / 终端选区自动滚动。

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::app::selection::SelectionUnit;
use crate::app::state::{AppState, AutoscrollDirection, SelectionAutoscroll};
use crate::tiling::PaneId;

use super::super::editor::place_prompt_cursor;
use super::super::pane::scroll_pane_lines;
use super::{WHEEL_LINES, drag_selection, drag_terminal_selection};

/// 边缘自动滚动每步的间隔。
pub const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(30);

/// 视口上下两端各两行作为自动滚动检测区。
pub const AUTOSCROLL_BAND_ROWS: u16 = 2;

/// 指针相对视口的位置：返回滚动方向与越出视口的行数（0 表示仍在检测区内）。
pub fn autoscroll_edge(rect: Rect, row: u16) -> Option<(AutoscrollDirection, u16)> {
    if rect.height == 0 {
        return None;
    }
    let band = AUTOSCROLL_BAND_ROWS.min(rect.height / 2).max(1);
    if row < rect.y {
        return Some((AutoscrollDirection::Up, rect.y - row));
    }
    if row >= rect.bottom() {
        return Some((AutoscrollDirection::Down, row - (rect.bottom() - 1)));
    }
    if row < rect.y.saturating_add(band) {
        return Some((AutoscrollDirection::Up, 0));
    }
    if row >= rect.bottom().saturating_sub(band) {
        return Some((AutoscrollDirection::Down, 0));
    }
    None
}

/// 每 tick 的滚动行数：越靠外越快，夹在 1..=3 行。
pub fn autoscroll_tick_lines(distance: u16) -> isize {
    (1 + distance / 3).clamp(1, 3) as isize
}

/// 边缘距离对应的立即滚动行数（对齐 herdr：距离 × 3，夹在 3..=15）。
pub fn edge_scroll_lines(distance: u16) -> isize {
    usize::from(distance).saturating_mul(3).clamp(3, 15) as isize
}

/// 拖拽选择时按鼠标相对窗格内容区的位置安排边缘自动滚动。
///
/// 鼠标越出上下边缘：立即按距离滚动并延伸选区，随后进入定时自动滚动；
/// 停在边缘行：保留已有同向定时节拍（避免横向移动时定时器饥饿）；回到内部：清除计划。
pub fn arm_selection_autoscroll(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    mouse: (u16, u16),
    now: Instant,
) {
    if inner.width == 0 || inner.height == 0 || !selection_dragging_on(state, pane) {
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
    let prev = state
        .selection_autoscroll
        .filter(|plan| plan.pane == pane && plan.direction == direction);
    let prev_distance = prev.map_or(0, |plan| edge_distance(plan.inner, plan.mouse.1));
    let next_at = prev.map_or(now + AUTOSCROLL_INTERVAL, |plan| plan.next_at);
    let should_jump =
        distance > 0 && (prev.is_none() || prev_distance == 0 || distance > prev_distance);
    if should_jump {
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
        next_at,
    });
}

fn edge_distance(inner: Rect, row: u16) -> u16 {
    if inner.height == 0 {
        return 0;
    }
    let bottom = inner.bottom().saturating_sub(1);
    if row < inner.y {
        inner.y - row
    } else {
        row.saturating_sub(bottom)
    }
}

/// 停止边缘自动滚动（prompt/终端、lx 主内容区与拖拽列表共用的清理入口）。
pub fn stop_selection_autoscroll(state: &mut AppState) {
    state.selection_autoscroll = None;
    state.lx_selection_autoscroll = None;
    state.drag_autoscroll = None;
}

/// 停止 lx 主内容区拖选的边缘自动滚动。
pub fn stop_lx_selection_autoscroll(state: &mut AppState) {
    state.lx_selection_autoscroll = None;
}

/// 自动滚动到点后补一步；返回是否产生变化。
pub(crate) fn tick_selection_autoscroll(state: &mut AppState, now: Instant) -> bool {
    let Some(autoscroll) = state.selection_autoscroll else {
        return false;
    };
    if state.overlay.is_some() || !selection_dragging_on(state, autoscroll.pane) {
        state.selection_autoscroll = None;
        return false;
    }
    if now < autoscroll.next_at {
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
    if inner.width == 0 || inner.height == 0 {
        return false;
    }
    let step = match direction {
        AutoscrollDirection::Up => -1,
        AutoscrollDirection::Down => 1,
    };
    let terminal = state.terminal_selection == Some(pane);
    let moved = if terminal {
        scroll_pane_lines(state, pane, step, lines)
    } else {
        // 拖选自动滚动到真实正文边界即停，不进入底部虚拟留白；向上从底部同样允许回滚到顶。
        match direction {
            AutoscrollDirection::Up => scroll_prompt_selection(state, -1, lines),
            AutoscrollDirection::Down => {
                let cap = state.prompt.text_max_scroll();
                let before = state.prompt.scroll();
                if before >= cap {
                    false
                } else {
                    scroll_prompt_selection(state, 1, lines);
                    if state.prompt.scroll() > cap {
                        state.prompt.scroll_to(cap);
                    }
                    state.prompt.scroll() != before
                }
            }
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
        if state
            .selection
            .is_none_or(|sel| sel.unit() == SelectionUnit::Character)
        {
            place_prompt_cursor(state, row, col);
        }
        drag_selection(state, pane, row, col);
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
    if state
        .selection
        .is_none_or(|sel| sel.unit() == SelectionUnit::Character)
    {
        place_prompt_cursor(state, row, col);
    }
    drag_selection(state, prompt, row, col);
    true
}

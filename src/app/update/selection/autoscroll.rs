//! lx 消息列表 / Agents 详情拖选边缘滚动。
//!
//! 从 `selection.rs` 拆出：保持主选区文件在 1000 行以内，滚动计划与 tick 只改
//! 视口和选区端点，不涉及渲染。

use std::time::Instant;

use ratatui::layout::Rect;

use crate::app::state::{AppState, AutoscrollDirection, LxAutoscrollTarget, LxSelectionAutoscroll};
use crate::layout::PaneId;

use super::{
    AUTOSCROLL_INTERVAL, WHEEL_LINES, drag_lx_selection, drag_screen_selection, edge_scroll_lines,
};

/// 登记 lx 消息列表 / Agents 详情的拖选边缘自动滚动。
#[allow(clippy::too_many_arguments)]
pub fn arm_lx_selection_autoscroll(
    state: &mut AppState,
    pane: PaneId,
    target: LxAutoscrollTarget,
    area: Rect,
    inner: Rect,
    total: usize,
    visible: usize,
    mouse: (u16, u16),
    now: Instant,
) {
    let active = state
        .lx_selection
        .is_some_and(|selection| selection.is_dragging() && selection.pane() == pane)
        || state
            .screen_selection
            .as_ref()
            .is_some_and(|selection| selection.is_dragging());
    if !active || area.width == 0 || area.height == 0 {
        state.lx_selection_autoscroll = None;
        return;
    }
    let top = area.y;
    let bottom = area.bottom().saturating_sub(1);
    let (direction, distance) = if mouse.1 < top {
        (AutoscrollDirection::Up, top - mouse.1)
    } else if mouse.1 > bottom {
        (AutoscrollDirection::Down, mouse.1 - bottom)
    } else if mouse.1 == top {
        (AutoscrollDirection::Up, 0)
    } else if mouse.1 == bottom {
        (AutoscrollDirection::Down, 0)
    } else {
        state.lx_selection_autoscroll = None;
        return;
    };
    if distance > 0 {
        lx_autoscroll_step(
            state,
            pane,
            target,
            direction,
            edge_scroll_lines(distance),
            area,
            inner,
            total,
            visible,
            mouse,
        );
    }
    state.lx_selection_autoscroll = Some(LxSelectionAutoscroll {
        pane,
        target,
        direction,
        area,
        inner,
        total,
        visible,
        mouse,
        next_at: now + AUTOSCROLL_INTERVAL,
    });
}

/// 滚轮在 lx 消息列表拖选中滚动；滚动视口并按实际位移平移选区端点。
#[allow(clippy::too_many_arguments)]
pub fn wheel_lx_selection(
    state: &mut AppState,
    pane: PaneId,
    target: LxAutoscrollTarget,
    area: Rect,
    inner: Rect,
    total: usize,
    visible: usize,
    direction: isize,
    mouse: (u16, u16),
) -> bool {
    if direction == 0 {
        return false;
    }
    let direction = if direction < 0 {
        AutoscrollDirection::Up
    } else {
        AutoscrollDirection::Down
    };
    lx_autoscroll_step(
        state,
        pane,
        target,
        direction,
        WHEEL_LINES,
        area,
        inner,
        total,
        visible,
        mouse,
    )
}

/// lx 拖选自动滚动到点后补一步；返回是否产生变化。
pub(crate) fn tick_lx_selection_autoscroll(state: &mut AppState, now: Instant) -> bool {
    let Some(autoscroll) = state.lx_selection_autoscroll else {
        return false;
    };
    if now < autoscroll.next_at {
        return false;
    }
    let active = state
        .lx_selection
        .is_some_and(|selection| selection.is_dragging() && selection.pane() == autoscroll.pane)
        || state
            .screen_selection
            .as_ref()
            .is_some_and(|selection| selection.is_dragging());
    if !active {
        state.lx_selection_autoscroll = None;
        return false;
    }
    if !lx_autoscroll_step(
        state,
        autoscroll.pane,
        autoscroll.target,
        autoscroll.direction,
        1,
        autoscroll.area,
        autoscroll.inner,
        autoscroll.total,
        autoscroll.visible,
        autoscroll.mouse,
    ) {
        state.lx_selection_autoscroll = None;
        return false;
    }
    if let Some(autoscroll) = state.lx_selection_autoscroll.as_mut() {
        autoscroll.next_at = now + AUTOSCROLL_INTERVAL;
    }
    true
}

/// 滚动 lx 历史 / Agents 详情，并按实际滚动行数平移选区。
#[allow(clippy::too_many_arguments)]
fn lx_autoscroll_step(
    state: &mut AppState,
    pane: PaneId,
    target: LxAutoscrollTarget,
    direction: AutoscrollDirection,
    lines: isize,
    area: Rect,
    inner: Rect,
    total: usize,
    visible: usize,
    mouse: (u16, u16),
) -> bool {
    let step: isize = match direction {
        AutoscrollDirection::Up => -1,
        AutoscrollDirection::Down => 1,
    };
    let before = lx_scroll_position(state, pane, target, total, visible);
    let moved = match target {
        LxAutoscrollTarget::History => crate::app::update::chat::scroll_lx_history(
            state,
            pane,
            step.saturating_mul(lines),
            total,
            visible,
        ),
        LxAutoscrollTarget::AgentsDetail => {
            crate::app::update::chat::scroll_agents_detail(state, pane, step.saturating_mul(lines))
        }
    };
    if !moved {
        return false;
    }
    let after =
        lx_scroll_position(state, pane, target, total, visible).unwrap_or(before.unwrap_or(0));
    let delta = match before {
        Some(before) => after as isize - before as isize,
        None => 0,
    };
    if delta == 0 {
        return false;
    }
    if let Some(selection) = state.lx_selection.as_mut()
        && selection.pane() == pane
    {
        selection.shift_rows(-delta as i32);
    }
    if let Some(selection) = state.screen_selection.as_mut() {
        selection.shift_rows(-delta as i32);
        state.screen_selection_refresh = Some(mouse);
    }
    if state.lx_selection.is_some() {
        let row = mouse.1.clamp(area.y, area.bottom().saturating_sub(1)) - inner.y;
        let col = mouse.0.clamp(area.x, area.right().saturating_sub(1)) - inner.x;
        drag_lx_selection(state, pane, row, col);
    }
    true
}

/// 当前 lx 滚动目标的顶部行；用于计算一次滚动的实际位移。
fn lx_scroll_position(
    state: &AppState,
    pane: PaneId,
    kind: LxAutoscrollTarget,
    total: usize,
    visible: usize,
) -> Option<usize> {
    let pane = state.pane_anywhere(pane)?;
    match kind {
        LxAutoscrollTarget::History => {
            let max_top = total.saturating_sub(visible);
            Some(if pane.lx.chat.follow_bottom {
                max_top
            } else {
                pane.lx.chat.history_scroll.min(max_top)
            })
        }
        LxAutoscrollTarget::AgentsDetail => pane
            .lx
            .agents
            .runs
            .get(pane.lx.task.as_ref()?.agents_window.active)
            .map(|run| run.scroll),
    }
}

/// 在下一帧最终画面取样完成后，刷新屏幕选区的终点。
pub fn apply_pending_screen_selection(
    state: &mut AppState,
    screen: &crate::ui::screen::ScreenText,
) -> bool {
    let Some((row, col)) = state.screen_selection_refresh.take() else {
        return false;
    };
    let Some(selection) = state.screen_selection.as_ref() else {
        return false;
    };
    if !selection.is_dragging() {
        return false;
    }
    let (width, height) = screen.size();
    let point = crate::app::selection::ScreenPoint::new(
        row.min(height.saturating_sub(1)),
        col.min(width.saturating_sub(1)),
    );
    drag_screen_selection(state, screen, point);
    true
}

//! 拖拽排序列表的边缘自动滚动：上下各两行检测区、30ms 一步、越靠外越快。

use std::time::Instant;

use ratatui::layout::Rect;

use crate::app::state::{AppState, AutoscrollDirection, DragAutoscroll, DragScrollSurface};
use crate::app::update;
use crate::ui;
use crate::ui::layout::ViewLayout;

/// 指针进入检测区或越出列表后，按列表类型滚动并同步落点。
pub(crate) fn arm_drag(
    state: &mut AppState,
    surface: DragScrollSurface,
    view: &ViewLayout,
    mouse: (u16, u16),
    now: Instant,
) {
    if state.overlay.is_some() || !drag_active(state, surface) {
        state.drag_autoscroll = None;
        return;
    }
    let Some(rect) = surface_rect(state, surface, view) else {
        state.drag_autoscroll = None;
        return;
    };
    if mouse.0 < rect.x || mouse.0 >= rect.right() {
        state.drag_autoscroll = None;
        return;
    }
    let Some((direction, distance)) = update::autoscroll_edge(rect, mouse.1) else {
        state.drag_autoscroll = None;
        return;
    };
    if distance > 0 {
        step(
            state,
            surface,
            view,
            direction,
            update::edge_scroll_lines(distance),
            mouse,
        );
    }
    state.drag_autoscroll = Some(DragAutoscroll {
        surface,
        direction,
        distance,
        mouse,
        next_at: now + update::AUTOSCROLL_INTERVAL,
    });
}

/// 到点后补一步；返回是否产生变化。
pub(crate) fn tick_drag(state: &mut AppState, view: &ViewLayout, now: Instant) -> bool {
    let Some(autoscroll) = state.drag_autoscroll else {
        return false;
    };
    if !drag_active(state, autoscroll.surface) {
        state.drag_autoscroll = None;
        return false;
    }
    if now < autoscroll.next_at {
        return false;
    }
    if !step(
        state,
        autoscroll.surface,
        view,
        autoscroll.direction,
        update::autoscroll_tick_lines(autoscroll.distance),
        autoscroll.mouse,
    ) {
        state.drag_autoscroll = None;
        return false;
    }
    if let Some(autoscroll) = state.drag_autoscroll.as_mut() {
        autoscroll.next_at = now + update::AUTOSCROLL_INTERVAL;
    }
    true
}

fn drag_active(state: &AppState, surface: DragScrollSurface) -> bool {
    match surface {
        DragScrollSurface::WorkspaceActive => state.workspace_drag.is_some(),
        DragScrollSurface::WorkspaceHistory => state.workspace_history.drag.is_some(),
        DragScrollSurface::Todo => state.todo_drag.is_some(),
        DragScrollSurface::PromptTree => state.prompt_drag.is_some(),
    }
}

fn surface_rect(state: &AppState, surface: DragScrollSurface, view: &ViewLayout) -> Option<Rect> {
    match surface {
        DragScrollSurface::WorkspaceActive => ui::workspace_list_rect(view, state),
        DragScrollSurface::WorkspaceHistory => ui::workspace_history_list_rect(view, state),
        DragScrollSurface::Todo => Some(ui::right_panel::todo::effective_list_rect(
            view.right_panel,
            state,
        )),
        DragScrollSurface::PromptTree => {
            let area = ui::right_panel::prompt::frame::prompt_main_rect(view.right_panel, state);
            let area = Rect {
                y: area.y.saturating_add(2),
                height: area.height.saturating_sub(2),
                ..area
            };
            (area.width > 0 && area.height > 0).then_some(area)
        }
    }
}

fn step(
    state: &mut AppState,
    surface: DragScrollSurface,
    view: &ViewLayout,
    direction: AutoscrollDirection,
    lines: isize,
    mouse: (u16, u16),
) -> bool {
    let delta = match direction {
        AutoscrollDirection::Up => -lines,
        AutoscrollDirection::Down => lines,
    };
    match surface {
        DragScrollSurface::WorkspaceActive => {
            let Some(rows) = ui::workspace_list_rows(view, state) else {
                return false;
            };
            if !update::scroll_workspace_list(state, delta, rows) {
                return false;
            }
            if let Some(target) = ui::workspace_drop_index(view, state, mouse.1) {
                update::drag_workspace_to(state, target);
            }
            true
        }
        DragScrollSurface::WorkspaceHistory => {
            let Some(rows) = ui::workspace_history_visible_rows(view, state) else {
                return false;
            };
            if !update::scroll_workspace_history(state, delta, rows) {
                return false;
            }
            if let Some(target) = ui::workspace_history_drop_index(view, state, mouse.1) {
                update::drag_workspace_history_to(state, target);
            }
            true
        }
        DragScrollSurface::Todo => {
            if !state.todo.scroll_by(delta) {
                return false;
            }
            let hit = ui::right_panel::todo::row_at(view.right_panel, state, mouse.0, mouse.1);
            update::drag_todo_to(state, hit);
            true
        }
        DragScrollSurface::PromptTree => {
            let visible = update::prompt_tree_visible_rows(state);
            if !update::scroll_prompt_tree(state, delta, visible) {
                return false;
            }
            let area = ui::right_panel::prompt::frame::prompt_main_rect(view.right_panel, state);
            let hit = {
                let panel = &state.prompt_owner().prompt_panel;
                ui::right_panel::prompt::tree::drop_hit_at(area, panel, mouse.0, mouse.1)
            };
            update::drag_prompt_to(state, hit);
            true
        }
    }
}

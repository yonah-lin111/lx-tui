//! 拖拽排序列表与 lx 主内容区选区拖拽的边缘自动滚动：
//! 上下各两行检测区、30ms 一步、越靠外越快；保持既定节拍不因横向移动而饥饿。

use std::time::Instant;

use ratatui::layout::Rect;

use crate::app::prompt::PromptDropHit;
use crate::app::selection::SelectionUnit;
use crate::app::state::{
    AppState, AutoscrollDirection, DragAutoscroll, DragScrollSurface, LxScrollSurface,
    LxSelectionAutoscroll, PaneView, RightTab, TaskTab, WorkspaceMode,
};
use crate::app::update;
use crate::tiling::{self, PaneId};
use crate::ui;
use crate::ui::layout::ViewLayout;

use super::super::session::Geometry;

/// 事件循环 tick：推进列表拖拽与 lx 选区边缘自动滚动；返回是否产生视觉变化。
pub(crate) fn tick(
    state: &mut AppState,
    geometry: &Geometry,
    cache: &mut ui::RenderCache,
    now: Instant,
) -> bool {
    let drag = tick_drag(state, &geometry.view, now);
    let lx = tick_lx_selection(state, geometry, cache, now);
    drag || lx
}

/// 列表拖拽期间：指针进入检测区或越出列表后，按列表类型滚动并同步落点。
pub(crate) fn arm_drag(
    state: &mut AppState,
    surface: DragScrollSurface,
    view: &ViewLayout,
    mouse: (u16, u16),
    now: Instant,
) {
    if state.overlay.is_some() || !horizontal_in_bounds(surface, view, mouse.0) {
        state.drag_autoscroll = None;
        return;
    }
    let Some(rect) = surface_rect(state, surface, view) else {
        state.drag_autoscroll = None;
        return;
    };
    let Some((direction, distance)) = update::autoscroll_edge(rect, mouse.1) else {
        state.drag_autoscroll = None;
        return;
    };
    if !drag_active(state, surface, distance) {
        state.drag_autoscroll = None;
        return;
    }
    let prev = state
        .drag_autoscroll
        .filter(|plan| plan.surface == surface && plan.direction == direction);
    let next_at = prev.map_or(now + update::AUTOSCROLL_INTERVAL, |plan| plan.next_at);
    let should_jump =
        distance > 0 && prev.is_none_or(|plan| plan.distance == 0 || distance > plan.distance);
    if should_jump
        && !step(
            state,
            surface,
            view,
            direction,
            update::edge_scroll_lines(distance),
            mouse,
        )
    {
        state.drag_autoscroll = None;
        return;
    }
    state.drag_autoscroll = Some(DragAutoscroll {
        surface,
        direction,
        distance,
        mouse,
        next_at,
    });
}

/// 列表拖拽到点后补一步；返回是否产生变化。
pub(crate) fn tick_drag(state: &mut AppState, view: &ViewLayout, now: Instant) -> bool {
    let Some(autoscroll) = state.drag_autoscroll else {
        return false;
    };
    if state.overlay.is_some()
        || !drag_active(state, autoscroll.surface, autoscroll.distance)
        || surface_rect(state, autoscroll.surface, view).is_none()
    {
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

/// lx 主内容区选区拖拽：指针进入上下检测区或越出正文区时登记/即时推进选区。
pub(crate) fn arm_lx_selection(
    state: &mut AppState,
    pane: PaneId,
    geometry: &Geometry,
    cache: &mut ui::RenderCache,
    mouse: (u16, u16),
    now: Instant,
) {
    if state.overlay.is_some() || !update::lx_selection_dragging_on(state, pane) {
        state.lx_selection_autoscroll = None;
        return;
    }
    let Some((rect, surface)) = lx_surface_geom(state, pane, &geometry.rects, cache) else {
        state.lx_selection_autoscroll = None;
        return;
    };
    let Some((direction, distance)) = update::autoscroll_edge(rect, mouse.1) else {
        state.lx_selection_autoscroll = None;
        return;
    };
    let prev = state
        .lx_selection_autoscroll
        .filter(|plan| plan.pane == pane && plan.surface == surface && plan.direction == direction);
    let next_at = prev.map_or(now + update::AUTOSCROLL_INTERVAL, |plan| plan.next_at);
    let should_jump =
        distance > 0 && prev.is_none_or(|plan| plan.distance == 0 || distance > plan.distance);
    if should_jump
        && !lx_step(
            state,
            pane,
            surface,
            direction,
            update::edge_scroll_lines(distance),
            mouse,
            geometry,
            cache,
        )
    {
        state.lx_selection_autoscroll = None;
        return;
    }
    state.lx_selection_autoscroll = Some(LxSelectionAutoscroll {
        pane,
        surface,
        direction,
        distance,
        mouse,
        next_at,
    });
}

/// lx 主内容区选区到点后补一步并按多击粒度推进终点；返回是否产生变化。
pub(crate) fn tick_lx_selection(
    state: &mut AppState,
    geometry: &Geometry,
    cache: &mut ui::RenderCache,
    now: Instant,
) -> bool {
    let Some(autoscroll) = state.lx_selection_autoscroll else {
        return false;
    };
    if state.overlay.is_some() || !update::lx_selection_dragging_on(state, autoscroll.pane) {
        state.lx_selection_autoscroll = None;
        return false;
    }
    if now < autoscroll.next_at {
        return false;
    }
    if !lx_step(
        state,
        autoscroll.pane,
        autoscroll.surface,
        autoscroll.direction,
        update::autoscroll_tick_lines(autoscroll.distance),
        autoscroll.mouse,
        geometry,
        cache,
    ) {
        state.lx_selection_autoscroll = None;
        return false;
    }
    if let Some(autoscroll) = state.lx_selection_autoscroll.as_mut() {
        autoscroll.next_at = now + update::AUTOSCROLL_INTERVAL;
    }
    true
}

/// Todo 拖拽落点（右栏水平范围内包含滚动条列，纵向钳到列表可见行；拖出右栏取消）。
pub(super) fn todo_drag_hit(
    view: &ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    if column < view.right_panel.x || column >= view.right_panel.right() {
        return None;
    }
    let list = ui::right_panel::todo::effective_list_rect(view.right_panel, state);
    if list.width == 0 || list.height == 0 {
        return None;
    }
    let col = column.clamp(list.x, list.right().saturating_sub(1));
    let clamped_row = row.clamp(list.y, list.bottom().saturating_sub(1));
    ui::right_panel::todo::row_at(view.right_panel, state, col, clamped_row)
}

/// Prompt 树拖拽落点（右栏水平范围内包含滚动条列，纵向钳到树行区以保持边沿落点；拖出右栏返回 Outside）。
pub(super) fn prompt_drag_hit(
    view: &ViewLayout,
    state: &AppState,
    column: u16,
    row: u16,
) -> PromptDropHit {
    if column < view.right_panel.x || column >= view.right_panel.right() {
        return PromptDropHit::Outside;
    }
    let area = ui::right_panel::prompt::frame::prompt_main_rect(view.right_panel, state);
    if area.width == 0 || area.height <= 2 {
        return PromptDropHit::Outside;
    }
    let rows_y = area.y.saturating_add(2);
    let col = column.clamp(area.x, area.right().saturating_sub(1));
    let clamped_row = row.clamp(rows_y, area.bottom().saturating_sub(1));
    ui::right_panel::prompt::tree::drop_hit_at(
        area,
        &state.prompt_owner().prompt_panel,
        col,
        clamped_row,
    )
}

fn horizontal_in_bounds(surface: DragScrollSurface, view: &ViewLayout, column: u16) -> bool {
    match surface {
        // 工作区活跃/历史落点本身横向不限，不因移出窄侧栏中断滚动。
        DragScrollSurface::WorkspaceActive | DragScrollSurface::WorkspaceHistory => true,
        // Todo/Prompt 树保持与右栏对齐（含滚动条列），横移离开右栏进主区时停止滚动。
        DragScrollSurface::Todo | DragScrollSurface::PromptTree => {
            column >= view.right_panel.x && column < view.right_panel.right()
        }
    }
}

fn drag_active(state: &AppState, surface: DragScrollSurface, distance: u16) -> bool {
    match surface {
        DragScrollSurface::WorkspaceActive => {
            state.workspace_drag.is_some() && state.workspace_history.mode == WorkspaceMode::Active
        }
        DragScrollSurface::WorkspaceHistory => {
            state.workspace_history.drag.is_some()
                && state.workspace_history.mode == WorkspaceMode::History
                && state.workspace_history.search.text().trim().is_empty()
        }
        DragScrollSurface::Todo => {
            state.right_tab == RightTab::Todo
                && !state.prompt_collapsed
                && state
                    .todo_drag
                    .as_ref()
                    .is_some_and(|drag| drag.dragging || distance > 0)
        }
        DragScrollSurface::PromptTree => {
            update::prompt_panel_active(state)
                && update::prompt_tree_mode(state)
                && state
                    .prompt_drag
                    .as_ref()
                    .is_some_and(|drag| drag.enabled && (drag.dragging || distance > 0))
        }
    }
}

fn surface_rect(state: &AppState, surface: DragScrollSurface, view: &ViewLayout) -> Option<Rect> {
    match surface {
        DragScrollSurface::WorkspaceActive => ui::workspace_list_rect(view, state),
        DragScrollSurface::WorkspaceHistory => ui::workspace_history_list_rect(view, state),
        DragScrollSurface::Todo => {
            if (!view.compact && state.prompt_collapsed)
                || view.right_panel.width <= crate::tiling::COLLAPSED_STRIP
                || state.right_tab != RightTab::Todo
            {
                return None;
            }
            let rect = ui::right_panel::todo::effective_list_rect(view.right_panel, state);
            (rect.width > 0 && rect.height > 0).then_some(rect)
        }
        DragScrollSurface::PromptTree => {
            if !update::prompt_panel_active(state)
                || !update::prompt_tree_mode(state)
                || view.right_panel.width <= crate::tiling::COLLAPSED_STRIP
            {
                return None;
            }
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
            let hit = todo_drag_hit(view, state, mouse.0, mouse.1);
            update::drag_todo_to(state, hit);
            true
        }
        DragScrollSurface::PromptTree => {
            let area = ui::right_panel::prompt::frame::prompt_main_rect(view.right_panel, state);
            let visible = usize::from(area.height.saturating_sub(2).max(1));
            if !update::scroll_prompt_tree(state, delta, visible) {
                return false;
            }
            let hit = prompt_drag_hit(view, state, mouse.0, mouse.1);
            update::drag_prompt_to(state, hit);
            true
        }
    }
}

fn lx_surface_geom(
    state: &AppState,
    pane: PaneId,
    rects: &[(PaneId, Rect)],
    cache: &mut ui::RenderCache,
) -> Option<(Rect, LxScrollSurface)> {
    let (_, rect) = rects.iter().find(|(id, _)| *id == pane)?;
    let inner = tiling::pane_inner_rect(*rect);
    let target = state.pane_anywhere(pane)?;
    if target.view != PaneView::Lx {
        return None;
    }
    let geom = ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane))?;
    if geom.rect.width == 0 || geom.rect.height == 0 {
        return None;
    }
    let surface = match target.lx.task.as_ref().map(|page| page.tab) {
        None => LxScrollSurface::History,
        Some(TaskTab::Processes) => LxScrollSurface::TaskOutput,
        Some(TaskTab::Agents) => LxScrollSurface::TaskAgents,
        Some(TaskTab::Btw) => LxScrollSurface::TaskBtw,
    };
    Some((geom.rect, surface))
}

#[allow(clippy::too_many_arguments)]
fn lx_step(
    state: &mut AppState,
    pane: PaneId,
    surface: LxScrollSurface,
    direction: AutoscrollDirection,
    lines: isize,
    mouse: (u16, u16),
    geometry: &Geometry,
    cache: &mut ui::RenderCache,
) -> bool {
    let Some((_, rect)) = geometry.rects.iter().find(|(id, _)| *id == pane) else {
        return false;
    };
    let inner = tiling::pane_inner_rect(*rect);
    if inner.width == 0 || inner.height == 0 {
        return false;
    }
    let Some(current_surface) = state
        .pane_anywhere(pane)
        .filter(|target| target.view == PaneView::Lx)
        .map(
            |target| match target.lx.task.as_ref().map(|page| page.tab) {
                None => LxScrollSurface::History,
                Some(TaskTab::Processes) => LxScrollSurface::TaskOutput,
                Some(TaskTab::Agents) => LxScrollSurface::TaskAgents,
                Some(TaskTab::Btw) => LxScrollSurface::TaskBtw,
            },
        )
    else {
        return false;
    };
    if current_surface != surface {
        return false;
    }
    let delta = match direction {
        AutoscrollDirection::Up => -lines,
        AutoscrollDirection::Down => lines,
    };
    let moved = match surface {
        LxScrollSurface::History => {
            let Some((total, visible)) = state.pane_anywhere(pane).and_then(|target| {
                ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane))
            }) else {
                return false;
            };
            update::scroll_lx_history(state, pane, delta, total, visible)
        }
        LxScrollSurface::TaskOutput => {
            update::resolve_task_output_visible(state, &geometry.rects);
            update::scroll_task_output(state, pane, delta)
        }
        LxScrollSurface::TaskAgents => {
            update::resolve_task_agents_visible(state, &geometry.rects, cache);
            update::scroll_agents_detail(state, pane, delta)
        }
        LxScrollSurface::TaskBtw => {
            update::resolve_task_btw_visible(state, &geometry.rects, cache);
            update::scroll_btw_detail(state, pane, delta)
        }
    };
    if !moved {
        return false;
    }
    let unit = state
        .lx_selection
        .map_or(SelectionUnit::Character, |selection| selection.unit());
    if let Some((start, end)) = state.pane_anywhere(pane).and_then(|target| {
        let geom = ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane))?;
        let cell = geom.cell(mouse.0, mouse.1);
        Some(ui::main_content::lx::unit_cell_range(
            inner,
            &target.lx,
            cell,
            unit,
            cache.pane(pane),
        ))
    }) {
        update::drag_lx_selection_range(state, pane, start, end);
    }
    true
}

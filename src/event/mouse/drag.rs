//! 鼠标拖拽与左键释放处理：滚动条/分割线/排序拖拽、选区延伸与选区复制。

use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::MouseEvent;
use ratatui::layout::Direction;

use crate::app::overlay::Overlay;
use crate::app::state::AppState;
use crate::app::update;
use crate::config::Config;
use crate::layout::{self, PaneId};
use crate::pty::PtySession;
use crate::ui;

use super::super::session::Geometry;
use super::hit::{pane_at, pane_content_inner};
use super::wheel::forward_pane_mouse_event;

/// 鼠标拖拽事件处理。
pub(crate) fn handle_mouse_drag(
    mouse: &MouseEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    config: &Config,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    let Geometry { rects, view, .. } = geometry;
    if let Some(grab) = state.referenced_search_scroll_drag {
        if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref()
            && let Some(layout) = ui::overlay::referenced_search_layout(geometry.screen, search)
            && let Some(bar) = layout.scrollbar
        {
            let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
            if update::set_referenced_search_viewport(state, offset, layout.visible()) {
                *dirty = true;
            }
        }
        return;
    }
    if let Some(grab) = state.referenced_menu_scroll_drag {
        if let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() {
            let layout = ui::overlay::referenced_menu_layout(geometry.screen, menu);
            if let Some(bar) = layout.scrollbar {
                let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
                if update::set_referenced_menu_viewport(state, offset, layout.visible()) {
                    *dirty = true;
                }
            }
        }
        return;
    }
    if let Some(grab) = state.task_scroll_drag {
        // 后台任务输出容器：按拖拽行换算目标偏移（与轨道点击同一套几何）。
        for (pane, rect) in rects {
            let inner = layout::pane_inner_rect(*rect);
            let Some(target) = state.pane_anywhere(*pane) else {
                continue;
            };
            let Some(rects) = ui::main_content::lx::task_rects(inner, &target.lx) else {
                continue;
            };
            let Some(page) = target.lx.task.as_ref() else {
                continue;
            };
            let Some(bar) = ui::main_content::lx::task::output_scrollbar(rects.output, page) else {
                continue;
            };
            let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
            if update::set_task_output_scroll(state, *pane, offset) {
                *dirty = true;
            }
            return;
        }
        return;
    }
    if let Some((pane, grab)) = state.task_agents_scroll_drag {
        // Agents 详情滚动条：与主历史滚动条同一套 thumb 几何。
        let bar = rects
            .iter()
            .find(|(id, _)| *id == pane)
            .and_then(|(_, rect)| {
                let inner = layout::pane_inner_rect(*rect);
                state.pane_anywhere(pane).and_then(|target| {
                    ui::main_content::lx::task_agents_scrollbar(inner, &target.lx, cache.pane(pane))
                })
            });
        let Some(bar) = bar else {
            return;
        };
        let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
        if update::set_task_agents_scroll(state, pane, offset) {
            *dirty = true;
        }
        return;
    }
    if let Some(grab) = state.prompt_scroll_drag {
        let Some(bar) = ui::prompt_scrollbar(view, state) else {
            return;
        };
        let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
        if update::prompt_tree_mode(state) {
            let visible = update::prompt_tree_visible_rows(state);
            if update::set_prompt_tree_scroll(state, offset, visible) {
                *dirty = true;
            }
        } else if update::set_prompt_scroll(state, offset) {
            *dirty = true;
        }
        return;
    }
    if let Some(grab) = state.todo_scroll_drag {
        let Some(bar) = ui::right_panel::todo::scrollbar(view.prompt, state) else {
            return;
        };
        let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
        if update::set_todo_scroll(state, offset) {
            *dirty = true;
        }
        return;
    }
    if let Some(grab) = state.agent_scroll_drag {
        let Some(bar) = ui::agent_scrollbar(view, state) else {
            return;
        };
        let rows = ui::agent_list_rows(view, state).unwrap_or(0);
        let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
        if update::set_agent_scroll(state, offset, rows) {
            *dirty = true;
        }
        return;
    }
    if let Some((pane, grab)) = state.terminal_scroll_drag {
        let bar = rects
            .iter()
            .find(|(id, _)| *id == pane)
            .and_then(|(_, rect)| {
                let inner = layout::pane_inner_rect(*rect);
                state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::scrollbar(inner, target))
            });
        let Some(bar) = bar else {
            return;
        };
        let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
        if update::set_terminal_scroll(state, pane, offset) {
            *dirty = true;
        }
        return;
    }
    if let Some((pane, grab)) = state.lx_scroll_drag {
        let bar = rects
            .iter()
            .find(|(id, _)| *id == pane)
            .and_then(|(_, rect)| {
                let inner = layout::pane_inner_rect(*rect);
                state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::input_scrollbar(inner, &target.lx))
            });
        let Some(bar) = bar else {
            return;
        };
        let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
        if update::set_lx_scroll(state, pane, offset) {
            *dirty = true;
        }
        return;
    }
    if let Some((pane, grab)) = state.lx_history_scroll_drag {
        let bar = rects
            .iter()
            .find(|(id, _)| *id == pane)
            .and_then(|(_, rect)| {
                let inner = layout::pane_inner_rect(*rect);
                state.pane_anywhere(pane).and_then(|target| {
                    ui::main_content::lx::history_scrollbar(inner, &target.lx, cache.pane(pane))
                })
            });
        let Some(bar) = bar else {
            return;
        };
        let offset = ui::widgets::scrollbar::offset_from_drag_row(&bar, mouse.row, grab);
        if update::set_lx_history_scroll(state, pane, offset, bar.total, bar.visible) {
            *dirty = true;
        }
        return;
    }
    if let Some(hit) = state.resizing_pane {
        let pos = match hit.direction {
            Direction::Horizontal => mouse.column,
            Direction::Vertical => mouse.row,
        };
        if update::drag_pane_boundary(
            state,
            hit,
            view.panes,
            pos,
            config.min_pane_width,
            config.min_pane_height,
        ) {
            *dirty = true;
        }
        return;
    }
    if state.resizing_sidebar {
        let width = ui::layout::sidebar_width_at(view, config, mouse.column);
        update::drag_sidebar(state, width);
        *dirty = true;
        return;
    }
    if state.resizing_prompt {
        let width = ui::layout::prompt_width_at(view, mouse.column, config.min_pane_width);
        update::drag_prompt(state, width);
        *dirty = true;
        return;
    }
    if state.workspace_drag.is_some() {
        if let Some(target) = ui::workspace_drop_index(view, state, mouse.row)
            && update::drag_workspace_to(state, target)
        {
            *dirty = true;
        }
        return;
    }
    // prompt 列表拖拽：必须在窗格转发之前 return，否则拖动会被当成终端选区/输入。
    if state.prompt_drag.is_some() {
        let hit = ui::prompt::tree::drop_hit_at(
            layout::prompt_content_rect(view.prompt),
            &state.prompt_owner().prompt_panel,
            mouse.column,
            mouse.row,
        );
        if update::drag_prompt_to(state, hit) {
            *dirty = true;
        }
        return;
    }
    // 屏幕选择优先于窗格转发与组件选区：Shift+拖动与静态文字拖动都走这里。
    if state
        .screen_selection
        .as_ref()
        .is_some_and(|selection| selection.is_dragging())
    {
        let point = crate::app::selection::ScreenPoint::new(
            mouse.row.min(geometry.screen.bottom().saturating_sub(1)),
            mouse.column.min(geometry.screen.right().saturating_sub(1)),
        );
        if let Some(pending) = state.pending_screen_click.as_mut()
            && point != pending.anchor
        {
            pending.dragged = true;
        }
        update::drag_screen_selection(state, &geometry.screen_text, point);
        *dirty = true;
        return;
    }
    // 没有本地选区时，鼠标上报模式的应用自己处理拖拽（对齐 herdr）。
    if state.terminal_selection.is_none()
        && state.selection.is_none()
        && state.lx_selection.is_none()
        && state.lx_input_drag.is_none()
        && let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row)
        && forward_pane_mouse_event(state, sessions, pane, inner, mouse)
    {
        *dirty = true;
        return;
    }
    // lx 输入框文本拖拽：焦点跟到鼠标所在单元格（超出则钳到输入内容区）。
    if let Some(pane) = state.lx_input_drag {
        let Some((_, rect)) = rects.iter().find(|(id, _)| *id == pane) else {
            return;
        };
        let inner = layout::pane_inner_rect(*rect);
        if let Some(geom) = state
            .pane_anywhere(pane)
            .and_then(|target| ui::main_content::lx::input_geom(inner, &target.lx))
        {
            let (visual_row, col) = geom.cell(mouse.column, mouse.row);
            update::drag_lx_input_selection(state, pane, visual_row, col);
            *dirty = true;
        }
        return;
    }
    // lx 页选区拖拽：终点更新到鼠标位置的内容坐标。
    if let Some(selection) = state.lx_selection {
        let pane = selection.pane();
        let Some((_, rect)) = rects.iter().find(|(id, _)| *id == pane) else {
            return;
        };
        let inner = pane_content_inner(state, view, pane, layout::pane_inner_rect(*rect));
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let row = mouse.row.clamp(inner.y, inner.bottom() - 1) - inner.y;
        let col = mouse.column.clamp(inner.x, inner.right() - 1) - inner.x;
        update::drag_lx_selection(state, pane, row, col);
        *dirty = true;
        return;
    }
    // 终端拖拽选区：把终点更新到鼠标位置的内容；prompt 选区同时移动编辑器光标。
    let pane = if let Some(pane) = state.terminal_selection {
        pane
    } else if let Some(selection) = state.selection {
        selection.pane()
    } else {
        return;
    };
    let Some((_, rect)) = rects.iter().find(|(id, _)| *id == pane) else {
        return;
    };
    let inner = pane_content_inner(state, view, pane, layout::pane_inner_rect(*rect));
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let row = mouse.row.clamp(inner.y, inner.bottom() - 1) - inner.y;
    let col = mouse.column.clamp(inner.x, inner.right() - 1) - inner.x;
    if state.terminal_selection == Some(pane) {
        update::drag_terminal_selection(state, pane, row, col);
    } else {
        update::drag_selection(state, pane, row, col);
    }
    if pane == state.prompt.id()
        && update::prompt_panel_active(state)
        && !state.prompt.row_in_padding(row)
    {
        update::place_prompt_cursor(state, row, col);
    }
    update::arm_selection_autoscroll(
        state,
        pane,
        inner,
        (mouse.column, mouse.row),
        Instant::now(),
    );
    *dirty = true;
}

/// 鼠标左键释放事件处理。
pub(crate) fn handle_mouse_up_left(
    mouse: &MouseEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    _cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    let Geometry { rects, view, .. } = geometry;
    // 卡片正文的「按下可能是一次点击」标记只服务本次释放：任何提前返回的路径
    // （滚动条拖拽、浮层、其它窗格）都必须丢弃它，绝不能让它在下一次松开时误折叠。
    let card_press = state.lx_card_press.take();
    if state.referenced_search_scroll_drag.take().is_some() {
        *dirty = true;
        return;
    }
    if state.referenced_menu_scroll_drag.take().is_some() {
        *dirty = true;
        return;
    }
    // 延迟的文字行点击：没有位移才补执行原动作；拖动过则只收尾选区。
    if let Some(pending) = state.pending_screen_click.take() {
        let point = crate::app::selection::ScreenPoint::new(
            mouse.row.min(geometry.screen.bottom().saturating_sub(1)),
            mouse.column.min(geometry.screen.right().saturating_sub(1)),
        );
        let dragged = pending.dragged || point != pending.anchor;
        if !dragged {
            update::clear_selection(state);
            *dirty = true;
            super::press::apply_pending_screen_click(
                state,
                pending.action,
                sessions,
                geometry,
                dirty,
            );
            return;
        }
        if state.screen_selection.is_some() {
            update::drag_screen_selection(state, &geometry.screen_text, point);
            update::finish_screen_selection(state);
            *dirty = true;
        }
        return;
    }
    // 屏幕选区：松开只结束拖动，不写剪贴板、不清高亮。
    if state
        .screen_selection
        .as_ref()
        .is_some_and(|selection| selection.is_dragging())
    {
        update::finish_screen_selection(state);
        *dirty = true;
        return;
    }
    // prompt 列表拖拽：必须在任何更早的早退之前收尾，保证一次 Down 必被一次 Up 清理；
    // 未跨行时这里走点击/双击语义（折叠与打开编辑器都发生在松开）。
    if state.prompt_drag.is_some() {
        let hit = ui::prompt::tree::drop_hit_at(
            layout::prompt_content_rect(view.prompt),
            &state.prompt_owner().prompt_panel,
            mouse.column,
            mouse.row,
        );
        update::finish_prompt_drag(state, hit);
        *dirty = true;
        return;
    }
    if state.overlay.is_some() {
        return;
    }
    if state.workspace_scroll_drag.take().is_some() {
        return;
    }
    if state.prompt_scroll_drag.take().is_some() {
        return;
    }
    // 漏掉这一条会让 drag 分支永久停在 Todo 滚动条上：之后拖分隔线、拖选区、
    // 拖 lx 卡片全部被静默吞掉（只有再点一次 Todo 滚动条才能解除）。
    if state.todo_scroll_drag.take().is_some() {
        return;
    }
    if state.task_scroll_drag.take().is_some() {
        return;
    }
    if state.task_agents_scroll_drag.take().is_some() {
        return;
    }
    if state.agent_scroll_drag.take().is_some() {
        return;
    }
    if state.terminal_scroll_drag.take().is_some() {
        return;
    }
    if state.lx_scroll_drag.take().is_some() {
        return;
    }
    if state.lx_history_scroll_drag.take().is_some() {
        return;
    }
    if state.resizing_pane.is_some() {
        update::end_pane_resize(state);
        *dirty = true;
        return;
    }
    if state.workspace_drag.is_some() {
        update::end_workspace_drag(state);
        *dirty = true;
        return;
    }
    if state.resizing_sidebar {
        update::end_sidebar_resize(state);
        *dirty = true;
        return;
    }
    if state.resizing_prompt {
        update::end_prompt_resize(state);
        *dirty = true;
        return;
    }
    // 没有本地选区时，鼠标上报模式的应用自己处理释放（对齐 herdr）。
    if state.terminal_selection.is_none()
        && state.selection.is_none()
        && let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row)
        && forward_pane_mouse_event(state, sessions, pane, inner, mouse)
    {
        *dirty = true;
        return;
    }
    if state
        .selection
        .is_some_and(|selection| selection.pane() == state.prompt.id())
    {
        // 松开只结束拖动；高亮保留，复制由 Ctrl/Cmd+C 触发。
        update::end_selection_drag(state);
        *dirty = true;
        return;
    }
    if let Some(pane) = state.terminal_selection {
        // 松开只结束拖动；终端仿真器选区继续保留供复制。
        update::finish_terminal_selection(state, pane);
        *dirty = true;
        return;
    }
    // lx 输入框：松开保留选区，空选区清除锚点。
    if update::finish_lx_input_selection(state).is_some() {
        *dirty = true;
        return;
    }
    // lx 页：有选区时只结束拖动；没有拖出范围才是点击（折叠卡片）。
    // 拖过哪怕一列就是选区，绝不顺手收起卡片。
    let selection = update::finish_lx_selection(state);
    let mut clicked_card = None;
    if selection.and_then(|selection| selection.range()).is_none()
        && let Some(pressed) = card_press
    {
        clicked_card = Some(pressed);
        update::clear_selection(state);
    }
    if let Some((pane, card)) = clicked_card {
        update::toggle_lx_card(state, pane, card);
    }
    *dirty = true;
}

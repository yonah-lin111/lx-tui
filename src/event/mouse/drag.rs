//! 鼠标拖拽与左键释放处理：滚动条/分割线/排序拖拽、选区延伸与选区复制。

use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::MouseEvent;
use ratatui::layout::Direction;

use crate::app::state::AppState;
use crate::app::toast::{Toast, ToastKind};
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
    dirty: &mut bool,
) {
    let Geometry { rects, view, .. } = geometry;
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
    // 没有本地选区时，鼠标上报模式的应用自己处理拖拽（对齐 herdr）。
    if state.terminal_selection.is_none()
        && state.selection.is_none()
        && state.lx_selection.is_none()
        && let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row)
        && forward_pane_mouse_event(state, sessions, pane, inner, mouse)
    {
        *dirty = true;
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
    if pane == state.prompt.id() && !state.prompt.row_in_padding(row) {
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
    dirty: &mut bool,
) {
    let Geometry { rects, .. } = geometry;
    if state.overlay.is_some() {
        return;
    }
    if state.workspace_scroll_drag.take().is_some() {
        return;
    }
    if state.prompt_scroll_drag.take().is_some() {
        return;
    }
    if state.agent_scroll_drag.take().is_some() {
        return;
    }
    if state.terminal_scroll_drag.take().is_some() {
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
        update::end_selection_drag(state);
        *dirty = true;
        return;
    }
    if let Some(pane) = state.terminal_selection
        && let Some(text) = update::finish_terminal_selection(state, pane)
    {
        let now = Instant::now();
        if crate::platform::write_clipboard(&text) {
            update::show_toast(
                state,
                Toast::new(ToastKind::Info, ui::text::TOAST_COPIED, Some(pane), now)
                    .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
            );
        } else {
            tracing::warn!("clipboard write failed");
            update::show_toast(
                state,
                Toast::new(
                    ToastKind::Error,
                    ui::text::TOAST_COPY_FAILED,
                    Some(pane),
                    now,
                )
                .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
            );
        }
    }
    // lx 页选区松开：复制选中文本（空选区不复制）。
    if let Some(selection) = update::take_lx_selection(state)
        && let Some(range) = selection.range()
    {
        let pane = selection.pane();
        let area = rects
            .iter()
            .find(|(id, _)| *id == pane)
            .map(|(_, rect)| layout::pane_inner_rect(*rect));
        if let Some((area, target)) = area.zip(state.pane_anywhere(pane)) {
            let text = ui::main_content::lx::copy_text(area, &target.lx, range);
            if !text.trim().is_empty() {
                let now = Instant::now();
                if crate::platform::write_clipboard(&text) {
                    update::show_toast(
                        state,
                        Toast::new(ToastKind::Info, ui::text::TOAST_COPIED, Some(pane), now)
                            .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
                    );
                } else {
                    tracing::warn!("clipboard write failed");
                    update::show_toast(
                        state,
                        Toast::new(
                            ToastKind::Error,
                            ui::text::TOAST_COPY_FAILED,
                            Some(pane),
                            now,
                        )
                        .with_title(ui::text::TOAST_CLIPBOARD_TITLE),
                    );
                }
            }
        }
    }
    update::clear_selection(state);
    *dirty = true;
}

//! 鼠标滚轮分派：二级面板、选区滚动、窗格与侧栏滚动。

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::state::{AppState, PaneKind, PaneView};
use crate::app::update;
use crate::input;
use crate::layout::{self, PaneId};
use crate::pty::PtySession;
use crate::terminal::WheelRouting;
use crate::ui;
use alacritty_terminal::term::TermMode;

use super::super::session::{Geometry, write_to_pane};
use super::hit::{
    block_panel_layout, mention_panel_layout, pane_at, pane_content_inner, slash_panel_layout,
};

/// 每次滚轮滚动的视觉行数（与 prompt 视口一致）。
const WHEEL_LINES: isize = 3;

/// 纵向滚轮方向（负数向上）；横向滚轮返回 None。
pub(crate) fn vertical_wheel_direction(kind: MouseEventKind) -> Option<isize> {
    match kind {
        MouseEventKind::ScrollUp => Some(-1),
        MouseEventKind::ScrollDown => Some(1),
        _ => None,
    }
}

/// 拖拽选择中滚轮：滚动选区所在窗格，并把选区光标延伸到鼠标位置（钳在窗格内）。
///
/// 纵向滚轮消费事件（对齐 herdr：选区进行中优先于鼠标上报）；横向返回 false 交回常规路径。
pub(crate) fn scroll_selection_wheel(
    state: &mut AppState,
    pane: PaneId,
    rects: &[(PaneId, Rect)],
    mouse: &MouseEvent,
) -> bool {
    let Some(direction) = vertical_wheel_direction(mouse.kind) else {
        return false;
    };
    let Some((_, rect)) = rects.iter().find(|(id, _)| *id == pane) else {
        return true;
    };
    let inner = layout::pane_inner_rect(*rect);
    if inner.width == 0 || inner.height == 0 {
        return true;
    }
    update::scroll_pane(state, pane, direction);
    let row = mouse.row.clamp(inner.y, inner.bottom() - 1) - inner.y;
    let col = mouse.column.clamp(inner.x, inner.right() - 1) - inner.x;
    update::drag_terminal_selection(state, pane, row, col);
    true
}

/// 按终端模式把滚轮交给窗格：转发鼠标上报、备用屏方向键或本地回滚。
///
/// 前两种由应用接管滚动，本地视口吸回底部；返回事件是否被消费（需要重绘）。
pub(crate) fn handle_pane_wheel(
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    pane: PaneId,
    inner: Rect,
    mouse: &MouseEvent,
) -> bool {
    let Some((routing, mode)) = state
        .pane_anywhere(pane)
        .map(|target| (target.terminal.wheel_routing(), target.terminal.mode()))
    else {
        return false;
    };
    match routing {
        WheelRouting::HostScroll => {
            let Some(direction) = vertical_wheel_direction(mouse.kind) else {
                return false;
            };
            update::scroll_pane(state, pane, direction)
        }
        WheelRouting::MouseReport => {
            let column = mouse.column.saturating_sub(inner.x);
            let row = mouse.row.saturating_sub(inner.y);
            let Some(bytes) =
                input::encode::encode_mouse_wheel(mouse.kind, column, row, mouse.modifiers, mode)
            else {
                return false;
            };
            update::reset_pane_scroll(state, pane);
            write_to_pane(sessions, pane, &bytes);
            true
        }
        WheelRouting::AlternateScroll => {
            let code = match mouse.kind {
                MouseEventKind::ScrollUp => KeyCode::Up,
                MouseEventKind::ScrollDown => KeyCode::Down,
                _ => return false,
            };
            let Some(bytes) =
                input::encode::encode_key(KeyEvent::new(code, KeyModifiers::NONE), mode)
            else {
                return false;
            };
            update::reset_pane_scroll(state, pane);
            write_to_pane(sessions, pane, &bytes);
            true
        }
    }
}

/// 把鼠标按键、拖拽或移动按应用声明的协议转发给窗格；非鼠标上报模式返回 false 交回本地选区。
///
/// 对齐 herdr：应用接管鼠标时 lx-tui 不做本地选区，滚动与选择都由应用自己实现；
/// 拖拽与无按键移动还需应用分别开启 1002/1003 才上报，避免多发未订阅的事件。
pub(crate) fn forward_pane_mouse_event(
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    pane: PaneId,
    inner: Rect,
    mouse: &MouseEvent,
) -> bool {
    // lx 视图不向隐藏终端转发任何鼠标事件。
    if state
        .pane_anywhere(pane)
        .is_some_and(|target| target.view == PaneView::Lx)
    {
        return false;
    }
    let Some((routing, mode)) = state
        .pane_anywhere(pane)
        .map(|target| (target.terminal.wheel_routing(), target.terminal.mode()))
    else {
        return false;
    };
    let reportable = routing == WheelRouting::MouseReport
        && match mouse.kind {
            MouseEventKind::Drag(_) => {
                mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
            }
            MouseEventKind::Moved => mode.contains(TermMode::MOUSE_MOTION),
            _ => true,
        };
    if !reportable {
        return false;
    }
    let column = mouse.column.saturating_sub(inner.x);
    let row = mouse.row.saturating_sub(inner.y);
    let Some(bytes) =
        input::encode::encode_mouse_button(mouse.kind, column, row, mouse.modifiers, mode)
    else {
        return false;
    };
    if !matches!(mouse.kind, MouseEventKind::Moved) {
        update::reset_pane_scroll(state, pane);
    }
    write_to_pane(sessions, pane, &bytes);
    true
}

/// 滚轮事件统一处理。
pub(crate) fn handle_mouse_wheel(
    mouse: &MouseEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    let Geometry { rects, view, .. } = geometry;
    // 拖动中禁止改视口：落点与阈值都按绝对行号换算，滚动会让指针原地"跨行"
    // （design §1.3 明确不做边缘自动滚动）。
    if state.prompt_drag.is_some() {
        return;
    }
    if let Some(direction) = vertical_wheel_direction(mouse.kind)
        && let Some(crate::app::overlay::Overlay::ReferencedFolderSearch(search)) =
            state.overlay.as_ref()
        && let Some(layout) = ui::overlay::referenced_search_layout(geometry.screen, search)
        && layout.area.contains((mouse.column, mouse.row).into())
    {
        // 弹窗内滚轮滚动列表视口（高亮不动、越界钳制），不穿透到底层。
        if update::scroll_referenced_search(state, direction, layout.start, layout.visible()) {
            *dirty = true;
        }
        return;
    }
    if let Some(direction) = vertical_wheel_direction(mouse.kind)
        && let Some(crate::app::overlay::Overlay::ReferencedFolderMenu(menu)) =
            state.overlay.as_ref()
    {
        // 弹窗内滚轮滚动列表视口（高亮不动、越界钳制），不穿透到底层；与 prompt 的 @ 面板一致。
        let layout = ui::overlay::referenced_menu_layout(geometry.screen, menu);
        if layout.area.contains((mouse.column, mouse.row).into()) {
            if update::scroll_referenced_menu(state, direction, layout.start, layout.visible()) {
                *dirty = true;
            }
            return;
        }
    }
    if state.overlay.is_some() {
        return;
    }
    if state.resizing_prompt || state.resizing_pane.is_some() {
        return;
    }
    // @ 提及面板打开且指针在其上：滚轮滚动面板视口（高亮不动），不穿透到 prompt 与窗格。
    if let Some(direction) = vertical_wheel_direction(mouse.kind)
        && let Some(panel) = mention_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        update::scroll_mention(state, direction, panel.start);
        *dirty = true;
        return;
    }
    // 斜杠命令面板同上：滚轮滚动视口，到底/顶后钳制不循环。
    if let Some(direction) = vertical_wheel_direction(mouse.kind)
        && let Some(panel) = slash_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        update::scroll_slash(state, direction, panel.start);
        *dirty = true;
        return;
    }
    // 块命令面板同上。
    if let Some(direction) = vertical_wheel_direction(mouse.kind)
        && let Some(panel) = block_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        update::scroll_panel(state, direction, panel.start);
        *dirty = true;
        return;
    }
    // prompt 拖拽选区：滚轮滚动视口，锚点钉在文本上，终点跟到鼠标。
    if let Some(selection) = state.selection
        && selection.is_dragging()
    {
        if let Some(direction) = vertical_wheel_direction(mouse.kind)
            && let Some((_, rect)) = rects.iter().find(|(id, _)| *id == selection.pane())
        {
            let inner = pane_content_inner(
                state,
                view,
                selection.pane(),
                layout::pane_inner_rect(*rect),
            );
            if inner.width > 0 && inner.height > 0 {
                let row = mouse.row.clamp(inner.y, inner.bottom() - 1) - inner.y;
                let col = mouse.column.clamp(inner.x, inner.right() - 1) - inner.x;
                if update::wheel_prompt_selection(state, direction, row, col) {
                    *dirty = true;
                }
            }
        }
        return;
    }
    // 终端拖拽选区优先于鼠标上报：滚动选区所在窗格并延伸选区。
    if let Some(pane) = state.terminal_selection
        && scroll_selection_wheel(state, pane, rects, mouse)
    {
        *dirty = true;
        return;
    }
    if let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) {
        if pane == state.prompt.id() {
            // Todo 面板：滚轮只滚列表视口（不动高亮、不聚焦）。
            if state.right_tab == crate::app::state::RightTab::Todo && !state.prompt_collapsed {
                if let Some(direction) = vertical_wheel_direction(mouse.kind)
                    && update::scroll_todo(state, direction)
                {
                    *dirty = true;
                }
                return;
            }
            // 折叠态容器：滚轮不作用于隐藏的编辑器。
            if !update::prompt_panel_active(state) {
                return;
            }
            update::clear_selection(state);
            if let Some(direction) = vertical_wheel_direction(mouse.kind) {
                if update::prompt_tree_mode(state) {
                    // 树视图滚轮按行滚动，光标/选中不动。
                    let visible = update::prompt_tree_visible_rows(state);
                    if update::scroll_prompt_tree(state, direction, visible) {
                        *dirty = true;
                    }
                } else {
                    // 无滚动条时上限为 0，滚轮不产生位移、也不触发重绘。
                    if update::scroll_prompt(state, direction) {
                        *dirty = true;
                    }
                }
            }
        } else if state
            .pane_anywhere(pane)
            .is_some_and(|target| target.view == PaneView::Lx)
        {
            // lx 页：提及面板候选视口优先于输入框滚动；面板内滚轮不穿透到输入框。
            if let Some(direction) = vertical_wheel_direction(mouse.kind)
                && let Some(bar) = state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::mention_layout(inner, &target.lx))
                && bar.rect.contains((mouse.column, mouse.row).into())
            {
                if update::scroll_lx_mention(state, pane, direction, bar.start) {
                    *dirty = true;
                }
                return;
            }
            // 后台任务面板的输出容器：滚轮滚动输出视口（不穿透到下方 item 列表）。
            if let Some(direction) = vertical_wheel_direction(mouse.kind)
                && let Some(rects) = state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::task_rects(inner, &target.lx))
                && rects.output.contains((mouse.column, mouse.row).into())
            {
                let agents_tab = state
                    .pane_anywhere(pane)
                    .and_then(|target| target.lx.task.as_ref())
                    .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
                // Agents 详情按主聊天历史的步长（1 行/格）；进程输出沿用 3 行。
                let changed = if agents_tab {
                    update::scroll_agents_detail(state, pane, direction)
                } else {
                    update::scroll_task_output(state, pane, direction.saturating_mul(WHEEL_LINES))
                };
                if changed {
                    *dirty = true;
                }
                return;
            }
            // 后台任务面板的 **item 列表**：进程超过一屏时滚动显式视口。
            //
            // 之前这里没有分支，滚轮直接漏到下方 `scroll_lx_input`——滚动一个看不见的
            // 输入缓冲并置脏，而列表实际滚不动（`window.viewport` 永远没人写）。
            if let Some(direction) = vertical_wheel_direction(mouse.kind)
                && let Some(rects) = state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::task_rects(inner, &target.lx))
                && rects.list.contains((mouse.column, mouse.row).into())
            {
                if update::scroll_task_list(state, pane, direction.saturating_mul(WHEEL_LINES)) {
                    *dirty = true;
                }
                return;
            }
            // `ask` 面板：预览区只滚预览、选项区只滚列表视口（**不动高亮**）。
            // 后台任务页可见时面板不可见，滚轮必须留给任务列表。
            if let Some(direction) = vertical_wheel_direction(mouse.kind)
                && let Some((zone, window)) = state
                    .pane_anywhere(pane)
                    .filter(|target| target.lx.task.is_none())
                    .and_then(|target| {
                        let zone = ui::main_content::lx::ask_wheel_zone(
                            inner,
                            &target.lx,
                            mouse.column,
                            mouse.row,
                        )?;
                        Some((
                            zone,
                            ui::main_content::lx::ask_list_window(inner, &target.lx),
                        ))
                    })
            {
                if update::apply_ask_wheel(state, pane, zone, direction, window) {
                    *dirty = true;
                }
                return;
            }
            // 聊天历史区滚轮：滚动历史视口（保留输入框滚轮给输入缓冲区）。
            if let Some(direction) = vertical_wheel_direction(mouse.kind)
                && let Some(history_rect) = state.pane_anywhere(pane).and_then(|target| {
                    ui::main_content::lx::history_wheel_region(inner, &target.lx)
                })
                && history_rect.contains((mouse.column, mouse.row).into())
                && let Some((total, visible)) = state.pane_anywhere(pane).and_then(|target| {
                    ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane))
                })
                && update::scroll_lx_history(state, pane, direction, total, visible)
            {
                *dirty = true;
                return;
            }
            // `/` 命令面板滚轮：滚动候选窗口（不穿透到输入框）。
            if let Some(direction) = vertical_wheel_direction(mouse.kind)
                && let Some((pane, inner)) =
                    super::hit::lx_pane_at(state, rects, mouse.column, mouse.row)
                && let Some(bar) = super::hit::lx_slash_panel_layout(state, pane, inner)
                && bar.rect.contains((mouse.column, mouse.row).into())
            {
                if update::scroll_lx_slash(state, pane, direction, bar.start) {
                    *dirty = true;
                }
                return;
            }
            // 提及面板外：滚轮滚动输入缓冲区视口，不穿透到隐藏终端。
            if let Some(direction) = vertical_wheel_direction(mouse.kind)
                && update::scroll_lx_input(state, pane, direction)
            {
                *dirty = true;
            }
        } else if state.pane_anywhere(pane).is_some_and(|target| {
            target.kind == PaneKind::Terminal && target.view == PaneView::Terminal
        }) && handle_pane_wheel(state, sessions, pane, inner, mouse)
        {
            *dirty = true;
        }
    } else if ui::workspace_section_at(view, state, mouse.column, mouse.row)
        && let Some(rows) = ui::workspace_list_rows(view, state)
        && let Some(direction) = vertical_wheel_direction(mouse.kind)
        && update::scroll_workspace_list(state, direction, rows)
    {
        // 列表滚动后指针下的工作区可能已换行：按新偏移重算悬停。
        let hover = ui::workspace_item_at(view, state, mouse.column, mouse.row);
        update::set_workspace_hover(state, hover);
        *dirty = true;
    } else if ui::agents_section_at(view, state, mouse.column, mouse.row)
        && let Some(rows) = ui::agent_list_rows(view, state)
        && let Some(direction) = vertical_wheel_direction(mouse.kind)
        && update::scroll_agent_list(state, direction, rows)
    {
        // 列表滚动后指针下的条目可能已换行：按新偏移重算悬停。
        let hover = ui::agent_item_index_at(view, state, mouse.column, mouse.row);
        update::set_agent_hover(state, hover);
        *dirty = true;
    }
}

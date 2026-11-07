//! 鼠标滚轮分派：二级面板、选区滚动、窗格与侧栏滚动。

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::state::{AppState, PaneKind, PaneView};
use crate::app::update::{self, WHEEL_LINES};
use crate::input;
use crate::layout::{self, PaneId};
use crate::pty::PtySession;
use crate::terminal::WheelRouting;
use crate::ui;
use alacritty_terminal::term::TermMode;

use super::super::session::{Geometry, write_to_pane};
use super::hit::{
    block_panel_layout, mention_panel_layout, pane_at, pane_content_inner, paste_panel_rect,
    refer_panel_layout, slash_panel_layout,
};

/// 纵向滚轮方向（负数向上）；横向滚轮返回 None。
pub(crate) fn vertical_wheel_direction(kind: MouseEventKind) -> Option<isize> {
    match kind {
        MouseEventKind::ScrollUp => Some(-1),
        MouseEventKind::ScrollDown => Some(1),
        _ => None,
    }
}

/// 纵向滚轮单格步长（负数向上，每格 `WHEEL_LINES` 行/项）；横向滚轮返回 None。
pub(crate) fn vertical_wheel_delta(kind: MouseEventKind) -> Option<isize> {
    vertical_wheel_direction(kind).map(|direction| direction.saturating_mul(WHEEL_LINES))
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

/// lx 主内容区拖拽选区中滚轮：滚动活动容器视口并把选区终点延伸到鼠标所在的新内容行。
fn wheel_lx_selection(
    state: &mut AppState,
    pane: PaneId,
    rects: &[(PaneId, Rect)],
    direction: isize,
    mouse: &MouseEvent,
    cache: &mut ui::RenderCache,
) -> bool {
    let Some((_, rect)) = rects.iter().find(|(id, _)| *id == pane) else {
        return false;
    };
    let inner = layout::pane_inner_rect(*rect);
    if inner.width == 0 || inner.height == 0 {
        return false;
    }
    let Some(has_task) = state
        .pane_anywhere(pane)
        .filter(|target| target.view == PaneView::Lx)
        .map(|target| target.lx.task.is_some())
    else {
        return false;
    };
    let step = direction.saturating_mul(WHEEL_LINES);
    let moved = if has_task {
        update::resolve_task_output_visible(state, rects);
        update::resolve_task_agents_visible(state, rects);
        let agents_tab = state
            .pane_anywhere(pane)
            .and_then(|target| target.lx.task.as_ref())
            .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
        if agents_tab {
            update::scroll_agents_detail(state, pane, step)
        } else {
            update::scroll_task_output(state, pane, step)
        }
    } else if let Some((total, visible)) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane)))
    {
        update::scroll_lx_history(state, pane, step, total, visible)
    } else {
        false
    };
    if !moved {
        return false;
    }
    let unit = state
        .lx_selection
        .map_or(crate::app::selection::SelectionUnit::Character, |sel| {
            sel.unit()
        });
    if let Some((start, end)) = state.pane_anywhere(pane).and_then(|target| {
        let geom = ui::main_content::lx::content_geom(inner, &target.lx, cache.pane(pane))?;
        let cell = geom.cell(mouse.column, mouse.row);
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
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(crate::app::overlay::Overlay::ReferencedFolderSearch(search)) =
            state.overlay.as_ref()
        && let Some(layout) = ui::overlay::referenced_search_layout(geometry.screen, search)
        && layout.area.contains((mouse.column, mouse.row).into())
    {
        // 弹窗内滚轮滚动列表视口（高亮不动、越界钳制），不穿透到底层。
        if update::scroll_referenced_search(state, delta, layout.start, layout.visible()) {
            *dirty = true;
        }
        return;
    }
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(crate::app::overlay::Overlay::Connect(panel)) = state.overlay.as_ref()
        && let Some(layout) = ui::overlay::connect_layout(geometry.screen, panel)
        && layout.area.contains((mouse.column, mouse.row).into())
    {
        // 连接模态内滚轮滚动列表视口（高亮不动、越界钳制），不穿透到底层。
        if update::scroll_connect(state, delta, layout.start, layout.visible) {
            *dirty = true;
        }
        return;
    }
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(crate::app::overlay::Overlay::ReferencedFolderMenu(menu)) =
            state.overlay.as_ref()
    {
        // 弹窗内滚轮滚动列表视口（高亮不动、越界钳制），不穿透到底层；与 prompt 的 @ 面板一致。
        let layout = ui::overlay::referenced_menu_layout(geometry.screen, menu);
        if layout.area.contains((mouse.column, mouse.row).into()) {
            if update::scroll_referenced_menu(state, delta, layout.start, layout.visible()) {
                *dirty = true;
            }
            return;
        }
    }
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(crate::app::overlay::Overlay::ConfirmClose(confirm)) = state.overlay.as_ref()
        && let Some(layout) = ui::overlay::confirm_close_layout(state, geometry.screen, confirm)
        && layout.shell.area.contains((mouse.column, mouse.row).into())
    {
        // 失效工作区列表在模态内滚动（列表高度受限），不穿透到底层。
        if update::scroll_confirm_close_with_rows(state, delta, layout.visible_rows) {
            *dirty = true;
        }
        return;
    }
    if state.overlay.is_some() {
        return;
    }
    if state.resizing_prompt || state.resizing_pane.is_some() {
        return;
    }
    // 粘贴面板打开且指针在其上：吞掉滚轮，不穿透到 prompt 与窗格。
    if vertical_wheel_direction(mouse.kind).is_some()
        && let Some(rect) = paste_panel_rect(state, view)
        && rect.contains((mouse.column, mouse.row).into())
    {
        return;
    }
    // @ 提及面板打开且指针在其上：滚轮滚动面板视口（高亮不动），不穿透到 prompt 与窗格。
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(panel) = mention_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_mention(state, delta, panel.start) {
            *dirty = true;
        }
        return;
    }
    // 字母快捷面板打开且指针在其上：滚轮滚动面板视口（高亮不动），不穿透到 prompt 与窗格。
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(panel) = refer_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_refer(state, delta, panel.start) {
            *dirty = true;
        }
        return;
    }
    // 斜杠命令面板同上：滚轮滚动视口，到底/顶后钳制不循环。
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(panel) = slash_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_slash(state, delta, panel.start) {
            *dirty = true;
        }
        return;
    }
    // 块命令面板同上。
    if let Some(delta) = vertical_wheel_delta(mouse.kind)
        && let Some(panel) = block_panel_layout(state, view)
        && panel.rect.contains((mouse.column, mouse.row).into())
    {
        if update::scroll_panel(state, delta, panel.start) {
            *dirty = true;
        }
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
    // lx 主内容区拖拽选区：滚轮滚动对应容器视口，锚点钉在原内容行上，终点延伸到鼠标所在内容行。
    if let Some(selection) = state.lx_selection
        && selection.is_dragging()
    {
        if let Some(direction) = vertical_wheel_direction(mouse.kind) {
            let pane = selection.pane();
            if wheel_lx_selection(state, pane, rects, direction, mouse, cache) {
                *dirty = true;
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
            // 最近页面抽屉：指针在抽屉区域时滚轮只滚抽屉，不穿透到上方视图。
            if let Some(direction) = vertical_wheel_direction(mouse.kind) {
                let recent_rows = ui::prompt_frame::recent_rows(state);
                let split = crate::layout::prompt_body_split(
                    view.prompt,
                    state.prompt_owner().prompt_panel.recent_open,
                    recent_rows.len(),
                );
                if let Some(divider) = split.recent_divider
                    && mouse.row >= divider.y
                {
                    if update::scroll_prompt_recent(
                        state,
                        direction,
                        split.visible_entries,
                        recent_rows.len(),
                    ) {
                        *dirty = true;
                    }
                    return;
                }
            }
            update::clear_selection(state);
            if let Some(direction) = vertical_wheel_direction(mouse.kind) {
                if update::prompt_tree_mode(state) {
                    // 树视图滚轮按全局步长滚动，光标/选中不动。
                    let visible = update::prompt_tree_visible_rows(state);
                    if update::scroll_prompt_tree(
                        state,
                        direction.saturating_mul(WHEEL_LINES),
                        visible,
                    ) {
                        *dirty = true;
                    }
                } else if update::prompt_pages_mode(state) {
                    let visible = update::prompt_tree_visible_rows(state);
                    if update::scroll_page_list(
                        state,
                        direction.saturating_mul(WHEEL_LINES),
                        visible,
                    ) {
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
            // lx 页：粘贴选择面板打开且指针在其上时吞掉滚轮，不穿透到历史区与输入框。
            if vertical_wheel_direction(mouse.kind).is_some()
                && let Some(bar) = state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::paste_layout(inner, &target.lx))
                && bar.rect.contains((mouse.column, mouse.row).into())
            {
                return;
            }
            // lx 页：提及面板候选视口优先于历史区与输入框滚动；面板内滚轮不穿透。
            if let Some(delta) = vertical_wheel_delta(mouse.kind)
                && let Some(bar) = state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::mention_layout(inner, &target.lx))
                && bar.rect.contains((mouse.column, mouse.row).into())
            {
                if update::scroll_lx_mention(state, pane, delta, bar.start) {
                    *dirty = true;
                }
                return;
            }
            // lx 页：快捷引用面板候选视口优先于历史区与输入框滚动。
            if let Some(delta) = vertical_wheel_delta(mouse.kind)
                && let Some(bar) = state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::refer_layout(inner, &target.lx))
                && bar.rect.contains((mouse.column, mouse.row).into())
            {
                if update::scroll_lx_refer(state, pane, delta, bar.start) {
                    *dirty = true;
                }
                return;
            }
            // lx 页：`/` 命令面板（含 `/model`、`/resume`、`/rewind`）优先于历史区与输入框滚动。
            if let Some(delta) = vertical_wheel_delta(mouse.kind)
                && let Some(bar) = super::hit::lx_slash_panel_layout(state, pane, inner)
                && bar.rect.contains((mouse.column, mouse.row).into())
            {
                if update::scroll_lx_slash(state, pane, delta, bar.start) {
                    *dirty = true;
                }
                return;
            }
            // 后台任务面板的输出容器（含下方一行的聊天栏「回到底部」）：
            // 滚轮滚动输出视口（不穿透到下方 item 列表或隐藏输入框）。
            if let Some(delta) = vertical_wheel_delta(mouse.kind)
                && let Some(task_rects) = state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::task_rects(inner, &target.lx))
                && (task_rects.output.y..task_rects.list.y).contains(&mouse.row)
            {
                update::resolve_task_output_visible(state, rects);
                update::resolve_task_agents_visible(state, rects);
                let agents_tab = state
                    .pane_anywhere(pane)
                    .and_then(|target| target.lx.task.as_ref())
                    .is_some_and(|page| page.tab == crate::app::state::TaskTab::Agents);
                // Agents 详情与进程输出统一按 WHEEL_LINES（3 行/格）滚动。
                let changed = if agents_tab {
                    update::scroll_agents_detail(state, pane, delta)
                } else {
                    update::scroll_task_output(state, pane, delta)
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
            if let Some(delta) = vertical_wheel_delta(mouse.kind)
                && let Some(rects) = state
                    .pane_anywhere(pane)
                    .and_then(|target| ui::main_content::lx::task_rects(inner, &target.lx))
                && rects.list.contains((mouse.column, mouse.row).into())
            {
                if update::scroll_task_list(state, pane, delta) {
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
            // 聊天历史区滚轮：滚动历史视口；即使触顶/触底未产生位移也必须在此消费并返回，
            // 严禁向下穿透滚动底部输入框。
            if let Some(delta) = vertical_wheel_delta(mouse.kind)
                && let Some(history_rect) = state.pane_anywhere(pane).and_then(|target| {
                    ui::main_content::lx::history_wheel_region(inner, &target.lx)
                })
                && history_rect.contains((mouse.column, mouse.row).into())
            {
                if let Some((total, visible)) = state.pane_anywhere(pane).and_then(|target| {
                    ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane))
                }) && update::scroll_lx_history(state, pane, delta, total, visible)
                {
                    *dirty = true;
                }
                return;
            }
            // 提及面板与历史区外：滚轮滚动输入缓冲区视口，不穿透到隐藏终端。
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
        && let Some(delta) = vertical_wheel_delta(mouse.kind)
    {
        if update::workspace_history_mode(state) {
            if let Some(rows) = ui::workspace_history_visible_rows(view, state)
                && update::scroll_workspace_history(state, delta, rows)
            {
                let hover = ui::workspace_history_row_at(view, state, mouse.column, mouse.row);
                update::set_workspace_history_hover(state, hover);
                *dirty = true;
            }
        } else if let Some(rows) = ui::workspace_list_rows(view, state)
            && update::scroll_workspace_list(state, delta, rows)
        {
            // 列表滚动后指针下的工作区可能已换行：按新偏移重算悬停。
            let hover = ui::workspace_item_at(view, state, mouse.column, mouse.row);
            update::set_workspace_hover(state, hover);
            *dirty = true;
        }
    } else if ui::agents_section_at(view, state, mouse.column, mouse.row)
        && let Some(rows) = ui::agent_list_rows(view, state)
        && let Some(delta) = vertical_wheel_delta(mouse.kind)
        && update::scroll_agent_list(state, delta, rows)
    {
        // 列表滚动后指针下的条目可能已换行：按新偏移重算悬停。
        let hover = ui::agent_item_index_at(view, state, mouse.column, mouse.row);
        update::set_agent_hover(state, hover);
        *dirty = true;
    }
}

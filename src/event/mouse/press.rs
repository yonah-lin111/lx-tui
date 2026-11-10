//! 鼠标左键按下与右键按下分派：浮层/toast 拦截、控件优先级、工作区与窗格焦点切换。

use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::MouseEvent;

use crate::app::actions::Action;
use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneKind, PaneView};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::config::Config;
use crate::layout::{self, PaneId};
use crate::pty::PtySession;
use crate::ui;

use super::super::overlay::{
    confirm_close_scrollbar_press, handle_overlay_click, open_right_click_menu,
};
use super::super::paste;
use super::super::session::{Geometry, ensure_workspace_visible, reveal_active_tab, write_to_pane};
use super::hit::{
    apply_block_button, apply_template_button, block_button_at, dismiss_lx_toast_outside,
    handle_agent_scrollbar_press, handle_prompt_recent_scrollbar_press,
    handle_prompt_scrollbar_press, handle_sidebar_press, handle_terminal_scrollbar_press,
    handle_todo_scrollbar_press, handle_workspace_scrollbar_press, lx_toast_at, mention_item_at,
    mention_tab_at, pane_at, pane_boundary_draggable, pane_content_inner, pane_toggle_button_at,
    panel_item_at, prompt_boundary_hit, prompt_header_at, slash_item_at, template_button_at,
};
use super::hover::reset_pointer_shape;
use super::wheel::forward_pane_mouse_event;

mod lx;

/// 鼠标左键按下事件处理。
pub(crate) fn handle_mouse_down_left(
    mouse: &MouseEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    config: &Config,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    let (screen, rects, view) = (&geometry.screen, &geometry.rects, &geometry.view);
    if update::clear_tooltip(state) {
        *dirty = true;
    }
    // 新的一次按下意味着上一轮 Down→Up 已结束：拖出窗口丢失 Up 后残留的手势与滚动条状态在此清空。
    state.prompt_drag = None;
    state.connect_scroll_drag = None;
    state.referenced_search_scroll_drag = None;
    state.referenced_menu_scroll_drag = None;
    state.task_scroll_drag = None;
    state.task_list_scroll_drag = None;
    state.task_agents_scroll_drag = None;
    state.overlay_input_drag = false;
    if let Some(Overlay::ConfirmClose(confirm)) = state.overlay.as_mut() {
        confirm.scroll_drag = None;
    }
    state.workspace_history.search_drag = false;
    state.workspace_history.scroll_drag = None;
    state.workspace_history.drag = None;
    state.workspace_history.dragging = false;
    state.workspace_history.drag_moved = false;
    // 全局 toast 绘制在模态浮层之上：盒内点击关闭并消费，盒外点击先关闭再继续处理。
    if let Some(toast) = ui::toast::rect(state, view, rects, *screen, config) {
        update::dismiss_toast(state);
        *dirty = true;
        if toast.contains((mouse.column, mouse.row).into()) {
            return;
        }
    }
    if state.overlay.is_some() {
        // `/connect` 模态的滚动条独立于模态点击：命中轨道/thumb 时滚动视口后再走浮层点击。
        if let Some(Overlay::Connect(panel)) = state.overlay.as_ref()
            && let Some(layout) = ui::overlay::connect_layout(*screen, panel)
            && let Some(bar) = layout.scrollbar
            && bar.track.contains((mouse.column, mouse.row).into())
        {
            match ui::widgets::scrollbar::thumb_grab_offset(&bar, mouse.row) {
                Some(grab) => state.connect_scroll_drag = Some(grab),
                None => {
                    let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, mouse.row);
                    if update::set_connect_viewport(state, offset, layout.visible) {
                        *dirty = true;
                    }
                }
            }
            return;
        }
        // 引用文件夹搜索弹窗的滚动条独立于模态点击：命中轨道/thumb 时滚动视口后再走浮层点击。
        if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref()
            && let Some(layout) = ui::overlay::referenced_search_layout(*screen, search)
            && let Some(bar) = layout.scrollbar
            && bar.track.contains((mouse.column, mouse.row).into())
        {
            match ui::widgets::scrollbar::thumb_grab_offset(&bar, mouse.row) {
                Some(grab) => state.referenced_search_scroll_drag = Some(grab),
                None => {
                    let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, mouse.row);
                    if update::set_referenced_search_viewport(state, offset, layout.visible()) {
                        *dirty = true;
                    }
                }
            }
            return;
        }
        // 引用文件夹目录弹窗滚动条同上：命中轨道/thumb 时滚动视口后再走浮层点击。
        if let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() {
            let layout = ui::overlay::referenced_menu_layout(*screen, menu);
            if let Some(bar) = layout.scrollbar
                && bar.track.contains((mouse.column, mouse.row).into())
            {
                match ui::widgets::scrollbar::thumb_grab_offset(&bar, mouse.row) {
                    Some(grab) => state.referenced_menu_scroll_drag = Some(grab),
                    None => {
                        let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, mouse.row);
                        if update::set_referenced_menu_viewport(state, offset, layout.visible()) {
                            *dirty = true;
                        }
                    }
                }
                return;
            }
        }
        // 失效工作区确认弹窗的滚动条同上：命中轨道/thumb 时滚动列表视口后再走浮层点击。
        if confirm_close_scrollbar_press(state, *screen, mouse.column, mouse.row) {
            *dirty = true;
            return;
        }
        let was_menu = matches!(state.overlay, Some(Overlay::Menu(_)));
        let menu_item_clicked = if let Some(Overlay::Menu(menu)) = state.overlay.as_ref() {
            let layout = ui::overlay::menu_layout(state, *screen, menu);
            ui::widgets::menu::item_at(&layout, mouse.column, mouse.row).is_some()
        } else {
            false
        };
        let was_worktree = matches!(state.overlay, Some(Overlay::WorktreeOpen(_)));
        let was_new_ws = matches!(
            state.overlay,
            Some(Overlay::NewWorkspace(_) | Overlay::TrustWorkspace(_))
        );
        if let Some((pane, bytes)) = handle_overlay_click(state, *screen, mouse.column, mouse.row) {
            write_to_pane(sessions, pane, &bytes);
            update::reset_pane_scroll(state, pane);
        }
        let menu_stays_open = matches!(state.overlay, Some(Overlay::Menu(_)));
        if (menu_item_clicked && !menu_stays_open) || was_worktree || was_new_ws {
            ensure_workspace_visible(state, view);
            reveal_active_tab(state, view);
        } else if !was_menu {
            reveal_active_tab(state, view);
        }
        *dirty = true;
        return;
    }
    // lx 页自己的 toast 盒：点击盒内关闭并消费点击，点击盒外关闭后继续处理本次点击。
    if let Some((pane, _toast)) = lx_toast_at(rects, state, mouse.column, mouse.row) {
        update::dismiss_lx_toast(state, pane);
        *dirty = true;
        return;
    }
    if dismiss_lx_toast_outside(rects, state, mouse.column, mouse.row) {
        *dirty = true;
    }
    if ui::exit_button_at(view, mouse.column, mouse.row) {
        update::apply(Action::Quit, state);
        *dirty = true;
        return;
    }
    let tab_bar = ui::tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
    if tab_bar
        .scroll_left
        .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
    {
        if update::scroll_tab_bar(state, -1, tab_bar.max_scroll) {
            *dirty = true;
        }
        return;
    }
    if tab_bar
        .scroll_right
        .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
    {
        if update::scroll_tab_bar(state, 1, tab_bar.max_scroll) {
            *dirty = true;
        }
        return;
    }
    if let Some(index) = ui::tab_bar::tab_at(&tab_bar, mouse.column, mouse.row) {
        update::switch_tab(state, index);
        reveal_active_tab(state, view);
        *dirty = true;
        return;
    }
    if tab_bar
        .add
        .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
    {
        update::create_tab(state);
        reveal_active_tab(state, view);
        *dirty = true;
        return;
    }
    if ui::settings_button_at(view, state, mouse.column, mouse.row) {
        if state.settings_available {
            state.settings_open_requested = true;
        } else {
            let now = Instant::now();
            update::show_toast(
                state,
                Toast::new(
                    ToastKind::Danger,
                    ui::text::TOAST_SETTINGS_UNAVAILABLE,
                    None,
                    now,
                ),
            );
        }
        *dirty = true;
        return;
    }
    if ui::prompt_pin_at(view, state, mouse.column, mouse.row) {
        update::toggle_prompt_pin(state);
        *dirty = true;
        return;
    }
    if let Some(target) =
        ui::collapse_button_at(view, state.agents_collapsed, mouse.column, mouse.row)
    {
        let action = match target {
            ui::CollapseTarget::Sidebar => Action::ToggleSidebar,
            ui::CollapseTarget::Prompt => Action::TogglePrompt,
            ui::CollapseTarget::Panes => Action::TogglePanes,
            ui::CollapseTarget::Agents => Action::ToggleAgents,
        };
        update::apply(action, state);
        let mut pointer_reset = update::set_prompt_hover(state, false);
        pointer_reset |= update::set_sidebar_hover(state, false);
        if pointer_reset {
            reset_pointer_shape();
        }
        *dirty = true;
    } else if ui::sidebar_boundary_at(view, mouse.column, mouse.row) {
        update::begin_sidebar_resize(state);
        *dirty = true;
    } else if handle_workspace_scrollbar_press(state, view, mouse.column, mouse.row)
        || handle_prompt_scrollbar_press(state, view, mouse.column, mouse.row)
        || handle_prompt_recent_scrollbar_press(state, view, mouse.column, mouse.row)
        || handle_todo_scrollbar_press(state, view, mouse.column, mouse.row)
        || handle_agent_scrollbar_press(state, view, mouse.column, mouse.row)
    {
        *dirty = true;
    } else if handle_sidebar_press(state, view, mouse.column, mouse.row, dirty) {
    } else if ui::swap_button_at(view, mouse.column, mouse.row) {
        // 交换按钮是控件：命中优先于分割线拖拽与面板点击；
        // 宽度按位置模型重置：右列取侧栏默认宽、左列占其余。
        let available = view.panes.width.saturating_add(view.prompt.width);
        let side = ui::layout::side_width(available, config.min_pane_width);
        update::swap_prompt_side(state, available, side);
        *dirty = true;
    } else if layout::resize_boundary_at(rects, mouse.column, mouse.row)
        .is_some_and(|hit| prompt_boundary_hit(state, hit))
    {
        update::begin_prompt_resize(state);
        *dirty = true;
    } else if !state.prompt_collapsed
        && let Some(tab) = ui::right_panel::tab_at(view.prompt, mouse.column, mouse.row)
    {
        if update::select_right_tab(state, tab) {
            *dirty = true;
        }
    } else if !state.prompt_collapsed
        && ui::right_panel::tab_row_at(view.prompt, mouse.column, mouse.row)
    {
        // 顶边框 tab 之外的空白属于容器控件区：吞掉，不聚焦编辑器。
        // Todo 面板例外：点容器任意位置都应让容器进入选中态（用户口径）。
        if state.right_tab == crate::app::state::RightTab::Todo
            && update::focus_todo_from_click(state)
        {
            *dirty = true;
        }
    } else if state.right_tab == crate::app::state::RightTab::Todo
        && !state.prompt_collapsed
        && view.prompt.contains((mouse.column, mouse.row).into())
    {
        // Todo 面板：先按钮、再行、最后空白吞掉（顺序不可调换，
        // 否则会先被下面的整块吞掉分支吃掉）。
        let mut changed = false;
        if let Some(char_idx) =
            ui::right_panel::todo::edit_input_char_at(view.prompt, state, mouse.column, mouse.row)
        {
            let clicks = crate::app::selection::next_click_count(
                &mut state.selection_click,
                mouse.column,
                mouse.row,
                Instant::now(),
            );
            update::clear_selection(state);
            if let Some(input) = state.todo.edit_input_mut() {
                input.begin_selection_with_clicks(char_idx, clicks);
                state.todo_input_drag = true;
                changed = true;
            }
        } else if let Some(button) =
            ui::right_panel::todo::button_at(view.prompt, mouse.column, mouse.row)
        {
            changed = update::apply_todo_button(state, button);
        } else if let Some(hit) =
            ui::right_panel::todo::week_strip_hit_at(view.prompt, state, mouse.column, mouse.row)
        {
            changed = match hit {
                ui::right_panel::todo::WeekStripHit::PrevWeek => update::shift_todo_week(state, -1),
                ui::right_panel::todo::WeekStripHit::NextWeek => update::shift_todo_week(state, 1),
                ui::right_panel::todo::WeekStripHit::Day(date) => {
                    update::select_todo_date(state, date)
                }
            };
        } else if let Some(hit) =
            ui::right_panel::todo::rollover_hit_at(view.prompt, state, mouse.column, mouse.row)
        {
            changed = match hit {
                ui::right_panel::todo::RolloverHit::MoveToToday => {
                    update::execute_todo_rollover(state)
                }
                ui::right_panel::todo::RolloverHit::Dismiss => update::dismiss_todo_rollover(state),
            };
        } else if let Some(hit) =
            ui::right_panel::todo::chart_switch_hit_at(view.prompt, state, mouse.column, mouse.row)
        {
            changed = match hit {
                ui::right_panel::todo::ChartSwitchHit::Prev => update::prev_todo_chart(state),
                ui::right_panel::todo::ChartSwitchHit::Next => update::next_todo_chart(state),
            };
        } else if let Some(display) =
            ui::right_panel::todo::priority_at(view.prompt, state, mouse.column, mouse.row)
        {
            changed = update::click_todo_priority(state, display);
        } else if let Some(index) =
            ui::right_panel::todo::row_at(view.prompt, state, mouse.column, mouse.row)
        {
            changed = update::click_todo_row(state, index, Instant::now());
        }
        // 容器内其余位置（工具栏空白 / 分割线 / 统计块 / 列表外空白）同样让容器进入
        // 选中态，但不改数据、不打断编辑、不穿透到窗格。焦点高亮只由边框表达，
        // 不再依赖"点进输入框"才算选中。
        changed |= update::focus_todo_from_click(state);
        if changed {
            *dirty = true;
        }
    } else if !update::prompt_panel_active(state)
        && view.prompt.contains((mouse.column, mouse.row).into())
    {
        // 折叠态容器：一律吞掉，不聚焦、不穿透到窗格。
    } else if ui::prompt_frame::prompt_recent_close_at(view.prompt, state, mouse.column, mouse.row)
    {
        // 抽屉右上角关闭按钮：优先于上方编辑器控件，收起抽屉并交还键盘焦点。
        update::close_prompt_recent(state);
        *dirty = true;
    } else if let Some(index) =
        ui::prompt_frame::prompt_recent_row_at(view.prompt, state, mouse.column, mouse.row)
    {
        // 最近页面抽屉条目：单击打开对应条目 + 页，随后收起抽屉。
        update::open_recent_page(state, index);
        *dirty = true;
    } else if ui::prompt_frame::prompt_recent_contains(view.prompt, state, mouse.column, mouse.row)
    {
        // 抽屉分割线/空态/滚动条槽：吞掉点击，绝不穿透到被遮住的正文或块按钮。
    } else if let Some(index) = paste::paste_item_at(state, view, mouse.column, mouse.row) {
        update::select_paste(state, index);
        *dirty = true;
    } else if let Some(index) = paste::refer_item_at(state, view, mouse.column, mouse.row) {
        update::select_refer(state, index);
        *dirty = true;
    } else if let Some(tab) = mention_tab_at(state, view, mouse.column, mouse.row) {
        update::switch_mention_tab(state, tab);
        *dirty = true;
    } else if let Some(index) = mention_item_at(state, view, mouse.column, mouse.row) {
        update::select_mention(state, index);
        *dirty = true;
    } else if let Some(index) = slash_item_at(state, view, mouse.column, mouse.row) {
        update::select_slash(state, index);
        *dirty = true;
    } else if let Some(index) = panel_item_at(state, view, mouse.column, mouse.row) {
        update::select_panel(state, index);
        *dirty = true;
    } else if super::hit::prompt_command_panel_contains(state, view, mouse.column, mouse.row) {
        // 浮层边界拦截：点击落在 Prompt 已打开命令面板的非条目区域（顶底边框、标题行、
        // 呼吸空白行、底栏快捷键提示、空状态占位行或滚动条列）时直接吞掉，
        // 绝不穿透触发底层模板/补充块按钮或移动编辑器正文光标。
        update::focus_prompt(state);
        *dirty = true;
    } else if let Some(hit) = template_button_at(state, view, mouse.column, mouse.row) {
        apply_template_button(state, hit);
        *dirty = true;
    } else if let Some(hit) = block_button_at(state, view, mouse.column, mouse.row) {
        apply_block_button(state, hit);
        *dirty = true;
    } else if !state.prompt_collapsed
        && let Some(button) = layout::prompt_toolbar_button_at(
            view.prompt,
            mouse.column,
            mouse.row,
            state.prompt_owner().prompt_panel.mode,
            prompt_counter(state).as_deref(),
        )
    {
        // 工具栏按钮是控件：执行动作，不抢焦点、不移动文本光标。
        let editor_view =
            state.prompt_owner().prompt_panel.mode == crate::app::prompt::PromptMode::Editor;
        match button {
            layout::PromptToolbarButton::Mode => {
                update::toggle_prompt_panel_mode(state);
            }
            layout::PromptToolbarButton::Recent => {
                update::toggle_prompt_recent(state);
                if update::prompt_recent_open(state)
                    && !ui::prompt_frame::prompt_recent_visible(view.prompt, state)
                {
                    update::close_prompt_recent(state);
                }
            }
            layout::PromptToolbarButton::TempPrompt => {
                update::open_temp_prompt(state);
            }
            layout::PromptToolbarButton::Folders => {
                update::open_referenced_folder_menu(state, (mouse.column, mouse.row));
            }
            layout::PromptToolbarButton::Undo if editor_view => state.prompt.undo(),
            layout::PromptToolbarButton::Redo if editor_view => state.prompt.redo(),
            layout::PromptToolbarButton::Save if editor_view => {
                update::save_prompt_now(state);
            }
            _ => {}
        }
        *dirty = true;
    } else if !state.prompt_collapsed
        && ui::prompt_frame::prompt_status_at(view.prompt, state, mouse.column, mouse.row).is_some()
    {
        // 标题行的状态标签是控件：循环本页模板块状态，不移动正文光标。
        update::focus_prompt(state);
        update::clear_selection(state);
        update::cycle_page_template_status(state);
        *dirty = true;
    } else if let Some(control) = (!state.prompt_collapsed)
        .then(|| ui::prompt_page_control_at(view.prompt, state, mouse.column, mouse.row))
        .flatten()
    {
        // 工具栏页控件是控件：不移动正文光标；命中后仍把焦点交给编辑器。
        update::focus_prompt(state);
        match control {
            ui::prompt::PageControl::Prev => {
                update::switch_prompt_page(state, -1);
            }
            ui::prompt::PageControl::Next => {
                update::switch_prompt_page(state, 1);
            }
            ui::prompt::PageControl::Counter => {
                update::toggle_prompt_pages(state);
            }
        }
        *dirty = true;
    } else if !state.prompt_collapsed && ui::prompt_title_at(view.prompt, mouse.column, mouse.row) {
        // 标题行是控件区：点条目名/空白只把焦点交给编辑器，不移动正文光标。
        update::focus_prompt(state);
        *dirty = true;
    } else if !state.prompt_collapsed && prompt_header_at(view.prompt, mouse.column, mouse.row) {
        // 工具栏与分割线的空白区域吞掉点击：不聚焦文本视口、不移动文本光标。
    } else if let Some(pane) = pane_toggle_button_at(state, rects, mouse.column, mouse.row) {
        // 按钮是控件：只切换视图，不改变焦点。
        update::toggle_pane_view(state, pane);
        *dirty = true;
    } else if let Some(hit) = layout::resize_boundary_at(rects, mouse.column, mouse.row)
        .filter(|hit| pane_boundary_draggable(state, *hit))
    {
        update::begin_pane_resize(state, hit);
        *dirty = true;
    } else if let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) {
        let inner = pane_content_inner(state, view, pane, inner);
        // 内容区可能为零（prompt 面板不可交互时 `pane_content_inner` 返回零矩形）：
        // 先判空再换算坐标，避免拿屏幕坐标当内容坐标去定位光标/建选区。
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let row = mouse.row - inner.y;
        let col = mouse.column - inner.x;
        if pane == state.prompt.id() {
            update::focus_prompt(state);
            if state.prompt.paste_panel().is_some() || state.prompt.text_paste_panel().is_some() {
                // 粘贴面板打开时点击编辑区：关闭面板且不穿透、不移动光标。
                state.prompt.paste_cancel();
                state.prompt.text_paste_cancel();
            } else if update::prompt_tree_mode(state) {
                // 顶部搜索框：单击/双击/三击与拖拽选中文本。
                let search_hit = ui::prompt::tree::search_char_at(
                    inner,
                    &state.prompt_owner().prompt_panel,
                    mouse.column,
                    mouse.row,
                );
                if let Some(char_idx) = search_hit {
                    let clicks = crate::app::selection::next_click_count(
                        &mut state.selection_click,
                        mouse.column,
                        mouse.row,
                        Instant::now(),
                    );
                    update::clear_selection(state);
                    let workspace = state.prompt_workspace();
                    if let Some(ws) = state.workspaces.get_mut(workspace) {
                        ws.prompt_panel
                            .search
                            .begin_selection_with_clicks(char_idx, clicks);
                    }
                    state.prompt_search_drag = true;
                } else {
                    // 树视图：按下只选中并登记拖拽起点；折叠/展开与双击判定推迟到松开。
                    let hit = ui::prompt::tree::row_at(
                        inner,
                        &state.prompt_owner().prompt_panel,
                        mouse.column,
                        mouse.row,
                    );
                    if let Some(index) = hit {
                        update::begin_prompt_drag(state, index);
                    }
                }
            } else if update::prompt_pages_mode(state) {
                // 顶部搜索框：单击/双击/三击与拖拽选中文本。
                let search_hit = ui::prompt::pages::search_char_at(
                    inner,
                    &state.prompt_owner().prompt_panel,
                    mouse.column,
                    mouse.row,
                );
                if let Some(char_idx) = search_hit {
                    let clicks = crate::app::selection::next_click_count(
                        &mut state.selection_click,
                        mouse.column,
                        mouse.row,
                        Instant::now(),
                    );
                    update::clear_selection(state);
                    let workspace = state.prompt_workspace();
                    if let Some(ws) = state.workspaces.get_mut(workspace) {
                        ws.prompt_panel
                            .pages_view
                            .search
                            .begin_selection_with_clicks(char_idx, clicks);
                    }
                    state.prompt_search_drag = true;
                } else {
                    // 页列表视图：单击选中，400ms 内双击同一行打开该页（不支持拖拽）。
                    let hit = ui::prompt::pages::page_row_at(
                        inner,
                        &state.prompt_owner().prompt_panel,
                        mouse.column,
                        mouse.row,
                    );
                    if let Some(index) = hit {
                        let now = Instant::now();
                        update::select_page_row(state, index);
                        if update::page_row_double_click(state, index, now) {
                            update::open_selected_page(state);
                        }
                    }
                }
            } else {
                // 底部虚拟空占位只聚焦编辑器，不把光标移到不存在的文本行。
                if !state.prompt.row_in_padding(row) {
                    if let Some(hit) = state.prompt.paste_hit_at(row, col) {
                        if !hit.expanded {
                            state.selection_click = None;
                            update::clear_selection(state);
                            state.prompt.toggle_paste(hit.id, hit.occurrence);
                        } else {
                            let clicks = crate::app::selection::next_click_count(
                                &mut state.selection_click,
                                mouse.column,
                                mouse.row,
                                Instant::now(),
                            );
                            update::clear_selection(state);
                            state.prompt.begin_preview_selection(row, col, clicks);
                        }
                    } else {
                        let clicks = crate::app::selection::next_click_count(
                            &mut state.selection_click,
                            mouse.column,
                            mouse.row,
                            Instant::now(),
                        );
                        if clicks == 1 {
                            update::place_prompt_cursor(state, row, col);
                        }
                        update::begin_selection_with_clicks(state, pane, row, col, clicks);
                    }
                }
            }
            *dirty = true;
        } else if matches!(
            state.pane_anywhere(pane).map(|pane| pane.kind),
            Some(PaneKind::Terminal)
        ) {
            if state
                .pane_anywhere(pane)
                .is_some_and(|target| target.view == PaneView::Lx)
            {
                lx::handle_lx_pane_mouse_down_left(mouse, state, pane, inner, cache, dirty);
            } else if handle_terminal_scrollbar_press(state, pane, inner, mouse.column, mouse.row) {
                // 滚动条按下先于焦点与选区：thumb 拖拽，轨道点击跳转。
                *dirty = true;
            } else {
                update::focus_pane(state, pane);
                // 应用在鼠标上报模式时按键归它自己（对齐 herdr）：转发并按它的
                // 选区逻辑处理，不启动本地选区。
                if forward_pane_mouse_event(state, sessions, pane, inner, mouse) {
                    update::clear_selection(state);
                } else {
                    let clicks = crate::app::selection::next_click_count(
                        &mut state.selection_click,
                        mouse.column,
                        mouse.row,
                        Instant::now(),
                    );
                    update::begin_terminal_selection_with_clicks(state, pane, row, col, clicks);
                }
                *dirty = true;
            }
        } else {
            update::clear_selection(state);
        }
    } else {
        update::clear_selection(state);
    }
}

/// 当前页计数字符串（`n/m`）；无页信息时为 None。
fn prompt_counter(state: &AppState) -> Option<String> {
    ui::prompt_frame::prompt_page_header(state)
        .map(|page| ui::text::page_counter(page.current, page.total))
}

/// 鼠标右键按下事件处理。
pub(crate) fn handle_mouse_down_right(
    mouse: &MouseEvent,
    state: &mut AppState,
    geometry: &Geometry,
    config: &Config,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    let Geometry { rects, view, .. } = geometry;
    // macOS 部分终端/触控板会将 `Ctrl + 左键点击` 映射为带 CONTROL 修饰键的右键事件：
    // 若落在 lx 聊天区的超链接或 AI Web 生图目标上，直接按 `Ctrl + 点击` 打开，不弹右键菜单。
    if lx::handle_lx_ctrl_right_click(mouse, state, geometry, config, cache, dirty) {
        return;
    }
    // 菜单打开时右键不穿透：标签、工作区项或窗格上改挂菜单，其余位置关闭。
    if matches!(state.overlay, Some(Overlay::Menu(_))) {
        if open_right_click_menu(state, rects, view, mouse.column, mouse.row) {
            // 右键落点位于菜单容器内：初始显示悬停反馈。
            state.menu_hover = true;
        } else {
            update::close_overlay(state);
        }
        *dirty = true;
        return;
    }
    // 重命名与关闭确认期间右键吞掉，不替换模态。
    if state.overlay.is_some() {
        return;
    }
    if open_right_click_menu(state, rects, view, mouse.column, mouse.row) {
        // 右键落点位于菜单容器内：初始显示悬停反馈。
        state.menu_hover = true;
        *dirty = true;
    }
}

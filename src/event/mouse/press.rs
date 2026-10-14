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

use super::super::overlay::{handle_overlay_click, open_right_click_menu};
use super::super::paste;
use super::super::session::{Geometry, ensure_workspace_visible, reveal_active_tab, write_to_pane};
use super::hit::{
    apply_block_button, apply_template_button, block_button_at, handle_agent_scrollbar_press,
    handle_prompt_scrollbar_press, handle_terminal_scrollbar_press,
    handle_workspace_scrollbar_press, mention_item_at, pane_at, pane_boundary_draggable,
    pane_content_inner, pane_toggle_button_at, panel_item_at, prompt_boundary_hit,
    prompt_header_at, slash_item_at, template_button_at,
};
use super::hover::reset_pointer_shape;
use super::wheel::forward_pane_mouse_event;

/// 鼠标左键按下事件处理。
pub(crate) fn handle_mouse_down_left(
    mouse: &MouseEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    config: &Config,
    dirty: &mut bool,
) {
    let Geometry {
        screen,
        rects,
        view,
    } = geometry;
    if update::clear_tooltip(state) {
        *dirty = true;
    }
    if state.overlay.is_some() {
        let was_confirm = matches!(
            state.overlay,
            Some(Overlay::ConfirmClose(_) | Overlay::ConfirmSyncWorkspaceCwd(_))
        );
        let was_menu = matches!(state.overlay, Some(Overlay::Menu(_)));
        let was_worktree = matches!(state.overlay, Some(Overlay::WorktreeOpen(_)));
        let was_new_ws = matches!(
            state.overlay,
            Some(Overlay::NewWorkspace(_) | Overlay::TrustWorkspace(_))
        );
        if let Some((pane, bytes)) = handle_overlay_click(state, *screen, mouse.column, mouse.row) {
            write_to_pane(sessions, pane, &bytes);
            update::reset_pane_scroll(state, pane);
        }
        if was_confirm || was_menu || was_worktree || was_new_ws {
            ensure_workspace_visible(state, view);
        }
        reveal_active_tab(state, view);
        *dirty = true;
        return;
    }
    if let Some(toast) = ui::toast::rect(state, view, rects, *screen, config) {
        if toast.contains((mouse.column, mouse.row).into()) {
            // 点击 toast 内部内容不关闭；仅右上角 [✕] 关闭。
            if ui::toast::close_button_at(toast, mouse.column, mouse.row) {
                update::dismiss_toast(state);
                *dirty = true;
            }
            return;
        }
        // 点击 toast 之外：关闭 toast 后继续处理本次点击。
        update::dismiss_toast(state);
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
                    ToastKind::Error,
                    ui::text::TOAST_SETTINGS_UNAVAILABLE,
                    None,
                    now,
                ),
            );
        }
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
        || handle_agent_scrollbar_press(state, view, mouse.column, mouse.row)
    {
        *dirty = true;
    } else if let Some(pane) = ui::agent_item_at(view, state, mouse.column, mouse.row) {
        if update::focus_agent_pane(state, pane) {
            ensure_workspace_visible(state, view);
            reveal_active_tab(state, view);
            *dirty = true;
        }
    } else if let Some(index) = ui::workspace_group_toggle_at(view, state, mouse.column, mouse.row)
    {
        update::toggle_workspace_group(state, index);
        if let Some(rows) = ui::workspace_list_rows(view, state) {
            update::clamp_workspace_scroll(state, rows);
        }
        ensure_workspace_visible(state, view);
        *dirty = true;
    } else if let Some(index) = ui::workspace_item_at(view, state, mouse.column, mouse.row) {
        update::switch_workspace(state, index);
        ensure_workspace_visible(state, view);
        reveal_active_tab(state, view);
        update::begin_workspace_drag(state, index);
        *dirty = true;
    } else if ui::add_workspace_button(view)
        .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
    {
        update::open_new_workspace_dialog(state);
        ensure_workspace_visible(state, view);
        reveal_active_tab(state, view);
        *dirty = true;
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
    } else if let Some(index) = paste::paste_item_at(state, view, mouse.column, mouse.row) {
        update::select_paste(state, index);
        *dirty = true;
    } else if let Some(index) = paste::refer_item_at(state, view, mouse.column, mouse.row) {
        update::select_refer(state, index);
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
            update::prompt_tree_mode(state),
        )
    {
        // 工具栏按钮是控件：执行动作，不抢焦点、不移动文本光标；
        // 页重命名输入先提交（空名保留旧名）。
        update::finish_page_rename(state);
        let editor_view = !update::prompt_tree_mode(state);
        match button {
            layout::PromptToolbarButton::Mode => {
                update::toggle_prompt_panel_mode(state);
            }
            layout::PromptToolbarButton::TempPrompt => {
                update::open_temp_prompt(state);
            }
            layout::PromptToolbarButton::Pin => {
                update::toggle_prompt_pin(state);
            }
            layout::PromptToolbarButton::Undo if editor_view => state.prompt.undo(),
            layout::PromptToolbarButton::Redo if editor_view => state.prompt.redo(),
            layout::PromptToolbarButton::SelectAll if editor_view => {
                state.select_all_prompt();
            }
            layout::PromptToolbarButton::Save if editor_view => {
                update::save_prompt_now(state);
            }
            _ => {}
        }
        *dirty = true;
    } else if !state.prompt_collapsed
        && update::page_rename_active(state)
        && state.overlay.is_none()
    {
        // 页重命名输入期间：点击输入区保持输入态；点击其余位置提交
        // （空名保留旧名）并吞掉本次点击。
        if ui::prompt_page_control_at(view.prompt, state, mouse.column, mouse.row)
            != Some(ui::prompt::PageControl::Name)
        {
            update::finish_page_rename(state);
        }
        *dirty = true;
    } else if let Some(control) = (!state.prompt_collapsed)
        .then(|| ui::prompt_page_control_at(view.prompt, state, mouse.column, mouse.row))
        .flatten()
    {
        // 标题行页控件是控件：不移动正文光标；命中后仍把焦点交给编辑器。
        update::focus_prompt(state);
        match control {
            ui::prompt::PageControl::Prev => {
                update::switch_prompt_page(state, -1);
            }
            ui::prompt::PageControl::Next => {
                update::switch_prompt_page(state, 1);
            }
            ui::prompt::PageControl::Counter => {
                update::open_prompt_page_menu(state, (mouse.column, mouse.row));
            }
            ui::prompt::PageControl::Name => {
                update::begin_page_rename(state);
            }
        }
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
        let row = mouse.row - inner.y;
        let col = mouse.column - inner.x;
        if pane == state.prompt.id() {
            update::focus_prompt(state);
            if state.prompt.paste_panel().is_some() {
                // 粘贴面板打开时点击编辑区：关闭面板且不穿透、不移动光标。
                state.prompt.paste_cancel();
            } else if update::prompt_tree_mode(state) {
                // 树视图：点击行选中；文件夹行同时折叠/展开。
                let hit = ui::prompt::tree::row_at(
                    inner,
                    &state.prompt_owner().prompt_panel,
                    mouse.column,
                    mouse.row,
                );
                if let Some(index) = hit {
                    if update::prompt_row_double_click(state, index, Instant::now()) {
                        update::open_prompt_row(state, index);
                    } else {
                        update::click_prompt_row(state, index);
                    }
                }
            } else {
                // 底部虚拟空占位只聚焦编辑器，不把光标移到不存在的文本行。
                if !state.prompt.row_in_padding(row) {
                    update::place_prompt_cursor(state, row, col);
                    update::begin_selection(state, pane, row, col);
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
                // lx 页：命中入口按钮则切换选中，否则开始整页选区。
                let entry = state.pane_anywhere(pane).and_then(|target| {
                    ui::main_content::lx::entry_hit(inner, &target.lx, mouse.column, mouse.row)
                });
                update::focus_pane(state, pane);
                update::clear_selection(state);
                match entry {
                    Some(entry) => {
                        update::set_lx_entry(state, pane, entry);
                        update::clear_lx_selection(state);
                    }
                    None => update::begin_lx_selection(state, pane, row, col),
                }
                *dirty = true;
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
                    update::begin_terminal_selection(state, pane, row, col);
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

/// 鼠标右键按下事件处理。
pub(crate) fn handle_mouse_down_right(
    mouse: &MouseEvent,
    state: &mut AppState,
    geometry: &Geometry,
    dirty: &mut bool,
) {
    let Geometry { rects, view, .. } = geometry;
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

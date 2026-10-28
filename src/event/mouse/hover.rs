//! 鼠标移动悬停：工作区/标签/Agents 高亮、tooltip、分割线指针形状与终端鼠标转发。

use std::collections::HashMap;
use std::time::Instant;

use crossterm::event::MouseEvent;
use ratatui::layout::Direction;

use crate::app::overlay::Overlay;
use crate::app::state::{AppState, PaneView};
use crate::app::update;
use crate::config::Config;
use crate::layout::{self, PaneId};
use crate::platform::PointerShape;
use crate::pty::PtySession;
use crate::ui;

use super::super::paste;
use super::super::session::Geometry;
use super::hit::{
    lx_slash_item_at, lx_toast_at, mention_item_at, pane_at, pane_boundary_draggable,
    pane_content_inner, panel_item_at, prompt_boundary_hit, slash_item_at,
};
use super::wheel::forward_pane_mouse_event;

/// 设置鼠标指针形状；终端不支持时静默回退。
pub(crate) fn set_pointer_shape(shape: PointerShape) {
    if let Err(error) = crate::platform::set_pointer_shape(shape) {
        tracing::debug!(%error, "pointer shape write failed");
    }
}

/// 恢复默认鼠标指针形状。
pub(crate) fn reset_pointer_shape() {
    set_pointer_shape(PointerShape::Default);
}

/// 鼠标移动悬停事件处理。
pub(crate) fn handle_mouse_moved(
    mouse: &MouseEvent,
    state: &mut AppState,
    sessions: &mut HashMap<PaneId, PtySession>,
    geometry: &Geometry,
    config: &Config,
    cache: &mut ui::RenderCache,
    dirty: &mut bool,
) {
    let Geometry {
        screen,
        rects,
        view,
    } = geometry;
    // 浮层打开时悬停只服务菜单高亮，不触碰底层 hover 状态。
    if state.overlay.is_some() {
        if let Some(Overlay::Menu(menu)) = state.overlay.as_ref() {
            let layout = ui::overlay::menu_layout(state, *screen, menu);
            if let Some(index) = ui::widgets::menu::item_at(&layout, mouse.column, mouse.row) {
                if update::set_menu_selection(state, index) {
                    *dirty = true;
                }
            } else if !layout.area.contains((mouse.column, mouse.row).into())
                && update::clear_menu_hover(state)
            {
                *dirty = true;
            }
        }
        if let Some(Overlay::PageMenu(menu)) = state.overlay.as_ref() {
            let layout = ui::overlay::page_menu_layout(state, *screen, menu);
            if let Some(index) =
                ui::overlay::page_menu_item_at(state, *screen, menu, mouse.column, mouse.row)
            {
                if update::set_page_menu_selection(state, index) {
                    *dirty = true;
                }
            } else if !layout.area.contains((mouse.column, mouse.row).into())
                && update::clear_menu_hover(state)
            {
                *dirty = true;
            }
        }
        if let Some(Overlay::ReferencedFolderMenu(menu)) = state.overlay.as_ref() {
            // 悬停只铺悬停底色，不移动 `❯` 选中标记（对齐 prompt 树列表）；离开弹窗清空。
            let hover = ui::overlay::referenced_menu_row_at(*screen, menu, mouse.column, mouse.row);
            if update::hover_referenced_menu(state, hover) {
                *dirty = true;
            }
        }
        if let Some(Overlay::ReferencedFolderSearch(search)) = state.overlay.as_ref()
            && let Some(index) =
                ui::overlay::referenced_search_item_at(*screen, search, mouse.column, mouse.row)
            && update::set_referenced_search_selection(state, index)
        {
            *dirty = true;
        }
        if update::set_prompt_tree_hover(state, None) {
            *dirty = true;
        }
        // 页码菜单与引用文件夹内容弹窗的边框按钮可提示；其余浮层抑制 tooltip。
        let tooltip_target = if matches!(
            state.overlay,
            Some(Overlay::PageMenu(_) | Overlay::ReferencedFolderSearch(_))
        ) {
            ui::tooltip::target_at(view, state, rects, *screen, mouse.column, mouse.row)
        } else {
            None
        };
        if update::set_tooltip_target(state, tooltip_target, Instant::now()) {
            *dirty = true;
        }
        return;
    }
    // 侧栏工作区行与标签栏的悬停高亮先于浮层命中计算：指针离开条目区即清空。
    let workspace_hover = if state.sidebar_collapsed || state.workspace_drag.is_some() {
        None
    } else {
        ui::workspace_item_at(view, state, mouse.column, mouse.row)
    };
    let tab_layout = ui::tab_bar::layout(view, state.active_workspace(), state.tab_scroll);
    let tab_hover = ui::tab_bar::tab_at(&tab_layout, mouse.column, mouse.row);
    let agent_hover = ui::agent_item_index_at(view, state, mouse.column, mouse.row);
    let mut item_hover_changed = update::set_workspace_hover(state, workspace_hover);
    item_hover_changed |= update::set_tab_hover(state, tab_hover);
    item_hover_changed |= update::set_agent_hover(state, agent_hover);
    if item_hover_changed {
        *dirty = true;
    }
    if let Some(index) = paste::paste_item_at(state, view, mouse.column, mouse.row) {
        if update::hover_paste(state, index) {
            *dirty = true;
        }
        if update::clear_tooltip(state) {
            *dirty = true;
        }
        return;
    }
    if let Some(index) = paste::refer_item_at(state, view, mouse.column, mouse.row) {
        if update::hover_refer(state, index) {
            *dirty = true;
        }
        if update::clear_tooltip(state) {
            *dirty = true;
        }
        return;
    }
    if let Some(index) = mention_item_at(state, view, mouse.column, mouse.row) {
        if update::hover_mention(state, index) {
            *dirty = true;
        }
        if update::clear_tooltip(state) {
            *dirty = true;
        }
        return;
    }
    if let Some((pane, index)) = lx_slash_item_at(state, rects, mouse.column, mouse.row) {
        if update::hover_lx_slash(state, pane, index) {
            *dirty = true;
        }
        if update::clear_tooltip(state) {
            *dirty = true;
        }
        return;
    }
    if let Some(index) = slash_item_at(state, view, mouse.column, mouse.row) {
        if update::hover_slash(state, index) {
            *dirty = true;
        }
        if update::clear_tooltip(state) {
            *dirty = true;
        }
        return;
    }
    if let Some(index) = panel_item_at(state, view, mouse.column, mouse.row) {
        if update::hover_panel(state, index) {
            *dirty = true;
        }
        if update::clear_tooltip(state) {
            *dirty = true;
        }
        return;
    }
    let hovering_toast = ui::toast::rect(state, view, rects, *screen, config)
        .is_some_and(|toast| toast.contains((mouse.column, mouse.row).into()));
    if update::set_toast_hover(state, hovering_toast) {
        *dirty = true;
    }
    // lx 窗格自己的 toast：悬停同样冻结消失计时（与全局 toast 同一语义）。
    // 命中的是「指针下那个窗格的盒」，不放宽到其它窗格。
    let hovering_lx_toast = lx_toast_at(rects, state, mouse.column, mouse.row);
    let lx_toast_panes = state.all_pane_ids();
    for pane in lx_toast_panes {
        let hovered = hovering_lx_toast.is_some_and(|(id, _)| id == pane);
        if update::set_lx_toast_hover(state, pane, hovered) {
            *dirty = true;
        }
    }
    // 指针停在任意一条 lx 提示上时，都不触发它下面那些控件的悬停反馈。
    let hovering_toast = hovering_toast || hovering_lx_toast.is_some();
    // lx 输入框上方「回到底部」按钮悬停高亮：不在按钮上即清空（含离开窗格）。
    let jump_hover = if hovering_toast {
        None
    } else {
        ui::main_content::lx::chat_bar::jump_bottom_hit(rects, state, mouse.column, mouse.row)
    };
    for (id, _) in rects {
        if update::hover_lx_jump_bottom(state, *id, jump_hover == Some(*id)) {
            *dirty = true;
        }
    }
    // lx 聊天卡片悬停：折叠内容提亮 / 思考块恢复品红。悬停状态归窗格所有——
    // 指针下的那个 lx 窗格拿到目标，其余窗格一律清空，否则上一张卡片会一直亮着。
    let pointer = pane_at(rects, mouse.column, mouse.row).filter(|(pane, _)| {
        state
            .pane_anywhere(*pane)
            .is_some_and(|target| target.view == PaneView::Lx && target.lx.task.is_none())
    });
    let card_hover = pointer.and_then(|(pane, inner)| {
        let target = state.pane_anywhere(pane)?;
        let inner = pane_content_inner(state, view, pane, inner);
        ui::main_content::lx::fold_target_at(
            inner,
            &target.lx,
            mouse.column,
            mouse.row,
            cache.pane(pane),
        )
    });
    let pointer_pane = pointer.map(|(pane, _)| pane);
    for (id, _) in rects {
        let hover = card_hover.clone().filter(|_| Some(*id) == pointer_pane);
        if update::hover_lx_chat(state, *id, hover) {
            *dirty = true;
        }
    }
    // 容器 tab 悬停：仅在未折叠且指针不在 toast 上时生效；离开即清空。
    let right_tab_hover = if hovering_toast || state.prompt_collapsed {
        None
    } else {
        ui::right_panel::tab_at(view.prompt, mouse.column, mouse.row)
    };
    if update::set_right_tab_hover(state, right_tab_hover) {
        *dirty = true;
    }
    // Todo 行悬停：仅在 Todo 面板可见时计算；指针离开即清空。
    let todo_hover = if hovering_toast
        || state.prompt_collapsed
        || state.right_tab != crate::app::state::RightTab::Todo
    {
        None
    } else {
        // `row_at` 给的是**显示**下标；悬停状态与渲染层都用列表下标比较，
        // 必须在这里换算一次，否则幽灵行打开时悬停会高亮到下一行。
        ui::right_panel::todo::row_at(view.prompt, state, mouse.column, mouse.row)
            .and_then(|display| ui::right_panel::todo::list_index(state, display))
    };
    if update::set_todo_hover(state, todo_hover) {
        *dirty = true;
    }
    // 提示词列表悬停：仅树视图且在文本区内；指针离开即清空。
    let tree_hover = if hovering_toast
        || state.prompt_drag.is_some()
        || !update::prompt_panel_active(state)
        || !update::prompt_tree_mode(state)
    {
        None
    } else {
        ui::prompt::tree::row_at(
            crate::layout::prompt_text_rect(view.prompt),
            &state.prompt_owner().prompt_panel,
            mouse.column,
            mouse.row,
        )
    };
    if update::set_prompt_tree_hover(state, tree_hover) {
        *dirty = true;
    }
    // 按钮 tooltip：拖拽排序/列表拖拽或指针在 toast 上时不触发；目标变化重置显示计时。
    let tooltip_target =
        if hovering_toast || state.workspace_drag.is_some() || state.prompt_drag.is_some() {
            None
        } else {
            ui::tooltip::target_at(view, state, rects, *screen, mouse.column, mouse.row)
        };
    if update::set_tooltip_target(state, tooltip_target, Instant::now()) {
        *dirty = true;
    }
    let boundary = layout::resize_boundary_at(rects, mouse.column, mouse.row);
    let sidebar_hover = !hovering_toast && ui::sidebar_boundary_at(view, mouse.column, mouse.row);
    // 交换按钮是控件：悬停它不视为分割线悬停（不高亮、不显示拖拽指针）。
    let swap_hover = !hovering_toast && ui::swap_button_at(view, mouse.column, mouse.row);
    let prompt_hover = !hovering_toast
        && !sidebar_hover
        && !swap_hover
        && boundary.is_some_and(|hit| prompt_boundary_hit(state, hit));
    let pane_hover = if hovering_toast || sidebar_hover || prompt_hover || swap_hover {
        None
    } else {
        boundary.filter(|hit| pane_boundary_draggable(state, *hit))
    };
    let mut hover_changed = update::set_sidebar_hover(state, sidebar_hover);
    hover_changed |= update::set_prompt_hover(state, prompt_hover);
    hover_changed |= update::set_pane_hover(state, pane_hover);
    if hover_changed {
        let shape = if sidebar_hover || prompt_hover {
            PointerShape::EwResize
        } else {
            match pane_hover.map(|hit| hit.direction) {
                Some(Direction::Horizontal) => PointerShape::EwResize,
                Some(Direction::Vertical) => PointerShape::NsResize,
                None => PointerShape::Default,
            }
        };
        set_pointer_shape(shape);
        *dirty = true;
    }
    if let Some((pane, inner)) = pane_at(rects, mouse.column, mouse.row) {
        forward_pane_mouse_event(state, sessions, pane, inner, mouse);
    }
}

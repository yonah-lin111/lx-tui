//! 鼠标命中测试、几何辅助与滚动条抓取计算。

use std::time::Instant;

use ratatui::layout::Rect;

use crate::app::state::{AppState, PaneKind};
use crate::app::toast::{Toast, ToastKind};
use crate::app::update;
use crate::layout::{self, BoundaryHit, PaneId};
use crate::ui;

use super::super::paste;

/// 工作区滚动条按下：thumb 开始拖拽，轨道点击跳转；返回是否命中。
pub(crate) fn handle_workspace_scrollbar_press(
    state: &mut AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = ui::workspace_scrollbar(view, state) else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    let rows = ui::workspace_list_rows(view, state).unwrap_or(0);
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.workspace_scroll_drag = Some(grab),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_workspace_scroll(state, offset, rows);
        }
    }
    true
}

/// prompt 滚动条按下：thumb 开始拖拽，轨道点击跳转；树视图滚动树、编辑视图滚动文本；
/// 不改变焦点与光标；返回是否命中。
pub(crate) fn handle_prompt_scrollbar_press(
    state: &mut AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = ui::prompt_scrollbar(view, state) else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.prompt_scroll_drag = Some(grab),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            if update::prompt_tree_mode(state) {
                let visible = update::prompt_tree_visible_rows(state);
                update::set_prompt_tree_scroll(state, offset, visible);
            } else {
                update::set_prompt_scroll(state, offset);
            }
        }
    }
    true
}

/// Agents 滚动条按下：thumb 开始拖拽，轨道点击跳转；不改变焦点；返回是否命中。
pub(crate) fn handle_agent_scrollbar_press(
    state: &mut AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = ui::agent_scrollbar(view, state) else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    let rows = ui::agent_list_rows(view, state).unwrap_or(0);
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.agent_scroll_drag = Some(grab),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_agent_scroll(state, offset, rows);
        }
    }
    true
}

/// 终端窗格滚动条按下：thumb 开始拖拽，轨道点击跳转；不改变焦点；返回是否命中。
pub(crate) fn handle_terminal_scrollbar_press(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::scrollbar(inner, target))
    else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.terminal_scroll_drag = Some((pane, grab)),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_terminal_scroll(state, pane, offset);
        }
    }
    true
}

/// lx 输入框滚动条按下：thumb 开始拖拽，轨道点击跳转；不改变焦点；返回是否命中。
pub(crate) fn handle_lx_input_scrollbar_press(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::input_scrollbar(inner, &target.lx))
    else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.lx_scroll_drag = Some((pane, grab)),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_lx_scroll(state, pane, offset);
        }
    }
    true
}

/// lx 聊天历史滚动条按下：thumb 开始拖拽，轨道点击跳转；返回是否命中。
pub(crate) fn handle_lx_history_scrollbar_press(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = state
        .pane_anywhere(pane)
        .and_then(|target| ui::main_content::lx::history_scrollbar(inner, &target.lx))
    else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.lx_history_scroll_drag = Some((pane, grab)),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_lx_history_scroll(state, pane, offset, bar.total, bar.visible);
        }
    }
    true
}

/// 提及面板命中：返回条目索引；面板未打开或未命中返回 None。
pub(crate) fn mention_item_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = paste::prompt_text_area(view);
    ui::prompt::mention_item_at(&state.prompt, area, column, row)
}

/// 提及面板布局；用于滚轮命中与视口滚动。
pub(crate) fn mention_panel_layout(
    state: &AppState,
    view: &ui::layout::ViewLayout,
) -> Option<ui::widgets::command_panel::PanelLayout> {
    let area = paste::prompt_text_area(view);
    ui::prompt::mention_layout(&state.prompt, area)
}

/// 定位鼠标下的 lx 窗格及其内容区（仅 lx 视图）；用于 `/` 面板命中/悬停/滚轮。
pub(crate) fn lx_pane_at(
    state: &AppState,
    rects: &[(PaneId, Rect)],
    column: u16,
    row: u16,
) -> Option<(PaneId, Rect)> {
    let (pane, inner) = pane_at(rects, column, row)?;
    state
        .pane_anywhere(pane)
        .filter(|target| target.view == crate::app::state::PaneView::Lx)
        .map(|_| (pane, inner))
}

/// lx `/` 命令面板命中：返回 (窗格, 条目索引)；未命中返回 None。
pub(crate) fn lx_slash_item_at(
    state: &AppState,
    rects: &[(PaneId, Rect)],
    column: u16,
    row: u16,
) -> Option<(PaneId, usize)> {
    let (pane, inner) = lx_pane_at(state, rects, column, row)?;
    let target = state.pane_anywhere(pane)?;
    ui::main_content::lx::slash_item_at(inner, &target.lx, column, row).map(|index| (pane, index))
}

/// lx `/` 命令面板布局；用于滚轮命中与视口滚动。
pub(crate) fn lx_slash_panel_layout(
    state: &AppState,
    pane: PaneId,
    inner: Rect,
) -> Option<ui::widgets::command_panel::PanelLayout> {
    let target = state.pane_anywhere(pane)?;
    ui::main_content::lx::slash_layout(inner, &target.lx)
}

/// 块命令面板命中：返回条目索引；面板未打开或未命中返回 None。
pub(crate) fn panel_item_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = paste::prompt_text_area(view);
    ui::prompt::panel_item_at(&state.prompt, area, column, row)
}

/// 块命令面板布局；用于滚轮命中与视口滚动。
pub(crate) fn block_panel_layout(
    state: &AppState,
    view: &ui::layout::ViewLayout,
) -> Option<ui::widgets::command_panel::PanelLayout> {
    let area = paste::prompt_text_area(view);
    ui::prompt::panel_layout(&state.prompt, area)
}

/// 斜杠命令面板命中：返回条目索引；面板未打开或未命中返回 None。
pub(crate) fn slash_item_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<usize> {
    let area = paste::prompt_text_area(view);
    ui::prompt::slash_item_at(&state.prompt, area, column, row)
}

/// 斜杠命令面板布局；用于滚轮命中与视口滚动。
pub(crate) fn slash_panel_layout(
    state: &AppState,
    view: &ui::layout::ViewLayout,
) -> Option<ui::widgets::command_panel::PanelLayout> {
    let area = paste::prompt_text_area(view);
    ui::prompt::slash_layout(&state.prompt, area)
}

/// 模板块操作按钮命中；未命中返回 None。
pub(crate) fn template_button_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<ui::prompt::TemplateButtonHit> {
    let area = paste::prompt_text_area(view);
    ui::prompt::template_button_at(&state.prompt, area, column, row)
}

/// 执行模板块按钮动作：复制写剪贴板并 Toast，其余就地变更（均单步可撤销）。
pub(crate) fn apply_template_button(state: &mut AppState, hit: ui::prompt::TemplateButtonHit) {
    match hit.button {
        ui::prompt::TemplateBlockButton::Copy => {
            if let Some(text) = update::copy_template_block(state, hit.line) {
                copy_with_toast(state, &text);
            }
        }
        ui::prompt::TemplateBlockButton::Clean => {
            update::clean_template_block(state, hit.line);
        }
        ui::prompt::TemplateBlockButton::Delete => {
            update::delete_template_block(state, hit.line);
        }
        ui::prompt::TemplateBlockButton::Status => {
            update::toggle_template_status(state, hit.line);
        }
    }
}

/// 补充块操作按钮命中；未命中返回 None。
pub(crate) fn block_button_at(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> Option<ui::prompt::BlockButtonHit> {
    let area = paste::prompt_text_area(view);
    ui::prompt::block_button_at(&state.prompt, area, column, row)
}

/// 执行补充块按钮动作（单步可撤销）。
pub(crate) fn apply_block_button(state: &mut AppState, hit: ui::prompt::BlockButtonHit) {
    use crate::app::markdown::BlockKind;
    use ui::prompt::BlockButton;
    match (hit.kind, hit.button) {
        (BlockKind::Supple, BlockButton::Copy) => {
            if let Some(text) = update::copy_supple_block(state, hit.line) {
                copy_with_toast(state, &text);
            }
        }
        (BlockKind::Supple, BlockButton::Clean) => {
            update::clean_supple_block(state, hit.line);
        }
        (BlockKind::Supple, BlockButton::Delete) => {
            update::delete_supple_block(state, hit.line);
        }
        _ => {}
    }
}

/// 写系统剪贴板并在 prompt 锚点弹出成功/失败 toast。
pub(crate) fn copy_with_toast(state: &mut AppState, text: &str) {
    let anchor = Some(state.prompt.id());
    let now = Instant::now();
    let (kind, message) = if crate::platform::write_clipboard(text) {
        (ToastKind::Info, ui::text::TOAST_COPIED)
    } else {
        tracing::warn!("clipboard write failed");
        (ToastKind::Error, ui::text::TOAST_COPY_FAILED)
    };
    update::show_toast(
        state,
        Toast::new(kind, message, anchor, now).with_title(ui::text::TOAST_CLIPBOARD_TITLE),
    );
}

/// 坐标是否落在 prompt 工具栏/分割线表头行（用于吞掉空白点击）。
pub(crate) fn prompt_header_at(panel: Rect, column: u16, row: u16) -> bool {
    layout::prompt_header_rect(panel).is_some_and(|header| header.contains((column, row).into()))
}

/// 窗格交互内容区：prompt 用文本区（顶部表头下移），其余窗格用整块内容区。
pub(crate) fn pane_content_inner(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    pane: PaneId,
    inner: Rect,
) -> Rect {
    if pane == state.prompt.id() {
        paste::prompt_text_area(view)
    } else {
        inner
    }
}

/// 命中窗格内容区：返回窗格标识与其内容区矩形。
pub(crate) fn pane_at(rects: &[(PaneId, Rect)], column: u16, row: u16) -> Option<(PaneId, Rect)> {
    rects.iter().find_map(|(id, rect)| {
        let inner = layout::pane_inner_rect(*rect);
        if inner.width > 0 && inner.height > 0 && inner.contains((column, row).into()) {
            Some((*id, inner))
        } else {
            None
        }
    })
}

/// 边界命中是否属于 prompt 面板：prompt 在右时为 `second`、换到左侧时为 `first`。
pub(crate) fn prompt_boundary_hit(state: &AppState, hit: BoundaryHit) -> bool {
    hit.first == state.prompt.id() || hit.second == state.prompt.id()
}

/// 主区窗格边框可拖：两侧均为活动标签窗格且都未折叠；prompt 右栏边界归 prompt 拖拽。
pub(crate) fn pane_boundary_draggable(state: &AppState, hit: BoundaryHit) -> bool {
    let tab = state.active_tab();
    tab.pane(hit.first).is_some()
        && tab.pane(hit.second).is_some()
        && Some(hit.first) != tab.layout.collapsed()
        && Some(hit.second) != tab.layout.collapsed()
}

/// 命中窗格顶边框右端的视图切换按钮：仅 Terminal 窗格有按钮（prompt 栏与占位窗格无）。
pub(crate) fn pane_toggle_button_at(
    state: &AppState,
    rects: &[(PaneId, Rect)],
    column: u16,
    row: u16,
) -> Option<PaneId> {
    ui::main_content::toggle_button_at(rects, column, row).filter(|pane| {
        state
            .pane_anywhere(*pane)
            .is_some_and(|target| target.kind == PaneKind::Terminal)
    })
}

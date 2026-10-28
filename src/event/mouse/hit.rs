//! 鼠标命中测试、几何辅助与滚动条抓取计算。

use ratatui::layout::Rect;

use crate::app::state::{AppState, LxAutoscrollTarget, PaneKind, PaneView};
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
    cache: &mut ui::RenderCache,
) -> bool {
    let Some(bar) = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::history_scrollbar(inner, &target.lx, cache.pane(pane))
    }) else {
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

/// Agents 详情滚动条按下：thumb 拖拽或轨道跳转；返回是否命中。
pub(crate) fn handle_task_agents_scrollbar_press(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    column: u16,
    row: u16,
    cache: &mut ui::RenderCache,
) -> bool {
    let Some(bar) = state.pane_anywhere(pane).and_then(|target| {
        ui::main_content::lx::task_agents_scrollbar(inner, &target.lx, cache.pane(pane))
    }) else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.task_agents_scroll_drag = Some((pane, grab)),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_task_agents_scroll(state, pane, offset);
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
        (BlockKind::Supple, BlockButton::Fold) => {
            state.prompt.toggle_supple_fold(hit.line);
        }
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

/// 屏幕选区在 lx 窗格内允许覆盖的内容区：主历史或 Agents 详情。
///
/// 输入框、任务列表与工具栏不在该矩形内，避免拖选越过历史边界。
pub(crate) fn lx_screen_region(state: &AppState, pane: PaneId, inner: Rect) -> Option<Rect> {
    let target = state.pane_anywhere(pane)?;
    if target.view != PaneView::Lx {
        return None;
    }
    if target.lx.task.is_some() {
        return ui::main_content::lx::task_output_area(inner, &target.lx);
    }
    ui::main_content::lx::history_rect(inner, &target.lx)
}

/// lx 消息列表 / Agents 详情的拖选滚动区。
///
/// 返回目标、边缘判断矩形与当前 `(总行数, 可视行数)`；主历史与子 agent
/// 详情共用同一入口，避免事件层与绘制层各算一套行数。
pub(crate) fn lx_autoscroll_region(
    state: &AppState,
    pane: PaneId,
    inner: Rect,
    cache: &mut ui::RenderCache,
) -> Option<(LxAutoscrollTarget, Rect, usize, usize)> {
    let target = state.pane_anywhere(pane)?;
    if target.view != PaneView::Lx {
        return None;
    }
    if let Some(page) = target.lx.task.as_ref()
        && page.tab == crate::app::state::TaskTab::Agents
    {
        let area = ui::main_content::lx::task_output_inner(inner, &target.lx)?;
        let (visible, total) = ui::main_content::lx::task_agents_rows(inner, &target.lx)?;
        return Some((LxAutoscrollTarget::AgentsDetail, area, total, visible));
    }
    if target.lx.task.is_some() {
        return None;
    }
    let area = ui::main_content::lx::history_rect(inner, &target.lx)?;
    let (total, visible) = ui::main_content::lx::history_rows(inner, &target.lx, cache.pane(pane))?;
    Some((LxAutoscrollTarget::History, area, total, visible))
}

/// 命中某个 lx 窗格的 toast 盒：返回（窗格，盒矩形）。
///
/// 每个 lx 窗格至多一条 toast；命中区与绘制共用 `ui::main_content::lx::toast_rect`。
pub(crate) fn lx_toast_at(
    rects: &[(PaneId, Rect)],
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<(PaneId, Rect)> {
    rects.iter().find_map(|(pane, rect)| {
        let target = state.pane_anywhere(*pane)?;
        if target.view != PaneView::Lx || target.lx.toast.is_none() {
            return None;
        }
        let area = layout::pane_inner_rect(*rect);
        let toast = ui::main_content::lx::toast_rect(area, &target.lx)?;
        toast
            .contains((column, row).into())
            .then_some((*pane, toast))
    })
}

/// 点击落在**指针所在 lx 窗格**的 toast 盒之外时关闭那条提示；返回是否关闭。
///
/// 与全局 toast 同一套「点外部关闭」语义，但作用域收在本窗格：在别的窗格里点击
/// 不会顺手清掉这一页正在显示的反馈（全局槽只有一个，所以那边不存在这个歧义）。
/// 不吞掉点击——调用方关闭后继续按原逻辑处理。
pub(crate) fn dismiss_lx_toast_outside(
    rects: &[(PaneId, Rect)],
    state: &mut AppState,
    column: u16,
    row: u16,
) -> bool {
    let Some((pane, rect)) = pane_at(rects, column, row) else {
        return false;
    };
    let Some(target) = state.pane_anywhere(pane) else {
        return false;
    };
    if target.view != PaneView::Lx || target.lx.toast.is_none() {
        return false;
    }
    let inside = ui::main_content::lx::toast_rect(rect, &target.lx)
        .is_some_and(|toast| toast.contains((column, row).into()));
    if inside {
        return false;
    }
    crate::app::update::dismiss_lx_toast(state, pane)
}

/// 写系统剪贴板并统一在屏幕顶部居中反馈字符数。
///
/// 复制不跟随 lx/prompt/窗格锚点：锚定容器过窄时 toast 会被截成半截，用户要求
/// 所有复制反馈固定在顶部、显示完整正文。
pub(crate) fn copy_with_toast(state: &mut AppState, text: &str) {
    let success = crate::platform::write_clipboard(text);
    if !success {
        tracing::warn!("clipboard write failed");
    }
    update::show_copy_result(state, text.chars().count(), false, success);
}

/// 坐标是否落在 prompt 工具栏/分割线表头行（用于吞掉空白点击）。
pub(crate) fn prompt_header_at(panel: Rect, column: u16, row: u16) -> bool {
    layout::prompt_header_rect(panel).is_some_and(|header| header.contains((column, row).into()))
}

/// 窗格交互内容区：prompt 用文本区（顶部表头下移），其余窗格用整块内容区。
///
/// prompt 面板不可交互（容器停在 Todo）时返回零矩形：内容区不存在，
/// 任何按内容区坐标换算的路径都得到空区域而不是一次看不见的光标定位。
pub(crate) fn pane_content_inner(
    state: &AppState,
    view: &ui::layout::ViewLayout,
    pane: PaneId,
    inner: Rect,
) -> Rect {
    if pane == state.prompt.id() {
        if !update::prompt_panel_active(state) {
            return Rect::ZERO;
        }
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

/// Todo 滚动条按下：thumb 开始拖拽，轨道点击跳转；不改变焦点与高亮；返回是否命中。
pub(crate) fn handle_todo_scrollbar_press(
    state: &mut AppState,
    view: &ui::layout::ViewLayout,
    column: u16,
    row: u16,
) -> bool {
    let Some(bar) = ui::right_panel::todo::scrollbar(view.prompt, state) else {
        return false;
    };
    if !bar.track.contains((column, row).into()) {
        return false;
    }
    match ui::widgets::scrollbar::thumb_grab_offset(&bar, row) {
        Some(grab) => state.todo_scroll_drag = Some(grab),
        None => {
            let offset = ui::widgets::scrollbar::offset_from_track_row(&bar, row);
            update::set_todo_scroll(state, offset);
        }
    }
    true
}

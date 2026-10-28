//! 选区操作：prompt、终端仿真器与 lx 页选区拖拽、自动滚动与复制。

use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use crate::app::selection::{Selection, SelectionSource};
use crate::app::state::{AppState, AutoscrollDirection, PaneKind, PaneView, SelectionAutoscroll};
use crate::layout::PaneId;

use super::editor::place_prompt_cursor;
use super::pane::scroll_pane_lines;

/// 一次可复制选区：文本、反馈锚点与来源（决定清选区/剪切语义）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedSelection {
    pub text: String,
    /// lx 窗格的反馈落该窗格自己的 toast；其余为全局 toast。
    pub anchor: Option<PaneId>,
    pub source: SelectionSource,
}

/// 当前焦点下可复制的选区；无选区返回 None。
///
/// 浮层与页重命名是模态，优先于底层编辑器。复制只取真实选区，不回退全文。
pub(crate) fn copyable_selection(
    state: &AppState,
    rects: &[(PaneId, ratatui::layout::Rect)],
    cache: &mut crate::ui::RenderCache,
) -> Option<CopiedSelection> {
    if let Some(text) = screen_selection_text(state) {
        // 选区起点落在 lx 窗格时，反馈仍落该窗格自己的 toast。
        let anchor = state.screen_selection.as_ref().and_then(|selection| {
            let point = selection.range()?.0;
            rects
                .iter()
                .find(|(_, rect)| rect.contains((point.col, point.row).into()))
                .map(|(pane, _)| *pane)
                .filter(|pane| {
                    state
                        .pane_anywhere(*pane)
                        .is_some_and(|pane| pane.view == PaneView::Lx)
                })
        });
        return Some(CopiedSelection {
            text,
            anchor,
            source: SelectionSource::Screen,
        });
    }
    if let Some(text) =
        super::dialog::active_text_input(state).and_then(|input| input.selected_text())
    {
        return Some(CopiedSelection {
            text,
            anchor: None,
            source: if state.overlay.is_none() && super::page_rename_active(state) {
                SelectionSource::PageRename
            } else {
                SelectionSource::OverlayText
            },
        });
    }
    // 模态浮层遮住底层：非可编辑浮层不复制底下的 prompt/todo/终端选区。
    if state.overlay.is_some() {
        return None;
    }
    match state.focus() {
        crate::app::state::FocusedContainer::Prompt => {
            let text = prompt_selection_text(state)?;
            Some(CopiedSelection {
                text,
                anchor: Some(state.prompt.id()),
                source: SelectionSource::Prompt,
            })
        }
        crate::app::state::FocusedContainer::Todo => {
            let text = state
                .todo
                .editing()
                .and_then(|edit| edit.input.selected_text())?;
            Some(CopiedSelection {
                text,
                anchor: None,
                source: SelectionSource::Todo,
            })
        }
        crate::app::state::FocusedContainer::Sidebar => None,
        crate::app::state::FocusedContainer::Panes => {
            let id = state.active_tab().layout.focus();
            let pane = state.pane_anywhere(id)?;
            if pane.view == PaneView::Lx {
                if let Some(ask) = pane.lx.ask.as_ref()
                    && ask.editing
                    && let Some(editor) = ask.custom.get(ask.current)
                    && let Some(text) = editor.selected_text()
                {
                    return Some(CopiedSelection {
                        text,
                        anchor: Some(id),
                        source: SelectionSource::Ask(id),
                    });
                }
                if let Some(text) = pane.lx.selected_text() {
                    return Some(CopiedSelection {
                        text,
                        anchor: Some(id),
                        source: SelectionSource::LxInput(id),
                    });
                }
                let selection = state.lx_selection?;
                let area = rects
                    .iter()
                    .find(|(pane, _)| *pane == id)
                    .map(|(_, rect)| crate::layout::pane_inner_rect(*rect))?;
                let text = crate::ui::main_content::lx::copy_text(
                    area,
                    &pane.lx,
                    selection.range()?,
                    cache.pane(id),
                );
                return Some(CopiedSelection {
                    text,
                    anchor: Some(id),
                    source: SelectionSource::Lx(id),
                });
            }
            if state.terminal_selection == Some(id) {
                let text = pane.terminal.selection_text()?;
                return Some(CopiedSelection {
                    text,
                    anchor: Some(id),
                    source: SelectionSource::Terminal(id),
                });
            }
            None
        }
    }
}

/// 剪切当前焦点下的可编辑选区；无选区或只读选区返回 None。
///
/// 调用方必须先确认剪贴板写入成功再调用；本函数只负责删除并生成一条撤销记录。
/// 滚轮一格滚动的视觉行数；对齐 opencode 默认步长。
pub(crate) const WHEEL_LINES: isize = 3;

/// 在最终画面上开始一次屏幕选择；单击/双击/三击按 250 ms 同格计数。
pub fn begin_screen_selection(
    state: &mut AppState,
    screen: &crate::ui::screen::ScreenText,
    point: crate::app::selection::ScreenPoint,
    now: Instant,
) {
    let unit = click_unit(state, point, now);
    begin_screen_selection_unit(state, screen, point, unit);
}

/// 记录一次点击计数并返回本次应使用的选择单位。
pub fn click_unit(
    state: &mut AppState,
    point: crate::app::selection::ScreenPoint,
    now: Instant,
) -> crate::app::selection::SelectionUnit {
    match state.screen_click.as_mut() {
        Some(click) => click.next(point, now),
        None => {
            let mut click = crate::app::selection::ScreenClick {
                point,
                at: now,
                count: 0,
            };
            let unit = click.next(point, now);
            state.screen_click = Some(click);
            unit
        }
    }
}

/// 用已确定的单位开始屏幕选择（调用方已更新点击计数）。
pub fn begin_screen_selection_unit(
    state: &mut AppState,
    screen: &crate::ui::screen::ScreenText,
    point: crate::app::selection::ScreenPoint,
    unit: crate::app::selection::SelectionUnit,
) {
    begin_screen_selection_unit_in(state, screen, point, unit, None);
}

/// 在指定可见内容区内开始屏幕选择；区域外的行/列不进入高亮与复制。
pub fn begin_screen_selection_unit_in(
    state: &mut AppState,
    screen: &crate::ui::screen::ScreenText,
    point: crate::app::selection::ScreenPoint,
    unit: crate::app::selection::SelectionUnit,
    region: Option<Rect>,
) {
    clear_selection(state);
    if region.is_some_and(|region| !region.contains((point.col, point.row).into())) {
        return;
    }
    let (start, end) = screen.unit_range(point, unit);
    // Word/Line 可能只覆盖一个非空白格：冻结该格并保留折叠范围，否则
    // `selection_cells(start, end)` 会把两个相同端点误判成空单击。
    let frozen = if start == end && unit != crate::app::selection::SelectionUnit::Character {
        if screen.selectable_cell(start) {
            screen.selection_cells_single(start)
        } else {
            crate::app::selection::SelectedCells::default()
        }
    } else {
        screen.selection_cells_in(start, end, region)
    };
    let mut selection = crate::app::selection::ScreenSelection::new(start, unit, frozen);
    selection.set_region(region);
    selection.set_range(start, end);
    state.screen_selection = Some(selection);
}

/// 把 prompt 内的屏幕双击/三击选区同步成编辑器本地选区。
///
/// 屏幕选区用于只读文字；prompt 是可编辑目标，若继续只保留屏幕选区，
/// 输入/粘贴/剪切不会替换词或行。这里把最终屏幕坐标换回 prompt 的内容
/// 视觉行/列，并交给原有 `Selection` 模型。
pub fn begin_prompt_screen_selection(
    state: &mut AppState,
    screen: &crate::ui::screen::ScreenText,
    point: crate::app::selection::ScreenPoint,
    unit: crate::app::selection::SelectionUnit,
    inner: ratatui::layout::Rect,
) -> bool {
    if unit == crate::app::selection::SelectionUnit::Character
        || inner.width == 0
        || inner.height == 0
    {
        return false;
    }
    // 三击按 prompt 的真实逻辑行选：软折行只影响视觉行，不切断整行。
    if unit == crate::app::selection::SelectionUnit::Line {
        let Some(row) = point.row.checked_sub(inner.y) else {
            return false;
        };
        let Some(content_row) = state.prompt.scroll().checked_add(usize::from(row)) else {
            return false;
        };
        let rows = state.prompt.visual_rows();
        let Some(logical_line) = rows.get(content_row).map(|row| row.line) else {
            return false;
        };
        let Some(first) = rows.iter().position(|row| row.line == logical_line) else {
            return false;
        };
        let Some(last) = rows.iter().rposition(|row| row.line == logical_line) else {
            return false;
        };
        let (Ok(first), Ok(last)) = (u16::try_from(first), u16::try_from(last)) else {
            return false;
        };
        let start = (first, 0);
        let end = (last, u16::MAX);
        if state.prompt.selection_bounds(start, end).is_none() {
            return false;
        }
        clear_selection(state);
        let mut selection =
            crate::app::selection::Selection::begin(state.prompt.id(), i32::from(start.0), start.1);
        selection.drag(i32::from(end.0), end.1);
        state.selection = Some(selection);
        return true;
    }
    let (start, mut end) = screen.unit_range(point, unit);
    if start == end && screen.selectable_cell(start) {
        // 单字符词：本地选区终点取该字符之后；屏幕端仍是折叠点。
        end = crate::app::selection::ScreenPoint::new(end.row, end.col.saturating_add(1));
    }
    let to_cell = |point: crate::app::selection::ScreenPoint| {
        let row = point.row.checked_sub(inner.y)?;
        let col = point.col.checked_sub(inner.x)?;
        let content_row = state
            .prompt
            .scroll()
            .checked_add(usize::from(row))
            .and_then(|row| u16::try_from(row).ok())?;
        Some((content_row, col))
    };
    let (Some((start_row, start_col)), Some((end_row, end_col))) = (to_cell(start), to_cell(end))
    else {
        return false;
    };
    if state
        .prompt
        .selection_bounds((start_row, start_col), (end_row, end_col))
        .is_none()
    {
        return false;
    }
    clear_selection(state);
    let mut selection =
        crate::app::selection::Selection::begin(state.prompt.id(), i32::from(start_row), start_col);
    selection.drag(i32::from(end_row), end_col);
    if selection.range().is_none() {
        return false;
    }
    state.selection = Some(selection);
    true
}

/// 拖动扩展屏幕选区；初始词/行单位随拖动保留。
pub fn drag_screen_selection(
    state: &mut AppState,
    screen: &crate::ui::screen::ScreenText,
    point: crate::app::selection::ScreenPoint,
) {
    let Some(selection) = state.screen_selection.as_ref() else {
        return;
    };
    let base = selection.base();
    let unit = selection.unit();
    let region = selection.region();
    let point = region.map_or(point, |region| {
        crate::app::selection::ScreenPoint::new(
            point.row.clamp(region.y, region.bottom().saturating_sub(1)),
            point.col.clamp(region.x, region.right().saturating_sub(1)),
        )
    });
    let (start, end) = screen.drag_range(base, point, unit);
    let frozen = if start == end
        && unit != crate::app::selection::SelectionUnit::Character
        && screen.selectable_cell(start)
    {
        screen.selection_cells_single(start)
    } else {
        screen.selection_cells_in(start, end, region)
    };
    if let Some(selection) = state.screen_selection.as_mut() {
        selection.set_range(start, end);
        selection.set_frozen(frozen);
        selection.set_visible(true);
    }
}

/// 松开屏幕选择：只结束拖动；空选区清除。高亮与冻结 payload 保留。
pub fn finish_screen_selection(state: &mut AppState) {
    let keep = state
        .screen_selection
        .as_ref()
        .is_some_and(|selection| selection.range().is_some());
    if keep {
        if let Some(selection) = state.screen_selection.as_mut() {
            selection.finish();
        }
    } else {
        state.screen_selection = None;
    }
    state.screen_selection_refresh = None;
    state.lx_selection_autoscroll = None;
}

/// 每帧按冻结内容重定位屏幕选区；找不到内容时隐藏高亮但保留选区。
pub fn refresh_screen_selection(
    state: &mut AppState,
    screen: &crate::ui::screen::ScreenText,
) -> bool {
    let Some(selection) = state.screen_selection.as_ref() else {
        return false;
    };
    if selection.is_dragging() {
        return false;
    }
    let (anchor, cursor, visible) = (selection.anchor(), selection.cursor(), selection.visible());
    match screen.reanchor(selection.frozen()) {
        Some((start, end)) => {
            if (start, end) == (anchor, cursor) && visible {
                return false;
            }
            if let Some(selection) = state.screen_selection.as_mut() {
                selection.remap(start, end);
            }
            true
        }
        None => {
            if !visible {
                return false;
            }
            if let Some(selection) = state.screen_selection.as_mut() {
                selection.set_visible(false);
            }
            true
        }
    }
}

/// 屏幕选区文本；无选区或空 payload 返回 None。
pub fn screen_selection_text(state: &AppState) -> Option<String> {
    let selection = state.screen_selection.as_ref()?;
    let text = selection.frozen().text();
    (!text.is_empty()).then_some(text)
}

/// 边缘自动滚动每步的间隔。
pub(crate) const AUTOSCROLL_INTERVAL: Duration = Duration::from_millis(30);

/// 在 lx 页开始一次文本选择（内容行、列相对内容区左上角）。
pub fn begin_lx_selection(state: &mut AppState, id: PaneId, row: u16, col: u16) {
    clear_selection(state);
    state.lx_selection = Some(Selection::begin(id, i32::from(row), col));
}

/// 扩展 lx 页选区；窗格不一致时忽略。
pub fn drag_lx_selection(state: &mut AppState, id: PaneId, row: u16, col: u16) {
    if let Some(selection) = state.lx_selection.as_mut()
        && selection.pane() == id
    {
        selection.drag(i32::from(row), col);
    }
}

/// 结束 lx 页拖动但保留选区；空选区（未拖出范围）直接清除。
pub fn finish_lx_selection(state: &mut AppState) -> Option<Selection> {
    let keep = state
        .lx_selection
        .as_ref()
        .is_some_and(|selection| selection.range().is_some());
    let finished = if keep {
        if let Some(selection) = state.lx_selection.as_mut() {
            selection.finish();
        }
        state.lx_selection
    } else {
        state.lx_selection = None;
        None
    };
    state.lx_selection_autoscroll = None;
    finished
}

/// 清除 lx 页选区。
pub fn clear_lx_selection(state: &mut AppState) {
    state.lx_selection = None;
    state.lx_selection_autoscroll = None;
}

/// 在 lx 输入框内开始文本选择（视觉行、内容列）；光标落到该单元格。
pub fn begin_lx_input_selection(state: &mut AppState, id: PaneId, row: usize, col: u16) {
    clear_selection(state);
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.begin_text_selection(row, col);
        state.lx_input_drag = Some(id);
    }
}

/// 拖拽扩展 lx 输入框选区；窗格不一致时忽略。
pub fn drag_lx_input_selection(state: &mut AppState, id: PaneId, row: usize, col: u16) {
    if state.lx_input_drag != Some(id) {
        return;
    }
    if let Some(pane) = state.pane_mut_anywhere(id) {
        pane.lx.drag_text_selection(row, col);
    }
}

/// 结束 lx 输入框拖动但保留选区；空选区清除锚点。
pub fn finish_lx_input_selection(state: &mut AppState) -> Option<(PaneId, String)> {
    let id = state.lx_input_drag.take()?;
    let pane = state.pane_mut_anywhere(id)?;
    match pane.lx.selected_text() {
        Some(text) => Some((id, text)),
        None => {
            pane.lx.clear_text_selection();
            None
        }
    }
}

/// 在 prompt 开始一次文本选择；与右栏拖拽、终端选区互斥。
pub fn begin_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    state.resizing_prompt = false;
    state.screen_selection = None;
    clear_terminal_selection(state);
    state.selection_autoscroll = None;
    let row = selection_content_row(state, pane, row);
    state.selection = Some(Selection::begin(pane, row, col));
}

/// 扩展当前 prompt 选区；窗格不一致时忽略。
pub fn drag_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    let row = selection_content_row(state, pane, row);
    if let Some(selection) = state.selection.as_mut()
        && selection.pane() == pane
    {
        selection.drag(row, col);
    }
}

/// 视口行换算成内容行：只有 prompt 视口会滚动，加回滚动量即可。
fn selection_content_row(state: &AppState, pane: PaneId, row: u16) -> i32 {
    if pane == state.prompt.id() {
        i32::from(row) + state.prompt.scroll() as i32
    } else {
        i32::from(row)
    }
}

/// 在终端窗格开始拖拽选择；选区存于仿真器，随输出滚动钉在内容上。
pub fn begin_terminal_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    state.selection = None;
    state.screen_selection = None;
    state.selection_autoscroll = None;
    clear_terminal_selection(state);
    let started = state.pane_mut_anywhere(pane).is_some_and(|target| {
        if target.kind != PaneKind::Terminal {
            return false;
        }
        target.terminal.start_selection(row, col);
        true
    });
    state.terminal_selection = started.then_some(pane);
}

/// 扩展终端选区到新的视口坐标；目标窗格不一致时忽略。
pub fn drag_terminal_selection(state: &mut AppState, pane: PaneId, row: u16, col: u16) {
    if state.terminal_selection != Some(pane) {
        return;
    }
    if let Some(target) = state.pane_mut_anywhere(pane) {
        target.terminal.update_selection(row, col);
    }
}

/// 结束终端拖动但保留选区；返回当前选中文本（空选区返回 None）。
pub fn finish_terminal_selection(state: &mut AppState, pane: PaneId) -> Option<String> {
    state.selection_autoscroll = None;
    let target = state.pane_anywhere(pane)?;
    if target.kind != PaneKind::Terminal {
        state.terminal_selection = None;
        return None;
    }
    let text = target.terminal.selection_text();
    if text.is_none() {
        state.terminal_selection = None;
    }
    text
}

/// 放弃进行中的终端选区（清空仿真器高亮）。
pub fn clear_terminal_selection(state: &mut AppState) {
    if let Some(pane) = state.terminal_selection.take()
        && let Some(target) = state.pane_mut_anywhere(pane)
    {
        target.terminal.clear_selection();
    }
}

/// 清除 prompt、终端、lx 页与 lx 输入框选区以及输入框滚动条拖拽。
pub fn clear_selection(state: &mut AppState) {
    state.selection = None;
    state.screen_selection = None;
    state.pending_screen_click = None;
    state.pending_card_toggle = None;
    state.selection_autoscroll = None;
    state.lx_selection_autoscroll = None;
    state.screen_selection_refresh = None;
    state.lx_selection = None;
    state.lx_card_press = None;
    state.lx_scroll_drag = None;
    state.lx_history_scroll_drag = None;
    state.task_agents_scroll_drag = None;
    if let Some(pane) = state.lx_input_drag.take()
        && let Some(target) = state.pane_mut_anywhere(pane)
    {
        target.lx.clear_text_selection();
    }
    clear_terminal_selection(state);
}

/// 松开鼠标：结束拖动；空选区（未拖动）直接清除，非空选区保留供复制或删除。
pub fn end_selection_drag(state: &mut AppState) {
    state.selection_autoscroll = None;
    let keep = state
        .selection
        .as_ref()
        .is_some_and(|selection| selection.range().is_some());
    if keep {
        if let Some(selection) = state.selection.as_mut() {
            selection.finish();
        }
    } else {
        state.selection = None;
    }
}

/// 拖拽选择时按鼠标相对窗格内容区的位置安排边缘自动滚动。
///
/// 鼠标越出上下边缘：立即按距离滚动并延伸选区，随后进入定时自动滚动；
/// 停在边缘行：只登记计划；回到内部：清除计划。
pub fn arm_selection_autoscroll(
    state: &mut AppState,
    pane: PaneId,
    inner: Rect,
    mouse: (u16, u16),
    now: Instant,
) {
    if !selection_dragging_on(state, pane) {
        state.selection_autoscroll = None;
        return;
    }
    let top = inner.y;
    let bottom = inner.bottom().saturating_sub(1);
    let (direction, distance) = if mouse.1 < top {
        (AutoscrollDirection::Up, top - mouse.1)
    } else if mouse.1 > bottom {
        (AutoscrollDirection::Down, mouse.1 - bottom)
    } else if mouse.1 == top {
        (AutoscrollDirection::Up, 0)
    } else if mouse.1 == bottom {
        (AutoscrollDirection::Down, 0)
    } else {
        state.selection_autoscroll = None;
        return;
    };
    if distance > 0 {
        autoscroll_step(
            state,
            pane,
            direction,
            edge_scroll_lines(distance),
            mouse,
            inner,
        );
    }
    state.selection_autoscroll = Some(SelectionAutoscroll {
        pane,
        direction,
        mouse,
        inner,
        next_at: now + AUTOSCROLL_INTERVAL,
    });
}

/// 停止边缘自动滚动。
pub fn stop_selection_autoscroll(state: &mut AppState) {
    state.selection_autoscroll = None;
    state.lx_selection_autoscroll = None;
    state.screen_selection_refresh = None;
}

/// 自动滚动到点后补一步；返回是否产生变化。
pub(crate) fn tick_selection_autoscroll(state: &mut AppState, now: Instant) -> bool {
    let Some(autoscroll) = state.selection_autoscroll else {
        return false;
    };
    if now < autoscroll.next_at {
        return false;
    }
    if !selection_dragging_on(state, autoscroll.pane) {
        state.selection_autoscroll = None;
        return false;
    }
    if !autoscroll_step(
        state,
        autoscroll.pane,
        autoscroll.direction,
        1,
        autoscroll.mouse,
        autoscroll.inner,
    ) {
        // 到达回滚边界或视口尽头，停止。
        state.selection_autoscroll = None;
        return false;
    }
    if let Some(autoscroll) = state.selection_autoscroll.as_mut() {
        autoscroll.next_at = now + AUTOSCROLL_INTERVAL;
    }
    true
}

/// 自动滚动一步：滚动若干行并把选区终点推进到鼠标位置；返回视口是否移动。
fn autoscroll_step(
    state: &mut AppState,
    pane: PaneId,
    direction: AutoscrollDirection,
    lines: isize,
    mouse: (u16, u16),
    inner: Rect,
) -> bool {
    let step = match direction {
        AutoscrollDirection::Up => -1,
        AutoscrollDirection::Down => 1,
    };
    let terminal = state.terminal_selection == Some(pane);
    let moved = if terminal {
        scroll_pane_lines(state, pane, step, lines)
    } else {
        // 拖选自动滚动到真实正文边界即停，不进入底部虚拟留白。
        let cap = state.prompt.text_max_scroll();
        if state.prompt.scroll() >= cap {
            false
        } else {
            scroll_prompt_selection(state, step, lines);
            if state.prompt.scroll() > cap {
                state.prompt.scroll_to(cap);
            }
            true
        }
    };
    if !moved {
        return false;
    }
    let row = mouse.1.clamp(inner.y, inner.bottom().saturating_sub(1)) - inner.y;
    let col = mouse.0.clamp(inner.x, inner.right().saturating_sub(1)) - inner.x;
    if terminal {
        drag_terminal_selection(state, pane, row, col);
    } else {
        drag_selection(state, pane, row, col);
        place_prompt_cursor(state, row, col);
    }
    true
}

/// 该窗格上是否存在进行中的选择拖拽。
fn selection_dragging_on(state: &AppState, pane: PaneId) -> bool {
    if state.terminal_selection == Some(pane) {
        return true;
    }
    state
        .selection
        .is_some_and(|selection| selection.is_dragging() && selection.pane() == pane)
}

/// 边缘距离对应的立即滚动行数（对齐 herdr：距离 × 3，夹在 3..=15）。
fn edge_scroll_lines(distance: u16) -> isize {
    usize::from(distance).saturating_mul(3).clamp(3, 15) as isize
}

/// 滚动 prompt 视口；选区是内容行坐标，视口滚动天然不影响选区；返回视口是否移动。
pub(crate) fn scroll_prompt_selection(
    state: &mut AppState,
    direction: isize,
    lines: isize,
) -> bool {
    let before = state.prompt.scroll();
    state.prompt.scroll_by(direction.signum() * lines);
    state.prompt.scroll() != before
}

/// prompt 拖拽选区时滚轮：滚动视口并保持选区锚点，终点跟到鼠标单元格；返回视口是否移动。
pub fn wheel_prompt_selection(state: &mut AppState, direction: isize, row: u16, col: u16) -> bool {
    if !scroll_prompt_selection(state, direction, WHEEL_LINES) {
        return false;
    }
    let prompt = state.prompt.id();
    drag_selection(state, prompt, row, col);
    place_prompt_cursor(state, row, col);
    true
}

/// prompt 选区文本（保留选区，供 Ctrl/Cmd+C 复制）；空选区返回 None。
pub fn prompt_selection_text(state: &AppState) -> Option<String> {
    let (start, end) = prompt_selection_range(state)?;
    state.prompt.selection_text(start, end)
}

/// prompt 复制文本：只复制真实选区；无选区返回 None（不再回退全文）。
pub fn prompt_copy_text(state: &AppState) -> Option<String> {
    prompt_selection_text(state)
}

/// prompt 选区对应的内容行列范围；选区不在 prompt 或为空时返回 None。
pub(crate) fn prompt_selection_range(state: &AppState) -> Option<((u16, u16), (u16, u16))> {
    let selection = state.selection?;
    if selection.pane() != state.prompt.id() {
        return None;
    }
    let (start, end) = selection.range()?;
    let content_row = |row: i32| u16::try_from(row).ok();
    Some((
        (content_row(start.0)?, start.1),
        (content_row(end.0)?, end.1),
    ))
}

/// prompt 选区对应的字节范围；选区不在 prompt 或为空时返回 None。
pub(crate) fn prompt_selection_bounds(state: &AppState) -> Option<(usize, usize)> {
    let (start, end) = prompt_selection_range(state)?;
    state.prompt.selection_bounds(start, end)
}

mod autoscroll;
pub(crate) use autoscroll::tick_lx_selection_autoscroll;
pub use autoscroll::{
    apply_pending_screen_selection, arm_lx_selection_autoscroll, wheel_lx_selection,
};

#[cfg(test)]
mod tests;

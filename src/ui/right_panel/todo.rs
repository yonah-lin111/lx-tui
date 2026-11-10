//! 右侧容器的 Todo 日程面板体：工具栏按钮、一周快速切日感知带、历史顺延横幅、
//! 带优先级徽标与聚合树连接符的三态列表行、行内多行编辑、滚动条与底部多图表统计。
//!
//! 绘制与命中共用同一组几何函数（项目既有不变量）；本模块只读状态，不改状态、不做 IO。

pub mod charts;
mod item;
pub mod schedule;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

pub use charts::{ChartSwitchHit, chart_switch_hit_at};
pub use schedule::{
    RolloverHit, WeekStripHit, rollover_banner_rect, rollover_hit_at, week_strip_hit_at,
    week_strip_rect,
};

use crate::app::state::AppState;
use crate::app::tooltip::TodoButton;
use crate::db::todo::TodoRecord;
use crate::ui::text;

use super::prompt;
use crate::ui::{style, widgets};

/// 工具栏按钮宽度（列）；与 `text::TODO_*_LABEL` 的显示宽度一一对应。
const NEW_WIDTH: u16 = 3;
const SORT_WIDTH: u16 = 6;
/// 按钮之间的空档（列）。
const BUTTON_GAP: u16 = 1;
/// 行首选中标记槽宽（列）：`❯` 与未选中的空格共用同一列，保证文本左对齐。
pub(super) const MARKER_WIDTH: u16 = 1;
/// 状态框宽度（列）：`[ ]` / `[~]` / `[x]` 均为 3。
pub(super) const STATUS_WIDTH: u16 = 3;
/// 聚合组树形连接符槽宽（列）：`"-┌─ "` / `" ┌─ "` / `"-├─ "` / `" ├─ "` / `" └─ "` / `" │  "` 均为 4。
pub(super) const GROUP_BRANCH_WIDTH: u16 = 4;
/// 优先级徽标宽度（列）：`P0` / `P1` / `P2` / `P3` 均为 2。
pub(super) const PRIORITY_WIDTH: u16 = 2;
/// 单个待办条目最多占用的视觉行数；超出时在末行用 `…` 截断。
pub const MAX_TODO_ITEM_ROWS: usize = 3;
/// 主体区低于此行数时不上下两分：列表独占，统计块整块隐藏。
const MIN_SPLIT_BODY: u16 = 7;
/// 统计块的最少行数：摘要 1 行 + 柱区 1 行 + 轴标签 1 行。
const MIN_STATS_ROWS: u16 = 3;
/// 柱状图每根柱子的宽度（列）；槽位更窄时取槽宽。
const BAR_WIDTH: u16 = 3;

/// 工具栏左侧组：新建（`[+]`，贴近左边框）。
const LEFT_BUTTONS: [(TodoButton, u16); 1] = [(TodoButton::New, NEW_WIDTH)];
/// 工具栏右侧组：智能排序。
const RIGHT_BUTTONS: [(TodoButton, u16); 1] = [(TodoButton::Sort, SORT_WIDTH)];
/// 全部按钮（命中与文案查表用）。
const BUTTONS: [(TodoButton, u16); 2] =
    [(TodoButton::New, NEW_WIDTH), (TodoButton::Sort, SORT_WIDTH)];

/// 主体区：表头两行之下的整块（含最右 1 列滚动条槽）。
fn body_rect(panel: Rect) -> Rect {
    super::geometry::body_rect(crate::tiling::pane_inner_rect(panel))
}

/// 统计块高度：扣掉顶部 1 行周感知带后，与列表**平分**剩余主体区（分割线占 1 行，取整余数留给列表）。
///
/// 返回 0 表示不分块：主体区太矮时列表独占，避免统计块把列表挤到 1 行。
fn stats_height(panel: Rect) -> u16 {
    let body = body_rect(panel).height;
    if body < MIN_SPLIT_BODY {
        return 0;
    }
    let strip = u16::from(week_strip_rect(panel).is_some());
    let rem = body.saturating_sub(strip);
    (rem.saturating_sub(1)) / 2
}

/// 列表区矩形：主体区扣掉顶部周感知带、分割线与统计块，且让出最右 1 列滚动条槽。
pub fn list_rect(panel: Rect) -> Rect {
    let body = body_rect(panel);
    let strip = u16::from(week_strip_rect(panel).is_some());
    let stats = stats_height(panel);
    let rem = body.height.saturating_sub(strip);
    let height = if stats == 0 {
        rem
    } else {
        rem.saturating_sub(stats).saturating_sub(1)
    };
    Rect {
        y: body.y.saturating_add(strip),
        width: body.width.saturating_sub(1),
        height,
        ..body
    }
}

/// 实际承载待办行的列表矩形：若顺延横幅可见则扣掉首行。
pub fn effective_list_rect(panel: Rect, state: &AppState) -> Rect {
    let base = list_rect(panel);
    if rollover_banner_rect(panel, state).is_some() {
        Rect {
            y: base.y.saturating_add(1),
            height: base.height.saturating_sub(1),
            ..base
        }
    } else {
        base
    }
}

/// 列表与统计块之间的分割线行（整宽，两端 `├` / `┤` 衔接外框）。
pub fn stats_divider_rect(panel: Rect) -> Option<Rect> {
    let body = body_rect(panel);
    (stats_height(panel) > 0).then(|| Rect::new(body.x, list_rect(panel).bottom(), body.width, 1))
}

/// 统计块矩形（整宽，位于分割线之下、贴主体区底边）。
pub fn stats_rect(panel: Rect) -> Option<Rect> {
    let body = body_rect(panel);
    let stats = stats_height(panel);
    (stats > 0).then(|| {
        Rect::new(
            body.x,
            body.bottom().saturating_sub(stats),
            body.width,
            stats,
        )
    })
}

/// 可见行数（基础几何）。
pub fn visible_rows(panel: Rect) -> usize {
    usize::from(list_rect(panel).height)
}

/// 当前状态下的实际可见列表行数（扣掉顺延横幅占用的 1 行）。
pub fn visible_rows_for(panel: Rect, state: &AppState) -> usize {
    if rollover_banner_rect(panel, state).is_some() {
        visible_rows(panel).saturating_sub(1)
    } else {
        visible_rows(panel)
    }
}

/// 左侧组占用宽度（`[+]` 恒显示，永不参与丢弃）。
fn left_width() -> u16 {
    LEFT_BUTTONS
        .iter()
        .enumerate()
        .fold(0u16, |acc, (index, (_, width))| {
            acc.saturating_add(if index == 0 { 0 } else { BUTTON_GAP })
                .saturating_add(*width)
        })
}

/// 右侧组放得下的按钮数（宽度阶梯：从右缘往左丢弃）。
fn right_button_count(panel: Rect) -> usize {
    let width = crate::tiling::pane_inner_rect(panel).width;
    let mut total = left_width().saturating_add(BUTTON_GAP);
    let mut count = 0;
    for (index, (_, button_width)) in RIGHT_BUTTONS.iter().enumerate() {
        let next = total
            .saturating_add(if index == 0 { 0 } else { BUTTON_GAP })
            .saturating_add(*button_width);
        if next > width {
            break;
        }
        total = next;
        count += 1;
    }
    count
}

/// 工具栏按钮矩形；放不下时返回 None。
pub fn button_rect(panel: Rect, button: TodoButton) -> Option<Rect> {
    let bar = super::geometry::toolbar_rect(panel)?;
    if LEFT_BUTTONS.iter().any(|(kind, _)| *kind == button) {
        let width = LEFT_BUTTONS[0].1;
        let rect = Rect::new(bar.x, bar.y, width, 1);
        return (rect.right() <= bar.right()).then_some(rect);
    }
    let count = right_button_count(panel);
    if count == 0 {
        return None;
    }
    let visible = &RIGHT_BUTTONS[..count.min(RIGHT_BUTTONS.len())];
    let total = visible
        .iter()
        .enumerate()
        .fold(0u16, |acc, (index, (_, width))| {
            acc.saturating_add(if index == 0 { 0 } else { BUTTON_GAP })
                .saturating_add(*width)
        });
    let mut x = bar.right().saturating_sub(total);
    for (index, (kind, width)) in visible.iter().enumerate() {
        if index > 0 {
            x = x.saturating_add(BUTTON_GAP);
        }
        if *kind == button {
            return Some(Rect::new(x, bar.y, *width, 1));
        }
        x = x.saturating_add(*width);
    }
    None
}

/// 按钮命中。
pub fn button_at(panel: Rect, column: u16, row: u16) -> Option<TodoButton> {
    BUTTONS.iter().find_map(|(button, _)| {
        button_rect(panel, *button)
            .filter(|rect| rect.contains((column, row).into()))
            .map(|_| *button)
    })
}

/// 状态框起始 X 坐标（`list.x + 2`，宽 3 列）。
pub(super) fn row_status_x(list: Rect) -> u16 {
    list.x.saturating_add(MARKER_WIDTH).saturating_add(1)
}

/// 聚合组树形连接符起始 X 坐标（`list.x + 5`，宽 4 列）。
pub(super) fn row_branch_x(list: Rect) -> u16 {
    row_status_x(list).saturating_add(STATUS_WIDTH)
}

/// 条目优先级徽标起始 X 坐标：普通单条为 `list.x + 6`，聚合组子项为 `list.x + 9`。
pub(super) fn row_priority_x_for(list: Rect, grouped: bool) -> u16 {
    let base = row_branch_x(list);
    if grouped {
        base.saturating_add(GROUP_BRANCH_WIDTH)
    } else {
        base.saturating_add(1)
    }
}

/// 条目首行正文起始 X 坐标：普通单条为 `list.x + 9`，聚合组子项为 `list.x + 12`。
pub(super) fn row_text_x_for(list: Rect, grouped: bool) -> u16 {
    row_priority_x_for(list, grouped)
        .saturating_add(PRIORITY_WIDTH)
        .saturating_add(1)
}

/// 条目折行续行正文起始 X 坐标（对齐本条 `P0..P3` 徽标）：普通单条为 `list.x + 6`，聚合组子项为 `list.x + 9`。
pub(super) fn row_cont_x_for(list: Rect, grouped: bool) -> u16 {
    row_priority_x_for(list, grouped)
}

/// 条目第 `offset` 个可见行的正文起始 X 坐标。
pub(super) fn row_line_x_for(list: Rect, grouped: bool, offset: usize) -> u16 {
    if offset == 0 {
        row_text_x_for(list, grouped)
    } else {
        row_cont_x_for(list, grouped)
    }
}

/// 条目首行正文可用显示列数（至少 1）。
pub(super) fn text_width_for(list: Rect, grouped: bool) -> usize {
    usize::from(list.right().saturating_sub(row_text_x_for(list, grouped))).max(1)
}

/// 条目折行续行正文可用显示列数（至少 1）。
pub(super) fn cont_width_for(list: Rect, grouped: bool) -> usize {
    usize::from(list.right().saturating_sub(row_cont_x_for(list, grouped))).max(1)
}

/// 普通单条优先级徽标起始 X 坐标（`list.x + 6`）。
#[cfg(test)]
fn row_priority_x(list: Rect) -> u16 {
    row_priority_x_for(list, false)
}

/// 普通单条首行正文起始 X 坐标（`list.x + 9`）。
#[cfg(test)]
fn row_text_x(list: Rect) -> u16 {
    row_text_x_for(list, false)
}

/// 可见列表项的纯几何：渲染、`row_at`、`status_at`、`priority_at` 与 `edit_input_char_at` 共用。
#[derive(Debug, Clone)]
struct VisibleList {
    list: Rect,
    items: Vec<VisibleItem>,
}

/// 单个可见条目：显示下标、首行 Y、按列表底边裁剪后的绘制行数与显示层聚合角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibleItem {
    display: usize,
    y: u16,
    rendered_rows: u16,
    role: item::DisplayGroupRole,
}

impl VisibleList {
    /// 屏幕行命中的条目（点击条目的任意视觉行都返回同一个条目）。
    fn item_at(&self, row: u16) -> Option<VisibleItem> {
        self.items
            .iter()
            .copied()
            .find(|item| row >= item.y && row < item.y.saturating_add(item.rendered_rows))
    }
}

/// 计算当前面板宽度下的 Todo 列表视口度量：
/// `(列表可见物理行数, 各显示条目视觉行高, 编辑视口起始行, 编辑框首行可用列宽, 编辑框续行可用列宽)`。
///
/// 供 `resolve_todo_viewport` 与 `scrollbar` 复用同一份多行折行高度与最大滚动偏移。
pub fn viewport_metrics(
    panel: Rect,
    state: &AppState,
) -> (usize, Vec<u16>, Option<usize>, usize, usize) {
    let list = effective_list_rect(panel, state);
    let rows = visible_rows_for(panel, state);
    let ghost = state
        .todo
        .editing()
        .filter(|edit| edit.target.is_none())
        .map(|edit| edit.at);
    let total = state.todo.display_len();
    let heights = (0..total)
        .map(|display| {
            let role = item::display_group_role(state, display, ghost, total);
            let grouped = role.is_grouped();
            item::item_rows(
                state,
                display,
                ghost,
                text_width_for(list, grouped),
                cont_width_for(list, grouped),
            )
        })
        .collect();
    let edit_grouped = state
        .todo
        .editing()
        .and_then(|edit| match &edit.target {
            None => Some(edit.at),
            Some(id) => state.todo.items().iter().position(|item| item.id == *id),
        })
        .is_some_and(|display| item::display_group_role(state, display, ghost, total).is_grouped());
    let edit_text_width = text_width_for(list, edit_grouped);
    let edit_cont_width = cont_width_for(list, edit_grouped);
    let edit_scroll_row = state
        .todo
        .editing()
        .map(|edit| item::edit_view(edit, edit_text_width, edit_cont_width).start_row);
    (
        rows,
        heights,
        edit_scroll_row,
        edit_text_width,
        edit_cont_width,
    )
}

/// 根据可见物理行数与各显示条目的视觉行高，计算能完整露出末尾条目的最大起始条目下标。
fn max_scroll_for(rows: usize, item_rows: &[u16]) -> usize {
    let total = item_rows.len();
    if total == 0 {
        return 0;
    }
    if rows == 0 {
        return total.saturating_sub(1);
    }
    let mut used = 0usize;
    for index in (0..total).rev() {
        let height = usize::from(item_rows[index]).max(1);
        if used + height > rows {
            return if index == total - 1 {
                total - 1
            } else {
                index + 1
            };
        }
        used += height;
    }
    0
}

/// 从 `state.todo.scroll()` 起按每条目的实际视觉行高向下排布，直到列表底边。
///
/// 只读状态：无 IO、无副作用；渲染与命中共用同一份几何，避免「看到」与「点到」错位。
fn layout_visible_items(panel: Rect, state: &AppState) -> VisibleList {
    let list = effective_list_rect(panel, state);
    let ghost = state
        .todo
        .editing()
        .filter(|edit| edit.target.is_none())
        .map(|edit| edit.at);
    let total = state.todo.len() + usize::from(ghost.is_some());
    let mut items = Vec::new();
    let mut y = list.y;
    while y < list.bottom() {
        let display = state.todo.scroll().saturating_add(items.len());
        if display >= total {
            break;
        }
        let role = item::display_group_role(state, display, ghost, total);
        let grouped = role.is_grouped();
        let height = item::item_rows(
            state,
            display,
            ghost,
            text_width_for(list, grouped),
            cont_width_for(list, grouped),
        );
        let rendered_rows = height.min(list.bottom().saturating_sub(y));
        items.push(VisibleItem {
            display,
            y,
            rendered_rows,
            role,
        });
        y = y.saturating_add(height);
    }
    VisibleList { list, items }
}

/// 显示下标 → 既有条目（幽灵行返回 None）。
pub(super) fn display_record(state: &AppState, display: usize) -> Option<&TodoRecord> {
    let index = list_index(state, display)?;
    state.todo.items().get(index)
}

/// 行命中的**显示**下标（含幽灵行槽位）；落在列表区之外或越界时为 None。
pub fn row_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let layout = layout_visible_items(panel, state);
    if !layout.list.contains((column, row).into()) {
        return None;
    }
    layout.item_at(row).map(|item| item.display)
}

/// 命中某行的状态框 `[ ]` 区域（返回显示下标；非状态锚点子项或续行不命中）。
///
/// 只有状态锚点子项（独立单条或聚合组 `offset == (count - 1) / 2`）的首行承载状态框。
pub fn status_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let visible = layout_visible_items(panel, state);
    let sx = row_status_x(visible.list);
    if column < sx || column >= sx.saturating_add(STATUS_WIDTH) {
        return None;
    }
    let item = visible.item_at(row)?;
    if row != item.y || !item.role.is_status_anchor() {
        return None;
    }
    Some(item.display)
}

/// 命中某行的优先级徽标区域（返回显示下标；幽灵行不命中）。
///
/// 只有条目的**首行**承载徽标：单条命中 `list.x + 6..8`，聚合组子项命中 `list.x + 9..11`。
pub fn priority_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let layout = layout_visible_items(panel, state);
    if !layout.list.contains((column, row).into()) {
        return None;
    }
    let item = layout.item_at(row)?;
    list_index(state, item.display)?;
    let px = row_priority_x_for(layout.list, item.role.is_grouped());
    (row == item.y && column >= px && column < px.saturating_add(PRIORITY_WIDTH))
        .then_some(item.display)
}

/// 显示下标 → 列表下标；落在幽灵行上返回 None（它不是任何条目）。
pub fn list_index(state: &AppState, display: usize) -> Option<usize> {
    let at = state
        .todo
        .editing()
        .filter(|edit| edit.target.is_none())
        .map(|edit| edit.at);
    match at {
        Some(at) if display == at => None,
        Some(at) if display > at => (display - 1 < state.todo.len()).then_some(display - 1),
        _ => (display < state.todo.len()).then_some(display),
    }
}

/// 若鼠标落在正在编辑的 Todo 文本输入框内，返回其字符索引。
///
/// 命中编辑条目的任意可见行，按行偏移与列偏移换算回全文字符索引（支持多行点击/拖拽）。
pub fn edit_input_char_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let edit = state.todo.editing()?;
    let display = match &edit.target {
        None => edit.at,
        Some(id) => state.todo.items().iter().position(|item| item.id == *id)?,
    };
    let layout = layout_visible_items(panel, state);
    let item = layout.items.iter().find(|item| item.display == display)?;
    if row < item.y || row >= item.y.saturating_add(item.rendered_rows) {
        return None;
    }
    let grouped = item.role.is_grouped();
    let visual_row_offset = usize::from(row - item.y);
    let line_x = row_line_x_for(layout.list, grouped, visual_row_offset);
    if column < line_x || column >= layout.list.right() {
        return None;
    }
    Some(widgets::input::multiline_char_index_at_scrolled_indented(
        edit.input.text(),
        edit.input.cursor(),
        text_width_for(layout.list, grouped),
        cont_width_for(layout.list, grouped),
        MAX_TODO_ITEM_ROWS,
        edit.scroll_row,
        visual_row_offset,
        column - line_x,
    ))
}

/// 拖拽选区时将鼠标坐标钳制到正在编辑的 Todo 文本区边界并换算为全文字符索引。
///
/// 行号钳制到该编辑项的可见行范围 `[item.y, item.y + item.rendered_rows - 1]`，
/// 列号落入该行左侧缩进区（单条 9/6 列、聚合组子项 12/9 列）时钳为 `0`（行首），越过列表右缘时钳为 `u16::MAX`（行尾）。
pub fn edit_input_drag_char_at(
    panel: Rect,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<usize> {
    let edit = state.todo.editing()?;
    let display = match &edit.target {
        None => edit.at,
        Some(id) => state.todo.items().iter().position(|item| item.id == *id)?,
    };
    let layout = layout_visible_items(panel, state);
    let item = layout.items.iter().find(|item| item.display == display)?;
    let grouped = item.role.is_grouped();
    let text_x = row_text_x_for(layout.list, grouped);
    if item.rendered_rows == 0 || layout.list.right() <= text_x {
        return None;
    }
    let clamped_row = row.clamp(
        item.y,
        item.y.saturating_add(item.rendered_rows.saturating_sub(1)),
    );
    let visual_row_offset = usize::from(clamped_row - item.y);
    let line_x = row_line_x_for(layout.list, grouped, visual_row_offset);
    let rel_col = if column < line_x {
        0
    } else if column >= layout.list.right() {
        u16::MAX
    } else {
        column - line_x
    };
    Some(widgets::input::multiline_char_index_at_scrolled_indented(
        edit.input.text(),
        edit.input.cursor(),
        text_width_for(layout.list, grouped),
        cont_width_for(layout.list, grouped),
        MAX_TODO_ITEM_ROWS,
        edit.scroll_row,
        visual_row_offset,
        rel_col,
    ))
}

/// 滚动条几何（无溢出时为 None）；轨道只覆盖列表区，不含底部统计块。
pub fn scrollbar(panel: Rect, state: &AppState) -> Option<widgets::scrollbar::ScrollbarLayout> {
    if state.right_tab != crate::app::state::RightTab::Todo || state.prompt_collapsed {
        return None;
    }
    let body = body_rect(panel);
    if body.width < 2 {
        return None;
    }
    let list = effective_list_rect(panel, state);
    let slot = Rect::new(list.right(), list.y, 1, list.height);
    let (visible, item_rows, _, _, _) = viewport_metrics(panel, state);
    let max_scroll = max_scroll_for(visible, &item_rows);
    let total = if max_scroll > 0 {
        visible.saturating_add(max_scroll)
    } else {
        0
    };
    widgets::scrollbar::layout(slot, total, visible, state.todo.scroll())
}

/// 绘制 Todo 面板体；有编辑现场时返回输入光标坐标（走与 prompt 相同的硬件光标链路）。
pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> Option<(u16, u16)> {
    let header = super::geometry::header_rect(area)?;
    let focused = state.todo_focused();
    prompt::render_divider(area, header, frame.buffer_mut(), focused);
    let buf = frame.buffer_mut();
    render_toolbar(area, buf, state);
    schedule::render_divider_labels(area, buf, state);
    schedule::render_week_strip(area, buf, state);
    schedule::render_rollover_banner(area, buf, state);
    let cursor = item::render_rows(area, buf, state).filter(|_| focused);
    if let Some(bar) = scrollbar(area, state) {
        widgets::scrollbar::render_buffer(buf, &bar);
    }
    charts::render_stats(area, buf, state, focused);
    cursor
}

/// 绘制工具栏按钮（无效按钮置灰）。
fn render_toolbar(panel: Rect, buf: &mut Buffer, state: &AppState) {
    for (button, _) in BUTTONS {
        let Some(rect) = button_rect(panel, button) else {
            continue;
        };
        let enabled = crate::app::update::todo_button_enabled(state, button);
        let label = match button {
            TodoButton::New => text::TODO_NEW_LABEL,
            TodoButton::Sort => text::TODO_SORT_LABEL,
        };
        let button_style = if enabled {
            style::accent()
        } else {
            style::muted()
        };
        for (offset, symbol) in label.chars().enumerate() {
            let x = rect
                .x
                .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
            if x >= rect.right() {
                break;
            }
            if let Some(cell) = buf.cell_mut((x, rect.y)) {
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(button_style);
            }
        }
    }
}

/// 在 `row` 矩形内从 `x` 起顺序写入一段文本，返回写完后的下一列坐标。
fn write_inline(buf: &mut Buffer, row: Rect, x: u16, value: &str, cell_style: Style) -> u16 {
    if x >= row.right() {
        return x;
    }
    let available = usize::from(row.right().saturating_sub(x));
    let (end_x, _) = buf.set_stringn(x, row.y, value, available, cell_style);
    end_x
}

/// 写入单个单元格（越界忽略）。
fn write_cell(buf: &mut Buffer, x: u16, y: u16, symbol: &str, cell_style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(symbol);
        cell.set_style(cell_style);
    }
}

/// 写入一段文本，按显示宽度截断到 `list` 右缘。
fn write_string_in(buf: &mut Buffer, list: Rect, x: u16, y: u16, value: &str, cell_style: Style) {
    if x >= list.right() {
        return;
    }
    let available = usize::from(list.right().saturating_sub(x));
    buf.set_stringn(x, y, value, available, cell_style);
}

/// 编辑行首行剩余宽度（测试与诊断用）。
#[cfg(test)]
pub(crate) fn edit_row_width(panel: Rect) -> usize {
    let list = list_rect(panel);
    let text_x = row_text_x(list);
    usize::from(list.right().saturating_sub(text_x))
}

/// 编辑行续行剩余宽度（从 `P(n)` 左边缘起，测试与诊断用）。
#[cfg(test)]
pub(crate) fn edit_cont_row_width(panel: Rect) -> usize {
    let list = list_rect(panel);
    let cont_x = row_priority_x(list);
    usize::from(list.right().saturating_sub(cont_x))
}

#[cfg(test)]
mod tests;

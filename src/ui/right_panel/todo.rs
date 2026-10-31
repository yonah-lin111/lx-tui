//! 右侧容器的 Todo 日程面板体：工具栏按钮、一周快速切日感知带、历史顺延横幅、
//! 带优先级徽标的三态列表行、行内单行编辑、滚动条与底部多图表统计。
//!
//! 绘制与命中共用同一组几何函数（项目既有不变量）；本模块只读状态，不改状态、不做 IO。

pub mod charts;
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

use crate::app::state::{AppState, TodoEdit};
use crate::app::tooltip::TodoButton;
use crate::db::todo::{TodoPriority, TodoRecord, TodoStatus};
use crate::ui::text;

use super::super::{prompt, style, widgets};

/// 工具栏按钮宽度（列）；与 `text::TODO_*_LABEL` 的显示宽度一一对应。
const NEW_WIDTH: u16 = 3;
const EDIT_WIDTH: u16 = 6;
const DELETE_WIDTH: u16 = 5;
const SORT_WIDTH: u16 = 6;
const CLEAR_DONE_WIDTH: u16 = 12;
/// 按钮之间的空档（列）。
const BUTTON_GAP: u16 = 1;
/// 行首选中标记槽宽（列）：`❯` 与未选中的空格共用同一列，保证文本左对齐。
const MARKER_WIDTH: u16 = 1;
/// 状态框宽度（列）：`[ ]` / `[~]` / `[x]` 均为 3。
const STATUS_WIDTH: u16 = 3;
/// 优先级徽标宽度（列）：`P0` / `P1` / `P2` / `P3` 均为 2。
const PRIORITY_WIDTH: u16 = 2;
/// 主体区低于此行数时不上下两分：列表独占，统计块整块隐藏。
const MIN_SPLIT_BODY: u16 = 7;
/// 统计块的最少行数：摘要 1 行 + 柱区 1 行 + 轴标签 1 行。
const MIN_STATS_ROWS: u16 = 3;
/// 柱状图每根柱子的宽度（列）；槽位更窄时取槽宽。
const BAR_WIDTH: u16 = 3;

/// 工具栏左侧组：新建（`[+]`，贴近左边框）。
const LEFT_BUTTONS: [(TodoButton, u16); 1] = [(TodoButton::New, NEW_WIDTH)];
/// 工具栏右侧组：条目操作，从右缘往左丢弃（先丢 `[clear done]`、再丢 `[sort]`、`[del]`、`[edit]`）。
const RIGHT_BUTTONS: [(TodoButton, u16); 4] = [
    (TodoButton::Edit, EDIT_WIDTH),
    (TodoButton::Delete, DELETE_WIDTH),
    (TodoButton::Sort, SORT_WIDTH),
    (TodoButton::ClearDone, CLEAR_DONE_WIDTH),
];
/// 全部按钮（命中与文案查表用）。
const BUTTONS: [(TodoButton, u16); 5] = [
    (TodoButton::New, NEW_WIDTH),
    (TodoButton::Edit, EDIT_WIDTH),
    (TodoButton::Delete, DELETE_WIDTH),
    (TodoButton::Sort, SORT_WIDTH),
    (TodoButton::ClearDone, CLEAR_DONE_WIDTH),
];

/// 主体区：表头两行之下的整块（含最右 1 列滚动条槽）。
fn body_rect(panel: Rect) -> Rect {
    crate::layout::prompt_body_rect(crate::layout::pane_inner_rect(panel))
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
    let width = crate::layout::pane_inner_rect(panel).width;
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
    let bar = crate::layout::prompt_toolbar_rect(panel)?;
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

/// 行内优先级徽标的起始 X 坐标。
fn row_priority_x(list: Rect) -> u16 {
    list.x
        .saturating_add(MARKER_WIDTH)
        .saturating_add(1)
        .saturating_add(STATUS_WIDTH)
        .saturating_add(1)
}

/// 行内正文的起始 X 坐标（含优先级徽标及其后 1 列间隔）。
fn row_text_x(list: Rect) -> u16 {
    row_priority_x(list)
        .saturating_add(PRIORITY_WIDTH)
        .saturating_add(1)
}

/// 行命中的**显示**下标（含幽灵行槽位）；落在列表区之外或越界时为 None。
pub fn row_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let list = effective_list_rect(panel, state);
    if !list.contains((column, row).into()) {
        return None;
    }
    let offset = usize::from(row.saturating_sub(list.y));
    let index = state.todo.scroll().saturating_add(offset);
    let ghost = state
        .todo
        .editing()
        .is_some_and(|edit| edit.target.is_none());
    let total = state.todo.len() + usize::from(ghost);
    (index < total).then_some(index)
}

/// 命中某行的优先级徽标区域（返回显示下标；幽灵行不命中）。
pub fn priority_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let display = row_at(panel, state, column, row)?;
    let _record_index = list_index(state, display)?;
    let list = effective_list_rect(panel, state);
    let px = row_priority_x(list);
    (column >= px && column < px.saturating_add(PRIORITY_WIDTH)).then_some(display)
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
pub fn edit_input_char_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let edit = state.todo.editing()?;
    let list = effective_list_rect(panel, state);
    if !list.contains((column, row).into()) {
        return None;
    }
    let display_idx = match &edit.target {
        None => edit.at,
        Some(id) => state.todo.items().iter().position(|item| item.id == *id)?,
    };
    let scroll = state.todo.scroll();
    if display_idx < scroll {
        return None;
    }
    let offset = display_idx - scroll;
    let y = list.y.checked_add(u16::try_from(offset).ok()?)?;
    if row != y {
        return None;
    }
    let text_x = row_text_x(list);
    if column < text_x || column >= list.right() {
        return None;
    }
    let width = usize::from(list.right().saturating_sub(text_x));
    let rel_col = column.saturating_sub(text_x);
    Some(widgets::input::char_index_at_col(
        edit.input.text(),
        edit.input.cursor(),
        width,
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
    widgets::scrollbar::layout(
        slot,
        state.todo.len(),
        visible_rows_for(panel, state),
        state.todo.scroll(),
    )
}

/// 状态框文案。
fn status_label(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Todo => text::TODO_STATUS_TODO,
        TodoStatus::InProgress => text::TODO_STATUS_IN_PROGRESS,
        TodoStatus::Done => text::TODO_STATUS_DONE,
    }
}

/// 状态框样式。
fn status_style(status: TodoStatus) -> Style {
    match status {
        TodoStatus::Todo => style::todo_status_open(),
        TodoStatus::InProgress => style::todo_status_active(),
        TodoStatus::Done => style::todo_status_done(),
    }
}

/// 行文本样式。
fn text_style(status: TodoStatus) -> Style {
    match status {
        TodoStatus::Todo => style::text(),
        TodoStatus::InProgress => style::todo_status_active(),
        TodoStatus::Done => style::todo_status_done(),
    }
}

/// 绘制 Todo 面板体；有编辑现场时返回输入光标坐标（走与 prompt 相同的硬件光标链路）。
pub(crate) fn render(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> Option<(u16, u16)> {
    let header = crate::layout::prompt_header_rect(area)?;
    let focused = state.todo_focused();
    prompt::render_divider(area, header, frame.buffer_mut(), focused);
    let buf = frame.buffer_mut();
    render_toolbar(area, buf, state);
    schedule::render_divider_labels(area, buf, state);
    schedule::render_week_strip(area, buf, state);
    schedule::render_rollover_banner(area, buf, state);
    let cursor = render_rows(area, buf, state).filter(|_| focused);
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
            TodoButton::Edit => text::TODO_EDIT_LABEL,
            TodoButton::Delete => text::TODO_DELETE_LABEL,
            TodoButton::Sort => text::TODO_SORT_LABEL,
            TodoButton::ClearDone => text::TODO_CLEAR_DONE_LABEL,
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

/// 绘制可见列表行；返回编辑光标坐标。
fn render_rows(panel: Rect, buf: &mut Buffer, state: &AppState) -> Option<(u16, u16)> {
    let list = effective_list_rect(panel, state);
    if list.width == 0 || list.height == 0 {
        return None;
    }
    buf.set_style(list, Style::reset());
    let pending_new = state
        .todo
        .editing()
        .filter(|edit| edit.target.is_none())
        .map(|edit| edit.at);
    if state.todo.is_empty() && pending_new.is_none() {
        return render_empty(list, buf);
    }
    let visible = usize::from(list.height);
    let start = state.todo.scroll();
    let total = state.todo.len() + usize::from(pending_new.is_some());
    let mut cursor = None;
    for (slot, index) in (start..start.saturating_add(visible)).enumerate() {
        if index >= total {
            break;
        }
        let y = list.y.saturating_add(slot as u16);
        let row_selected = state.todo_focused() && state.todo.selected() == Some(index);
        if pending_new == Some(index) {
            cursor = cursor.or(render_row(
                buf,
                list,
                y,
                RowView {
                    status: TodoStatus::Todo,
                    priority: TodoPriority::P1,
                    text: "",
                    selected: false,
                    marker: state.todo_focused(),
                    edit: state.todo.editing(),
                },
            ));
            continue;
        }
        let record_index = if pending_new.is_some_and(|at| index > at) {
            index - 1
        } else {
            index
        };
        let Some(record) = state.todo.items().get(record_index) else {
            break;
        };
        let hovered = state.todo_hover == Some(record_index);
        cursor = cursor.or(render_row(
            buf,
            list,
            y,
            RowView {
                status: record.status,
                priority: record.priority,
                text: &record.text,
                selected: row_selected || hovered,
                marker: state.todo.selected() == Some(record_index),
                edit: editing_row(state, record),
            },
        ));
    }
    cursor
}

/// 一行的绘制数据。
struct RowView<'a> {
    status: TodoStatus,
    priority: TodoPriority,
    text: &'a str,
    selected: bool,
    marker: bool,
    edit: Option<&'a TodoEdit>,
}

/// 绘制一整行：标记 + 状态框 + 优先级徽标 + 文本（或编辑输入）；返回编辑光标坐标。
fn render_row(buf: &mut Buffer, list: Rect, y: u16, row: RowView<'_>) -> Option<(u16, u16)> {
    if row.selected {
        buf.set_style(Rect::new(list.x, y, list.width, 1), style::selection());
    }
    let marker = if row.marker {
        text::AGENT_SELECTED_MARKER
    } else {
        " "
    };
    write_cell(
        buf,
        list.x,
        y,
        marker,
        if row.selected {
            style::selection()
        } else {
            Style::reset()
        },
    );
    let status_x = list.x.saturating_add(MARKER_WIDTH).saturating_add(1);
    write_string_in(
        buf,
        list,
        status_x,
        y,
        status_label(row.status),
        status_style(row.status),
    );
    let priority_x = row_priority_x(list);
    render_priority_badge(buf, list, priority_x, y, row.priority, row.status);
    let text_x = row_text_x(list);
    match row.edit {
        Some(edit) => render_edit_row(buf, list, text_x, y, edit),
        None => {
            write_string_in(buf, list, text_x, y, row.text, text_style(row.status));
            None
        }
    }
}

/// 绘制优先级徽标：未完成条目每格独立重置后写入专属底色与前景色，确保行选中/悬停底色不污染徽标。
fn render_priority_badge(
    buf: &mut Buffer,
    list: Rect,
    x: u16,
    y: u16,
    priority: TodoPriority,
    status: TodoStatus,
) {
    let badge_style = style::todo_priority_badge(priority, status);
    for (offset, ch) in priority.as_str().chars().enumerate() {
        let cell_x = x.saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        if cell_x >= list.right() {
            break;
        }
        if let Some(cell) = buf.cell_mut((cell_x, y)) {
            if status != TodoStatus::Done {
                cell.reset();
            }
            cell.set_char(ch);
            cell.set_style(badge_style);
        }
    }
}

/// 空列表：首行居中占位文案。
fn render_empty(list: Rect, buf: &mut Buffer) -> Option<(u16, u16)> {
    let width = u16::try_from(text::TODO_EMPTY.chars().count()).unwrap_or(u16::MAX);
    let x = list.x.saturating_add(list.width.saturating_sub(width) / 2);
    buf.set_stringn(
        x,
        list.y,
        text::TODO_EMPTY,
        usize::from(list.width),
        style::muted(),
    );
    None
}

/// 该行是否为编辑现场（仅既有条目；新建走上面的幽灵行）。
fn editing_row<'a>(state: &'a AppState, record: &TodoRecord) -> Option<&'a TodoEdit> {
    let edit = state.todo.editing()?;
    (edit.target.as_deref() == Some(record.id.as_str())).then_some(edit)
}

/// 绘制编辑行：与页重命名/浮层输入共用 `widgets::input` 的视口与光标算法。
fn render_edit_row(
    buf: &mut Buffer,
    list: Rect,
    text_x: u16,
    y: u16,
    edit: &TodoEdit,
) -> Option<(u16, u16)> {
    let width = usize::from(list.right().saturating_sub(text_x));
    if width == 0 {
        return None;
    }
    let text = edit.input.text();
    let input = widgets::input::view(text, edit.input.cursor(), width);
    if text.is_empty() && edit.target.is_none() {
        write_string_in(
            buf,
            list,
            text_x,
            y,
            text::TODO_EDIT_PLACEHOLDER,
            style::muted(),
        );
    } else {
        let spans = widgets::input::build_styled_spans(
            input.visible,
            input.start_char,
            edit.input.selection(),
        );
        let mut cur_x = text_x;
        for span in spans {
            if cur_x >= list.right() {
                break;
            }
            write_string_in(buf, list, cur_x, y, &span.content, span.style);
            let span_width =
                u16::try_from(unicode_width::UnicodeWidthStr::width(span.content.as_ref()))
                    .unwrap_or(0);
            cur_x = cur_x.saturating_add(span_width);
        }
    }
    Some((
        text_x
            .saturating_add(input.cursor_col)
            .min(list.right().saturating_sub(1)),
        y,
    ))
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

/// 编辑行剩余宽度（测试与诊断用）。
#[cfg(test)]
pub(crate) fn edit_row_width(panel: Rect) -> usize {
    let list = list_rect(panel);
    let text_x = row_text_x(list);
    usize::from(list.right().saturating_sub(text_x))
}

#[cfg(test)]
mod tests;

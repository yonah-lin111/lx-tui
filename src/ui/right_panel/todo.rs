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
const SORT_WIDTH: u16 = 6;
/// 按钮之间的空档（列）。
const BUTTON_GAP: u16 = 1;
/// 行首选中标记槽宽（列）：`❯` 与未选中的空格共用同一列，保证文本左对齐。
const MARKER_WIDTH: u16 = 1;
/// 状态框宽度（列）：`[ ]` / `[~]` / `[x]` 均为 3。
const STATUS_WIDTH: u16 = 3;
/// 优先级徽标宽度（列）：`P0` / `P1` / `P2` / `P3` 均为 2。
const PRIORITY_WIDTH: u16 = 2;
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

/// 可见列表项的纯几何：渲染、`row_at`、`priority_at` 与 `edit_input_char_at` 共用。
#[derive(Debug, Clone)]
struct VisibleList {
    list: Rect,
    /// 正文起始列：续行同样从这里开始，左侧 9 列留空。
    text_x: u16,
    /// 正文可用显示列数（至少 1，避免折行退化为死循环）。
    text_width: usize,
    items: Vec<VisibleItem>,
}

/// 单个可见条目：显示下标、首行 Y 与按列表底边裁剪后的绘制行数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VisibleItem {
    display: usize,
    y: u16,
    rendered_rows: u16,
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

/// 从 `state.todo.scroll()` 起按每条目的实际视觉行高向下排布，直到列表底边。
///
/// 只读状态：无 IO、无副作用；渲染与命中共用同一份几何，避免「看到」与「点到」错位。
fn layout_visible_items(panel: Rect, state: &AppState) -> VisibleList {
    let list = effective_list_rect(panel, state);
    let text_x = row_text_x(list);
    let text_width = usize::from(list.right().saturating_sub(text_x)).max(1);
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
        let height = item_rows(state, display, ghost, text_width);
        let rendered_rows = height.min(list.bottom().saturating_sub(y));
        items.push(VisibleItem {
            display,
            y,
            rendered_rows,
        });
        y = y.saturating_add(height);
    }
    VisibleList {
        list,
        text_x,
        text_width,
        items,
    }
}

/// 单个显示条目占用的视觉行数（`1..=MAX_TODO_ITEM_ROWS`）。
fn item_rows(state: &AppState, display: usize, ghost: Option<usize>, text_width: usize) -> u16 {
    if ghost == Some(display) {
        return state
            .todo
            .editing()
            .map_or(1, |edit| edit_rows(edit, text_width));
    }
    let Some(record) = display_record(state, display) else {
        return 1;
    };
    if let Some(edit) = editing_row(state, record) {
        return edit_rows(edit, text_width);
    }
    let lines = widgets::input::wrap_clamped_lines(&record.text, text_width, MAX_TODO_ITEM_ROWS);
    rows_of(lines.len())
}

/// 编辑态的视觉行数：空的新建幽灵行按 1 行占位，其余跟随软折行（上限 3 行）。
fn edit_rows(edit: &TodoEdit, text_width: usize) -> u16 {
    if edit.input.text().is_empty() && edit.target.is_none() {
        return 1;
    }
    let view = widgets::input::multiline_view(
        edit.input.text(),
        edit.input.cursor(),
        text_width,
        MAX_TODO_ITEM_ROWS,
    );
    rows_of(view.visible_lines.len())
}

/// 视觉行数钳制到 `1..=MAX_TODO_ITEM_ROWS`。
fn rows_of(lines: usize) -> u16 {
    u16::try_from(lines.clamp(1, MAX_TODO_ITEM_ROWS)).unwrap_or(1)
}

/// 显示下标 → 既有条目（幽灵行返回 None）。
fn display_record(state: &AppState, display: usize) -> Option<&TodoRecord> {
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

/// 命中某行的优先级徽标区域（返回显示下标；幽灵行不命中）。
///
/// 只有条目的**首行**承载徽标：点击续行同一列不算优先级。
pub fn priority_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let layout = layout_visible_items(panel, state);
    if !layout.list.contains((column, row).into()) {
        return None;
    }
    let item = layout.item_at(row)?;
    list_index(state, item.display)?;
    let px = row_priority_x(layout.list);
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
    if column < layout.text_x || column >= layout.list.right() {
        return None;
    }
    Some(widgets::input::multiline_char_index_at(
        edit.input.text(),
        edit.input.cursor(),
        layout.text_width,
        MAX_TODO_ITEM_ROWS,
        usize::from(row - item.y),
        column - layout.text_x,
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
        state.todo.display_len(),
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

/// 绘制可见列表行；返回编辑光标坐标。
fn render_rows(panel: Rect, buf: &mut Buffer, state: &AppState) -> Option<(u16, u16)> {
    let layout = layout_visible_items(panel, state);
    let list = layout.list;
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
    let mut cursor = None;
    for item in &layout.items {
        let Some(row) = row_view(state, *item, pending_new, layout.text_width) else {
            continue;
        };
        cursor = cursor.or(render_item(buf, &layout, *item, &row));
    }
    cursor
}

/// 一个条目的绘制数据。
struct RowView<'a> {
    status: TodoStatus,
    priority: TodoPriority,
    /// 非编辑态：折行后的正文（`1..=MAX_TODO_ITEM_ROWS` 行）。
    lines: Vec<String>,
    selected: bool,
    marker: bool,
    edit: Option<&'a TodoEdit>,
}

/// 组装单个条目的绘制数据（幽灵行与既有条目同构）。
fn row_view<'a>(
    state: &'a AppState,
    item: VisibleItem,
    pending_new: Option<usize>,
    text_width: usize,
) -> Option<RowView<'a>> {
    if pending_new == Some(item.display) {
        return Some(RowView {
            status: TodoStatus::Todo,
            priority: TodoPriority::P1,
            lines: vec![String::new()],
            selected: false,
            marker: state.todo_focused(),
            edit: state.todo.editing(),
        });
    }
    let index = list_index(state, item.display)?;
    let record = state.todo.items().get(index)?;
    let row_selected = state.todo_focused() && state.todo.selected() == Some(index);
    let hovered = state.todo_hover == Some(index);
    Some(RowView {
        status: record.status,
        priority: record.priority,
        lines: widgets::input::wrap_clamped_lines(&record.text, text_width, MAX_TODO_ITEM_ROWS),
        selected: row_selected || hovered,
        marker: state.todo.selected() == Some(index),
        edit: editing_row(state, record),
    })
}

/// 绘制一个条目：整块选中底色 + 首行前缀 + 折行正文（或行内编辑）。
fn render_item(
    buf: &mut Buffer,
    layout: &VisibleList,
    item: VisibleItem,
    row: &RowView<'_>,
) -> Option<(u16, u16)> {
    let list = layout.list;
    if row.selected {
        buf.set_style(
            Rect::new(list.x, item.y, list.width, item.rendered_rows),
            style::selection(),
        );
    }
    let marker = if row.marker {
        text::AGENT_SELECTED_MARKER
    } else {
        " "
    };
    write_cell(
        buf,
        list.x,
        item.y,
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
        item.y,
        status_label(row.status),
        status_style(row.status),
    );
    render_priority_badge(
        buf,
        list,
        row_priority_x(list),
        item.y,
        row.priority,
        row.status,
    );
    match row.edit {
        Some(edit) => render_edit_item(buf, layout, item, edit),
        None => {
            render_wrapped_text(buf, layout, item, &row.lines, text_style(row.status));
            None
        }
    }
}

/// 逐行绘制折行正文：首行与续行都从 `text_x` 起，续行左侧 9 列保持空白。
fn render_wrapped_text(
    buf: &mut Buffer,
    layout: &VisibleList,
    item: VisibleItem,
    lines: &[String],
    cell_style: Style,
) {
    for (offset, line) in lines
        .iter()
        .enumerate()
        .take(usize::from(item.rendered_rows))
    {
        let y = item
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        write_string_in(buf, layout.list, layout.text_x, y, line, cell_style);
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

/// 绘制行内编辑：1~3 行软折行、续行与正文左对齐，并返回硬件光标坐标。
///
/// 与行内多行输入共用 `widgets::input::multiline_view` 的视口与光标算法。
fn render_edit_item(
    buf: &mut Buffer,
    layout: &VisibleList,
    item: VisibleItem,
    edit: &TodoEdit,
) -> Option<(u16, u16)> {
    let list = layout.list;
    let text_x = layout.text_x;
    let text = edit.input.text();
    if text.is_empty() && edit.target.is_none() {
        write_string_in(
            buf,
            list,
            text_x,
            item.y,
            text::TODO_EDIT_PLACEHOLDER,
            style::muted(),
        );
        return Some((text_x.min(list.right().saturating_sub(1)), item.y));
    }
    let view = widgets::input::multiline_view(
        text,
        edit.input.cursor(),
        layout.text_width,
        MAX_TODO_ITEM_ROWS,
    );
    for (offset, line) in view
        .visible_lines
        .iter()
        .enumerate()
        .take(usize::from(item.rendered_rows))
    {
        let y = item
            .y
            .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        let spans =
            widgets::input::build_styled_spans(line.text, line.start_char, edit.input.selection());
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
    let cursor_x = text_x
        .saturating_add(view.cursor_col)
        .min(list.right().saturating_sub(1));
    let cursor_y = item
        .y
        .saturating_add(view.cursor_row_offset)
        .min(list.bottom().saturating_sub(1));
    Some((cursor_x, cursor_y))
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

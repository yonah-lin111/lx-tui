//! Todo 列表条目渲染：独立单条与聚合组子项（共享状态框锚点、树形连接符、多行折行与行内编辑）。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::state::{AppState, TodoEdit};
use crate::db::todo::{TodoPriority, TodoRecord, TodoStatus};
use crate::ui::{style, text, widgets};

use super::{
    MAX_TODO_ITEM_ROWS, VisibleItem, cont_width_for, display_record, layout_visible_items,
    list_index, row_branch_x, row_line_x_for, row_priority_x, row_status_x, row_text_x_for,
    text_width_for, write_cell, write_string_in,
};

/// 显示层条目（含正在组内新建的幽灵行槽位）在列表中的聚合角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DisplayGroupRole {
    /// 独立普通待办（无 `group_id` 或同组可见槽位不足 2 项）。
    Single,
    /// 聚合组内子项（`count >= 2`）。
    Member {
        /// 在组内的 0 基下标（`0..count`）。
        offset: usize,
        /// 组内总槽位数（含组内新建幽灵行）。
        count: usize,
    },
}

impl DisplayGroupRole {
    /// 是否属于聚合组子项（`count >= 2`）。
    pub(super) fn is_grouped(self) -> bool {
        matches!(self, Self::Member { .. })
    }

    /// 是否承载唯一的状态框 `[ ]`（单条恒为真；聚合组仅锚定在 `offset == (count - 1) / 2`）。
    pub(super) fn is_status_anchor(self) -> bool {
        match self {
            Self::Single => true,
            Self::Member { offset, count } => offset == (count.saturating_sub(1)) / 2,
        }
    }
}

/// 解析指定显示槽位的有效 `group_id`（幽灵行取 `edit.group_id`，实体行取 `record.group_id`）。
fn display_group_id(state: &AppState, display: usize, ghost: Option<usize>) -> Option<&str> {
    if ghost == Some(display) {
        return state
            .todo
            .editing()
            .and_then(|edit| edit.group_id.as_deref());
    }
    let index = list_index(state, display)?;
    state
        .todo
        .items()
        .get(index)
        .and_then(|record| record.group_id.as_deref())
}

/// 计算显示槽位 `display` 在 `0..total` 中的聚合角色。
pub(super) fn display_group_role(
    state: &AppState,
    display: usize,
    ghost: Option<usize>,
    total: usize,
) -> DisplayGroupRole {
    let Some(gid) = display_group_id(state, display, ghost) else {
        return DisplayGroupRole::Single;
    };
    let mut start = display;
    while start > 0 && display_group_id(state, start - 1, ghost) == Some(gid) {
        start -= 1;
    }
    let mut end = display.saturating_add(1);
    while end < total && display_group_id(state, end, ghost) == Some(gid) {
        end += 1;
    }
    let count = end.saturating_sub(start);
    if count >= 2 {
        DisplayGroupRole::Member {
            offset: display.saturating_sub(start),
            count,
        }
    } else {
        DisplayGroupRole::Single
    }
}

/// 聚合组徽标的归属：组内最高优先级的显示槽位与优先级（并列取最靠前；忽略幽灵行）。
pub(super) struct GroupPriority {
    pub display: usize,
    pub priority: TodoPriority,
}

/// 计算显示槽位 `display` 所在聚合组的徽标归属；单条返回自身。
pub(super) fn group_priority(
    state: &AppState,
    display: usize,
    ghost: Option<usize>,
    total: usize,
) -> Option<GroupPriority> {
    let role = display_group_role(state, display, ghost, total);
    let DisplayGroupRole::Member { offset, count } = role else {
        let record = display_record(state, display)?;
        return Some(GroupPriority {
            display,
            priority: record.priority,
        });
    };
    let start = display.saturating_sub(offset);
    let end = start.saturating_add(count).min(total);
    let mut best: Option<GroupPriority> = None;
    for candidate in start..end {
        if ghost == Some(candidate) {
            continue;
        }
        let Some(record) = display_record(state, candidate) else {
            continue;
        };
        if best
            .as_ref()
            .is_none_or(|current| record.priority < current.priority)
        {
            best = Some(GroupPriority {
                display: candidate,
                priority: record.priority,
            });
        }
    }
    best
}

/// 聚合组子项首行与折行续行的 3 列连接符（位于 `list.x + 9..list.x + 12`）。
///
/// 锚点行的状态框与 `P(n)` 已移到连接符左侧，`─` 前导不再需要，锚点与非锚点同形。
pub(super) fn group_connector_glyph(
    offset: usize,
    count: usize,
    line_offset: usize,
) -> &'static str {
    if line_offset > 0 {
        return if offset + 1 < count { "│  " } else { "   " };
    }
    if offset == 0 {
        "┌─ "
    } else if offset + 1 < count {
        "├─ "
    } else {
        "└─ "
    }
}

/// 单个显示条目占用的视觉行数（`1..=MAX_TODO_ITEM_ROWS`）。
pub(super) fn item_rows(
    state: &AppState,
    display: usize,
    ghost: Option<usize>,
    text_width: usize,
    cont_width: usize,
) -> u16 {
    if ghost == Some(display) {
        return state
            .todo
            .editing()
            .map_or(1, |edit| edit_rows(edit, text_width, cont_width));
    }
    let Some(record) = display_record(state, display) else {
        return 1;
    };
    if let Some(edit) = editing_row(state, record) {
        return edit_rows(edit, text_width, cont_width);
    }
    let lines = widgets::input::wrap_clamped_lines_indented(
        &record.text,
        text_width,
        cont_width,
        MAX_TODO_ITEM_ROWS,
    );
    rows_of(lines.len())
}

/// 编辑态的视觉行数：空的新建幽灵行按 1 行占位，其余跟随软折行（上限 3 行）。
fn edit_rows(edit: &TodoEdit, text_width: usize, cont_width: usize) -> u16 {
    if edit.input.text().is_empty() && edit.target.is_none() {
        return 1;
    }
    let view = edit_view(edit, text_width, cont_width);
    rows_of(view.visible_lines.len())
}

/// 编辑现场的多行输入视口（复用 `edit.scroll_row` 保持滑动窗口稳定）。
pub(super) fn edit_view(
    edit: &TodoEdit,
    text_width: usize,
    cont_width: usize,
) -> widgets::input::MultilineInputView<'_> {
    widgets::input::multiline_view_scrolled_indented(
        edit.input.text(),
        edit.input.cursor(),
        text_width,
        cont_width,
        MAX_TODO_ITEM_ROWS,
        edit.scroll_row,
    )
}

/// 视觉行数钳制到 `1..=MAX_TODO_ITEM_ROWS`。
fn rows_of(lines: usize) -> u16 {
    u16::try_from(lines.clamp(1, MAX_TODO_ITEM_ROWS)).unwrap_or(1)
}

/// 该行是否为编辑现场（仅既有条目；新建走幽灵行）。
pub(super) fn editing_row<'a>(state: &'a AppState, record: &TodoRecord) -> Option<&'a TodoEdit> {
    let edit = state.todo.editing()?;
    (edit.target.as_deref() == Some(record.id.as_str())).then_some(edit)
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

/// 绘制可见列表行；返回编辑光标坐标。
pub(super) fn render_rows(panel: Rect, buf: &mut Buffer, state: &AppState) -> Option<(u16, u16)> {
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
        let grouped = item.role.is_grouped();
        let text_width = text_width_for(list, grouped);
        let cont_width = cont_width_for(list, grouped);
        let Some(row) = row_view(state, *item, pending_new, text_width, cont_width) else {
            continue;
        };
        cursor = cursor.or(render_item(buf, list, *item, &row));
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
    drag_source_dimmed: bool,
    role: DisplayGroupRole,
    edit: Option<&'a TodoEdit>,
}

/// 组装单个条目的绘制数据（幽灵行与既有条目同构）。
fn row_view<'a>(
    state: &'a AppState,
    item: VisibleItem,
    pending_new: Option<usize>,
    text_width: usize,
    cont_width: usize,
) -> Option<RowView<'a>> {
    let total = state.todo.len() + usize::from(pending_new.is_some());
    if pending_new == Some(item.display) {
        let edit = state.todo.editing();
        let status = edit.map_or(TodoStatus::Todo, |e| e.initial_status);
        let priority = if item.role.is_status_anchor() {
            group_priority(state, item.display, pending_new, total)
                .map_or(TodoPriority::P1, |badge| badge.priority)
        } else {
            TodoPriority::P1
        };
        return Some(RowView {
            status,
            priority,
            lines: vec![String::new()],
            selected: false,
            marker: state.todo_focused(),
            drag_source_dimmed: false,
            role: item.role,
            edit,
        });
    }
    let index = list_index(state, item.display)?;
    let record = state.todo.items().get(index)?;
    let active_drag = state.todo_drag.as_ref().filter(|drag| drag.dragging);
    let drag_source_dimmed = active_drag.is_some_and(|drag| drag.source_id == record.id);
    let drag_target_highlighted = active_drag
        .and_then(|drag| drag.target_index)
        .is_some_and(|target_idx| state.todo.group_range(target_idx).contains(&index));
    let row_selected =
        state.todo_focused() && state.todo.selected() == Some(index) && !drag_source_dimmed;
    let hovered = active_drag.is_none() && state.todo_hover == Some(index);
    // 聚合组只在状态锚点行绘制一次徽标，取组内最高优先级；非锚点行的值仅占位。
    let priority = if item.role.is_status_anchor() {
        group_priority(state, item.display, pending_new, total)
            .map_or(record.priority, |badge| badge.priority)
    } else {
        record.priority
    };
    Some(RowView {
        status: record.status,
        priority,
        lines: widgets::input::wrap_clamped_lines_indented(
            &record.text,
            text_width,
            cont_width,
            MAX_TODO_ITEM_ROWS,
        ),
        selected: row_selected || hovered || drag_target_highlighted,
        marker: state.todo.selected() == Some(index),
        drag_source_dimmed,
        role: item.role,
        edit: editing_row(state, record),
    })
}

/// 绘制一个条目：整块选中底色 + 首行前缀（状态锚点 + 优先级徽标 + 聚合树连接符）+ 折行正文（或行内编辑）。
fn render_item(
    buf: &mut Buffer,
    list: Rect,
    item: VisibleItem,
    row: &RowView<'_>,
) -> Option<(u16, u16)> {
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
    if row.role.is_status_anchor() {
        let s_style = if row.drag_source_dimmed {
            style::muted()
        } else {
            status_style(row.status)
        };
        write_string_in(
            buf,
            list,
            row_status_x(list),
            item.y,
            status_label(row.status),
            s_style,
        );
    }
    if let DisplayGroupRole::Member { offset, count } = row.role {
        let conn_style = if row.drag_source_dimmed {
            style::muted()
        } else {
            style::todo_group_connector(row.status)
        };
        let branch_x = row_branch_x(list);
        for line_offset in 0..usize::from(item.rendered_rows) {
            let y = item
                .y
                .saturating_add(u16::try_from(line_offset).unwrap_or(u16::MAX));
            let glyph = group_connector_glyph(offset, count, line_offset);
            write_string_in(buf, list, branch_x, y, glyph, conn_style);
        }
    }
    let grouped = row.role.is_grouped();
    // 独立新建幽灵行没有可归属的真实条目，P1 徽标是不可点击的死控件；组内幽灵行
    // 仍作为组锚点显示组级徽标（点击可循环组内最高优先级条目）。
    let standalone_ghost = !grouped && row.edit.is_some_and(|edit| edit.target.is_none());
    if row.role.is_status_anchor() && !standalone_ghost {
        render_priority_badge(
            buf,
            list,
            row_priority_x(list),
            item.y,
            row.priority,
            row.status,
        );
    }
    match row.edit {
        Some(edit) => render_edit_item(buf, list, item, edit, grouped),
        None => {
            let cell_style = if row.drag_source_dimmed {
                style::muted()
            } else {
                text_style(row.status)
            };
            render_wrapped_text(buf, list, item, &row.lines, grouped, cell_style);
            None
        }
    }
}

/// 逐行绘制折行正文：单条首行缩进 9 列/续行 6 列，聚合组子项首行与续行统一缩进 12 列。
fn render_wrapped_text(
    buf: &mut Buffer,
    list: Rect,
    item: VisibleItem,
    lines: &[String],
    grouped: bool,
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
        let line_x = row_line_x_for(list, grouped, offset);
        write_string_in(buf, list, line_x, y, line, cell_style);
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

/// 绘制行内编辑：1~3 行软折行（单条续行与 `P(n)` 左对齐、聚合组续行与正文对齐），并返回硬件光标坐标。
///
/// 与行内多行输入共用 `widgets::input::multiline_view_scrolled_indented` 的视口与光标算法。
fn render_edit_item(
    buf: &mut Buffer,
    list: Rect,
    item: VisibleItem,
    edit: &TodoEdit,
    grouped: bool,
) -> Option<(u16, u16)> {
    let text_x = row_text_x_for(list, grouped);
    if list.right() <= text_x {
        return None;
    }
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
    let text_width = text_width_for(list, grouped);
    let cont_width = cont_width_for(list, grouped);
    let view = edit_view(edit, text_width, cont_width);
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
        let mut cur_x = row_line_x_for(list, grouped, offset);
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
    let cursor_line_x = row_line_x_for(list, grouped, usize::from(view.cursor_row_offset));
    let cursor_x = cursor_line_x
        .saturating_add(view.cursor_col)
        .min(list.right().saturating_sub(1));
    let cursor_y = item
        .y
        .saturating_add(view.cursor_row_offset)
        .min(list.bottom().saturating_sub(1));
    Some((cursor_x, cursor_y))
}

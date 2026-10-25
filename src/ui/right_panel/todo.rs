//! 右侧容器的 Todo 面板体：工具栏按钮、三态列表行、行内单行编辑与滚动条。
//!
//! 绘制与命中共用同一组几何函数（项目既有不变量）；本模块只读状态，不改状态、不做 IO。

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::state::{AppState, TodoEdit};
use crate::app::tooltip::TodoButton;
use crate::db::todo::{TodoRecord, TodoStatus};

use crate::ui::text;

use super::super::{prompt, style, widgets};

/// 工具栏按钮宽度（列）；与 `text::TODO_*_LABEL` 的显示宽度一一对应。
const NEW_WIDTH: u16 = 5;
const EDIT_WIDTH: u16 = 6;
const DELETE_WIDTH: u16 = 5;
const CLEAR_DONE_WIDTH: u16 = 12;
/// 按钮之间的空档（列）。
const BUTTON_GAP: u16 = 1;
/// 行首选中标记槽宽（列）：`❯` 与未选中的空格共用同一列，保证文本左对齐。
const MARKER_WIDTH: u16 = 1;
/// 状态框宽度（列）：`[ ]` / `[~]` / `[x]` 均为 3。
const STATUS_WIDTH: u16 = 3;

/// 全部按钮按显示顺序排列；宽度阶梯按**从右往左**丢弃。
const BUTTONS: [(TodoButton, u16); 4] = [
    (TodoButton::New, NEW_WIDTH),
    (TodoButton::Edit, EDIT_WIDTH),
    (TodoButton::Delete, DELETE_WIDTH),
    (TodoButton::ClearDone, CLEAR_DONE_WIDTH),
];

/// 列表区矩形：直接委托 prompt 的正文区几何（已扣表头两行、已让出最右 1 列滚动条槽）。
pub fn list_rect(panel: Rect) -> Rect {
    crate::layout::prompt_text_rect(panel)
}

/// 可见行数。
pub fn visible_rows(panel: Rect) -> usize {
    usize::from(list_rect(panel).height)
}

/// 放得下的按钮数（宽度阶梯）。
fn button_count(panel: Rect) -> usize {
    let width = crate::layout::pane_inner_rect(panel).width;
    let mut total: u16 = 0;
    let mut count = 0;
    for (index, (_, button_width)) in BUTTONS.iter().enumerate() {
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

/// 工具栏按钮矩形；放不下时返回 None（按 [`BUTTONS`] 的阶梯从右往左丢弃）。
pub fn button_rect(panel: Rect, button: TodoButton) -> Option<Rect> {
    let bar = crate::layout::prompt_toolbar_rect(panel)?;
    let count = button_count(panel);
    let mut x = bar.x;
    for (index, (kind, width)) in BUTTONS.iter().enumerate() {
        if index >= count {
            return None;
        }
        if *kind == button {
            return Some(Rect::new(x, bar.y, *width, 1));
        }
        x = x.saturating_add(*width).saturating_add(BUTTON_GAP);
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

/// 行命中：返回列表下标（已加视口偏移）；落在列表区之外或越界时为 None。
///
/// 新建编辑中的幽灵行也参与命中：点它等于点正在输入的那一行（不提交、不失焦）。
pub fn row_at(panel: Rect, state: &AppState, column: u16, row: u16) -> Option<usize> {
    let list = list_rect(panel);
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

/// 滚动条几何（无溢出时为 None）。
pub fn scrollbar(panel: Rect, state: &AppState) -> Option<widgets::scrollbar::ScrollbarLayout> {
    let slot = crate::layout::prompt_scrollbar_rect(panel)?;
    widgets::scrollbar::layout(
        slot,
        state.todo.len(),
        visible_rows(panel),
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
    let focused = state.todo_focused;
    prompt::render_divider(area, header, frame.buffer_mut(), focused);
    let buf = frame.buffer_mut();
    render_toolbar(area, buf, state);
    let cursor = render_rows(area, buf, state);
    if let Some(bar) = scrollbar(area, state) {
        widgets::scrollbar::render_buffer(buf, &bar);
    }
    cursor
}

/// 绘制工具栏按钮（无效按钮置灰）；右端按剩余宽度绘制完成计数。
fn render_toolbar(panel: Rect, buf: &mut Buffer, state: &AppState) {
    for (button, _) in BUTTONS {
        let Some(rect) = button_rect(panel, button) else {
            continue;
        };
        // 可用态与点击行为共用同一个判定，避免「看着亮却点不动」。
        let enabled = crate::app::update::todo_button_enabled(state, button);
        let label = match button {
            TodoButton::New => text::TODO_NEW_LABEL,
            TodoButton::Edit => text::TODO_EDIT_LABEL,
            TodoButton::Delete => text::TODO_DELETE_LABEL,
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
                // 每格先重置：直接叠加会继承聚焦边框的 `accent()`，非激活按钮也跟着发亮。
                cell.reset();
                cell.set_char(symbol);
                cell.set_style(button_style);
            }
        }
    }
    // 计数 `done/total`：只在按钮右侧剩余的列足够时绘制。
    if state.todo.is_empty() {
        return;
    }
    let Some(bar) = crate::layout::prompt_toolbar_rect(panel) else {
        return;
    };
    let count = format!(
        "{}{}{}",
        state.todo.done_count(),
        text::TODO_COUNT_SEPARATOR,
        state.todo.len()
    );
    let count_width = u16::try_from(count.chars().count()).unwrap_or(u16::MAX);
    let right_limit = bar.right().saturating_sub(1);
    let used = BUTTONS.iter().enumerate().take(button_count(panel)).fold(
        bar.x,
        |x, (index, (_, width))| {
            x.saturating_add(if index == 0 { 0 } else { BUTTON_GAP })
                .saturating_add(*width)
        },
    );
    if right_limit.saturating_sub(used) < count_width.saturating_add(1) {
        return;
    }
    let x = right_limit.saturating_sub(count_width);
    for (offset, symbol) in count.chars().enumerate() {
        let column = x.saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
        if let Some(cell) = buf.cell_mut((column, bar.y)) {
            cell.reset();
            cell.set_char(symbol);
            cell.set_style(style::muted());
        }
    }
}

/// 绘制可见列表行；返回编辑光标坐标。
///
/// 列表行 = 既有条目 + （新建编辑中）一个幽灵行。幽灵行插在 `edit.at` 处，
/// 因为此时列表里还没有这条记录——编辑态唯一的可见输入位置不能靠空白糊过去。
fn render_rows(panel: Rect, buf: &mut Buffer, state: &AppState) -> Option<(u16, u16)> {
    let list = list_rect(panel);
    if list.width == 0 || list.height == 0 {
        return None;
    }
    // 整块先重置：未使用行不能残留上一帧内容，也不能继承聚焦边框的样式。
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
        let row_selected = state.todo_focused && state.todo.selected() == Some(index);
        if pending_new == Some(index) {
            cursor = render_row(
                buf,
                list,
                y,
                RowView {
                    status: TodoStatus::Todo,
                    text: "",
                    selected: row_selected,
                    // 幽灵行就是当前编辑目标，恒标 `❯`（它还没有记录可供比对）。
                    marker: true,
                    edit: state.todo.editing(),
                },
            );
            continue;
        }
        // 幽灵行占了它自己的槽位，其后既有记录的下标整体前移一位。
        let record_index = if pending_new.is_some_and(|at| index > at) {
            index - 1
        } else {
            index
        };
        let Some(record) = state.todo.items().get(record_index) else {
            break;
        };
        let hovered = state.todo_hover == Some(record_index);
        cursor = render_row(
            buf,
            list,
            y,
            RowView {
                status: record.status,
                text: &record.text,
                // 悬停未选中行同样反显（与 prompt 树视图口径一致）。
                selected: row_selected || hovered,
                marker: state.todo.selected() == Some(record_index),
                edit: editing_row(state, record),
            },
        );
    }
    cursor
}

/// 一行的绘制数据。
struct RowView<'a> {
    status: TodoStatus,
    text: &'a str,
    /// 整行反显（选中或悬停）。
    selected: bool,
    /// 行首绘制 `❯`（仅真正选中的那一行）。
    marker: bool,
    edit: Option<&'a TodoEdit>,
}

/// 绘制一整行：标记 + 状态框 + 文本（或编辑输入）；返回编辑光标坐标。
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
    let text_x = status_x.saturating_add(STATUS_WIDTH).saturating_add(1);
    match row.edit {
        Some(edit) => render_edit_row(buf, list, text_x, y, edit),
        None => {
            write_string_in(buf, list, text_x, y, row.text, text_style(row.status));
            None
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

/// 绘制编辑行：按光标位置横向滚动，保证光标始终可见；返回光标绝对坐标。
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
    let chars: Vec<char> = edit.input.text().chars().collect();
    let cursor_index = edit.input.cursor().min(chars.len());
    // 光标落在文本末尾时也要能站住，故可见窗口按「光标至少要留在最后一列」裁剪。
    let start = cursor_index.saturating_sub(width.saturating_sub(1));
    let visible: String = chars.iter().skip(start).take(width).collect();
    write_string_in(buf, list, text_x, y, &visible, style::text());
    let cursor_x = text_x.saturating_add(
        u16::try_from(cursor_index.saturating_sub(start))
            .unwrap_or(u16::MAX)
            .min(u16::try_from(width.saturating_sub(1)).unwrap_or(u16::MAX)),
    );
    Some((cursor_x, y))
}

/// 写入单个字符格（宽字符按其显示宽度推进由 `set_stringn` 负责）。
fn write_cell(buf: &mut Buffer, x: u16, y: u16, symbol: &str, cell_style: Style) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.reset();
        cell.set_symbol(symbol);
        cell.set_style(cell_style);
    }
}

/// 写入一段文本，按显示宽度截断到 `list` 右缘（不折行、不加省略号，保持列对齐）。
///
/// 上限必须是**列表区**右缘而不是帧右缘：列表最右一列是滚动条槽，越界写入会把
/// thumb 盖掉。
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
    let text_x = list
        .x
        .saturating_add(MARKER_WIDTH)
        .saturating_add(1)
        .saturating_add(STATUS_WIDTH)
        .saturating_add(1);
    usize::from(list.right().saturating_sub(text_x))
}

#[cfg(test)]
mod tests;

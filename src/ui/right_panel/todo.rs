//! 右侧容器的 Todo 面板体：工具栏按钮、三态列表行、行内单行编辑、滚动条与底部统计。
//!
//! 绘制与命中共用同一组几何函数（项目既有不变量）；本模块只读状态，不改状态、不做 IO。
//! 主体区自上而下为「列表 / 分割线 / 统计行」：统计行固定 1 行，主体区太矮时整块降级隐藏。

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
const NEW_WIDTH: u16 = 3;
const EDIT_WIDTH: u16 = 6;
const DELETE_WIDTH: u16 = 5;
const CLEAR_DONE_WIDTH: u16 = 12;
/// 按钮之间的空档（列）。
const BUTTON_GAP: u16 = 1;
/// 行首选中标记槽宽（列）：`❯` 与未选中的空格共用同一列，保证文本左对齐。
const MARKER_WIDTH: u16 = 1;
/// 状态框宽度（列）：`[ ]` / `[~]` / `[x]` 均为 3。
const STATUS_WIDTH: u16 = 3;
/// 主体区低于此行数时不上下两分：列表独占，统计块整块隐藏。
const MIN_SPLIT_BODY: u16 = 7;
/// 统计块的最少行数：摘要 1 行 + 柱区 1 行 + 轴标签 1 行。
const MIN_STATS_ROWS: u16 = 3;
/// 柱状图每根柱子的宽度（列）；槽位更窄时取槽宽。
const BAR_WIDTH: u16 = 3;

/// 工具栏左侧组：新建（`[+]`，贴近左边框）。
const LEFT_BUTTONS: [(TodoButton, u16); 1] = [(TodoButton::New, NEW_WIDTH)];
/// 工具栏右侧组：条目操作，从右缘往左排。
const RIGHT_BUTTONS: [(TodoButton, u16); 3] = [
    (TodoButton::Edit, EDIT_WIDTH),
    (TodoButton::Delete, DELETE_WIDTH),
    (TodoButton::ClearDone, CLEAR_DONE_WIDTH),
];
/// 全部按钮（命中与文案查表用）。
const BUTTONS: [(TodoButton, u16); 4] = [
    (TodoButton::New, NEW_WIDTH),
    (TodoButton::Edit, EDIT_WIDTH),
    (TodoButton::Delete, DELETE_WIDTH),
    (TodoButton::ClearDone, CLEAR_DONE_WIDTH),
];

/// 主体区：表头两行之下的整块（含最右 1 列滚动条槽）。
fn body_rect(panel: Rect) -> Rect {
    crate::layout::prompt_body_rect(crate::layout::pane_inner_rect(panel))
}

/// 统计块高度：与列表**平分**主体区（分割线占 1 行，取整余数留给列表）。
///
/// 返回 0 表示不分块：主体区太矮时列表独占，避免统计块把列表挤到 1 行。
fn stats_height(panel: Rect) -> u16 {
    let body = body_rect(panel).height;
    if body < MIN_SPLIT_BODY {
        return 0;
    }
    (body - 1) / 2
}

/// 列表区矩形：主体区扣掉分割线与统计块，且让出最右 1 列滚动条槽。
pub fn list_rect(panel: Rect) -> Rect {
    let body = body_rect(panel);
    let stats = stats_height(panel);
    let height = if stats == 0 {
        body.height
    } else {
        body.height.saturating_sub(stats).saturating_sub(1)
    };
    Rect {
        width: body.width.saturating_sub(1),
        height,
        ..body
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

/// 可见行数。
pub fn visible_rows(panel: Rect) -> usize {
    usize::from(list_rect(panel).height)
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

/// 右侧组放得下的按钮数（宽度阶梯：从右缘往左排，先丢 `[clear done]` 再丢 `[del]`）。
///
/// 左侧组先扣掉并额外让出 1 列间隔：两组之间必须留出至少 1 列空白，
/// 否则窄面板下 `[+]` 会紧贴 `[edit]` 连成一串。
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
///
/// 布局：`[+]` 贴左；右侧组从**右缘往左**依次排 `[clear done]` `[del]` `[edit]`，
/// 这样窄面板丢按钮时是从左往右少（最常用的 `[edit]` 最后才丢）。
pub fn button_rect(panel: Rect, button: TodoButton) -> Option<Rect> {
    let bar = crate::layout::prompt_toolbar_rect(panel)?;
    if LEFT_BUTTONS.iter().any(|(kind, _)| *kind == button) {
        // 左侧组只有 `[+]`，宽度固定 3，放不下时整组消失。
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

/// 行命中的**显示**下标（含幽灵行槽位）；落在列表区之外或越界时为 None。
///
/// 新建编辑中的幽灵行也参与命中：点它等于点正在输入的那一行（不提交、不失焦）。
/// 调用方拿到的下标与屏幕上第几行一一对应，换算成列表下标是调用方的事
/// （见 [`list_index`]）——点击要按 id 锚定，悬停要按列表下标比较。
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

/// 显示下标 → 列表下标；落在幽灵行上返回 None（它不是任何条目）。
///
/// 幽灵行占着它自己的显示槽位，其下方的既有条目在列表里都前移一位。
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

/// 滚动条几何（无溢出时为 None）；轨道只覆盖列表区，不含底部统计块。
pub fn scrollbar(panel: Rect, state: &AppState) -> Option<widgets::scrollbar::ScrollbarLayout> {
    // 只在自己可见时存在：Todo 隐藏时这条几何与 prompt 正文共用最右一列，
    // 不判 tab 会让「点 prompt 正文右缘」滚到看不见的 Todo 列表（也会让拖拽/命中错位）。
    if state.right_tab != crate::app::state::RightTab::Todo || state.prompt_collapsed {
        return None;
    }
    let body = body_rect(panel);
    if body.width < 2 {
        return None;
    }
    let list = list_rect(panel);
    let slot = Rect::new(list.right(), list.y, 1, list.height);
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
    let focused = state.todo_focused();
    prompt::render_divider(area, header, frame.buffer_mut(), focused);
    let buf = frame.buffer_mut();
    render_toolbar(area, buf, state);
    // 只有面板持焦点时才交出硬件光标：否则上层会把它当成「当前输入位置」画出来，
    // 终端里表现为 IME 落在没有输入框的 Todo 行上。
    let cursor = render_rows(area, buf, state).filter(|_| focused);
    if let Some(bar) = scrollbar(area, state) {
        widgets::scrollbar::render_buffer(buf, &bar);
    }
    render_stats(area, buf, state, focused);
    cursor
}

/// 绘制工具栏按钮（无效按钮置灰）。计数不在这里：完成情况统一由底部统计行展示。
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
        let row_selected = state.todo_focused() && state.todo.selected() == Some(index);
        if pending_new == Some(index) {
            cursor = cursor.or(render_row(
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
            ));
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
        // 只能 `or` 累积：逐行覆盖会让「编辑行之后还有行」时把已拿到的光标清成 None，
        // 上层随即回落到窗格光标——终端里表现为 IME 跑到 lx 输入框。
        cursor = cursor.or(render_row(
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
        ));
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

/// 底部统计块：摘要行 + 三态竖向柱图 + 轴标签行。
///
/// ```text
/// total 4 · doing 1 · done 2 50%     ← 摘要（整行量宽，放不下整行不画）
///      █                             ← 柱区（高度随统计块伸缩）
///   █  █  █                          ←   柱子用 `█`，柱高按占比缩放
///  open doing done                   ← 轴标签（含各自计数）
/// ```
///
/// 统计块与列表平分主体区（见 [`stats_height`]），所以柱区高度不是常量：
/// 柱高 = `count × 可用行数 / 最大值`，非零计数至少 1 行。
fn render_stats(panel: Rect, buf: &mut Buffer, state: &AppState, focused: bool) {
    let (Some(divider), Some(area)) = (stats_divider_rect(panel), stats_rect(panel)) else {
        return;
    };
    prompt::render_rule(panel, divider.y, buf, focused);
    buf.set_style(area, Style::reset());
    if area.height == 0 {
        return;
    }
    let open = state.todo.open_count();
    let doing = state.todo.in_progress_count();
    let done = state.todo.done_count();
    let total = state.todo.len();
    render_summary(area, buf, total, doing, done);
    if area.height < MIN_STATS_ROWS {
        return;
    }
    let chart = Rect {
        y: area.y.saturating_add(1),
        height: area.height.saturating_sub(2),
        ..area
    };
    let axis = Rect {
        y: area.bottom().saturating_sub(1),
        height: 1,
        ..area
    };
    let columns = [
        (text::TODO_STATS_OPEN_LABEL, open, style::todo_bar_open()),
        (
            text::TODO_STATS_DOING_LABEL,
            doing,
            style::todo_bar_active(),
        ),
        (text::TODO_STATS_DONE_LABEL, done, style::todo_bar_done()),
    ];
    render_bars(chart, axis, buf, &columns);
}

/// 摘要行：`total N · doing N · done N N%`。
///
/// 整行量宽：放不下就整行不画。逐段「放不下就跳过」会让后面的短段顶到前面去，
/// 拼出 `total 4 · doing 1 · 2 50%` 这种残句。
fn render_summary(area: Rect, buf: &mut Buffer, total: usize, doing: usize, done: usize) {
    let percent = done.saturating_mul(100).checked_div(total).unwrap_or(0);
    let segments = [
        (text::TODO_STATS_TOTAL_LABEL, total.to_string()),
        (text::TODO_STATS_DOING_LABEL, doing.to_string()),
        (
            text::TODO_STATS_DONE_LABEL,
            format!("{done} {percent}{}", text::TODO_STATS_PERCENT_SUFFIX),
        ),
    ];
    let width: usize = segments
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let separator = if index > 0 {
                cells(text::TODO_STATS_SEPARATOR)
            } else {
                0
            };
            separator + cells(label) + 1 + cells(value)
        })
        .sum();
    let row = Rect { height: 1, ..area };
    if width > usize::from(row.width) {
        return;
    }
    let mut x = row.x;
    for (index, (label, value)) in segments.iter().enumerate() {
        if index > 0 {
            x = write_inline(buf, row, x, text::TODO_STATS_SEPARATOR, style::muted());
        }
        x = write_inline(buf, row, x, label, style::muted());
        x = write_inline(buf, row, x, " ", style::muted());
        x = write_inline(buf, row, x, value, style::text());
    }
}

/// 柱区与轴标签：三态各占一个等宽槽位，柱子在槽内水平居中、自下而上堆叠 `█`。
///
/// 计数全为 0 时柱子为空、只留轴标签——空列表也占同样的版面高度，面板不会跳。
fn render_bars(
    chart: Rect,
    axis: Rect,
    buf: &mut Buffer,
    columns: &[(&'static str, usize, Style)],
) {
    if chart.height == 0 || chart.width == 0 || columns.is_empty() {
        return;
    }
    let count = u16::try_from(columns.len()).unwrap_or(1).max(1);
    let slot_width = chart.width / count;
    if slot_width == 0 {
        return;
    }
    let bar_width = slot_width.min(BAR_WIDTH);
    let max = columns
        .iter()
        .map(|(_, value, _)| *value)
        .max()
        .unwrap_or(0);
    let rows = usize::from(chart.height);
    for (index, (label, value, bar_style)) in columns.iter().enumerate() {
        let slot = Rect::new(
            chart
                .x
                .saturating_add(slot_width.saturating_mul(u16::try_from(index).unwrap_or(0))),
            chart.y,
            slot_width,
            chart.height,
        );
        let bar_x = slot
            .x
            .saturating_add(slot.width.saturating_sub(bar_width) / 2);
        // 非零计数至少占 1 行，否则「有一条未完成」和「一条都没有」看起来一样。
        let filled = match (*value, max) {
            (0, _) => 0,
            (value, max) => (value.saturating_mul(rows) / max.max(1)).max(1).min(rows),
        };
        for offset in 0..filled {
            let y = chart
                .bottom()
                .saturating_sub(1)
                .saturating_sub(u16::try_from(offset).unwrap_or(u16::MAX));
            for x in bar_x..bar_x.saturating_add(bar_width).min(slot.right()) {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.reset();
                    cell.set_symbol(text::TODO_BAR_FILLED);
                    cell.set_style(*bar_style);
                }
            }
        }
        write_centered(
            buf,
            slot,
            axis.y,
            &format!("{label} {value}"),
            style::muted(),
        );
    }
}

/// 在槽位内水平居中写一行文本；放不下时整段不写（不截半句）。
fn write_centered(buf: &mut Buffer, slot: Rect, y: u16, value: &str, cell_style: Style) {
    let width = u16::try_from(cells(value)).unwrap_or(u16::MAX);
    if width > slot.width {
        return;
    }
    let x = slot.x.saturating_add(slot.width.saturating_sub(width) / 2);
    buf.set_stringn(x, y, value, usize::from(width), cell_style);
}

/// 文本显示列数。
fn cells(value: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(value)
}

/// 从 `x` 起写一段文本并返回结尾列；调用方已按整行宽度校验，越界时兜底不写。
fn write_inline(buf: &mut Buffer, row: Rect, x: u16, value: &str, cell_style: Style) -> u16 {
    let width = u16::try_from(cells(value)).unwrap_or(u16::MAX);
    if x.saturating_add(width) > row.right() {
        return x;
    }
    buf.set_stringn(x, row.y, value, usize::from(row.right() - x), cell_style);
    x.saturating_add(width)
}

/// 该行是否为编辑现场（仅既有条目；新建走上面的幽灵行）。
fn editing_row<'a>(state: &'a AppState, record: &TodoRecord) -> Option<&'a TodoEdit> {
    let edit = state.todo.editing()?;
    (edit.target.as_deref() == Some(record.id.as_str())).then_some(edit)
}

/// 绘制编辑行：与页重命名/浮层输入共用 `widgets::input` 的视口与光标算法
/// （按显示宽度滚动，宽字符占两列）；返回光标绝对坐标。
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
    // 新建时没有原文可参照，占位是唯一的「这里能打字」提示；编辑既有条目不加占位。
    let (visible, input_style) = match text.is_empty() {
        true if edit.target.is_none() => (text::TODO_EDIT_PLACEHOLDER, style::muted()),
        _ => (input.visible, style::text()),
    };
    write_string_in(buf, list, text_x, y, visible, input_style);
    Some((
        text_x
            .saturating_add(input.cursor_col)
            .min(list.right().saturating_sub(1)),
        y,
    ))
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

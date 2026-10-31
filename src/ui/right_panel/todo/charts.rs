//! Todo 底部统计块：分割线图表切换器与 3 款统计图表（三态与完成率、优先级分布、近 7 日趋势）。
//!
//! 一次仅渲染当前选中的一款图表，通过分割线上的 `‹ title n/3 ›` 控件或快捷键切换。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AppState, TodoChartKind};
use crate::db::todo::{TodoPriority, day_of_month};
use crate::ui::{prompt, style, text};

use super::{BAR_WIDTH, MIN_STATS_ROWS, stats_divider_rect, stats_rect, write_cell, write_inline};

/// 图表切换控件的鼠标点击命中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartSwitchHit {
    Prev,
    Next,
}

/// 当前图表的标题与范围标注（注明今天还是一周）。
pub fn chart_title(kind: TodoChartKind, is_today: bool) -> &'static str {
    match kind {
        TodoChartKind::StatusAndRate => {
            if is_today {
                "today · status"
            } else {
                "day · status"
            }
        }
        TodoChartKind::Priority => {
            if is_today {
                "today · priority"
            } else {
                "day · priority"
            }
        }
        TodoChartKind::Trend7Day => "week · 7d trend",
    }
}

/// 空间紧凑时的简短范围标注。
pub fn chart_short_title(kind: TodoChartKind, is_today: bool) -> &'static str {
    match kind {
        TodoChartKind::StatusAndRate => {
            if is_today {
                "today"
            } else {
                "day"
            }
        }
        TodoChartKind::Priority => {
            if is_today {
                "today"
            } else {
                "day"
            }
        }
        TodoChartKind::Trend7Day => "week",
    }
}

/// 分割线上的图表切换器矩形：返回 `(整体矩形, 左箭头 ‹ 矩形, 右箭头 › 矩形)`。
pub fn chart_switcher_rects(panel: Rect, state: &AppState) -> Option<(Rect, Rect, Rect)> {
    let divider = stats_divider_rect(panel)?;
    let is_today = state.todo.is_viewing_today();
    let kind = state.todo.active_chart();
    let full = format!(
        "{} {} {}/3 {}",
        text::TODO_CHART_PREV_LABEL,
        chart_title(kind, is_today),
        kind.number(),
        text::TODO_CHART_NEXT_LABEL
    );
    let full_w = u16::try_from(UnicodeWidthStr::width(full.as_str())).unwrap_or(u16::MAX);
    let short = format!(
        "{} {} {}/3 {}",
        text::TODO_CHART_PREV_LABEL,
        chart_short_title(kind, is_today),
        kind.number(),
        text::TODO_CHART_NEXT_LABEL
    );
    let short_w = u16::try_from(UnicodeWidthStr::width(short.as_str())).unwrap_or(u16::MAX);

    let width = if divider.width >= full_w.saturating_add(4) {
        full_w
    } else if divider.width >= short_w.saturating_add(4) {
        short_w
    } else {
        return None;
    };

    // 贴分割线右侧内嵌（右端留 1 列 `─` 衔接 `┤`）。
    // prev_x 是 `‹` 所在列，next_x 是 `›` 所在列，整段严格限制在 prev_x..=next_x，
    // 绝不外溢到两旁的 `─` 边框线单元格，保证左右边框线样式完全一致。
    let next_x = divider.right().saturating_sub(2);
    let prev_x = next_x.saturating_sub(width).saturating_add(1);
    let whole = Rect::new(prev_x, divider.y, width, 1);
    let prev = Rect::new(prev_x, divider.y, 1, 1);
    let next = Rect::new(next_x, divider.y, 1, 1);
    Some((whole, prev, next))
}

/// 图表切换控件点击命中：点左箭头切上一张，点右箭头或标题本身切下一张。
pub fn chart_switch_hit_at(
    panel: Rect,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<ChartSwitchHit> {
    let (whole, prev, _next) = chart_switcher_rects(panel, state)?;
    if !whole.contains((column, row).into()) {
        return None;
    }
    if prev.contains((column, row).into()) {
        Some(ChartSwitchHit::Prev)
    } else {
        Some(ChartSwitchHit::Next)
    }
}

/// 绘制统计分割线（含内嵌图表切换器）与当前激活的图表。
pub(super) fn render_stats(panel: Rect, buf: &mut Buffer, state: &AppState, focused: bool) {
    let (Some(divider), Some(area)) = (stats_divider_rect(panel), stats_rect(panel)) else {
        return;
    };
    prompt::render_rule(panel, divider.y, buf, focused);
    render_chart_switcher(panel, buf, state);
    buf.set_style(area, Style::reset());
    if area.height == 0 {
        return;
    }
    match state.todo.active_chart() {
        TodoChartKind::StatusAndRate => render_status_chart(area, buf, state),
        TodoChartKind::Priority => render_priority_chart(area, buf, state),
        TodoChartKind::Trend7Day => render_trend_chart(area, buf, state),
    }
}

fn render_chart_switcher(panel: Rect, buf: &mut Buffer, state: &AppState) {
    let Some((whole, prev, next)) = chart_switcher_rects(panel, state) else {
        return;
    };
    write_cell(
        buf,
        prev.x,
        prev.y,
        text::TODO_CHART_PREV_LABEL,
        style::accent(),
    );
    write_cell(
        buf,
        next.x,
        next.y,
        text::TODO_CHART_NEXT_LABEL,
        style::accent(),
    );

    let is_today = state.todo.is_viewing_today();
    let kind = state.todo.active_chart();
    let full_title = chart_title(kind, is_today);
    let full_mid = format!(" {full_title} {}/3 ", kind.number());
    let short_title = chart_short_title(kind, is_today);
    let short_mid = format!(" {short_title} {}/3 ", kind.number());

    let mid_w = next.x.saturating_sub(prev.right());
    let mid_text = if usize::from(mid_w) >= UnicodeWidthStr::width(full_mid.as_str()) {
        full_mid
    } else {
        short_mid
    };
    let mid_rect = Rect::new(prev.right(), whole.y, mid_w, 1);
    write_inline(buf, mid_rect, mid_rect.x, &mid_text, style::muted());
}

/// 图表 1：三态柱状分布与完成率摘要。
fn render_status_chart(area: Rect, buf: &mut Buffer, state: &AppState) {
    let open = state.todo.open_count();
    let doing = state.todo.in_progress_count();
    let done = state.todo.done_count();
    let total = state.todo.len();
    let percent = done.saturating_mul(100).checked_div(total).unwrap_or(0);
    let segments = [
        (text::TODO_STATS_TOTAL_LABEL, total.to_string()),
        (text::TODO_STATS_DOING_LABEL, doing.to_string()),
        (
            text::TODO_STATS_DONE_LABEL,
            format!("{done} {percent}{}", text::TODO_STATS_PERCENT_SUFFIX),
        ),
    ];
    render_summary_segments(area, buf, &segments);
    if area.height < MIN_STATS_ROWS {
        return;
    }
    let (chart, axis) = chart_and_axis_rects(area);
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

/// 图表 2：四档优先级分布（P0..P3）。
fn render_priority_chart(area: Rect, buf: &mut Buffer, state: &AppState) {
    let p0 = state.todo.priority_count(TodoPriority::P0);
    let p1 = state.todo.priority_count(TodoPriority::P1);
    let p2 = state.todo.priority_count(TodoPriority::P2);
    let p3 = state.todo.priority_count(TodoPriority::P3);
    let segments = [
        ("P0", p0.to_string()),
        ("P1", p1.to_string()),
        ("P2", p2.to_string()),
        ("P3", p3.to_string()),
    ];
    render_summary_segments(area, buf, &segments);
    if area.height < MIN_STATS_ROWS {
        return;
    }
    let (chart, axis) = chart_and_axis_rects(area);
    let columns = [
        ("P0", p0, style::todo_priority_bar(TodoPriority::P0)),
        ("P1", p1, style::todo_priority_bar(TodoPriority::P1)),
        ("P2", p2, style::todo_priority_bar(TodoPriority::P2)),
        ("P3", p3, style::todo_priority_bar(TodoPriority::P3)),
    ];
    render_bars(chart, axis, buf, &columns);
}

/// 图表 3：近 7 日计划与完成双柱趋势。
fn render_trend_chart(area: Rect, buf: &mut Buffer, state: &AppState) {
    let points = state.todo.trend_7_days();
    let total_planned: usize = points.iter().map(|p| p.planned_count).sum();
    let total_done: usize = points.iter().map(|p| p.completed_count).sum();
    let percent = total_done
        .saturating_mul(100)
        .checked_div(total_planned)
        .unwrap_or(0);
    let segments = [
        (text::TODO_STATS_PLANNED_LABEL, total_planned.to_string()),
        (
            text::TODO_STATS_DONE_LABEL,
            format!("{total_done} {percent}{}", text::TODO_STATS_PERCENT_SUFFIX),
        ),
    ];
    render_summary_segments(area, buf, &segments);
    if area.height < MIN_STATS_ROWS {
        return;
    }
    let (chart, axis) = chart_and_axis_rects(area);
    if chart.height == 0 || chart.width == 0 || points.is_empty() {
        return;
    }
    let count = u16::try_from(points.len()).unwrap_or(1).max(1);
    let slot_width = chart.width / count;
    if slot_width == 0 {
        return;
    }
    let max_val = points
        .iter()
        .map(|p| p.planned_count.max(p.completed_count))
        .max()
        .unwrap_or(0);
    let rows = usize::from(chart.height);
    let selected_date = state.todo.selected_date();
    for (index, point) in points.iter().enumerate() {
        let slot_x = chart
            .x
            .saturating_add(slot_width.saturating_mul(u16::try_from(index).unwrap_or(0)));
        let slot = Rect::new(slot_x, chart.y, slot_width, chart.height);
        let planned_h = bar_height(point.planned_count, max_val, rows);
        let done_h = bar_height(point.completed_count, max_val, rows);
        if slot_width >= 2 {
            // 双细柱并排：左列计划 `░`、右列完成 `█`。
            let left_x = slot.x.saturating_add(slot.width.saturating_sub(2) / 2);
            let right_x = left_x.saturating_add(1);
            for step in 0..planned_h {
                let y = chart
                    .bottom()
                    .saturating_sub(1)
                    .saturating_sub(u16::try_from(step).unwrap_or(0));
                write_cell(
                    buf,
                    left_x,
                    y,
                    text::TODO_BAR_PLANNED,
                    style::todo_bar_planned(),
                );
            }
            for step in 0..done_h {
                let y = chart
                    .bottom()
                    .saturating_sub(1)
                    .saturating_sub(u16::try_from(step).unwrap_or(0));
                write_cell(
                    buf,
                    right_x,
                    y,
                    text::TODO_BAR_FILLED,
                    style::todo_bar_done(),
                );
            }
        } else {
            // 极窄槽位：先画计划高度，再用完成高度覆盖底部。
            for step in 0..planned_h {
                let y = chart
                    .bottom()
                    .saturating_sub(1)
                    .saturating_sub(u16::try_from(step).unwrap_or(0));
                write_cell(
                    buf,
                    slot.x,
                    y,
                    text::TODO_BAR_PLANNED,
                    style::todo_bar_planned(),
                );
            }
            for step in 0..done_h {
                let y = chart
                    .bottom()
                    .saturating_sub(1)
                    .saturating_sub(u16::try_from(step).unwrap_or(0));
                write_cell(
                    buf,
                    slot.x,
                    y,
                    text::TODO_BAR_FILLED,
                    style::todo_bar_done(),
                );
            }
        }
        let day_label = format!("{:02}", day_of_month(&point.date));
        let label_style = if point.date == selected_date {
            style::accent()
        } else {
            style::muted()
        };
        render_axis_label_styled(
            Rect::new(slot.x, axis.y, slot_width, 1),
            buf,
            &day_label,
            label_style,
        );
    }
}

fn chart_and_axis_rects(area: Rect) -> (Rect, Rect) {
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
    (chart, axis)
}

/// 摘要行：整行量宽，放不下则整行不画，杜绝半截残句。
fn render_summary_segments(area: Rect, buf: &mut Buffer, segments: &[(&str, String)]) {
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

/// 柱区与轴标签：各占一个等宽槽位，柱子在槽内水平居中、自下而上堆叠 `█`。
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
        let filled = bar_height(*value, max, rows);
        let bar_x = slot
            .x
            .saturating_add(slot.width.saturating_sub(bar_width) / 2);
        for step in 0..filled {
            let y = chart
                .bottom()
                .saturating_sub(1)
                .saturating_sub(u16::try_from(step).unwrap_or(0));
            for dx in 0..bar_width {
                write_cell(
                    buf,
                    bar_x.saturating_add(dx),
                    y,
                    text::TODO_BAR_FILLED,
                    *bar_style,
                );
            }
        }
        render_axis_label(Rect::new(slot.x, axis.y, slot_width, 1), buf, label, *value);
    }
}

/// 单根柱子的高度（行）：非零值至少占 1 行，最大值占满 `rows`。
fn bar_height(value: usize, max: usize, rows: usize) -> usize {
    if value == 0 || max == 0 || rows == 0 {
        return 0;
    }
    value.saturating_mul(rows).div_ceil(max).clamp(1, rows)
}

/// 槽底轴标签：优先 `<label> <count>`，放不下时降级为 `<label>`，再放不下不画。
fn render_axis_label(slot: Rect, buf: &mut Buffer, label: &'static str, value: usize) {
    let full = format!("{label} {value}");
    let chosen: &str = if cells(&full) <= usize::from(slot.width) {
        &full
    } else if cells(label) <= usize::from(slot.width) {
        label
    } else {
        return;
    };
    render_axis_label_styled(slot, buf, chosen, style::muted());
}

fn render_axis_label_styled(slot: Rect, buf: &mut Buffer, label: &str, label_style: Style) {
    let width = u16::try_from(cells(label)).unwrap_or(u16::MAX);
    if width > slot.width {
        return;
    }
    let x = slot.x.saturating_add(slot.width.saturating_sub(width) / 2);
    write_inline(buf, slot, x, label, label_style);
}

fn cells(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

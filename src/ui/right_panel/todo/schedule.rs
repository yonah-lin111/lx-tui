//! Todo 面板的日程组件：一周快速点击感知带（Week Strip）与历史未完成顺延横幅（Rollover Banner）。
//!
//! 几何计算与渲染共用同一套纯函数，保证鼠标点击命中与画面像素严格对齐。

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AppState, WeekDayView};
use crate::ui::{style, text};

use super::{write_cell, write_inline};

/// 一周快速点击感知带的点击命中目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeekStripHit {
    /// 上一周（`‹`）。
    PrevWeek,
    /// 下一周（`›`）。
    NextWeek,
    /// 选中某一天（`YYYY-MM-DD`）。
    Day(String),
}

/// 顺延横幅的点击命中目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RolloverHit {
    /// 将历史未完成一键顺延到今天。
    MoveToToday,
    /// 关闭横幅提醒。
    Dismiss,
}

/// 一周感知带所在行矩形（位于工具栏分割线正下方、列表区正上方，占主体区首行）。
///
/// 主体区过矮（`< MIN_SPLIT_BODY`）或过窄（`< 11` 列）时自动降级隐藏，让列表独占。
pub fn week_strip_rect(panel: Rect) -> Option<Rect> {
    let body = super::body_rect(panel);
    if body.height < super::MIN_SPLIT_BODY || body.width < 11 {
        return None;
    }
    Some(Rect::new(body.x, body.y, body.width, 1))
}

/// 上一周按钮 `‹` 矩形。
pub fn prev_week_rect(panel: Rect) -> Option<Rect> {
    let strip = week_strip_rect(panel)?;
    Some(Rect::new(strip.x, strip.y, 1, 1))
}

/// 下一周按钮 `›` 矩形。
pub fn next_week_rect(panel: Rect) -> Option<Rect> {
    let strip = week_strip_rect(panel)?;
    Some(Rect::new(strip.right().saturating_sub(1), strip.y, 1, 1))
}

/// 周一至周日 7 个日期槽位的矩形（`index` 为 `0..7`）。
pub fn week_day_rect(panel: Rect, index: usize) -> Option<Rect> {
    if index >= 7 {
        return None;
    }
    let strip = week_strip_rect(panel)?;
    let mid_w = strip.width.saturating_sub(2);
    let slot_w = mid_w / 7;
    if slot_w == 0 {
        return None;
    }
    let total_w = slot_w.saturating_mul(7);
    let start_x = strip
        .x
        .saturating_add(1)
        .saturating_add(mid_w.saturating_sub(total_w) / 2);
    let x = start_x.saturating_add(slot_w.saturating_mul(u16::try_from(index).unwrap_or(0)));
    Some(Rect::new(x, strip.y, slot_w, 1))
}

/// 一周感知带鼠标点击命中。
pub fn week_strip_hit_at(
    panel: Rect,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<WeekStripHit> {
    if prev_week_rect(panel).is_some_and(|r| r.contains((column, row).into())) {
        return Some(WeekStripHit::PrevWeek);
    }
    if next_week_rect(panel).is_some_and(|r| r.contains((column, row).into())) {
        return Some(WeekStripHit::NextWeek);
    }
    let days = state.todo.week_days();
    for (index, day) in days.iter().enumerate() {
        if let Some(rect) = week_day_rect(panel, index)
            && rect.contains((column, row).into())
        {
            return Some(WeekStripHit::Day(day.date.clone()));
        }
    }
    None
}

/// 历史未完成顺延横幅矩形（仅在查看今天、有历史未完成且未忽略时占用列表区首行）。
pub fn rollover_banner_rect(panel: Rect, state: &AppState) -> Option<Rect> {
    if !state.todo.has_rollover_banner() {
        return None;
    }
    let base = super::list_rect(panel);
    if base.height < 2 || base.width < 14 {
        return None;
    }
    Some(Rect::new(base.x, base.y, base.width, 1))
}

/// 顺延横幅内 `[move to today]` 与 `[x]` 按钮矩形。
pub fn rollover_button_rects(panel: Rect, state: &AppState) -> Option<(Rect, Rect)> {
    let banner = rollover_banner_rect(panel, state)?;
    let dismiss_w =
        u16::try_from(UnicodeWidthStr::width(text::TODO_ROLLOVER_DISMISS_LABEL)).unwrap_or(3);
    let full_action_w =
        u16::try_from(UnicodeWidthStr::width(text::TODO_ROLLOVER_ACTION_LABEL)).unwrap_or(15);
    let short_action_w = u16::try_from(UnicodeWidthStr::width(
        text::TODO_ROLLOVER_SHORT_ACTION_LABEL,
    ))
    .unwrap_or(6);
    let action_w = if banner.width >= full_action_w.saturating_add(dismiss_w).saturating_add(10) {
        full_action_w
    } else {
        short_action_w
    };
    if banner.width < action_w.saturating_add(1).saturating_add(dismiss_w) {
        return None;
    }
    let dismiss_x = banner.right().saturating_sub(dismiss_w);
    let action_x = dismiss_x.saturating_sub(1).saturating_sub(action_w);
    Some((
        Rect::new(action_x, banner.y, action_w, 1),
        Rect::new(dismiss_x, banner.y, dismiss_w, 1),
    ))
}

/// 顺延横幅鼠标点击命中。
pub fn rollover_hit_at(
    panel: Rect,
    state: &AppState,
    column: u16,
    row: u16,
) -> Option<RolloverHit> {
    let (action_rect, dismiss_rect) = rollover_button_rects(panel, state)?;
    if action_rect.contains((column, row).into()) {
        return Some(RolloverHit::MoveToToday);
    }
    if dismiss_rect.contains((column, row).into()) {
        return Some(RolloverHit::Dismiss);
    }
    None
}

/// 绘制一周快速点击感知带（内嵌在工具栏下方分割线行）。
pub(super) fn render_week_strip(panel: Rect, buf: &mut Buffer, state: &AppState) {
    let Some(strip) = week_strip_rect(panel) else {
        return;
    };
    buf.set_style(strip, Style::reset());
    let (Some(prev), Some(next)) = (prev_week_rect(panel), next_week_rect(panel)) else {
        return;
    };
    write_cell(
        buf,
        prev.x,
        prev.y,
        text::TODO_WEEK_PREV_LABEL,
        style::accent(),
    );
    write_cell(
        buf,
        next.x,
        next.y,
        text::TODO_WEEK_NEXT_LABEL,
        style::accent(),
    );

    let days = state.todo.week_days();
    for (index, day) in days.iter().enumerate() {
        let Some(slot) = week_day_rect(panel, index) else {
            continue;
        };
        render_week_day_slot(buf, slot, day);
    }
}

fn render_week_day_slot(buf: &mut Buffer, slot: Rect, day: &WeekDayView) {
    let dot = if day.is_today {
        text::TODO_TODAY_DOT
    } else {
        ""
    };
    let width = usize::from(slot.width);
    let label = if width >= 7 {
        format!("{} {:02}{dot}", day.weekday_short, day.day_of_month)
    } else if width >= 5 {
        format!("{}{:02}{dot}", day.weekday_initial, day.day_of_month)
    } else if width >= 3 {
        format!("{}{:02}", day.weekday_initial, day.day_of_month)
    } else {
        day.weekday_initial.to_string()
    };
    let label_w = UnicodeWidthStr::width(label.as_str()).min(width);
    let pad_left = u16::try_from((width.saturating_sub(label_w)) / 2).unwrap_or(0);
    let slot_style = if day.is_selected {
        style::todo_week_day_selected()
    } else if day.is_today {
        style::todo_week_day_today()
    } else if day.planned_count > 0 && day.completed_count == day.planned_count {
        style::todo_bar_done()
    } else if day.planned_count > 0 {
        style::text()
    } else {
        style::muted()
    };
    buf.set_style(
        slot,
        if day.is_selected {
            slot_style
        } else {
            Style::reset()
        },
    );
    let x = slot.x.saturating_add(pad_left);
    buf.set_stringn(x, slot.y, &label, width, slot_style);
}

/// 绘制顺延横幅（位于列表顶部第一行）。
pub(super) fn render_rollover_banner(panel: Rect, buf: &mut Buffer, state: &AppState) {
    let Some(banner) = rollover_banner_rect(panel, state) else {
        return;
    };
    buf.set_style(banner, Style::reset());
    let count = state.todo.past_incomplete_count();
    let Some((action_rect, dismiss_rect)) = rollover_button_rects(panel, state) else {
        return;
    };
    let text_limit = usize::from(action_rect.x.saturating_sub(banner.x).saturating_sub(1));
    if text_limit > 0 {
        let prompt_str = if text_limit >= 18 {
            format!("! {count} past unfinished")
        } else {
            format!("! {count} past")
        };
        buf.set_stringn(
            banner.x,
            banner.y,
            &prompt_str,
            text_limit,
            style::todo_rollover_banner(),
        );
    }
    let action_label = if action_rect.width
        >= u16::try_from(UnicodeWidthStr::width(text::TODO_ROLLOVER_ACTION_LABEL)).unwrap_or(15)
    {
        text::TODO_ROLLOVER_ACTION_LABEL
    } else {
        text::TODO_ROLLOVER_SHORT_ACTION_LABEL
    };
    write_inline(
        buf,
        action_rect,
        action_rect.x,
        action_label,
        style::todo_rollover_button(),
    );
    write_inline(
        buf,
        dismiss_rect,
        dismiss_rect.x,
        text::TODO_ROLLOVER_DISMISS_LABEL,
        style::muted(),
    );
}

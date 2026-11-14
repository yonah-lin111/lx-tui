//! 右侧容器 Todo 面板的语义样式。

use ratatui::style::{Color, Modifier, Style};

use crate::db::todo::{TodoPriority, TodoStatus};

use super::{accent, muted, right_tab_active, text};

/// 待办行状态框：正文色（未开始，不强调）。
pub fn todo_status_open() -> Style {
    text()
}

/// 进行中行状态框与文本：强调色（与 prompt 焦点同色系，一眼可见）。
pub fn todo_status_active() -> Style {
    accent()
}

/// 已完成行：muted + 删除线。
pub fn todo_status_done() -> Style {
    muted().add_modifier(Modifier::CROSSED_OUT)
}

/// 聚合组树形连接符样式（`┌─ / ├─ / └─ / │`）：跟随整组状态着色，但已完成时不加删除线以免糊掉框线。
pub fn todo_group_connector(status: TodoStatus) -> Style {
    match status {
        TodoStatus::Todo => text(),
        TodoStatus::InProgress => accent(),
        TodoStatus::Done => muted(),
    }
}

/// 周感知带：已完成计数（Green，与 agent 成功标记同一语义色）。
pub fn todo_bar_done() -> Style {
    Style::default().fg(Color::Green)
}

/// 优先级徽标样式（各档拥有专属深暗背景色与高对比度明亮前景色，互不干扰可读性；已完成时压暗）。
pub fn todo_priority_badge(priority: TodoPriority, status: TodoStatus) -> Style {
    if status == TodoStatus::Done {
        return todo_status_done();
    }
    match priority {
        TodoPriority::P0 => Style::default()
            .fg(Color::Indexed(210))
            .bg(Color::Indexed(52))
            .add_modifier(Modifier::BOLD),
        TodoPriority::P1 => Style::default()
            .fg(Color::Indexed(221))
            .bg(Color::Indexed(58))
            .add_modifier(Modifier::BOLD),
        TodoPriority::P2 => Style::default()
            .fg(Color::Indexed(117))
            .bg(Color::Indexed(24))
            .add_modifier(Modifier::BOLD),
        TodoPriority::P3 => Style::default()
            .fg(Color::Indexed(250))
            .bg(Color::Indexed(236)),
    }
}

/// 周感知带：当前选中日期高亮块。
pub fn todo_week_day_selected() -> Style {
    right_tab_active()
}

/// 周感知带：今日标记色。
pub fn todo_week_day_today() -> Style {
    accent()
}

/// 顺延横幅提示文字样式。
pub fn todo_rollover_banner() -> Style {
    Style::default().fg(Color::Indexed(221))
}

/// 顺延横幅动作按钮样式。
pub fn todo_rollover_button() -> Style {
    accent()
}

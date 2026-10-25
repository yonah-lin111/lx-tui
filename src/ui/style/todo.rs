//! 右侧容器 Todo 面板的语义样式。

use ratatui::style::{Color, Modifier, Style};

use super::{accent, muted, text};

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

/// 统计柱图：未开始（次要信息，与行内 `[ ]` 同口径）。
pub fn todo_bar_open() -> Style {
    muted()
}

/// 统计柱图：进行中（与行内 `[~]`、分段标签同色）。
pub fn todo_bar_active() -> Style {
    accent()
}

/// 统计柱图：已完成（Green，与 agent 成功标记同一语义色）。
pub fn todo_bar_done() -> Style {
    Style::default().fg(Color::Green)
}

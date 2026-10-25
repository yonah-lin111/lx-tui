//! 右侧容器 Todo 面板的语义样式。

use ratatui::style::{Modifier, Style};

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

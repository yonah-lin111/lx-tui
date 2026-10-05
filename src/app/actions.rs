//! 输入行为定义；应用级快捷键仅保留退出，其余交互由鼠标承担。

/// 一次可应用的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleSidebar,
    TogglePrompt,
    ToggleAgents,
}

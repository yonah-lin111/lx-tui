//! 输入行为定义。

use crate::layout::NavDirection;

/// 一次可应用的行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    FocusNextPane,
    FocusPrevPane,
    MoveFocus(NavDirection),
    NextTab,
    PrevTab,
    SelectWorkspace(usize),
    ToggleSidebar,
    ToggleHelp,
    CloseOverlay,
}

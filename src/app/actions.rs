//! 输入行为定义；应用级快捷键仅保留退出，其余编辑键仅在 prompt 聚焦时生效。

/// 一次可应用的应用级行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleSidebar,
    TogglePrompt,
    ToggleAgents,
}

/// prompt 编辑器的输入命令；仅在 prompt 获得焦点时产生。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorCommand {
    InsertChar(char),
    InsertText(String),
    Newline,
    NewlineBelow,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    LineStart,
    LineEnd,
    WordLeft,
    WordRight,
    DeleteToLineStart,
    DeleteToLineEnd,
    DeleteWordBackward,
    DeleteWordForward,
    Indent,
    Outdent,
    Undo,
    Redo,
    Escape,
}

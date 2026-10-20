//! 输入行为定义；应用级快捷键仅保留退出，其余编辑键仅在 prompt 聚焦时生效。

/// 一次可应用的应用级行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    ToggleSidebar,
    TogglePrompt,
    /// 折叠/展开右列主区（prompt 在左时）。
    TogglePanes,
    ToggleAgents,
}

/// 浮层按键；仅在浮层打开时产生，按浮层种类过滤后交给 update。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKey {
    Esc,
    Up,
    Down,
    Enter,
    Char(char),
    Clear,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    /// PageUp / PageDown：UI 组件查看弹窗右侧预览翻页。
    PageUp,
    PageDown,
    /// Shift+Enter：引用文件夹内容弹窗进入高亮目录（沿用 @ 面板语义）。
    EnterFolder,
    /// Ctrl+Z：引用文件夹内容弹窗回退上层搜索范围。
    Back,
    /// Ctrl+C：引用文件夹内容弹窗复制高亮项引用（沿用参考面板的高亮项复制快捷键）。
    Copy,
    /// Ctrl/Cmd+D 或 Del：引用文件夹目录弹窗移除高亮记录（打开删除确认）。
    Remove,
}

/// lx 页输入命令；仅在焦点窗格显示 lx 页时产生。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LxInput {
    Char(char),
    /// Shift+Enter / Ctrl+Enter：换行（Enter 预留给发送）。
    Newline,
    /// Shift+Enter：`@` 提及面板高亮目录可进入时进入该目录；未消费时同 `Newline`。
    EnterFolder,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Ctrl+A：移到逻辑行首。
    LineStart,
    /// Ctrl+E：移到逻辑行尾。
    LineEnd,
    /// Ctrl+← / Alt+B：左移一个词。
    WordLeft,
    /// Ctrl+→ / Alt+F：右移一个词。
    WordRight,
    /// Ctrl+U：删除到逻辑行首。
    DeleteToLineStart,
    /// Ctrl+K：删除到逻辑行尾。
    DeleteToLineEnd,
    /// Ctrl+W / Ctrl+Backspace / Alt+Backspace：向前删词。
    DeleteWordBackward,
    /// Ctrl+Delete / Alt+D：向后删词。
    DeleteWordForward,
    /// Ctrl+Z：撤销。
    Undo,
    /// Ctrl+Y / Ctrl+Shift+Z：重做。
    Redo,
    /// Ctrl+C：清空输入框。
    Clear,
    /// Tab：md 面板打开时采纳高亮候选。
    Tab,
    /// Enter：md 面板打开时采纳高亮候选；否则预留给发送（当前不动作）。
    Enter,
    /// Esc：关闭 md 面板；无面板且在流中时停止生成。
    Escape,
    /// Ctrl+T：切换最近 thinking 块展开/折叠。
    ToggleThinking,
    /// Ctrl+O：切换当前（或最近）工具卡片详情展开。
    ToggleToolDetail,
    /// Ctrl+D：停止后台任务面板里高亮的进程。
    ///
    /// **不能复用 `Delete`**：裸 `Delete` 键也映射到 `Delete`（行内向后删字符），
    /// 面板里把 `Delete` 当停止用会让用户「想删一行」时 SIGTERM 掉一个正在跑的服务。
    StopProcess,
    /// Alt+↑：按时间线选择上一张可展开工具卡片。
    SelectPrevTool,
    /// Alt+↓：按时间线选择下一张可展开工具卡片。
    SelectNextTool,
    /// PageUp / PageDown：后台任务面板里翻滚动上方的消息列表。
    PageUp,
    PageDown,
}

/// prompt 编辑器的输入命令；仅在 prompt 获得焦点时产生。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorCommand {
    InsertChar(char),
    InsertText(String),
    Newline,
    NewlineBelow,
    /// Shift+Enter：@ 面板高亮目录可进入时进入；未消费时同 `NewlineBelow`。
    EnterFolder,
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

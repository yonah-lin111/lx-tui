//! 键盘路由：Ctrl+Q 退出；浮层打开时按键按浮层种类过滤；
//! prompt 聚焦时按键映射为编辑命令，否则原样编码进焦点窗格。

pub mod encode;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::actions::{Action, EditorCommand, LxInput, OverlayKey};
use crate::app::overlay::OverlayKind;
use crate::app::state::FocusedContainer;

/// 路由结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Routed {
    /// 应用行为。
    Action(Action),
    /// prompt 编辑器命令。
    Editor(EditorCommand),
    /// 浮层按键。
    Overlay(OverlayKey),
    /// 复制 prompt 选区（无选区为全文）到系统剪贴板。
    Copy,
    /// 剪切 prompt 选区（树/页搜索框或正文）到系统剪贴板。
    Cut,
    /// 立即保存 prompt（Ctrl/Cmd+S）：条目落库；临时提示词走保存选择器。
    SavePrompt,
    /// 复制光标所在模板块/补充块正文到系统剪贴板。
    CopyTemplateBlock,
    /// lx 页单行输入命令。
    Lx(LxInput),
    /// todo 面板按键（原样翻译，语义由 `update::apply_todo_key` 按是否编辑中判定）。
    Todo(TodoKey),
    /// 原样写入焦点窗格的字节。
    Pane(Vec<u8>),
}

/// todo 面板按键：**按键原样**，不做语义解释。
///
/// 「`Space` 是循环状态还是输入空格」「`Enter` 是插入新行还是提交」
/// 全部由 `update::apply_todo_key` 按「是否正在编辑」判定，
/// 输入层不感知应用状态（同 lx 输入「清空还是复制由 update 决定」的既有口径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoKey {
    Char(char),
    /// 裸空格（不带修饰键）。
    Space,
    Enter,
    /// Shift+Enter：列表态新建一条（与 `Enter` 的「编辑当前项」区分）。
    ShiftEnter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Ctrl+A：移到行首。
    LineStart,
    /// Ctrl/Cmd+Shift+A：全选。
    SelectAll,
    /// Ctrl+E：移到行尾。
    LineEnd,
    /// Ctrl+U：删除至行首。
    DeleteToLineStart,
    /// Ctrl+K：删除至行尾。
    DeleteToLineEnd,
    /// Ctrl/Cmd+Z：撤销文本编辑。
    Undo,
    /// Ctrl+Shift+Z / Ctrl+Y：重做文本编辑。
    Redo,
    /// Ctrl/Cmd+C：编辑态清空输入（列表态吞掉）。
    Clear,
    /// Ctrl/Cmd+X：编辑态剪切选区（列表态吞掉）。
    Cut,
    /// Ctrl/Alt+← / Alt+B：左移一个词。
    WordLeft,
    /// Ctrl/Alt+→ / Alt+F：右移一个词。
    WordRight,
    /// Ctrl+W / Ctrl/Alt+Backspace：向前删词。
    DeleteWordBackward,
    /// Alt+D / Ctrl+Delete：向后删词。
    DeleteWordForward,
}

/// todo 按键映射：只做逐键翻译；未识别的键返回 None（由 `route` 吞掉）。
fn todo_key(key: KeyEvent) -> Option<TodoKey> {
    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::SUPER);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    let alt = modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char('z' | 'Z') if ctrl && shift && !alt => Some(TodoKey::Redo),
        KeyCode::Char('z' | 'Z') if ctrl && !alt => Some(TodoKey::Undo),
        KeyCode::Char('y' | 'Y') if ctrl && !alt => Some(TodoKey::Redo),
        KeyCode::Char('c' | 'C') if ctrl && !alt => Some(TodoKey::Clear),
        KeyCode::Char('x' | 'X') if ctrl && !alt => Some(TodoKey::Cut),
        KeyCode::Char('a' | 'A') if ctrl && shift && !alt => Some(TodoKey::SelectAll),
        KeyCode::Char('a' | 'A') if ctrl && !alt => Some(TodoKey::LineStart),
        KeyCode::Char('e' | 'E') if ctrl && !alt => Some(TodoKey::LineEnd),
        KeyCode::Char('u' | 'U') if ctrl && !alt => Some(TodoKey::DeleteToLineStart),
        KeyCode::Char('k' | 'K') if ctrl && !alt => Some(TodoKey::DeleteToLineEnd),
        KeyCode::Char('w' | 'W') if ctrl && !alt => Some(TodoKey::DeleteWordBackward),
        KeyCode::Char('b' | 'B') if alt && !ctrl => Some(TodoKey::WordLeft),
        KeyCode::Char('f' | 'F') if alt && !ctrl => Some(TodoKey::WordRight),
        KeyCode::Char('d' | 'D') if alt && !ctrl => Some(TodoKey::DeleteWordForward),
        KeyCode::Char(' ') if !ctrl && !alt => Some(TodoKey::Space),
        KeyCode::Char(ch) if !ctrl && !alt => Some(TodoKey::Char(ch)),
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => Some(TodoKey::ShiftEnter),
        KeyCode::Enter => Some(TodoKey::Enter),
        KeyCode::Esc => Some(TodoKey::Esc),
        KeyCode::Tab => Some(TodoKey::Tab),
        KeyCode::Backspace if ctrl || alt => Some(TodoKey::DeleteWordBackward),
        KeyCode::Delete if ctrl => Some(TodoKey::DeleteWordForward),
        KeyCode::Backspace => Some(TodoKey::Backspace),
        KeyCode::Delete => Some(TodoKey::Delete),
        KeyCode::Left if ctrl || alt => Some(TodoKey::WordLeft),
        KeyCode::Right if ctrl || alt => Some(TodoKey::WordRight),
        KeyCode::Left => Some(TodoKey::Left),
        KeyCode::Right => Some(TodoKey::Right),
        KeyCode::Up => Some(TodoKey::Up),
        KeyCode::Down => Some(TodoKey::Down),
        KeyCode::Home => Some(TodoKey::Home),
        KeyCode::End => Some(TodoKey::End),
        _ => None,
    }
}

/// 路由一次按键；Release 事件与无法编码的按键返回 None。
///
/// `Ctrl+Q` 始终优先；浮层打开时只放行该浮层支持的键，其余吞掉；
/// 否则按**唯一焦点源** `focus` 决定去向，同一时刻只有一个容器收键：
/// - `Prompt`：按键只进入编辑器（`Ctrl/Cmd+C` 复制选区全文、`Ctrl/Cmd+Shift+C`
///   复制光标所在模板块正文），未映射键吞掉，绝不写入 PTY；
/// - `Todo`：按键只进入 todo 面板，未映射键吞掉；
/// - `Panes`：`pane_lx` 为真时进入 lx 单行输入，否则编码写进焦点窗格的 PTY；
/// - `Sidebar`：侧栏没有键盘交互，全部吞掉——绝不穿透到窗格。
pub fn route(
    key: KeyEvent,
    mode: TermMode,
    focus: FocusedContainer,
    overlay: Option<OverlayKind>,
    pane_lx: bool,
) -> Option<Routed> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Routed::Action(Action::Quit));
    }
    if let Some(kind) = overlay {
        return overlay_key(key, kind).map(Routed::Overlay);
    }
    if focus == FocusedContainer::Sidebar {
        // 侧栏只有鼠标交互：键盘一律吞掉，避免误写进别的容器的输入框。
        return None;
    }
    if focus == FocusedContainer::Prompt {
        let copy_chord = matches!(key.code, KeyCode::Char('c' | 'C'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !key.modifiers.contains(KeyModifiers::ALT);
        if copy_chord {
            if key.modifiers.contains(KeyModifiers::SHIFT) {
                return Some(Routed::CopyTemplateBlock);
            }
            return Some(Routed::Copy);
        }
        let cut_chord = matches!(key.code, KeyCode::Char('x' | 'X'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !key
                .modifiers
                .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT);
        if cut_chord {
            return Some(Routed::Cut);
        }
        let save_chord = matches!(key.code, KeyCode::Char('s' | 'S'))
            && (key.modifiers.contains(KeyModifiers::CONTROL)
                || key.modifiers.contains(KeyModifiers::SUPER))
            && !key.modifiers.contains(KeyModifiers::ALT);
        if save_chord {
            return Some(Routed::SavePrompt);
        }
        return match editor_command(key) {
            Some(command) => Some(Routed::Editor(command)),
            None => {
                // 终端编码差异排查用：prompt 聚焦时被吞掉的按键（仅 debug 级）。
                tracing::debug!(?key, "prompt key swallowed");
                None
            }
        };
    }
    if focus == FocusedContainer::Todo {
        // todo 面板持焦点：未映射键一律吞掉（同 prompt 口径），绝不写进隐藏终端。
        return todo_key(key).map(Routed::Todo);
    }
    if pane_lx {
        // 长按 Esc/Enter 的 Repeat 不推进撤销待命与二次确认，也不重复发送。
        if key.kind == KeyEventKind::Repeat && matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            return None;
        }
        return lx_input_key(key).map(Routed::Lx);
    }
    encode::encode_key(key, mode).map(Routed::Pane)
}

/// lx 页输入键映射：Shift+Enter 换行、单行编辑键、md 面板键；其余（含 Enter）吞掉。
fn lx_input_key(key: KeyEvent) -> Option<LxInput> {
    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    if modifiers.contains(KeyModifiers::SUPER) {
        return match key.code {
            KeyCode::Char('z' | 'Z') if shift => Some(LxInput::Redo),
            KeyCode::Char('z' | 'Z') => Some(LxInput::Undo),
            KeyCode::Char('y' | 'Y') => Some(LxInput::Redo),
            // Cmd+C 与 Ctrl+C 落到同一命令：是否「清空」还是「复制」由 update 层按
            // 当前面板决定，输入层不感知状态。
            KeyCode::Char('c' | 'C') => Some(LxInput::Clear),
            // Cmd+X：剪切输入框选区。
            KeyCode::Char('x' | 'X') => Some(LxInput::Cut),
            // macOS 原生编辑：Cmd+A 回行首，Cmd+Shift+A 全选；Cmd+←/→ 行首尾、Cmd+↑/↓ 文首尾。
            KeyCode::Char('a' | 'A') if shift => Some(LxInput::SelectAll),
            KeyCode::Char('a' | 'A') => Some(LxInput::LineStart),
            KeyCode::Left if shift => Some(LxInput::SelectHome),
            KeyCode::Right if shift => Some(LxInput::SelectEnd),
            KeyCode::Left => Some(LxInput::LineStart),
            KeyCode::Right => Some(LxInput::LineEnd),
            KeyCode::Up if shift => Some(LxInput::SelectDocStart),
            KeyCode::Down if shift => Some(LxInput::SelectDocEnd),
            KeyCode::Up | KeyCode::Home => Some(LxInput::DocStart),
            KeyCode::Down | KeyCode::End => Some(LxInput::DocEnd),
            KeyCode::Backspace => Some(LxInput::DeleteToLineStart),
            KeyCode::Delete => Some(LxInput::DeleteToLineEnd),
            _ => None,
        };
    }
    // Shift 选区：方向键扩展光标；Ctrl/Alt+Shift+方向键按词扩展，Home/End 扩到文首尾。
    if shift {
        if ctrl && !alt && matches!(key.code, KeyCode::Char('a' | 'A')) {
            return Some(LxInput::SelectAll);
        }
        match (ctrl || alt, key.code) {
            (true, KeyCode::Left) | (true, KeyCode::Char('b' | 'B')) => {
                return Some(LxInput::SelectWordLeft);
            }
            (true, KeyCode::Right) | (true, KeyCode::Char('f' | 'F')) => {
                return Some(LxInput::SelectWordRight);
            }
            (true, KeyCode::Home) => return Some(LxInput::SelectDocStart),
            (true, KeyCode::End) => return Some(LxInput::SelectDocEnd),
            (false, KeyCode::Left) => return Some(LxInput::SelectLeft),
            (false, KeyCode::Right) => return Some(LxInput::SelectRight),
            (false, KeyCode::Up) => return Some(LxInput::SelectUp),
            (false, KeyCode::Down) => return Some(LxInput::SelectDown),
            (false, KeyCode::Home) => return Some(LxInput::SelectHome),
            (false, KeyCode::End) => return Some(LxInput::SelectEnd),
            _ => {}
        }
    }
    let mapped = match (ctrl, alt, key.code) {
        (true, false, KeyCode::Enter) => Some(LxInput::Newline),
        (true, false, KeyCode::Home) => Some(LxInput::DocStart),
        (true, false, KeyCode::End) => Some(LxInput::DocEnd),
        (true, false, KeyCode::Char(ch)) => match ch {
            'o' | 'O' => Some(LxInput::ToggleToolDetail),
            'u' | 'U' => Some(LxInput::DeleteToLineStart),
            'k' | 'K' => Some(LxInput::DeleteToLineEnd),
            'w' | 'W' => Some(LxInput::DeleteWordBackward),
            'a' | 'A' => Some(LxInput::LineStart),
            'e' | 'E' => Some(LxInput::LineEnd),
            'd' | 'D' => Some(LxInput::StopProcess),
            'c' | 'C' => Some(LxInput::Clear),
            'x' | 'X' => Some(LxInput::Cut),
            't' | 'T' => Some(LxInput::ToggleThinking),
            'z' | 'Z' if shift => Some(LxInput::Redo),
            'z' | '-' => Some(LxInput::Undo),
            'Z' | 'y' | 'Y' | '.' => Some(LxInput::Redo),
            _ => None,
        },
        (true, false, KeyCode::Backspace) => Some(LxInput::DeleteWordBackward),
        (true, false, KeyCode::Delete) => Some(LxInput::DeleteWordForward),
        (true, false, KeyCode::Left) => Some(LxInput::WordLeft),
        (true, false, KeyCode::Right) => Some(LxInput::WordRight),
        // Alt+↑/↓：在可展开工具卡片之间移动键盘选择（草稿与光标保持不变）。
        (false, true, KeyCode::Up) => Some(LxInput::SelectPrevTool),
        (false, true, KeyCode::Down) => Some(LxInput::SelectNextTool),
        (false, true, KeyCode::Char('b')) => Some(LxInput::WordLeft),
        (false, true, KeyCode::Char('f')) => Some(LxInput::WordRight),
        (false, true, KeyCode::Char('d')) => Some(LxInput::DeleteWordForward),
        (false, true, KeyCode::Backspace) => Some(LxInput::DeleteWordBackward),
        (false, true, KeyCode::Left) => Some(LxInput::WordLeft),
        (false, true, KeyCode::Right) => Some(LxInput::WordRight),
        _ => None,
    };
    if mapped.is_some() {
        return mapped;
    }
    if ctrl || alt {
        return None;
    }
    match key.code {
        // Shift+Enter：提及面板高亮目录时进入；未消费时在 update 层回落为换行。
        KeyCode::Enter if shift => Some(LxInput::EnterFolder),
        // Enter 无面板时预留给发送，交给 update 判定。
        KeyCode::Enter => Some(LxInput::Enter),
        KeyCode::Backspace => Some(LxInput::Backspace),
        KeyCode::Delete => Some(LxInput::Delete),
        KeyCode::Left => Some(LxInput::Left),
        KeyCode::Right => Some(LxInput::Right),
        KeyCode::Up => Some(LxInput::Up),
        KeyCode::Down => Some(LxInput::Down),
        KeyCode::Home => Some(LxInput::Home),
        KeyCode::End => Some(LxInput::End),
        // Shift+Tab 与 `←` 同为「上一步」；lx 其它形态没有该语义。
        // 终端可能报 `Tab+SHIFT`（kitty 协议）或 `BackTab`，两种都要接。
        KeyCode::Tab if shift => Some(LxInput::PrevStep),
        KeyCode::BackTab => Some(LxInput::PrevStep),
        KeyCode::Tab => Some(LxInput::Tab),
        KeyCode::PageUp => Some(LxInput::PageUp),
        KeyCode::PageDown => Some(LxInput::PageDown),
        KeyCode::Esc => Some(LxInput::Escape),
        KeyCode::Char(ch) => Some(LxInput::Char(ch)),
        _ => None,
    }
}

/// 单行输入浮层共用的文本编辑按键映射。
fn overlay_text_edit_key(key: KeyEvent) -> Option<OverlayKey> {
    let plain = !key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let ctrl = key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER)
        && !key.modifiers.contains(KeyModifiers::ALT);
    let alt_only = key.modifiers == KeyModifiers::ALT;
    match key.code {
        KeyCode::Char('z' | 'Z') if ctrl && shift => Some(OverlayKey::Redo),
        KeyCode::Char('z' | 'Z') if ctrl => Some(OverlayKey::Undo),
        KeyCode::Char('y' | 'Y') if ctrl => Some(OverlayKey::Redo),
        KeyCode::Char('c' | 'C') if ctrl => Some(OverlayKey::Clear),
        KeyCode::Char('x' | 'X') if ctrl => Some(OverlayKey::Cut),
        KeyCode::Char('a' | 'A') if ctrl && shift => Some(OverlayKey::SelectAll),
        KeyCode::Char('a' | 'A') if ctrl => Some(OverlayKey::LineStart),
        KeyCode::Char('e' | 'E') if ctrl => Some(OverlayKey::LineEnd),
        KeyCode::Char('u' | 'U') if ctrl => Some(OverlayKey::DeleteToLineStart),
        KeyCode::Char('k' | 'K') if ctrl => Some(OverlayKey::DeleteToLineEnd),
        KeyCode::Char('w' | 'W') if ctrl => Some(OverlayKey::DeleteWordBackward),
        KeyCode::Char('b' | 'B') if alt_only => Some(OverlayKey::WordLeft),
        KeyCode::Char('f' | 'F') if alt_only => Some(OverlayKey::WordRight),
        KeyCode::Char('d' | 'D') if alt_only => Some(OverlayKey::DeleteWordForward),
        KeyCode::Backspace if ctrl || alt_only => Some(OverlayKey::DeleteWordBackward),
        KeyCode::Delete if ctrl => Some(OverlayKey::DeleteWordForward),
        KeyCode::Backspace => Some(OverlayKey::Backspace),
        KeyCode::Delete => Some(OverlayKey::Delete),
        KeyCode::Left if ctrl || alt_only => Some(OverlayKey::WordLeft),
        KeyCode::Right if ctrl || alt_only => Some(OverlayKey::WordRight),
        KeyCode::Left if plain => Some(OverlayKey::Left),
        KeyCode::Right if plain => Some(OverlayKey::Right),
        KeyCode::Home => Some(OverlayKey::Home),
        KeyCode::End => Some(OverlayKey::End),
        KeyCode::Char(ch) if plain => Some(OverlayKey::Char(ch)),
        _ => None,
    }
}

/// 浮层按键映射：只放行该浮层支持的键，其余返回 None（吞掉）。
fn overlay_key(key: KeyEvent, kind: OverlayKind) -> Option<OverlayKey> {
    let ctrl = key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::SUPER)
        && !key.modifiers.contains(KeyModifiers::ALT);
    match kind {
        OverlayKind::Menu => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => None,
        },
        // 引用文件夹目录弹窗：上下移动、回车打开；Ctrl/Cmd+C 复制引用、
        // Ctrl/Cmd+D 或 Del 移除记录（沿用弹窗内快捷键语义，行内不再放动作按钮）。
        OverlayKind::ReferencedFolderMenu => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            KeyCode::Enter => Some(OverlayKey::Enter),
            KeyCode::Delete => Some(OverlayKey::Remove),
            KeyCode::Char('c' | 'C') if ctrl => Some(OverlayKey::Copy),
            KeyCode::Char('d' | 'D') if ctrl => Some(OverlayKey::Remove),
            _ => None,
        },
        OverlayKind::ReferencedFolderDelete => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => None,
        },
        OverlayKind::ReferencedFolderInput => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => overlay_text_edit_key(key),
        },
        // 内容搜索：Shift+Enter 进目录，Ctrl/Cmd+C 复制高亮项引用（有选区时由事件层
        // 优先复制选区）；其余编辑键统一走单行输入映射（Ctrl+Z 优先撤销搜索词）。
        OverlayKind::ReferencedFolderSearch => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                Some(OverlayKey::EnterFolder)
            }
            KeyCode::Enter => Some(OverlayKey::Enter),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            KeyCode::Char('c' | 'C') if ctrl => Some(OverlayKey::Copy),
            _ => overlay_text_edit_key(key),
        },
        OverlayKind::ConfirmClose | OverlayKind::TrustWorkspace => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => None,
        },
        OverlayKind::ResumeSessionEdit => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            // `^D` 进入删除确认 / 确认删除。
            KeyCode::Char('d' | 'D') if ctrl => Some(OverlayKey::Remove),
            _ => overlay_text_edit_key(key),
        },
        OverlayKind::Rename | OverlayKind::NewWorkspace | OverlayKind::NewNode => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            _ => overlay_text_edit_key(key),
        },
        OverlayKind::WorktreeOpen | OverlayKind::SaveDraft => match key.code {
            KeyCode::Esc => Some(OverlayKey::Esc),
            KeyCode::Enter => Some(OverlayKey::Enter),
            KeyCode::Up => Some(OverlayKey::Up),
            KeyCode::Down => Some(OverlayKey::Down),
            _ => overlay_text_edit_key(key),
        },
    }
}

/// 编辑键映射；Shift 用于字符输入、@ 面板目录导航与 `Ctrl/Cmd+Enter` 变体。
///
/// `Shift+Enter` 映射为 @ 面板目录进入命令；面板未消费时回落为行尾另起一行
/// （见 `update::apply_editor`）。@ 面板的目录回退走编辑器撤销（`Ctrl+Z` / `Cmd+Z`）。
/// `Ctrl/Cmd+Shift+Enter` 与 `Ctrl/Cmd+Enter` 同为行尾另起一行并保留行首缩进
/// （对齐 lx-agent 的 `Ctrl/Cmd+Shift+Enter`）。
/// Ctrl/Alt 组合同 opencode/readline：Ctrl+U 删除到逻辑行首（已在行首时删除前一个换行）、
/// Ctrl+K 删除到行尾、Ctrl+W 向前删词、
/// Ctrl+A/E 逻辑行首尾、Ctrl+D 正向删除、Ctrl/Cmd+←/→ 切换上一页/下一页；撤销/重做用
/// Ctrl+Z / Ctrl+Y（legacy 终端可靠），并对齐 opencode 接受 Ctrl+Shift+Z、Ctrl+-、
/// Ctrl+. 与 Super 组合（终端支持才收得到）；Alt+B/F、Alt+←/→ 按词移动，Alt+D 向前删词；
/// Tab/BackTab 缩进/反缩进；其余 Ctrl/Alt 组合吞掉。
fn editor_command(key: KeyEvent) -> Option<EditorCommand> {
    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    if modifiers.contains(KeyModifiers::SUPER) && !alt {
        return match key.code {
            KeyCode::Enter => Some(EditorCommand::NewlineBelow),
            KeyCode::Left => Some(EditorCommand::PagePrev),
            KeyCode::Right if shift => Some(EditorCommand::NewPage),
            KeyCode::Right => Some(EditorCommand::PageNext),
            KeyCode::Char('l' | 'L') if !shift => Some(EditorCommand::TogglePromptList),
            KeyCode::Char('p' | 'P') if !shift => Some(EditorCommand::TogglePageList),
            KeyCode::Char('r' | 'R') if !shift => Some(EditorCommand::ToggleRecentPages),
            KeyCode::Char('z' | 'Z') if shift => Some(EditorCommand::Redo),
            KeyCode::Char('z' | 'Z') => Some(EditorCommand::Undo),
            KeyCode::Char('y' | 'Y') => Some(EditorCommand::Redo),
            KeyCode::Char('a' | 'A') if shift => Some(EditorCommand::SelectAll),
            KeyCode::Char('a' | 'A') => Some(EditorCommand::LineStart),
            _ => None,
        };
    }
    let command = match (ctrl, alt, key.code) {
        (true, false, KeyCode::Enter) => Some(EditorCommand::NewlineBelow),
        (true, false, KeyCode::Char(ch)) => match ch {
            'l' | 'L' if !shift => Some(EditorCommand::TogglePromptList),
            'p' | 'P' if !shift => Some(EditorCommand::TogglePageList),
            'r' | 'R' if !shift => Some(EditorCommand::ToggleRecentPages),
            'u' | 'U' => Some(EditorCommand::DeleteToLineStart),
            'k' | 'K' => Some(EditorCommand::DeleteToLineEnd),
            'w' | 'W' => Some(EditorCommand::DeleteWordBackward),
            'a' | 'A' if shift => Some(EditorCommand::SelectAll),
            'a' | 'A' => Some(EditorCommand::LineStart),
            'e' | 'E' => Some(EditorCommand::LineEnd),
            'd' | 'D' => Some(EditorCommand::Delete),
            'z' | 'Z' if shift => Some(EditorCommand::Redo),
            'z' | '-' => Some(EditorCommand::Undo),
            'Z' | 'y' | 'Y' | '.' => Some(EditorCommand::Redo),
            _ => None,
        },
        (true, false, KeyCode::Backspace) => Some(EditorCommand::DeleteWordBackward),
        (true, false, KeyCode::Delete) => Some(EditorCommand::DeleteWordForward),
        (true, false, KeyCode::Left) => Some(EditorCommand::PagePrev),
        (true, false, KeyCode::Right) if shift => Some(EditorCommand::NewPage),
        (true, false, KeyCode::Right) => Some(EditorCommand::PageNext),
        (false, true, KeyCode::Char('b')) => Some(EditorCommand::WordLeft),
        (false, true, KeyCode::Char('f')) => Some(EditorCommand::WordRight),
        (false, true, KeyCode::Char('d')) => Some(EditorCommand::DeleteWordForward),
        (false, true, KeyCode::Backspace) => Some(EditorCommand::DeleteWordBackward),
        (false, true, KeyCode::Left) => Some(EditorCommand::WordLeft),
        (false, true, KeyCode::Right) => Some(EditorCommand::WordRight),
        _ => None,
    };
    if command.is_some() {
        return command;
    }
    if ctrl || alt {
        return None;
    }
    match key.code {
        KeyCode::Char(ch) => Some(EditorCommand::InsertChar(ch)),
        KeyCode::Enter if shift => Some(EditorCommand::EnterFolder),
        KeyCode::Enter => Some(EditorCommand::Newline),
        KeyCode::Backspace => Some(EditorCommand::Backspace),
        KeyCode::Delete => Some(EditorCommand::Delete),
        KeyCode::Esc if modifiers.is_empty() => Some(EditorCommand::Escape),
        KeyCode::Tab if modifiers.is_empty() => Some(EditorCommand::Indent),
        KeyCode::Tab if modifiers == KeyModifiers::SHIFT => Some(EditorCommand::Outdent),
        KeyCode::BackTab => Some(EditorCommand::Outdent),
        KeyCode::Left if modifiers.is_empty() => Some(EditorCommand::Left),
        KeyCode::Right if modifiers.is_empty() => Some(EditorCommand::Right),
        KeyCode::Up if modifiers.is_empty() => Some(EditorCommand::Up),
        KeyCode::Down if modifiers.is_empty() => Some(EditorCommand::Down),
        KeyCode::Home if modifiers.is_empty() => Some(EditorCommand::Home),
        KeyCode::End if modifiers.is_empty() => Some(EditorCommand::End),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod process_keys;

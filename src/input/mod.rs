//! 键盘路由：Ctrl+Q 退出；prompt 聚焦时按键映射为编辑命令，否则原样编码进焦点窗格。

pub mod encode;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::actions::{Action, EditorCommand};

/// 路由结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Routed {
    /// 应用行为。
    Action(Action),
    /// prompt 编辑器命令。
    Editor(EditorCommand),
    /// 原样写入焦点窗格的字节。
    Pane(Vec<u8>),
}

/// 路由一次按键；Release 事件与无法编码的按键返回 None。
///
/// `prompt_focused` 为真时按键只进入编辑器：未映射的按键被吞掉，绝不写入 PTY。
pub fn route(key: KeyEvent, mode: TermMode, prompt_focused: bool) -> Option<Routed> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Routed::Action(Action::Quit));
    }
    if prompt_focused {
        return editor_command(key).map(Routed::Editor);
    }
    encode::encode_key(key, mode).map(Routed::Pane)
}

/// 编辑键映射；Shift 只用于字符输入，导航键要求无修饰符。
///
/// Ctrl/Alt 组合同 opencode/readline：Ctrl+U/K 删除到行首/行尾、Ctrl+W 向前删词、
/// Ctrl+A/E 逻辑行首尾、Ctrl+D 正向删除、Ctrl+←/→ 按词移动；撤销/重做用
/// Ctrl+Z / Ctrl+Y（legacy 终端可靠），并对齐 opencode 接受 Ctrl+Shift+Z、Ctrl+-、
/// Ctrl+. 与 Super 组合（终端支持才收得到）；Alt+B/F、Alt+D 作为别名；
/// Tab/BackTab 缩进/反缩进；其余 Ctrl/Alt 组合吞掉。
fn editor_command(key: KeyEvent) -> Option<EditorCommand> {
    let modifiers = key.modifiers;
    let ctrl = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    if modifiers.contains(KeyModifiers::SUPER) {
        return match key.code {
            KeyCode::Enter if !shift => Some(EditorCommand::NewlineBelow),
            KeyCode::Char('z' | 'Z') if shift => Some(EditorCommand::Redo),
            KeyCode::Char('z' | 'Z') => Some(EditorCommand::Undo),
            KeyCode::Char('y' | 'Y') => Some(EditorCommand::Redo),
            _ => None,
        };
    }
    let command = match (ctrl, alt, key.code) {
        (true, false, KeyCode::Enter) if !shift => Some(EditorCommand::NewlineBelow),
        (true, false, KeyCode::Char(ch)) => match ch {
            'u' | 'U' => Some(EditorCommand::DeleteToLineStart),
            'k' | 'K' => Some(EditorCommand::DeleteToLineEnd),
            'w' | 'W' => Some(EditorCommand::DeleteWordBackward),
            'a' | 'A' => Some(EditorCommand::LineStart),
            'e' | 'E' => Some(EditorCommand::LineEnd),
            'd' | 'D' => Some(EditorCommand::Delete),
            'z' | '-' => Some(EditorCommand::Undo),
            'Z' | 'y' | 'Y' | '.' => Some(EditorCommand::Redo),
            _ => None,
        },
        (true, false, KeyCode::Backspace) => Some(EditorCommand::DeleteWordBackward),
        (true, false, KeyCode::Delete) => Some(EditorCommand::DeleteWordForward),
        (true, false, KeyCode::Left) => Some(EditorCommand::WordLeft),
        (true, false, KeyCode::Right) => Some(EditorCommand::WordRight),
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
        KeyCode::Enter if shift => Some(EditorCommand::NewlineBelow),
        KeyCode::Enter => Some(EditorCommand::Newline),
        KeyCode::Backspace => Some(EditorCommand::Backspace),
        KeyCode::Delete => Some(EditorCommand::Delete),
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

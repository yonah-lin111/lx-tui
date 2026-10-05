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
fn editor_command(key: KeyEvent) -> Option<EditorCommand> {
    let modifiers = key.modifiers;
    if modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
        return None;
    }
    match key.code {
        KeyCode::Char(ch) => Some(EditorCommand::InsertChar(ch)),
        KeyCode::Enter => Some(EditorCommand::Newline),
        KeyCode::Backspace => Some(EditorCommand::Backspace),
        KeyCode::Delete => Some(EditorCommand::Delete),
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

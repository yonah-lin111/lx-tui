//! 键盘路由：仅 Ctrl+C 退出应用，其余按键原样编码进焦点窗格。

pub mod encode;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::actions::Action;

/// 路由结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Routed {
    /// 应用行为。
    Action(Action),
    /// 原样写入焦点窗格的字节。
    Pane(Vec<u8>),
}

/// 路由一次按键；Release 事件与无法编码的按键返回 None。
pub fn route(key: KeyEvent, mode: TermMode) -> Option<Routed> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Routed::Action(Action::Quit));
    }
    encode::encode_key(key, mode).map(Routed::Pane)
}

#[cfg(test)]
mod tests;

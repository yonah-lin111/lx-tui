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
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn bare_keys_go_to_pane() {
        assert_eq!(
            route(key(KeyCode::Char('q')), TermMode::empty()),
            Some(Routed::Pane(b"q".to_vec()))
        );
        assert_eq!(
            route(key(KeyCode::Char('h')), TermMode::empty()),
            Some(Routed::Pane(b"h".to_vec()))
        );
    }

    #[test]
    fn ctrl_c_quits() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            route(key, TermMode::empty()),
            Some(Routed::Action(Action::Quit))
        );
    }

    #[test]
    fn ctrl_b_is_passed_to_pane() {
        let key = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
        assert_eq!(
            route(key, TermMode::empty()),
            Some(Routed::Pane(vec![0x02]))
        );
    }

    #[test]
    fn release_events_are_ignored() {
        let mut key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        key.kind = KeyEventKind::Release;
        assert_eq!(route(key, TermMode::empty()), None);
    }

    #[test]
    fn navigation_keys_encode_to_pane() {
        assert_eq!(
            route(key(KeyCode::Tab), TermMode::empty()),
            Some(Routed::Pane(b"\t".to_vec()))
        );
        assert_eq!(
            route(key(KeyCode::Up), TermMode::APP_CURSOR),
            Some(Routed::Pane(b"\x1bOA".to_vec()))
        );
    }
}

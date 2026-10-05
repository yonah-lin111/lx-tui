//! 键盘路由：prefix 状态机；普通按键原样编码进焦点窗格，应用动作只在前缀后生效。

pub mod encode;

use alacritty_terminal::term::TermMode;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::actions::Action;
use crate::layout::NavDirection;

/// 前缀键：Ctrl+b（tmux 惯例）。
const PREFIX_KEY: KeyCode = KeyCode::Char('b');
const PREFIX_MODIFIER: KeyModifiers = KeyModifiers::CONTROL;

/// 路由结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Routed {
    /// 应用行为。
    Action(Action),
    /// 原样写入焦点窗格的字节。
    Pane(Vec<u8>),
    /// 被前缀状态机消费，无动作。
    Consumed,
}

/// 前缀状态；由事件循环持有。
#[derive(Debug, Default)]
pub struct InputState {
    pending: bool,
}

impl InputState {
    /// 是否处于前缀待接状态。
    pub fn prefix_pending(&self) -> bool {
        self.pending
    }
}

/// 路由一次按键；Release 事件与无法编码的按键返回 None。
pub fn route(key: KeyEvent, state: &mut InputState, mode: TermMode) -> Option<Routed> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.pending = false;
        return Some(Routed::Action(Action::Quit));
    }
    if state.pending {
        state.pending = false;
        return Some(prefix_command(key));
    }
    if key.code == PREFIX_KEY && key.modifiers.contains(PREFIX_MODIFIER) {
        state.pending = true;
        return Some(Routed::Consumed);
    }
    encode::encode_key(key, mode).map(Routed::Pane)
}

/// 前缀后的按键；未绑定按键被消费，不透传。
fn prefix_command(key: KeyEvent) -> Routed {
    if key.code == PREFIX_KEY && key.modifiers.contains(PREFIX_MODIFIER) {
        // 连按两次前缀：透传字面 Ctrl+b。
        return Routed::Pane(vec![0x02]);
    }
    let action = match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) => Action::Quit,
        (KeyCode::Char('b'), _) => Action::ToggleSidebar,
        (KeyCode::Char('p'), _) => Action::TogglePrompt,
        (KeyCode::Tab, _) => Action::FocusNextPane,
        (KeyCode::BackTab, _) => Action::FocusPrevPane,
        (KeyCode::Char('h'), _) | (KeyCode::Left, _) => Action::MoveFocus(NavDirection::Left),
        (KeyCode::Char('j'), _) | (KeyCode::Down, _) => Action::MoveFocus(NavDirection::Down),
        (KeyCode::Char('k'), _) | (KeyCode::Up, _) => Action::MoveFocus(NavDirection::Up),
        (KeyCode::Char('l'), _) | (KeyCode::Right, _) => Action::MoveFocus(NavDirection::Right),
        (KeyCode::Char('['), _) => Action::PrevTab,
        (KeyCode::Char(']'), _) => Action::NextTab,
        (KeyCode::Char(digit), _) => match digit.to_digit(10) {
            Some(value @ 1..=9) => Action::SelectWorkspace((value - 1) as usize),
            _ => return Routed::Consumed,
        },
        _ => return Routed::Consumed,
    };
    Routed::Action(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn prefix_key() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
    }

    #[test]
    fn bare_keys_go_to_pane() {
        let mut state = InputState::default();
        assert_eq!(
            route(key(KeyCode::Char('q')), &mut state, TermMode::empty()),
            Some(Routed::Pane(b"q".to_vec()))
        );
        assert_eq!(
            route(key(KeyCode::Char('h')), &mut state, TermMode::empty()),
            Some(Routed::Pane(b"h".to_vec()))
        );
    }

    #[test]
    fn prefix_key_arms_then_executes() {
        let mut state = InputState::default();
        assert_eq!(
            route(prefix_key(), &mut state, TermMode::empty()),
            Some(Routed::Consumed)
        );
        assert!(state.prefix_pending());
        assert_eq!(
            route(key(KeyCode::Char('q')), &mut state, TermMode::empty()),
            Some(Routed::Action(Action::Quit))
        );
        assert!(!state.prefix_pending());
    }

    #[test]
    fn prefix_navigation_maps_to_actions() {
        let mut state = InputState::default();
        route(prefix_key(), &mut state, TermMode::empty());
        assert_eq!(
            route(key(KeyCode::Char('l')), &mut state, TermMode::empty()),
            Some(Routed::Action(Action::MoveFocus(NavDirection::Right)))
        );
        route(prefix_key(), &mut state, TermMode::empty());
        assert_eq!(
            route(key(KeyCode::Char('2')), &mut state, TermMode::empty()),
            Some(Routed::Action(Action::SelectWorkspace(1)))
        );
        route(prefix_key(), &mut state, TermMode::empty());
        assert_eq!(
            route(key(KeyCode::Char('p')), &mut state, TermMode::empty()),
            Some(Routed::Action(Action::TogglePrompt))
        );
    }

    #[test]
    fn double_prefix_passes_literal_control_key() {
        let mut state = InputState::default();
        route(prefix_key(), &mut state, TermMode::empty());
        assert_eq!(
            route(prefix_key(), &mut state, TermMode::empty()),
            Some(Routed::Pane(vec![0x02]))
        );
    }

    #[test]
    fn unbound_prefix_command_is_consumed() {
        let mut state = InputState::default();
        route(prefix_key(), &mut state, TermMode::empty());
        assert_eq!(
            route(key(KeyCode::Char('z')), &mut state, TermMode::empty()),
            Some(Routed::Consumed)
        );
    }

    #[test]
    fn ctrl_c_quits_immediately() {
        let mut state = InputState::default();
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(
            route(key, &mut state, TermMode::empty()),
            Some(Routed::Action(Action::Quit))
        );
        route(prefix_key(), &mut state, TermMode::empty());
        assert!(state.prefix_pending());
        assert_eq!(
            route(key, &mut state, TermMode::empty()),
            Some(Routed::Action(Action::Quit))
        );
        assert!(!state.prefix_pending());
    }

    #[test]
    fn navigation_keys_encode_to_pane_when_bare() {
        let mut state = InputState::default();
        assert_eq!(
            route(key(KeyCode::Tab), &mut state, TermMode::empty()),
            Some(Routed::Pane(b"\t".to_vec()))
        );
        assert_eq!(
            route(key(KeyCode::Up), &mut state, TermMode::APP_CURSOR),
            Some(Routed::Pane(b"\x1bOA".to_vec()))
        );
    }
}

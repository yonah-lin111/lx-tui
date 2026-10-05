//! 单元测试；仅测试构建编译。

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

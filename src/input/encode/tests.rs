//! 单元测试；仅测试构建编译。

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn encodes_plain_and_control_chars() {
    assert_eq!(
        encode_key(key(KeyCode::Char('a')), TermMode::empty()),
        Some(b"a".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
            TermMode::empty()
        ),
        Some(vec![0x01])
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
            TermMode::empty()
        ),
        Some(b"\x1b x".iter().copied().filter(|b| *b != b' ').collect())
    );
}

#[test]
fn arrows_follow_deckm_mode() {
    assert_eq!(
        encode_key(key(KeyCode::Up), TermMode::empty()),
        Some(b"\x1b[A".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Up), TermMode::APP_CURSOR),
        Some(b"\x1bOA".to_vec())
    );
}

#[test]
fn encodes_modified_navigation_keys() {
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL),
            TermMode::empty()
        ),
        Some(b"\x1b[1;5C".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::SHIFT),
            TermMode::empty()
        ),
        Some(b"\x1b[5;2~".to_vec())
    );
}

#[test]
fn encodes_function_keys() {
    assert_eq!(
        encode_key(key(KeyCode::F(1)), TermMode::empty()),
        Some(b"\x1bOP".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::F(5)), TermMode::empty()),
        Some(b"\x1b[15~".to_vec())
    );
}

#[test]
fn super_combos_are_not_encoded() {
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::SUPER),
            TermMode::empty()
        ),
        None
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SUPER | KeyModifiers::SHIFT),
            TermMode::empty()
        ),
        None
    );
}

#[test]
fn encodes_special_keys() {
    assert_eq!(
        encode_key(key(KeyCode::Enter), TermMode::empty()),
        Some(b"\r".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Backspace), TermMode::empty()),
        Some(vec![0x7f])
    );
    assert_eq!(
        encode_key(key(KeyCode::BackTab), TermMode::empty()),
        Some(b"\x1b[Z".to_vec())
    );
}

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
fn encodes_sgr_wheel_with_modifiers_and_one_based_coords() {
    assert_eq!(
        encode_mouse_wheel(
            MouseEventKind::ScrollUp,
            0,
            0,
            KeyModifiers::NONE,
            TermMode::SGR_MOUSE
        ),
        Some(b"\x1b[<64;1;1M".to_vec())
    );
    assert_eq!(
        encode_mouse_wheel(
            MouseEventKind::ScrollDown,
            4,
            6,
            KeyModifiers::SHIFT,
            TermMode::SGR_MOUSE
        ),
        Some(b"\x1b[<69;5;7M".to_vec())
    );
    assert_eq!(
        encode_mouse_wheel(
            MouseEventKind::ScrollLeft,
            2,
            3,
            KeyModifiers::CONTROL,
            TermMode::SGR_MOUSE
        ),
        Some(b"\x1b[<82;3;4M".to_vec())
    );
}

#[test]
fn encodes_legacy_wheel_bytes() {
    assert_eq!(
        encode_mouse_wheel(
            MouseEventKind::ScrollUp,
            0,
            0,
            KeyModifiers::NONE,
            TermMode::empty()
        ),
        Some(vec![0x1b, b'[', b'M', 96, 33, 33])
    );
    assert_eq!(
        encode_mouse_wheel(
            MouseEventKind::ScrollDown,
            4,
            6,
            KeyModifiers::NONE,
            TermMode::empty()
        ),
        Some(vec![0x1b, b'[', b'M', 97, 37, 39])
    );
    assert_eq!(
        encode_mouse_wheel(
            MouseEventKind::Down(crossterm::event::MouseButton::Left),
            0,
            0,
            KeyModifiers::NONE,
            TermMode::empty()
        ),
        None
    );
}

#[test]
fn encodes_utf8_wheel_codepoints() {
    assert_eq!(
        encode_mouse_wheel(
            MouseEventKind::ScrollDown,
            4,
            6,
            KeyModifiers::NONE,
            TermMode::UTF8_MOUSE
        ),
        Some(b"\x1b[M\x61\x25\x27".to_vec())
    );
}

#[test]
fn encodes_sgr_button_press_drag_release_and_motion() {
    let sgr = TermMode::SGR_MOUSE;
    assert_eq!(
        encode_mouse_button(
            MouseEventKind::Down(MouseButton::Left),
            2,
            3,
            KeyModifiers::NONE,
            sgr
        ),
        Some(b"\x1b[<0;3;4M".to_vec())
    );
    assert_eq!(
        encode_mouse_button(
            MouseEventKind::Drag(MouseButton::Left),
            2,
            3,
            KeyModifiers::NONE,
            sgr
        ),
        Some(b"\x1b[<32;3;4M".to_vec())
    );
    assert_eq!(
        encode_mouse_button(MouseEventKind::Moved, 2, 3, KeyModifiers::NONE, sgr),
        Some(b"\x1b[<35;3;4M".to_vec())
    );
    assert_eq!(
        encode_mouse_button(
            MouseEventKind::Up(MouseButton::Left),
            2,
            3,
            KeyModifiers::NONE,
            sgr
        ),
        Some(b"\x1b[<0;3;4m".to_vec())
    );
    assert_eq!(
        encode_mouse_button(
            MouseEventKind::Down(MouseButton::Right),
            2,
            3,
            KeyModifiers::SHIFT,
            sgr
        ),
        Some(b"\x1b[<6;3;4M".to_vec())
    );
}

#[test]
fn encodes_legacy_button_release_as_button_three() {
    assert_eq!(
        encode_mouse_button(
            MouseEventKind::Down(MouseButton::Middle),
            0,
            0,
            KeyModifiers::NONE,
            TermMode::empty()
        ),
        Some(vec![0x1b, b'[', b'M', 33, 33, 33])
    );
    assert_eq!(
        encode_mouse_button(
            MouseEventKind::Up(MouseButton::Left),
            0,
            0,
            KeyModifiers::NONE,
            TermMode::empty()
        ),
        Some(vec![0x1b, b'[', b'M', 35, 33, 33])
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

#[test]
fn encodes_ghostty_aligned_legacy_modified_keys_for_tui_agents() {
    let legacy = TermMode::empty();
    // Shift+Enter / Ctrl+Enter / Alt+Enter（opencode、claude、codex 在 Legacy 模式下的换行序列）。
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), legacy),
        Some(b"\x1b[27;2;13~".to_vec())
    );
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), legacy),
        Some(b"\x1b[27;5;13~".to_vec())
    );
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), legacy),
        Some(b"\x1b\r".to_vec())
    );
    // Shift+Tab（无论宿主以 BackTab 还是 Tab+SHIFT 到达）。
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), legacy),
        Some(b"\x1b[Z".to_vec())
    );
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), legacy),
        Some(b"\x1b[Z".to_vec())
    );
    // Alt+Backspace 与 Ctrl+Backspace 整词删除。
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT), legacy),
        Some(vec![0x1b, 0x7f])
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
            legacy
        ),
        Some(vec![0x08])
    );
    // Ctrl+Shift+<字母> 走 fixterms CSI u。
    assert_eq!(
        encode_key(
            KeyEvent::new(
                KeyCode::Char('P'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
            legacy
        ),
        Some(b"\x1b[112;6u".to_vec())
    );
}

#[test]
fn encodes_kitty_csi_u_keys_for_opencode_codex_and_claude() {
    let kitty = TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALTERNATE_KEYS;
    // Shift+Enter / Alt+Enter / Ctrl+Enter。
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT), kitty),
        Some(b"\x1b[13;2u".to_vec())
    );
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT), kitty),
        Some(b"\x1b[13;3u".to_vec())
    );
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL), kitty),
        Some(b"\x1b[13;5u".to_vec())
    );
    // 无修饰 Enter 保持 `\r`，避免破坏普通交互。
    assert_eq!(encode_key(key(KeyCode::Enter), kitty), Some(b"\r".to_vec()));
    // Shift+Tab（Tab+SHIFT 与 BackTab）在 Kitty 下编码为 `\x1b[9;2u`。
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT), kitty),
        Some(b"\x1b[9;2u".to_vec())
    );
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT), kitty),
        Some(b"\x1b[9;2u".to_vec())
    );
    // Alt+Backspace / Ctrl+Backspace。
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::ALT), kitty),
        Some(b"\x1b[127;3u".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::CONTROL),
            kitty
        ),
        Some(b"\x1b[127;5u".to_vec())
    );
    // 普通大写与上档符号保持 UTF-8 原样输出。
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('!'), KeyModifiers::SHIFT),
            kitty
        ),
        Some(b"!".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT),
            kitty
        ),
        Some(b"A".to_vec())
    );
    // Kitty 模式下 Ctrl / Super 快捷键（含 opencode 撤销 Super+Z / Ctrl+Z / Ctrl+- 与重做 Super+Shift+Z / Ctrl+.）。
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::SUPER),
            kitty
        ),
        Some(b"\x1b[122;9u".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(
                KeyCode::Char('Z'),
                KeyModifiers::SUPER | KeyModifiers::SHIFT
            ),
            kitty
        ),
        Some(b"\x1b[122;10u".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
            kitty
        ),
        Some(b"\x1b[122;5u".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('-'), KeyModifiers::CONTROL),
            kitty
        ),
        Some(b"\x1b[45;5u".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('.'), KeyModifiers::CONTROL),
            kitty
        ),
        Some(b"\x1b[46;5u".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SUPER),
            kitty
        ),
        Some(b"\x1b[97;9u".to_vec())
    );
    assert_eq!(
        encode_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SUPER), kitty),
        Some(b"\x1b[1;9D".to_vec())
    );
    // 纯 Super+C 在无选区时仍不透传，避免在 claude 中误输入 `c`。
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::SUPER),
            kitty
        ),
        None
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('m'), KeyModifiers::CONTROL),
            kitty
        ),
        Some(b"\x1b[109;5u".to_vec())
    );
    assert_eq!(
        encode_key(
            KeyEvent::new(KeyCode::Char('i'), KeyModifiers::CONTROL),
            kitty
        ),
        Some(b"\x1b[105;5u".to_vec())
    );
}

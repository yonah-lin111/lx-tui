//! Agents 标题弹窗键位映射测试。

use super::*;

#[test]
fn agent_title_overlay_routes_navigation_and_editing_keys() {
    let overlay = |code| {
        overlay_key(
            KeyEvent::new(code, KeyModifiers::NONE),
            OverlayKind::AgentTitle,
        )
    };
    assert_eq!(overlay(KeyCode::Esc), Some(OverlayKey::Esc));
    assert_eq!(overlay(KeyCode::Enter), Some(OverlayKey::Enter));
    assert_eq!(overlay(KeyCode::Up), Some(OverlayKey::Up));
    assert_eq!(overlay(KeyCode::Down), Some(OverlayKey::Down));
    assert_eq!(overlay(KeyCode::Char('x')), Some(OverlayKey::Char('x')));
    assert_eq!(overlay(KeyCode::Backspace), Some(OverlayKey::Backspace));
}

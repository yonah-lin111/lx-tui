//! 进程 / 计划卡片键位测试：`^o` 与 Alt+↑/↓ 的映射与面板优先级。
//!
//! 这些键只在 Native 聊天上下文生效，**绝不**转发给后台 shell；已有浮层/补全
//! 打开时保持原优先级（路由在浮层分支提前返回）。

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;
use crate::app::actions::LxInput;
use crate::app::state::FocusedContainer;

/// 构造一个按键事件。
fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

#[test]
fn ctrl_o_toggles_tool_detail() {
    let routed = route(
        key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        alacritty_terminal::term::TermMode::empty(),
        FocusedContainer::Panes,
        None,
        true,
    );
    assert_eq!(routed, Some(Routed::Lx(LxInput::ToggleToolDetail)));
}

#[test]
fn alt_arrows_move_the_tool_selection() {
    let mode = alacritty_terminal::term::TermMode::empty();
    assert_eq!(
        route(
            key(KeyCode::Up, KeyModifiers::ALT),
            mode,
            FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::SelectPrevTool))
    );
    assert_eq!(
        route(
            key(KeyCode::Down, KeyModifiers::ALT),
            mode,
            FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::SelectNextTool))
    );
}

#[test]
fn plain_arrows_still_edit_the_draft() {
    let mode = alacritty_terminal::term::TermMode::empty();
    assert_eq!(
        route(
            key(KeyCode::Up, KeyModifiers::NONE),
            mode,
            FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::Up)),
        "无 Alt 的 ↑ 仍是光标/历史移动"
    );
}

#[test]
fn existing_panels_keep_priority_over_the_new_chords() {
    let mode = alacritty_terminal::term::TermMode::empty();
    // 浮层打开：按键交给浮层，不进入聊天卡片操作。
    let routed = route(
        key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        mode,
        FocusedContainer::Panes,
        Some(crate::app::overlay::OverlayKind::Menu),
        true,
    );
    assert!(
        matches!(routed, Some(Routed::Overlay(_)) | None),
        "浮层优先：{routed:?}"
    );
}

//! 单元测试；仅测试构建编译。

use super::*;

/// 无浮层、终端视图路由的测试别名；浮层与 lx 视图直接调用 `super::route`。
fn route(key: KeyEvent, mode: TermMode, prompt_focused: bool) -> Option<Routed> {
    super::route(key, mode, prompt_focused, None, false)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn bare_keys_go_to_pane() {
    assert_eq!(
        route(key(KeyCode::Char('q')), TermMode::empty(), false),
        Some(Routed::Pane(b"q".to_vec()))
    );
    assert_eq!(
        route(key(KeyCode::Char('h')), TermMode::empty(), false),
        Some(Routed::Pane(b"h".to_vec()))
    );
}

#[test]
fn ctrl_q_quits() {
    let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert_eq!(
        route(key, TermMode::empty(), false),
        Some(Routed::Action(Action::Quit))
    );
}

#[test]
fn ctrl_c_is_passed_to_pane() {
    let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(
        route(key, TermMode::empty(), false),
        Some(Routed::Pane(vec![0x03]))
    );
}

#[test]
fn ctrl_b_is_passed_to_pane() {
    let key = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(
        route(key, TermMode::empty(), false),
        Some(Routed::Pane(vec![0x02]))
    );
}

#[test]
fn release_events_are_ignored() {
    let mut key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
    key.kind = KeyEventKind::Release;
    assert_eq!(route(key, TermMode::empty(), false), None);
    assert_eq!(route(key, TermMode::empty(), true), None);
}

#[test]
fn navigation_keys_encode_to_pane() {
    assert_eq!(
        route(key(KeyCode::Tab), TermMode::empty(), false),
        Some(Routed::Pane(b"\t".to_vec()))
    );
    assert_eq!(
        route(key(KeyCode::Up), TermMode::APP_CURSOR, false),
        Some(Routed::Pane(b"\x1bOA".to_vec()))
    );
}

#[test]
fn prompt_focus_maps_editing_keys() {
    let route_prompt = |code| route(key(code), TermMode::empty(), true);
    assert_eq!(
        route_prompt(KeyCode::Char('a')),
        Some(Routed::Editor(EditorCommand::InsertChar('a')))
    );
    assert_eq!(
        route_prompt(KeyCode::Enter),
        Some(Routed::Editor(EditorCommand::Newline))
    );
    assert_eq!(
        route_prompt(KeyCode::Backspace),
        Some(Routed::Editor(EditorCommand::Backspace))
    );
    assert_eq!(
        route_prompt(KeyCode::Delete),
        Some(Routed::Editor(EditorCommand::Delete))
    );
    for (code, command) in [
        (KeyCode::Left, EditorCommand::Left),
        (KeyCode::Right, EditorCommand::Right),
        (KeyCode::Up, EditorCommand::Up),
        (KeyCode::Down, EditorCommand::Down),
        (KeyCode::Home, EditorCommand::Home),
        (KeyCode::End, EditorCommand::End),
    ] {
        assert_eq!(route_prompt(code), Some(Routed::Editor(command)));
    }
}

#[test]
fn prompt_focus_accepts_shifted_characters() {
    let key = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
    assert_eq!(
        route(key, TermMode::empty(), true),
        Some(Routed::Editor(EditorCommand::InsertChar('A')))
    );
}

#[test]
fn prompt_focus_maps_readline_shortcuts() {
    let route_prompt =
        |code, modifiers| route(KeyEvent::new(code, modifiers), TermMode::empty(), true);
    for (code, modifiers, command) in [
        (
            KeyCode::Char('u'),
            KeyModifiers::CONTROL,
            EditorCommand::DeleteToLineStart,
        ),
        (
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
            EditorCommand::DeleteToLineEnd,
        ),
        (
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
            EditorCommand::DeleteWordBackward,
        ),
        (
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
            EditorCommand::LineStart,
        ),
        (
            KeyCode::Char('e'),
            KeyModifiers::CONTROL,
            EditorCommand::LineEnd,
        ),
        (
            KeyCode::Char('d'),
            KeyModifiers::CONTROL,
            EditorCommand::Delete,
        ),
        (
            KeyCode::Backspace,
            KeyModifiers::CONTROL,
            EditorCommand::DeleteWordBackward,
        ),
        (
            KeyCode::Delete,
            KeyModifiers::CONTROL,
            EditorCommand::DeleteWordForward,
        ),
        (
            KeyCode::Left,
            KeyModifiers::CONTROL,
            EditorCommand::WordLeft,
        ),
        (
            KeyCode::Right,
            KeyModifiers::CONTROL,
            EditorCommand::WordRight,
        ),
        (
            KeyCode::Char('b'),
            KeyModifiers::ALT,
            EditorCommand::WordLeft,
        ),
        (
            KeyCode::Char('f'),
            KeyModifiers::ALT,
            EditorCommand::WordRight,
        ),
        (
            KeyCode::Char('d'),
            KeyModifiers::ALT,
            EditorCommand::DeleteWordForward,
        ),
        (
            KeyCode::Backspace,
            KeyModifiers::ALT,
            EditorCommand::DeleteWordBackward,
        ),
        (KeyCode::Left, KeyModifiers::ALT, EditorCommand::WordLeft),
        (KeyCode::Right, KeyModifiers::ALT, EditorCommand::WordRight),
    ] {
        assert_eq!(
            route_prompt(code, modifiers),
            Some(Routed::Editor(command)),
            "{code:?}+{modifiers:?}"
        );
    }
}

#[test]
fn prompt_focus_maps_history_and_indent_keys() {
    let route_prompt =
        |code, modifiers| route(KeyEvent::new(code, modifiers), TermMode::empty(), true);
    for (code, modifiers, command) in [
        (
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
            EditorCommand::Undo,
        ),
        (
            KeyCode::Char('-'),
            KeyModifiers::CONTROL,
            EditorCommand::Undo,
        ),
        (
            KeyCode::Char('Z'),
            KeyModifiers::CONTROL,
            EditorCommand::Redo,
        ),
        (
            KeyCode::Char('y'),
            KeyModifiers::CONTROL,
            EditorCommand::Redo,
        ),
        (
            KeyCode::Char('.'),
            KeyModifiers::CONTROL,
            EditorCommand::Redo,
        ),
        (KeyCode::Char('z'), KeyModifiers::SUPER, EditorCommand::Undo),
        (
            KeyCode::Char('z'),
            KeyModifiers::SUPER | KeyModifiers::SHIFT,
            EditorCommand::Redo,
        ),
        (KeyCode::Char('y'), KeyModifiers::SUPER, EditorCommand::Redo),
        (
            KeyCode::Enter,
            KeyModifiers::SUPER,
            EditorCommand::NewlineBelow,
        ),
        (
            KeyCode::Enter,
            KeyModifiers::SUPER | KeyModifiers::SHIFT,
            EditorCommand::NewlineBelow,
        ),
        (
            KeyCode::Enter,
            KeyModifiers::CONTROL,
            EditorCommand::NewlineBelow,
        ),
        (
            KeyCode::Enter,
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            EditorCommand::NewlineBelow,
        ),
        (
            KeyCode::Enter,
            KeyModifiers::SHIFT,
            EditorCommand::EnterFolder,
        ),
        (
            KeyCode::Enter,
            KeyModifiers::SHIFT,
            EditorCommand::EnterFolder,
        ),
        (
            KeyCode::Backspace,
            KeyModifiers::SHIFT,
            EditorCommand::Backspace,
        ),
        (
            KeyCode::Backspace,
            KeyModifiers::NONE,
            EditorCommand::Backspace,
        ),
        (KeyCode::Delete, KeyModifiers::SHIFT, EditorCommand::Delete),
        (KeyCode::Tab, KeyModifiers::NONE, EditorCommand::Indent),
        (KeyCode::BackTab, KeyModifiers::NONE, EditorCommand::Outdent),
        (KeyCode::Tab, KeyModifiers::SHIFT, EditorCommand::Outdent),
    ] {
        assert_eq!(
            route_prompt(code, modifiers),
            Some(Routed::Editor(command)),
            "{code:?}+{modifiers:?}"
        );
    }
}

#[test]
fn prompt_focus_maps_copy_keys() {
    let route_prompt =
        |code, modifiers| route(KeyEvent::new(code, modifiers), TermMode::empty(), true);
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
        assert_eq!(
            route_prompt(KeyCode::Char('c'), modifiers),
            Some(Routed::Copy),
            "{modifiers:?}"
        );
        assert_eq!(
            route_prompt(KeyCode::Char('c'), modifiers | KeyModifiers::SHIFT),
            Some(Routed::CopyTemplateBlock),
            "{modifiers:?}+SHIFT"
        );
    }
    assert_eq!(
        route_prompt(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        ),
        None
    );
}

#[test]
fn prompt_focus_swallows_unmapped_keys() {
    let cases = [
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL | KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT | KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT),
        KeyEvent::new(KeyCode::Char('B'), KeyModifiers::ALT | KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT),
        KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    ];
    for key in cases {
        assert_eq!(route(key, TermMode::empty(), true), None, "{key:?}");
    }
}

#[test]
fn super_combos_never_reach_pane() {
    let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::SUPER);
    assert_eq!(route(key, TermMode::empty(), false), None);
}

#[test]
fn ctrl_q_quits_even_when_prompt_focused() {
    let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert_eq!(
        route(key, TermMode::empty(), true),
        Some(Routed::Action(Action::Quit))
    );
}

#[test]
fn prompt_focus_maps_escape_and_unfocused_passes_to_pane() {
    assert_eq!(
        route(key(KeyCode::Esc), TermMode::empty(), true),
        Some(Routed::Editor(EditorCommand::Escape))
    );
    assert_eq!(
        route(key(KeyCode::Esc), TermMode::empty(), false),
        Some(Routed::Pane(vec![0x1b]))
    );
}

#[test]
fn menu_overlay_routes_navigation_keys_only() {
    let route_menu = |code, modifiers| {
        super::route(
            KeyEvent::new(code, modifiers),
            TermMode::empty(),
            false,
            Some(OverlayKind::Menu),
            false,
        )
    };
    for (code, expected) in [
        (KeyCode::Esc, OverlayKey::Esc),
        (KeyCode::Up, OverlayKey::Up),
        (KeyCode::Down, OverlayKey::Down),
        (KeyCode::Enter, OverlayKey::Enter),
    ] {
        assert_eq!(
            route_menu(code, KeyModifiers::NONE),
            Some(Routed::Overlay(expected)),
            "{code:?}"
        );
    }
    for code in [KeyCode::Char('q'), KeyCode::Backspace, KeyCode::Left] {
        assert_eq!(route_menu(code, KeyModifiers::NONE), None, "{code:?}");
    }
}

#[test]
fn confirm_overlay_routes_enter_and_esc_only() {
    let route_confirm = |code| {
        super::route(
            key(code),
            TermMode::empty(),
            false,
            Some(OverlayKind::ConfirmClose),
            false,
        )
    };
    assert_eq!(
        route_confirm(KeyCode::Enter),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        route_confirm(KeyCode::Esc),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    assert_eq!(route_confirm(KeyCode::Char('y')), None);
}

#[test]
fn confirm_switch_cwd_overlay_routes_enter_and_esc_only() {
    let route_confirm = |code| {
        super::route(
            key(code),
            TermMode::empty(),
            false,
            Some(OverlayKind::ConfirmSwitchCwd),
            false,
        )
    };
    assert_eq!(
        route_confirm(KeyCode::Enter),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        route_confirm(KeyCode::Esc),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    assert_eq!(route_confirm(KeyCode::Char('n')), None);
}

#[test]
fn rename_overlay_routes_editing_keys() {
    let route_rename = |code, modifiers| {
        super::route(
            KeyEvent::new(code, modifiers),
            TermMode::empty(),
            true,
            Some(OverlayKind::Rename),
            false,
        )
    };
    assert_eq!(
        route_rename(KeyCode::Char('a'), KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Char('a')))
    );
    assert_eq!(
        route_rename(KeyCode::Char('A'), KeyModifiers::SHIFT),
        Some(Routed::Overlay(OverlayKey::Char('A')))
    );
    assert_eq!(
        route_rename(KeyCode::Backspace, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Backspace))
    );
    assert_eq!(
        route_rename(KeyCode::Char('c'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Clear))
    );
    assert_eq!(
        route_rename(KeyCode::Delete, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Delete))
    );
    for (code, expected) in [
        (KeyCode::Left, OverlayKey::Left),
        (KeyCode::Right, OverlayKey::Right),
        (KeyCode::Home, OverlayKey::Home),
        (KeyCode::End, OverlayKey::End),
    ] {
        assert_eq!(
            route_rename(code, KeyModifiers::NONE),
            Some(Routed::Overlay(expected)),
            "{code:?}"
        );
    }
    assert_eq!(
        route_rename(KeyCode::Char('a'), KeyModifiers::CONTROL),
        None
    );
    assert_eq!(route_rename(KeyCode::Up, KeyModifiers::NONE), None);
}

#[test]
fn overlay_does_not_swallow_ctrl_q() {
    let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert_eq!(
        super::route(
            key,
            TermMode::empty(),
            false,
            Some(OverlayKind::Menu),
            false
        ),
        Some(Routed::Action(Action::Quit))
    );
}

#[test]
fn lx_view_swallows_keys_except_ctrl_q() {
    for code in [
        KeyCode::Char('l'),
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::Up,
        KeyCode::Backspace,
    ] {
        assert_eq!(
            super::route(key(code), TermMode::empty(), false, None, true),
            None,
            "{code:?} 应被 lx 页吞掉"
        );
    }
    let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert_eq!(
        super::route(quit, TermMode::empty(), false, None, true),
        Some(Routed::Action(Action::Quit))
    );
    // prompt 聚焦优先于 lx 吞键：按键仍进入编辑器。
    assert_eq!(
        super::route(key(KeyCode::Char('a')), TermMode::empty(), true, None, true),
        Some(Routed::Editor(EditorCommand::InsertChar('a')))
    );
}

#[test]
fn worktree_open_overlay_routes_search_and_navigation_keys() {
    let route_overlay = |code| {
        super::route(
            key(code),
            TermMode::empty(),
            false,
            Some(OverlayKind::WorktreeOpen),
            false,
        )
    };
    assert_eq!(
        route_overlay(KeyCode::Esc),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    assert_eq!(
        route_overlay(KeyCode::Enter),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        route_overlay(KeyCode::Up),
        Some(Routed::Overlay(OverlayKey::Up))
    );
    assert_eq!(
        route_overlay(KeyCode::Down),
        Some(Routed::Overlay(OverlayKey::Down))
    );
    assert_eq!(
        route_overlay(KeyCode::Char('f')),
        Some(Routed::Overlay(OverlayKey::Char('f')))
    );
    assert_eq!(
        route_overlay(KeyCode::Backspace),
        Some(Routed::Overlay(OverlayKey::Backspace))
    );
    assert_eq!(
        route_overlay(KeyCode::Home),
        Some(Routed::Overlay(OverlayKey::Home))
    );
    assert_eq!(route_overlay(KeyCode::Tab), None);

    let ctrl = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(
        super::route(
            ctrl,
            TermMode::empty(),
            false,
            Some(OverlayKind::WorktreeOpen),
            false
        ),
        None
    );
}

//! 浮层按键路由测试。

use super::*;

#[test]
fn menu_overlay_routes_navigation_keys_only() {
    let route_menu = |code, modifiers| {
        crate::input::route(
            KeyEvent::new(code, modifiers),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
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
fn referenced_menu_routes_navigation_copy_and_remove_keys() {
    let route_menu = |code, modifiers| {
        crate::input::route(
            KeyEvent::new(code, modifiers),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::ReferencedFolderMenu),
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
    // Ctrl/Cmd+C 复制引用、Ctrl/Cmd+D 或 Del 移除记录；纯文本键仍不映射。
    assert_eq!(
        route_menu(KeyCode::Char('c'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Copy))
    );
    assert_eq!(
        route_menu(KeyCode::Char('c'), KeyModifiers::SUPER),
        Some(Routed::Overlay(OverlayKey::Copy))
    );
    assert_eq!(
        route_menu(KeyCode::Char('d'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Remove))
    );
    assert_eq!(
        route_menu(KeyCode::Delete, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Remove))
    );
    assert_eq!(route_menu(KeyCode::Char('x'), KeyModifiers::NONE), None);
    assert_eq!(route_menu(KeyCode::Char('d'), KeyModifiers::NONE), None);
}

#[test]
fn referenced_input_routes_single_line_editing() {
    let route_input = |code, modifiers| {
        crate::input::route(
            KeyEvent::new(code, modifiers),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::ReferencedFolderInput),
            false,
        )
    };
    assert_eq!(
        route_input(KeyCode::Char('a'), KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Char('a')))
    );
    assert_eq!(
        route_input(KeyCode::Char('c'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Clear))
    );
    assert_eq!(
        route_input(KeyCode::Backspace, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Backspace))
    );
    assert_eq!(
        route_input(KeyCode::Enter, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        route_input(KeyCode::Esc, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    // 上下键不映射（输入框无导航语义）。
    assert_eq!(route_input(KeyCode::Up, KeyModifiers::NONE), None);
}

#[test]
fn referenced_search_routes_enter_folder_and_back() {
    let route_search = |code, modifiers| {
        crate::input::route(
            KeyEvent::new(code, modifiers),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::ReferencedFolderSearch),
            false,
        )
    };
    assert_eq!(
        route_search(KeyCode::Enter, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        route_search(KeyCode::Enter, KeyModifiers::SHIFT),
        Some(Routed::Overlay(OverlayKey::EnterFolder))
    );
    assert_eq!(
        route_search(KeyCode::Char('z'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Undo))
    );
    assert_eq!(
        route_search(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        Some(Routed::Overlay(OverlayKey::Redo))
    );
    assert_eq!(
        route_search(KeyCode::Char('y'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Redo))
    );
    assert_eq!(
        route_search(KeyCode::Char('x'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Cut))
    );
    assert_eq!(
        route_search(KeyCode::Char('c'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Copy))
    );
    assert_eq!(
        route_search(KeyCode::Up, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Up))
    );
    assert_eq!(
        route_search(KeyCode::Char('a'), KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Char('a')))
    );
    assert_eq!(
        route_search(KeyCode::Esc, KeyModifiers::NONE),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    // Ctrl+Q 优先于浮层（全局退出）。
    assert_eq!(
        crate::input::route(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::ReferencedFolderSearch),
            false,
        ),
        Some(Routed::Action(crate::app::actions::Action::RequestQuit))
    );
}

#[test]
fn referenced_delete_routes_enter_and_esc_only() {
    let route_delete = |code| {
        crate::input::route(
            key(code),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::ReferencedFolderDelete),
            false,
        )
    };
    assert_eq!(
        route_delete(KeyCode::Enter),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        route_delete(KeyCode::Esc),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    assert_eq!(route_delete(KeyCode::Char('a')), None);
}

#[test]
fn confirm_overlay_routes_editing_keys() {
    let route_confirm = |event| {
        crate::input::route(
            event,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::ConfirmClose),
            false,
        )
    };
    assert_eq!(
        route_confirm(key(KeyCode::Enter)),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        route_confirm(key(KeyCode::Esc)),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    assert_eq!(
        route_confirm(key(KeyCode::Char('c'))),
        Some(Routed::Overlay(OverlayKey::Char('c')))
    );
    assert_eq!(
        route_confirm(key(KeyCode::Backspace)),
        Some(Routed::Overlay(OverlayKey::Backspace))
    );
    assert_eq!(
        route_confirm(key(KeyCode::Left)),
        Some(Routed::Overlay(OverlayKey::Left))
    );
    assert_eq!(
        route_confirm(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)),
        Some(Routed::Overlay(OverlayKey::Undo))
    );
    // 失效工作区列表：上下键滚动视口，不得落进单行输入编辑键。
    for (code, arrow) in [
        (KeyCode::Up, OverlayKey::Up),
        (KeyCode::Down, OverlayKey::Down),
    ] {
        assert_eq!(route_confirm(key(code)), Some(Routed::Overlay(arrow)));
    }

    // 信任浮层仍只放行 Esc / Enter，避免输入被误当成确认。
    let route_trust = |code| {
        crate::input::route(
            key(code),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::TrustWorkspace),
            false,
        )
    };
    assert_eq!(
        route_trust(KeyCode::Enter),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        route_trust(KeyCode::Esc),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    assert_eq!(route_trust(KeyCode::Char('y')), None);
}

#[test]
fn rename_overlay_routes_editing_keys() {
    let route_rename = |code, modifiers| {
        crate::input::route(
            KeyEvent::new(code, modifiers),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Prompt,
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
        Some(Routed::Overlay(OverlayKey::LineStart))
    );
    assert_eq!(
        route_rename(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        Some(Routed::Overlay(OverlayKey::SelectAll))
    );
    assert_eq!(
        route_rename(KeyCode::Char('e'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::LineEnd))
    );
    assert_eq!(
        route_rename(KeyCode::Char('u'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::DeleteToLineStart))
    );
    assert_eq!(
        route_rename(KeyCode::Char('k'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::DeleteToLineEnd))
    );
    assert_eq!(
        route_rename(KeyCode::Char('z'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Undo))
    );
    assert_eq!(
        route_rename(
            KeyCode::Char('z'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        Some(Routed::Overlay(OverlayKey::Redo))
    );
    assert_eq!(
        route_rename(KeyCode::Char('y'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Redo))
    );
    assert_eq!(
        route_rename(KeyCode::Char('b'), KeyModifiers::CONTROL),
        None
    );
    assert_eq!(route_rename(KeyCode::Up, KeyModifiers::NONE), None);
}

#[test]
fn overlay_does_not_swallow_ctrl_q() {
    let key = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert_eq!(
        crate::input::route(
            key,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::Menu),
            false
        ),
        Some(Routed::Action(Action::RequestQuit))
    );
}

#[test]
fn worktree_open_overlay_routes_search_and_navigation_keys() {
    let route_overlay = |code| {
        crate::input::route(
            key(code),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
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
        crate::input::route(
            ctrl,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::WorktreeOpen),
            false
        ),
        Some(Routed::Overlay(OverlayKey::LineStart))
    );
    let ctrl_b = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(
        crate::input::route(
            ctrl_b,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::WorktreeOpen),
            false
        ),
        None
    );
}

#[test]
fn save_draft_overlay_routes_navigation_only() {
    let route_overlay = |code| {
        crate::input::route(
            key(code),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::SaveDraft),
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
        route_overlay(KeyCode::Char('x')),
        Some(Routed::Overlay(OverlayKey::Char('x'))),
        "名称/内联行文本输入"
    );
    assert_eq!(
        route_overlay(KeyCode::Backspace),
        Some(Routed::Overlay(OverlayKey::Backspace))
    );
    assert_eq!(
        route_overlay(KeyCode::Left),
        Some(Routed::Overlay(OverlayKey::Left))
    );
}

#[test]
fn all_text_overlays_route_word_ops_and_cut() {
    let kinds = [
        OverlayKind::Rename,
        OverlayKind::NewWorkspace,
        OverlayKind::NewNode,
        OverlayKind::ConfirmClose,
        OverlayKind::SaveDraft,
        OverlayKind::WorktreeOpen,
        OverlayKind::ReferencedFolderInput,
        OverlayKind::ReferencedFolderSearch,
        OverlayKind::ResumeSessionEdit,
        OverlayKind::Connect,
        OverlayKind::AgentTitle,
        OverlayKind::NewAgent,
    ];
    let cases = [
        (
            KeyCode::Char('w'),
            KeyModifiers::CONTROL,
            OverlayKey::DeleteWordBackward,
        ),
        (
            KeyCode::Backspace,
            KeyModifiers::ALT,
            OverlayKey::DeleteWordBackward,
        ),
        (
            KeyCode::Char('d'),
            KeyModifiers::ALT,
            OverlayKey::DeleteWordForward,
        ),
        (
            KeyCode::Delete,
            KeyModifiers::CONTROL,
            OverlayKey::DeleteWordForward,
        ),
        (KeyCode::Left, KeyModifiers::CONTROL, OverlayKey::WordLeft),
        (KeyCode::Left, KeyModifiers::ALT, OverlayKey::WordLeft),
        (KeyCode::Char('b'), KeyModifiers::ALT, OverlayKey::WordLeft),
        (KeyCode::Right, KeyModifiers::CONTROL, OverlayKey::WordRight),
        (KeyCode::Right, KeyModifiers::ALT, OverlayKey::WordRight),
        (KeyCode::Char('f'), KeyModifiers::ALT, OverlayKey::WordRight),
        (KeyCode::Char('x'), KeyModifiers::CONTROL, OverlayKey::Cut),
    ];
    for kind in kinds {
        for (code, modifiers, expected) in cases {
            assert_eq!(
                crate::input::overlay_key(KeyEvent::new(code, modifiers), kind),
                Some(expected),
                "{kind:?} {code:?} {modifiers:?}"
            );
        }
    }
}

#[test]
fn new_agent_overlay_routes_text_and_navigation_keys() {
    let key = |code, modifiers| {
        crate::input::overlay_key(KeyEvent::new(code, modifiers), OverlayKind::NewAgent)
    };
    assert_eq!(
        key(KeyCode::Char('o'), KeyModifiers::NONE),
        Some(OverlayKey::Char('o'))
    );
    assert_eq!(
        key(KeyCode::Backspace, KeyModifiers::NONE),
        Some(OverlayKey::Backspace)
    );
    assert_eq!(key(KeyCode::Esc, KeyModifiers::NONE), Some(OverlayKey::Esc));
    assert_eq!(
        key(KeyCode::Enter, KeyModifiers::NONE),
        Some(OverlayKey::Enter)
    );
    assert_eq!(key(KeyCode::Up, KeyModifiers::NONE), Some(OverlayKey::Up));
    assert_eq!(
        key(KeyCode::Down, KeyModifiers::NONE),
        Some(OverlayKey::Down)
    );
}

#[test]
fn connect_overlay_routes_refresh_navigation_and_text_edits() {
    let key = |code, modifiers| {
        crate::input::overlay_key(KeyEvent::new(code, modifiers), OverlayKind::Connect)
    };
    assert_eq!(
        key(KeyCode::Char('r'), KeyModifiers::CONTROL),
        Some(OverlayKey::Refresh)
    );
    assert_eq!(
        key(KeyCode::Char('r'), KeyModifiers::SUPER),
        Some(OverlayKey::Refresh)
    );
    assert_eq!(key(KeyCode::Esc, KeyModifiers::NONE), Some(OverlayKey::Esc));
    assert_eq!(
        key(KeyCode::Enter, KeyModifiers::NONE),
        Some(OverlayKey::Enter)
    );
    assert_eq!(key(KeyCode::Up, KeyModifiers::NONE), Some(OverlayKey::Up));
    assert_eq!(
        key(KeyCode::Down, KeyModifiers::NONE),
        Some(OverlayKey::Down)
    );
    assert_eq!(
        key(KeyCode::Char('a'), KeyModifiers::NONE),
        Some(OverlayKey::Char('a'))
    );
}

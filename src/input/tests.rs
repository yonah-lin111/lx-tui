//! 单元测试；仅测试构建编译。

use super::*;

/// 无浮层、终端视图路由的测试别名；浮层与 lx 视图直接调用 `super::route`。
fn route(key: KeyEvent, mode: TermMode, prompt_focused: bool) -> Option<Routed> {
    super::route(
        key,
        mode,
        if prompt_focused {
            crate::app::state::FocusedContainer::Prompt
        } else {
            crate::app::state::FocusedContainer::Panes
        },
        None,
        false,
    )
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
fn ctrl_a_is_passed_to_pane() {
    let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
    assert_eq!(
        route(key, TermMode::empty(), false),
        Some(Routed::Pane(vec![0x01])),
        "终端里的 Ctrl+A 仍是 readline 行首，不截获为全选"
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
fn prompt_focus_swallows_clipboard_paste_keys() {
    let route_prompt =
        |code, modifiers| route(KeyEvent::new(code, modifiers), TermMode::empty(), true);
    // 图片/文件一律走路径引用，应用不再读取系统剪贴板：粘贴键在 prompt 内被吞掉。
    for (code, modifiers) in [
        (KeyCode::Char('v'), KeyModifiers::CONTROL),
        (KeyCode::Char('V'), KeyModifiers::CONTROL),
        (KeyCode::Char('v'), KeyModifiers::SUPER),
        (
            KeyCode::Char('V'),
            KeyModifiers::SUPER | KeyModifiers::SHIFT,
        ),
    ] {
        assert_eq!(
            route_prompt(code, modifiers),
            None,
            "{code:?}+{modifiers:?} 在 prompt 中被吞掉"
        );
    }
    assert!(
        matches!(
            route(
                KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL),
                TermMode::empty(),
                false
            ),
            Some(Routed::Pane(_))
        ),
        "prompt 未聚焦时 Ctrl+V 照常写入 PTY"
    );
}

#[test]
fn prompt_focus_maps_save_keys() {
    let route_prompt =
        |code, modifiers| route(KeyEvent::new(code, modifiers), TermMode::empty(), true);
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
        assert_eq!(
            route_prompt(KeyCode::Char('s'), modifiers),
            Some(Routed::SavePrompt),
            "{modifiers:?}"
        );
    }
    assert_eq!(
        route_prompt(
            KeyCode::Char('S'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ),
        Some(Routed::SavePrompt),
        "Ctrl+Shift+S 同样触发保存"
    );
    assert_eq!(
        route_prompt(
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        ),
        None,
        "Ctrl+Alt+S 不触发保存"
    );
    assert_eq!(
        route(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            TermMode::empty(),
            false
        ),
        Some(Routed::Pane(vec![0x13])),
        "未聚焦时 Ctrl+S 原样写入窗格"
    );
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
fn global_edit_keys_reserve_ctrl_a_for_line_start() {
    use crate::input::{GlobalEdit, global_edit_key};
    for modifiers in [KeyModifiers::CONTROL, KeyModifiers::SUPER] {
        assert_eq!(
            global_edit_key(KeyEvent::new(KeyCode::Char('c'), modifiers)),
            Some(GlobalEdit::Copy),
            "{modifiers:?}"
        );
        assert_eq!(
            global_edit_key(KeyEvent::new(KeyCode::Char('x'), modifiers)),
            Some(GlobalEdit::Cut),
            "{modifiers:?}"
        );
        assert_eq!(
            global_edit_key(KeyEvent::new(KeyCode::Char('v'), modifiers)),
            Some(GlobalEdit::Paste),
            "{modifiers:?}"
        );
        assert_eq!(
            global_edit_key(KeyEvent::new(
                KeyCode::Char('c'),
                modifiers | KeyModifiers::SHIFT
            )),
            None,
            "复制键不加 Shift"
        );
    }
    assert_eq!(
        global_edit_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        None,
        "Ctrl+A 留给输入框行首"
    );
    assert_eq!(
        global_edit_key(KeyEvent::new(
            KeyCode::Char('A'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        Some(GlobalEdit::SelectAll)
    );
    assert_eq!(
        global_edit_key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )),
        Some(GlobalEdit::SelectAll)
    );
    assert_eq!(
        global_edit_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::SUPER)),
        Some(GlobalEdit::SelectAll),
        "macOS 保留 Cmd+A 全选"
    );
    assert_eq!(
        global_edit_key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )),
        None
    );
    // 复制/剪切不再由焦点路由产生。
    assert_eq!(
        super::route(
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Prompt,
            None,
            false,
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
        super::route(
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
        super::route(
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
    // Ctrl/Cmd+C 由全局复制统一处理，不再是输入框清空。
    assert_eq!(route_input(KeyCode::Char('c'), KeyModifiers::CONTROL), None);
    assert_eq!(
        route_input(KeyCode::Char('a'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Home))
    );
    assert_eq!(
        route_input(KeyCode::Char('e'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::End))
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
        super::route(
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
    // Ctrl+Z 现在是输入框撤销，不再回退目录；回退走 Esc/界面入口。
    assert_eq!(
        route_search(KeyCode::Char('z'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::Undo))
    );
    // Ctrl+C 由全局复制统一处理。
    assert_eq!(
        route_search(KeyCode::Char('c'), KeyModifiers::CONTROL),
        None
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
        super::route(
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::ReferencedFolderSearch),
            false,
        ),
        Some(Routed::Action(crate::app::actions::Action::Quit))
    );
}

#[test]
fn referenced_delete_routes_enter_and_esc_only() {
    let route_delete = |code| {
        super::route(
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
fn confirm_overlay_routes_enter_and_esc_only() {
    let route_confirm = |code| {
        super::route(
            key(code),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
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
            crate::app::state::FocusedContainer::Panes,
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
        None
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
        Some(Routed::Overlay(OverlayKey::Home))
    );
    assert_eq!(
        route_rename(KeyCode::Char('e'), KeyModifiers::CONTROL),
        Some(Routed::Overlay(OverlayKey::End))
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
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::Menu),
            false
        ),
        Some(Routed::Action(Action::Quit))
    );
}

#[test]
fn lx_view_routes_editing_keys_and_swallows_others() {
    assert_eq!(
        super::route(
            key(KeyCode::Char('l')),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::Char('l')))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Backspace),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::Backspace))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Left),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::Left))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Up),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::Up))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Tab),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::Tab))
    );
    // Shift+Tab：ask 面板的「上一步」。终端按 kitty 协议可能报 `Tab+SHIFT`
    // 而不是 `BackTab`，两种都必须映射到同一命令。
    assert_eq!(
        super::route(
            key(KeyCode::BackTab),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::PrevStep))
    );
    assert_eq!(
        super::route(
            KeyEvent::new(KeyCode::Tab, KeyModifiers::SHIFT),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::PrevStep))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Esc),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::Escape))
    );
    // Shift+Enter 在应用层区分：提及面板打开时进入高亮目录，否则回落为换行；
    // Enter 无面板时交给 update 判定（有面板则采纳候选）。
    let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
    assert_eq!(
        super::route(
            shift_enter,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::EnterFolder))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Enter),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::Enter))
    );
    // Ctrl+A/E/U/K/W/D 等编辑快捷键映射到 lx 输入命令。
    let ctrl = |ch| {
        super::route(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true,
        )
    };
    assert_eq!(ctrl('a'), Some(Routed::Lx(LxInput::LineStart)));
    assert_eq!(ctrl('e'), Some(Routed::Lx(LxInput::LineEnd)));
    assert_eq!(ctrl('u'), Some(Routed::Lx(LxInput::DeleteToLineStart)));
    assert_eq!(ctrl('k'), Some(Routed::Lx(LxInput::DeleteToLineEnd)));
    assert_eq!(ctrl('w'), Some(Routed::Lx(LxInput::DeleteWordBackward)));
    assert_eq!(ctrl('c'), None, "Ctrl+C 由全局复制处理");
    // Cmd+C（macOS）与 Ctrl+C 落到同一命令：是否复制还是清空由 update 层按面板决定。
    let cmd = |ch| {
        super::route(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::SUPER),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true,
        )
    };
    assert_eq!(cmd('c'), None, "Cmd+C 由全局复制处理");
    assert_eq!(cmd('C'), None, "Cmd+C 由全局复制处理");
    // SUPER 分支既有的撤销/重做不受影响。
    assert_eq!(cmd('z'), Some(Routed::Lx(LxInput::Undo)));
    assert_eq!(cmd('y'), Some(Routed::Lx(LxInput::Redo)));
    assert_eq!(ctrl('z'), Some(Routed::Lx(LxInput::Undo)));
    assert_eq!(ctrl('y'), Some(Routed::Lx(LxInput::Redo)));
    let ctrl_left = KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL);
    assert_eq!(
        super::route(
            ctrl_left,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Lx(LxInput::WordLeft))
    );
    let quit = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
    assert_eq!(
        super::route(
            quit,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            None,
            true
        ),
        Some(Routed::Action(Action::Quit))
    );
    // prompt 聚焦优先于 lx 输入：按键仍进入编辑器。
    assert_eq!(
        super::route(
            key(KeyCode::Char('a')),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Prompt,
            None,
            true
        ),
        Some(Routed::Editor(EditorCommand::InsertChar('a')))
    );
}

#[test]
fn worktree_open_overlay_routes_search_and_navigation_keys() {
    let route_overlay = |code| {
        super::route(
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
        super::route(
            ctrl,
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            Some(OverlayKind::WorktreeOpen),
            false
        ),
        Some(Routed::Overlay(OverlayKey::Home))
    );
}

#[test]
fn save_draft_overlay_routes_navigation_only() {
    let route_overlay = |code| {
        super::route(
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

// ===== 分页：页码菜单与页重命名输入路由 =====

#[test]
fn page_menu_overlay_routes_navigation_keys_only() {
    let overlay = Some(OverlayKind::PageMenu);
    assert_eq!(
        super::route(
            key(KeyCode::Esc),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            overlay,
            false
        ),
        Some(Routed::Overlay(OverlayKey::Esc))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Down),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            overlay,
            false
        ),
        Some(Routed::Overlay(OverlayKey::Down))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Up),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            overlay,
            false
        ),
        Some(Routed::Overlay(OverlayKey::Up))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Enter),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            overlay,
            false
        ),
        Some(Routed::Overlay(OverlayKey::Enter))
    );
    assert_eq!(
        super::route(
            key(KeyCode::Char('x')),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Panes,
            overlay,
            false
        ),
        None,
        "其余按键吞掉"
    );
}

#[test]
fn page_rename_key_maps_single_line_input_semantics() {
    assert_eq!(
        page_rename_key(key(KeyCode::Enter)),
        Some(OverlayKey::Enter)
    );
    assert_eq!(page_rename_key(key(KeyCode::Esc)), Some(OverlayKey::Esc));
    assert_eq!(
        page_rename_key(key(KeyCode::Char('a'))),
        Some(OverlayKey::Char('a'))
    );
    assert_eq!(
        page_rename_key(key(KeyCode::Backspace)),
        Some(OverlayKey::Backspace)
    );
    assert_eq!(page_rename_key(key(KeyCode::Left)), Some(OverlayKey::Left));
    assert_eq!(
        page_rename_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        None
    );
    assert_eq!(
        page_rename_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        Some(OverlayKey::Home)
    );
    assert_eq!(
        page_rename_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
        Some(OverlayKey::End)
    );
    assert_eq!(
        page_rename_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
        None,
        "Ctrl 组合不进入输入（Ctrl+Q 由上层退出）"
    );
    assert_eq!(page_rename_key(key(KeyCode::Tab)), None);
}

#[test]
fn todo_focus_maps_ctrl_a_and_e_to_line_edges() {
    let route_todo = |ch| {
        super::route(
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL),
            TermMode::empty(),
            crate::app::state::FocusedContainer::Todo,
            None,
            false,
        )
    };
    assert_eq!(route_todo('a'), Some(Routed::Todo(TodoKey::Home)));
    assert_eq!(route_todo('e'), Some(Routed::Todo(TodoKey::End)));
}

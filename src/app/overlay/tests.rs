//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn text_input_prefills_cursor_at_end() {
    let input = TextInput::new("workspace 2");
    assert_eq!(input.text(), "workspace 2");
    assert_eq!(input.cursor(), 11);
}

#[test]
fn text_input_edits_at_cursor() {
    let mut input = TextInput::new("ab");
    input.move_left();
    input.insert_char('X');
    assert_eq!(input.text(), "aXb");
    assert_eq!(input.cursor(), 2);
    input.backspace();
    assert_eq!(input.text(), "ab");
    input.delete();
    assert_eq!(input.text(), "a");
    input.move_home();
    input.delete();
    assert_eq!(input.text(), "");
    input.move_end();
    input.backspace();
    assert_eq!(input.text(), "");
    input.backspace();
    assert_eq!(input.cursor(), 0);
}

#[test]
fn text_input_handles_multibyte_characters() {
    let mut input = TextInput::new("工作区");
    input.backspace();
    assert_eq!(input.text(), "工作");
    input.move_home();
    input.insert_char('新');
    assert_eq!(input.text(), "新工作");
    assert_eq!(input.cursor(), 1);
    input.move_right();
    input.delete();
    assert_eq!(input.text(), "新工");
}

#[test]
fn text_input_movement_clamps_at_bounds() {
    let mut input = TextInput::new("");
    input.move_left();
    input.move_right();
    assert_eq!(input.cursor(), 0);
    input.insert_char('a');
    input.move_end();
    input.move_right();
    assert_eq!(input.cursor(), 1);
}

#[test]
fn overlay_kind_reports_variant() {
    let menu = Overlay::Menu(Menu {
        anchor: (0, 0),
        target: MenuTarget::Workspace(0),
        commands: vec![MenuCommand::RenameWorkspace],
        selected: 0,
    });
    assert_eq!(menu.kind(), OverlayKind::Menu);
    let rename = Overlay::Rename(Rename {
        target: 0,
        input: TextInput::new("a"),
    });
    assert_eq!(rename.kind(), OverlayKind::Rename);
    let confirm = Overlay::ConfirmClose(ConfirmClose { target: 0 });
    assert_eq!(confirm.kind(), OverlayKind::ConfirmClose);
}

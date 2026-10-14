//! lx 输入框粘贴单元测试：净化、撤销步、面板互斥与参照面板状态机。

use std::path::PathBuf;

use super::*;
use crate::app::paste::PasteCandidateKind;

/// 一个存在的目录候选（引用可无损表达）。
fn folder(path: &str) -> PasteCandidate {
    PasteCandidate::external(PathBuf::from(path), PasteCandidateKind::Folder)
}

#[test]
fn insert_str_sanitizes_line_endings_tabs_and_controls() {
    let mut lx = Lx::new();
    lx.insert_str("a\r\nb\tc\x07d");
    assert_eq!(lx.input, "a\nb  cd");
    assert_eq!(lx.cursor, lx.input.len(), "光标落在插入文本末尾");
}

#[test]
fn insert_str_is_a_single_undo_step() {
    let mut lx = Lx::new();
    lx.insert_str("hello\nworld");
    lx.undo();
    assert_eq!(lx.input, "", "整段粘贴一步撤销");
    lx.redo();
    assert_eq!(lx.input, "hello\nworld");
}

#[test]
fn insert_str_inserts_at_cursor() {
    let mut lx = Lx::new();
    lx.insert_str("ab");
    lx.move_left();
    lx.insert_str("X");
    assert_eq!(lx.input, "aXb");
    assert_eq!(lx.cursor, 2);
}

#[test]
fn insert_str_empty_after_sanitize_keeps_no_undo_step() {
    let mut lx = Lx::new();
    lx.insert_str("\x07\x07");
    assert_eq!(lx.input, "");
    lx.undo();
    assert_eq!(lx.input, "", "没有产生撤销步");
}

#[test]
fn insert_str_suppresses_markdown_block_panel() {
    let mut lx = Lx::new();
    lx.insert_str("##");
    assert!(lx.panel.is_none(), "粘贴不弹 md 块面板");
    // 触发标记确实存在，只是被粘贴压制；下一次编辑才刷新。
    lx.refresh_panel();
    assert!(lx.panel.is_some(), "行尾 ## 本是块触发标记");
}

#[test]
fn paste_open_inserts_padded_path_placeholder_and_opens_panel() {
    let mut lx = Lx::new();
    lx.insert_char('x');
    assert!(lx.paste_open(vec![folder("/tmp/dir")]));
    assert_eq!(lx.input, "x /tmp/dir ", "占位路径带前后空格");
    assert_eq!(lx.cursor, lx.input.len());
    let panel = lx.paste_panel().expect("panel");
    assert!(panel.reference_enabled());
    assert_eq!(panel.active_action(), Some(PasteAction::Reference));
    assert!(lx.panel.is_none(), "粘贴面板与 md 块面板互斥");
}

#[test]
fn paste_open_empty_candidates_is_noop() {
    let mut lx = Lx::new();
    assert!(!lx.paste_open(Vec::new()));
    assert_eq!(lx.input, "");
    assert!(lx.paste_panel().is_none());
}

#[test]
fn paste_move_cycles_to_path_action() {
    let mut lx = Lx::new();
    lx.paste_open(vec![folder("/tmp/dir")]);
    assert!(lx.paste_move(1));
    assert_eq!(
        lx.paste_panel().and_then(PastePanel::active_action),
        Some(PasteAction::Path)
    );
}

#[test]
fn paste_confirm_reference_rewrites_in_same_undo_step() {
    let mut lx = Lx::new();
    lx.paste_open(vec![folder("/tmp/dir")]);
    assert!(lx.paste_confirm(PasteAction::Reference));
    assert_eq!(lx.input, "@[refer-folder](/tmp/dir) ");
    assert!(lx.paste_panel().is_none());
    lx.undo();
    assert_eq!(lx.input, "", "占位插入与引用改写同属一步撤销");
}

#[test]
fn paste_confirm_path_keeps_placeholder_text() {
    let mut lx = Lx::new();
    lx.paste_open(vec![folder("/tmp/dir")]);
    assert!(lx.paste_confirm(PasteAction::Path));
    assert_eq!(lx.input, "/tmp/dir ");
    assert!(lx.paste_panel().is_none());
}

#[test]
fn paste_cancel_rolls_back_without_residual_undo_step() {
    let mut lx = Lx::new();
    lx.insert_char('a');
    lx.paste_open(vec![folder("/tmp/dir")]);
    assert!(lx.paste_cancel());
    assert_eq!(lx.input, "a", "取消回滚占位插入");
    assert!(lx.paste_panel().is_none());
    lx.undo();
    assert_eq!(lx.input, "", "回滚不残留粘贴撤销步");
}

#[test]
fn paste_cancel_without_panel_is_false() {
    let mut lx = Lx::new();
    assert!(!lx.paste_cancel());
}

//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn ellipsize_keeps_text_within_limit() {
    assert_eq!(ellipsize("abc", 3), "abc");
    assert_eq!(ellipsize("abc", 5), "abc");
}

#[test]
fn ellipsize_marks_truncation() {
    assert_eq!(ellipsize("Copied to clipboard", 5), "Copi…");
    assert_eq!(ellipsize("abc", 1), "…");
    assert_eq!(ellipsize("abc", 0), "");
}

#[test]
fn block_command_text_covers_all_commands() {
    assert_eq!(
        block_command_text(BlockCommandId::Heading(3)),
        ("Heading 3".to_string(), "###".to_string())
    );
    assert_eq!(
        block_command_text(BlockCommandId::UnorderedList),
        ("Bullet List".to_string(), "-".to_string())
    );
    assert_eq!(
        block_command_text(BlockCommandId::TaskList),
        ("Task List".to_string(), "- [ ]".to_string())
    );
    assert_eq!(
        block_command_text(BlockCommandId::OrderedList),
        ("Numbered List".to_string(), "1.".to_string())
    );
    assert_eq!(
        block_command_text(BlockCommandId::Quote),
        ("Quote".to_string(), ">".to_string())
    );
    assert_eq!(
        block_command_text(BlockCommandId::CodeBlock),
        ("Code Block".to_string(), "```".to_string())
    );
    assert_eq!(
        block_command_text(BlockCommandId::Table),
        ("Table".to_string(), "|  |  |".to_string())
    );
}

#[test]
fn pane_title_prefers_osc_then_cwd_label_then_id() {
    let id = PaneId::alloc();
    assert_eq!(
        pane_title(id, Some("Claude Code"), Some("lx-tui")),
        "Claude Code"
    );
    assert_eq!(pane_title(id, Some("  "), Some("lx-tui")), "lx-tui");
    assert_eq!(pane_title(id, None, Some("")), format!("pane {}", id.raw()));
    assert_eq!(pane_title(id, None, None), format!("pane {}", id.raw()));
}

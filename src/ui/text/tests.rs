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

#[test]
fn ui_icons_are_single_width_and_non_pua() {
    use unicode_width::UnicodeWidthStr;

    assert_eq!(MENTION_DIR_ICON.width(), 1, "目录图标必须严格占用 1 列宽");
    assert_eq!(MENTION_FILE_ICON.width(), 1, "文件图标必须严格占用 1 列宽");
    assert_ne!(
        MENTION_DIR_ICON, MENTION_FILE_ICON,
        "目录与文件图标必须不同"
    );

    assert_eq!(TAB_ITEM_ICON.width(), 1, "Tab 图标必须严格占用 1 列宽");
    assert_eq!(WORKSPACE_GIT_ICON.width(), 1, "Git 图标必须严格占用 1 列宽");
    assert_eq!(
        WORKSPACE_NON_GIT_ICON.width(),
        1,
        "非 Git 图标必须严格占用 1 列宽"
    );
    assert_ne!(
        WORKSPACE_GIT_ICON, WORKSPACE_NON_GIT_ICON,
        "Git 与非 Git 图标必须不同"
    );

    for (name, icon) in [
        ("MENTION_DIR_ICON", MENTION_DIR_ICON),
        ("MENTION_FILE_ICON", MENTION_FILE_ICON),
        ("TAB_ITEM_ICON", TAB_ITEM_ICON),
        ("WORKSPACE_GIT_ICON", WORKSPACE_GIT_ICON),
        ("WORKSPACE_NON_GIT_ICON", WORKSPACE_NON_GIT_ICON),
    ] {
        for ch in icon.chars() {
            let cp = ch as u32;
            let is_pua = (0xE000..=0xF8FF).contains(&cp)
                || (0xF0000..=0xFFFFD).contains(&cp)
                || (0x100000..=0x10FFFD).contains(&cp);
            assert!(
                !is_pua,
                "{name} 包含私有区(PUA)字符 U+{cp:04X}，在未安装专用字体的终端中会乱码"
            );
        }
    }
}

#[test]
fn mention_panel_footer_symbols_are_single_width_and_non_pua() {
    use unicode_width::UnicodeWidthChar;

    for (name, symbol) in [("Shift", '⇧'), ("Return", '↵'), ("Backspace", '⌫')] {
        assert!(
            MENTION_PANEL_FOOTER.contains(symbol),
            "底边提示缺少 {name} 符号"
        );
        assert_eq!(symbol.width(), Some(1), "{name} 符号必须严格占用 1 列宽");
        let cp = symbol as u32;
        let is_pua = (0xE000..=0xF8FF).contains(&cp)
            || (0xF0000..=0xFFFFD).contains(&cp)
            || (0x100000..=0x10FFFD).contains(&cp);
        assert!(
            !is_pua,
            "{name} 使用了私有区(PUA)字符 U+{cp:04X}，主流终端会乱码"
        );
    }
}

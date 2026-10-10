//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn ellipsize_keeps_text_within_limit() {
    assert_eq!(ellipsize("abc", 3), "abc");
    assert_eq!(ellipsize("abc", 5), "abc");
}

#[test]
fn prompt_status_labels_cover_all_states() {
    use crate::app::prompt::PromptItemStatus;
    assert_eq!(
        prompt_status_label(PromptItemStatus::Todo),
        TEMPLATE_BUTTON_TODO
    );
    assert_eq!(
        prompt_status_label(PromptItemStatus::InProgress),
        TEMPLATE_BUTTON_RUN
    );
    assert_eq!(
        prompt_status_label(PromptItemStatus::Completed),
        TEMPLATE_BUTTON_DONE
    );
    assert_eq!(
        prompt_status_menu_label(PromptItemStatus::Todo),
        MENU_PROMPT_MARK_TODO
    );
    assert_eq!(
        prompt_status_menu_label(PromptItemStatus::InProgress),
        MENU_PROMPT_MARK_RUN
    );
    assert_eq!(
        prompt_status_menu_label(PromptItemStatus::Completed),
        MENU_PROMPT_MARK_DONE
    );
}

#[test]
fn user_visible_labels_are_clean_ascii() {
    // 用户可见文案禁止 Unicode 替换字符（U+FFFD），且不得为空。
    for label in [
        PROMPT_MODE_LIST_LABEL,
        PROMPT_MODE_EDIT_LABEL,
        PROMPT_TREE_EMPTY,
        PROMPT_TREE_NO_MATCHES,
        PROMPT_TREE_SEARCH_PLACEHOLDER,
        PROMPT_TREE_DRAFT_LABEL,
        SAVE_DRAFT_TITLE,
        SAVE_DRAFT_ROOT,
        SAVE_DRAFT_NEW_FOLDER,
        SAVE_DRAFT_FOOTER,
        RENAME_NEW_FOLDER_TITLE,
        PROMPT_DEFAULT_NAME,
        PROMPT_FOLDER_DEFAULT_NAME,
        MENU_PROMPT_NEW_PROMPT,
        MENU_PROMPT_NEW_FOLDER,
        MENU_PROMPT_NEW_SUBFOLDER,
        MENU_PROMPT_EDIT,
        MENU_PROMPT_RENAME,
        MENU_PROMPT_COPY,
        MENU_PROMPT_DELETE,
        MENU_PROMPT_MARK_TODO,
        MENU_PROMPT_MARK_RUN,
        MENU_PROMPT_MARK_DONE,
        TRUST_WORKSPACE_TITLE,
        TRUST_WORKSPACE_QUESTION,
        BUTTON_TRUST,
        BUTTON_DONT_TRUST,
        TRUST_CONSOLE_HEADING,
        TRUST_CONSOLE_SAFETY,
        TRUST_CONSOLE_HINT,
        TRUST_CONSOLE_PROMPT_HINT,
        TRUST_CONSOLE_PROMPT_HINT,
        TRUST_OPTION_YES,
        TRUST_OPTION_NO,
        TRUST_CONSOLE_FOOTER,
        TRUST_REFUSED_MESSAGE,
        TOAST_CREATE_CANCELLED,
        TOAST_SYNC_CANCELLED,
        MENU_TITLE_PROMPT,
        MENU_TITLE_FOLDER,
        MENU_TITLE_PROMPT_TREE,
        CONFIRM_DELETE_PROMPT_TITLE,
        CONFIRM_DELETE_FOLDER_TITLE,
        RENAME_PROMPT_TITLE,
        RENAME_FOLDER_TITLE,
    ] {
        assert!(!label.is_empty(), "文案不得为空");
        assert!(!label.contains('\u{fffd}'), "文案禁止替换字符: {label}");
    }
    assert_eq!(confirm_delete_question("draft"), "delete \"draft\"?");
    assert_eq!(
        confirm_delete_folder_question("ideas"),
        "delete folder \"ideas\" and its contents?"
    );
}

#[test]
fn ellipsize_marks_truncation() {
    assert_eq!(ellipsize("Copied to clipboard", 5), "Copi…");
    assert_eq!(ellipsize("abc", 1), "…");
    assert_eq!(ellipsize("abc", 0), "");
}

#[test]
fn ellipsize_middle_keeps_head_and_tail() {
    assert_eq!(ellipsize_middle("abcdefgh", 8), "abcdefgh");
    assert_eq!(ellipsize_middle("abcdefgh", 9), "abcdefgh");
    assert_eq!(ellipsize_middle("abcdefgh", 5), "ab…gh");
    assert_eq!(ellipsize_middle("abcdefgh", 4), "ab…h");
    assert_eq!(ellipsize_middle("abcdefgh", 1), "…");
    assert_eq!(ellipsize_middle("abcdefgh", 0), "");
}

#[test]
fn ellipsize_middle_measures_display_width() {
    // 4 个 CJK 字符 = 8 列；预算 5 列：保留首 2 列 + `…` + 尾 2 列。
    assert_eq!(ellipsize_middle("中文路径", 5), "中…径");
    // 预算 4 列：尾预算 1 列放不下宽字符，省略尾部。
    assert_eq!(ellipsize_middle("中文路径", 4), "中…");
    // 零宽字符不占列宽。
    assert_eq!(ellipsize_middle("a\u{200b}bcdef", 5), "a\u{200b}b…ef");
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
fn confirm_switch_cwd_question_wraps_path_in_quotes() {
    assert_eq!(
        confirm_switch_cwd_question("/tmp/project"),
        "switch cwd to \"/tmp/project\"?"
    );
}

#[test]
fn confirm_sync_ws_cwd_question_ellipsizes_middle_when_budget_is_small() {
    assert_eq!(
        confirm_sync_ws_cwd_question("/tmp/project", 60),
        "switch workspace path to \"/tmp/project\"?"
    );
    // prefix (26) + suffix (2) = 28. max_width = 34 -> path_budget = 6 -> "/tm…ct"
    assert_eq!(
        confirm_sync_ws_cwd_question("/tmp/project", 34),
        "switch workspace path to \"/tm…ct\"?"
    );
}

#[test]
fn workspace_menu_copy_is_ws_abbreviated_english() {
    assert_eq!(MENU_NEW_TERMINAL, "New terminal");
    assert_eq!(MENU_OPEN_PROMPT, "Open prompt");
    assert_eq!(MENU_SWITCH_TO_WORKSPACE_CWD, "Switch to ws path");
    assert_eq!(MENU_SYNC_WS_TO_TERMINAL_CWD, "Sync ws to terminal path");
    assert_eq!(CONFIRM_SWITCH_CWD_TITLE, "switch to ws path");
    assert_eq!(CONFIRM_SYNC_WS_CWD_TITLE, "sync ws to terminal path");
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

    for (name, symbol) in [("Shift", '⇧'), ("Return", '↵')] {
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
    assert!(
        MENTION_PANEL_FOOTER.contains("^z"),
        "回退提示沿用 [clear ^c] 的 ^z 记法"
    );
}

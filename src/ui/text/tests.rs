//! 单元测试；仅测试构建编译。

use super::*;

#[test]
fn ellipsize_keeps_text_within_limit() {
    assert_eq!(ellipsize("abc", 3), "abc");
    assert_eq!(ellipsize("abc", 5), "abc");
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
        PROMPT_TEMP_PROMPT_LABEL,
        PROMPT_SECTION_TMP,
        PROMPT_SECTION_SAVED,
        MODAL_CLOSE,
        SAVE_DRAFT_TITLE,
        SAVE_DRAFT_ROOT,
        SAVE_DRAFT_NEW_FOLDER,
        PROMPT_DEFAULT_NAME,
        PROMPT_FOLDER_DEFAULT_NAME,
        MENU_PROMPT_NEW_PROMPT,
        MENU_PROMPT_NEW_FOLDER,
        MENU_PROMPT_NEW_SUBFOLDER,
        MENU_PROMPT_EDIT,
        MENU_PROMPT_RENAME,
        MENU_PROMPT_COPY,
        MENU_PROMPT_DELETE,
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
fn confirm_sync_ws_cwd_question_keeps_full_path() {
    assert_eq!(
        confirm_sync_ws_cwd_question("/tmp/project"),
        "switch workspace path to \"/tmp/project\"?"
    );
}

#[test]
fn wrap_breaks_at_spaces_within_display_width() {
    assert_eq!(
        wrap("delete folder \"New folder\" and its contents?", 38),
        vec!["delete folder \"New folder\" and its", "contents?"]
    );
    assert_eq!(wrap("close \"a\"?", 40), vec!["close \"a\"?"]);
    assert_eq!(wrap("", 10), vec![String::new()]);
}

#[test]
fn wrap_hard_breaks_overlong_words_by_character() {
    assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
    // 宽字符按两列计：每行 2 个字符（4 列）。
    assert_eq!(wrap("字字字字字", 4), vec!["字字", "字字", "字"]);
}

#[test]
fn workspace_menu_copy_is_ws_abbreviated_english() {
    assert_eq!(MENU_NEW_TERMINAL, "New terminal");
    assert_eq!(MENU_OPEN_PROMPT, "Open prompt");
    assert_eq!(MENU_SWITCH_TO_WORKSPACE_CWD, "Switch to ws path");
    assert_eq!(MENU_SYNC_WS_TO_TERMINAL_CWD, "Sync ws to terminal path");
    assert_eq!(CONFIRM_SWITCH_CWD_TITLE, "Switch to ws path");
    assert_eq!(CONFIRM_SYNC_WS_CWD_TITLE, "Sync ws to terminal path");
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
    assert_eq!(
        INPUT_PREFIX_ICON.width(),
        1,
        "输入框前缀图标必须严格占用 1 列宽"
    );
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
fn prompt_button_icons_are_single_width_and_non_pua() {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    // 按钮标签形如 `[x]`：方括号 ASCII，中间单个图标必须严格 1 列宽且非私有区。
    for (name, label) in [
        ("PROMPT_UNDO_LABEL", PROMPT_UNDO_LABEL),
        ("PROMPT_REDO_LABEL", PROMPT_REDO_LABEL),
        ("PROMPT_SELECT_ALL_LABEL", PROMPT_SELECT_ALL_LABEL),
        ("PROMPT_MODE_LIST_LABEL", PROMPT_MODE_LIST_LABEL),
        ("PROMPT_MODE_EDIT_LABEL", PROMPT_MODE_EDIT_LABEL),
        ("PROMPT_PINNED_LABEL", PROMPT_PINNED_LABEL),
        ("PROMPT_UNPINNED_LABEL", PROMPT_UNPINNED_LABEL),
        ("TEMPLATE_BUTTON_COPY", TEMPLATE_BUTTON_COPY),
        ("TEMPLATE_BUTTON_CLEAN", TEMPLATE_BUTTON_CLEAN),
        ("TEMPLATE_BUTTON_DEL", TEMPLATE_BUTTON_DEL),
    ] {
        let mut chars = label.chars();
        assert_eq!(chars.next(), Some('['), "{name} 左括号");
        let icon = chars.next().expect("图标字符");
        assert_eq!(chars.next(), Some(']'), "{name} 右括号");
        assert_eq!(chars.next(), None, "{name} 只含一个图标字符");
        assert_eq!(
            icon.width(),
            Some(1),
            "{name} 图标 {icon:?} 必须严格 1 列宽"
        );
        let cp = icon as u32;
        let is_pua = (0xE000..=0xF8FF).contains(&cp)
            || (0xF0000..=0xFFFFD).contains(&cp)
            || (0x100000..=0x10FFFD).contains(&cp);
        assert!(!is_pua, "{name} 图标 {icon:?} 位于私有区，终端会乱码");
    }
    assert_eq!(PROMPT_SAVE_DOT.width(), 1, "保存点必须严格 1 列宽");
    assert_eq!(PROMPT_DRAFT_DOT.width(), 1, "草稿空心点必须严格 1 列宽");
    assert_eq!(
        PROMPT_TEMP_PROMPT_LABEL.width(),
        5,
        "[tmp] 按钮标签宽度（布局常量同步）"
    );
    assert_eq!(PROMPT_SECTION_TMP, "tmp:");
    assert_eq!(PROMPT_SECTION_SAVED, "saved:");
}

/// 全局关闭按钮（模态 + toast）与 prompt 模板块删除按钮共用同一 `✕` 图标：
/// 图标严格相等、单宽、非私有区，且标签宽度与 `modal::close_button_rect` 的 3 列命中框一致。
#[test]
fn modal_close_matches_template_delete_icon() {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    let glyph = |label: &str| label.chars().nth(1).expect("图标字符");
    assert_eq!(
        glyph(MODAL_CLOSE),
        glyph(TEMPLATE_BUTTON_DEL),
        "全局关闭按钮与模板块删除按钮必须使用同一图标"
    );
    assert_eq!(MODAL_CLOSE.width(), 3, "关闭按钮宽度必须与 3 列命中框一致");
    let icon = glyph(MODAL_CLOSE);
    assert_eq!(icon.width(), Some(1), "关闭图标 {icon:?} 必须严格 1 列宽");
    let cp = icon as u32;
    let is_pua = (0xE000..=0xF8FF).contains(&cp)
        || (0xF0000..=0xFFFFD).contains(&cp)
        || (0x100000..=0x10FFFD).contains(&cp);
    assert!(!is_pua, "关闭图标 {icon:?} 位于私有区，终端会乱码");
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

#[test]
fn workspace_path_label_shows_last_two_segments_then_ellipsizes() {
    use std::path::Path;
    let path = Path::new("/work/demo/src/main.rs");
    // 末两段（`src/main.rs`）放得下。
    assert_eq!(
        workspace_path_label(path, 40).as_deref(),
        Some("src/main.rs")
    );
    // 放不下则中间省略。
    let short = workspace_path_label(path, 6).expect("ellipsized");
    assert!(short.contains('…'), "超宽截断带省略号: {short:?}");
    assert!(short.width() <= 6);
    // 连省略号也放不下（可用宽度 < 2）：不绘制。
    assert_eq!(workspace_path_label(path, 1), None);
    assert_eq!(workspace_path_label(path, 0), None);
}

#[test]
fn workspace_path_label_uses_terminal_columns_for_cjk() {
    use std::path::Path;
    // 末两段含 CJK：宽度按终端列计（每字 2 列）。
    let path = Path::new("/work/demo/文档");
    // 末两段 = `demo/文档`（宽度 4 + 1 + 4 = 9）。
    assert_eq!(workspace_path_label(path, 20).as_deref(), Some("demo/文档"));
    let clipped = workspace_path_label(path, 5).expect("clipped");
    assert!(clipped.width() <= 5, "宽度按列计算: {clipped:?}");
    assert!(clipped.contains('…'));
}

#[test]
fn workspace_path_label_single_segment_and_root() {
    use std::path::Path;
    // 单段：用现有全部段。
    assert_eq!(
        workspace_path_label(Path::new("/lx-tui"), 8).as_deref(),
        Some("lx-tui")
    );
    // 根路径无可用段：回退完整路径显示。
    assert_eq!(
        workspace_path_label(Path::new("/"), 8).as_deref(),
        Some("/")
    );
}

#[test]
fn tool_verb_and_icon_constants_are_shared_with_agent_gallery() {
    use super::agent as a;

    // 真实工具行与 gallery 样例共用同一份图标：四工具各有一个单宽符号，
    // 且未知工具回落到通用符号。
    for icon in [a::ICON_READ, a::ICON_SEARCH, a::ICON_EDIT, a::ICON_WRITE] {
        assert!(!icon.is_empty());
        assert_eq!(icon.width(), 1, "工具图标必须严格 1 列宽: {icon:?}");
    }
    assert_eq!(a::ICON_TOOL, a::ICON_MCP, "未知工具复用通用调用符号");

    // 动词常量非空且两处同源（`read` 与 gallery 的 `Read` 完全同形）。
    assert_eq!(a::VERB_TOOL_READ, a::VERB_READ);
    for verb in [
        a::VERB_TOOL_READ,
        a::VERB_TOOL_SEARCH,
        a::VERB_TOOL_EDIT,
        a::VERB_TOOL_WRITE,
    ] {
        assert!(!verb.is_empty(), "动词不得为空");
        assert!(!verb.contains('\u{fffd}'), "动词禁止替换字符: {verb}");
    }
}

#[test]
fn tool_count_labels_use_shared_prefixes() {
    use super::agent as a;

    // `+N` / `-N` 与 gallery 样例的 `+12` / `-4` 前缀完全一致（不各写一份）。
    assert_eq!(a::added_label(12), a::EDIT_DIFF_ADD);
    assert_eq!(a::added_label(18), a::WRITE_ADD);
    assert_eq!(a::removed_label(4), a::EDIT_DIFF_DEL);
    // 行范围与命中数（含单复数）。
    assert_eq!(a::range_label(12, 40), "12-40");
    assert_eq!(a::matches_label(0), "0 matches");
    assert_eq!(a::matches_label(1), "1 match");
    assert_eq!(a::matches_label(3), "3 matches");
    // 截断提示非空且明确。
    assert!(a::TOOL_TRUNCATED.contains("truncated"));
}

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

    // 按钮标签形如 `[x]`，只允许两种形态，不得混写：
    // 1. 单个非 PUA 图标字符，严格 1 列宽；
    // 2. 纯 `a-z` 英文单词（`[all]` / `[dir]` / `[tmp]`）。
    for (name, label) in [
        ("PROMPT_UNDO_LABEL", PROMPT_UNDO_LABEL),
        ("PROMPT_REDO_LABEL", PROMPT_REDO_LABEL),
        ("PROMPT_MODE_LIST_LABEL", PROMPT_MODE_LIST_LABEL),
        ("PROMPT_MODE_EDIT_LABEL", PROMPT_MODE_EDIT_LABEL),
        ("PROMPT_UNPINNED_LABEL", PROMPT_UNPINNED_LABEL),
        ("PROMPT_PINNED_LABEL", PROMPT_PINNED_LABEL),
        ("PROMPT_BLOCK_FOCUS_ALL_LABEL", PROMPT_BLOCK_FOCUS_ALL_LABEL),
        ("PROMPT_BLOCK_FOCUS_ONE_LABEL", PROMPT_BLOCK_FOCUS_ONE_LABEL),
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

    // 单词形态：方括号内全部为小写 ASCII 字母，不含图标、不含空白。
    for (name, label) in [
        ("PROMPT_SELECT_ALL_LABEL", PROMPT_SELECT_ALL_LABEL),
        ("PROMPT_TEMP_PROMPT_LABEL", PROMPT_TEMP_PROMPT_LABEL),
        ("PROMPT_FOLDERS_LABEL", PROMPT_FOLDERS_LABEL),
    ] {
        let inner = label
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .unwrap_or_else(|| panic!("{name} 必须形如 [word]: {label:?}"));
        assert!(!inner.is_empty(), "{name} 单词不得为空");
        assert!(
            inner.chars().all(|ch| ch.is_ascii_lowercase()),
            "{name} 单词只能含 a-z，不得混入图标或大写: {label:?}"
        );
        assert!(
            label.chars().filter(|ch| *ch == '[').count() == 1
                && label.chars().filter(|ch| *ch == ']').count() == 1,
            "{name} 只允许一对方括号: {label:?}"
        );
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

/// Todo 三态状态框必须同宽：渲染按固定 3 列推进，宽窄不一会让正文列错位。
#[test]
fn todo_labels_align_to_three_columns() {
    use unicode_width::UnicodeWidthStr;

    for (name, label) in [
        ("TODO_STATUS_TODO", TODO_STATUS_TODO),
        ("TODO_STATUS_IN_PROGRESS", TODO_STATUS_IN_PROGRESS),
        ("TODO_STATUS_DONE", TODO_STATUS_DONE),
    ] {
        assert_eq!(label.chars().count(), 3, "{name} 必须 3 个字符: {label:?}");
        assert_eq!(label.width(), 3, "{name} 必须占 3 列: {label:?}");
        assert!(label.starts_with('[') && label.ends_with(']'), "{name}");
    }
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
fn tool_verb_and_icon_constants_are_single_width_and_non_empty() {
    use super::agent as a;

    // 工具图标各有一个单宽符号（真实工具行自 G11 起改用统一步骤方块，
    // 不再按工具区分类型图标；图标仍供步骤分类与测试基线使用）。
    for icon in [a::ICON_READ, a::ICON_SEARCH, a::ICON_EDIT, a::ICON_WRITE] {
        assert!(!icon.is_empty());
        assert_eq!(icon.width(), 1, "工具图标必须严格 1 列宽: {icon:?}");
    }

    // 动词常量非空，且 `read` 只写一份（工具行复用同一常量）。
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

    // `+N` / `-N` 前缀与常量同源（不各写一份）。
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

/// 快捷键符号是全项目唯一来源：单宽、非私有区、且各 hint 不出现全拼修饰键。
#[test]
fn shortcut_symbols_are_single_width_non_pua_and_used_consistently() {
    use unicode_width::UnicodeWidthChar;

    for (name, symbol) in [
        ("Shift", key::SHIFT),
        ("Enter", key::ENTER),
        ("Cmd", key::CMD),
        ("Backspace", key::BACKSPACE),
    ] {
        let mut chars = symbol.chars();
        let ch = chars.next().expect("非空");
        assert!(chars.next().is_none(), "{name} 必须是单个字符");
        assert_eq!(ch.width(), Some(1), "{name} 符号必须严格占 1 列宽");
        let cp = ch as u32;
        let is_pua = (0xE000..=0xF8FF).contains(&cp)
            || (0xF0000..=0xFFFFD).contains(&cp)
            || (0x100000..=0x10FFFD).contains(&cp);
        assert!(!is_pua, "{name} 使用了私有区(PUA)字符 U+{cp:04X}");
    }

    // 所有面向用户的快捷键提示：修饰键与"要按的键"一律用符号，不出现全拼。
    for (name, hint) in [
        ("slash panel", LX_SLASH_PANEL_FOOTER),
        ("history panel", LX_HISTORY_PANEL_FOOTER),
        ("resume edit confirm", RESUME_EDIT_CONFIRM_HINT),
        ("mention panel", MENTION_PANEL_FOOTER),
        ("referenced menu", REFERENCED_MENU_FOOTER),
        ("referenced search", REFERENCED_SEARCH_FOOTER),
        ("trust console", TRUST_CONSOLE_FOOTER),
    ] {
        let lower = hint.to_lowercase();
        for forbidden in ["shift", "ctrl", "cmd", "backspace"] {
            assert!(
                !lower.contains(forbidden),
                "{name} 提示出现全拼 `{forbidden}`（应用符号）: {hint}"
            );
        }
        // `enter` 作为按键提示时也用 `↵`；`esc` 保留全拼（规则允许）。
        if lower.contains("enter") {
            panic!("{name} 提示出现全拼 `enter`（应用 ↵）: {hint}");
        }
    }
}

/// 模态按钮与页脚里 `esc` 保留全拼（规则：动作词用文字，符号只给修饰键与键名）。
#[test]
fn action_words_stay_spelled_out_while_keys_use_symbols() {
    // `esc` 全拼保留；按键（enter/delete）用符号 + `^` 记法。
    assert!(BUTTON_CANCEL.contains("esc"));
    for hint in [
        BUTTON_SAVE,
        BUTTON_CONFIRM,
        BUTTON_CONFIRM_DELETE,
        BUTTON_OPEN,
    ] {
        assert!(hint.contains(key::ENTER), "按键用 ↵: {hint}");
        assert!(!hint.to_lowercase().contains("enter"), "不用全拼: {hint}");
    }
    // Ctrl 组合沿用 `^` 记法（已是全项目统一且终端圈通用）。
    assert!(BUTTON_CLEAR.contains("^c"));
    assert!(BUTTON_DELETE.contains("^d"));
    assert!(MENTION_PANEL_FOOTER.contains("^z"));
    // Cmd 只用一个字形，且与 web 图标不重名混淆：`⌘` 就是 `key::CMD`。
    assert_eq!(key::CMD, "⌘");
}

/// 身份标签：三段齐全时为 `项目 · b:分支 · wt:工作区名`。
#[test]
fn workspace_identity_spans_full_format() {
    let identity = WorkspaceIdentity {
        project: "lx-tui".into(),
        branch: Some("dev".into()),
        worktree: Some("lx-file-tool-diff".into()),
    };
    let spans = workspace_identity_spans(&identity, 40).expect("fits");
    let line: String = spans.iter().map(|span| span.text.as_str()).collect();
    assert_eq!(line, "lx-tui · b:dev · wt:lx-file-tool-diff");
    // 项目名与 `b:`/`wt:` 前缀强调；分隔符与值不强调。
    assert!(spans[0].emphasis, "项目名强调");
    assert!(!spans[1].emphasis, "分隔符不强调");
    assert!(spans[2].emphasis, "b: 前缀强调");
    assert!(!spans[3].emphasis, "分支值不强调");
    assert!(!spans[4].emphasis, "分隔符不强调");
    assert!(spans[5].emphasis, "wt: 前缀强调");
    assert!(!spans[6].emphasis, "工作区名不强调");
}

/// 缺段自动省略：非 git 只有项目名，主 checkout 无 `wt:`。
#[test]
fn workspace_identity_spans_drop_missing_segments() {
    let plain = WorkspaceIdentity {
        project: "lx-tui".into(),
        branch: None,
        worktree: None,
    };
    let line: String = workspace_identity_spans(&plain, 20)
        .expect("fits")
        .iter()
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(line, "lx-tui");

    let main = WorkspaceIdentity {
        project: "lx-tui".into(),
        branch: Some("dev".into()),
        worktree: None,
    };
    let line: String = workspace_identity_spans(&main, 20)
        .expect("fits")
        .iter()
        .map(|span| span.text.as_str())
        .collect();
    assert_eq!(line, "lx-tui · b:dev");
}

/// 降级阶梯：先砍 `wt:`，再省略项目名但保留 `b:`，最后只留项目名。
#[test]
fn workspace_identity_spans_degrade_in_order() {
    let identity = WorkspaceIdentity {
        project: "lx-tui".into(),
        branch: Some("dev".into()),
        worktree: Some("worktree-name".into()),
    };
    let render = |available: usize| -> String {
        workspace_identity_spans(&identity, available)
            .expect("spans")
            .iter()
            .map(|span| span.text.as_str())
            .collect()
    };
    // 16 列：`lx-tui · b:dev` 刚好放得下 → 砍 `wt:`。
    assert_eq!(render(16), "lx-tui · b:dev");
    // 12 列：`b:` 段保留，项目名中间省略。
    let clipped = render(12);
    assert!(clipped.contains("b:dev"), "{clipped}");
    assert!(clipped.contains('…'), "{clipped}");
    assert!(clipped.chars().count() <= 12, "{clipped}");
    // 5 列：只留项目名（同样可能省略）。
    let project_only = render(5);
    assert!(!project_only.contains("b:"), "{project_only}");
    assert!(project_only.chars().count() <= 5, "{project_only}");
    // 连省略号都放不下：不绘制。
    assert!(workspace_identity_spans(&identity, 1).is_none());
}

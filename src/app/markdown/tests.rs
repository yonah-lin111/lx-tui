//! 单元测试；仅测试构建编译。

use super::*;

/// 在文本末尾触发并返回类别。
fn trigger(text: &str) -> Option<BlockTriggerKind> {
    block_trigger(text, text.len()).map(|trigger| trigger.kind)
}

/// 触发区间（前导空格 + 标记 + 光标）。
fn range(text: &str) -> Option<(usize, usize)> {
    block_trigger(text, text.len()).map(|trigger| (trigger.from, trigger.to))
}

#[test]
fn heading_trigger_forms() {
    assert_eq!(trigger("#"), Some(BlockTriggerKind::Heading));
    assert_eq!(trigger("# "), Some(BlockTriggerKind::Heading));
    assert_eq!(trigger("###"), Some(BlockTriggerKind::Heading));
    assert_eq!(trigger("######"), Some(BlockTriggerKind::Heading));
    assert_eq!(trigger("  ## "), Some(BlockTriggerKind::Heading));
    assert_eq!(range("  ##"), Some((2, 4)));
    assert_eq!(trigger("#######"), None);
    assert_eq!(trigger("#x"), None);
    assert_eq!(trigger("# x"), None);
}

#[test]
fn list_quote_and_table_triggers() {
    assert_eq!(trigger("-"), Some(BlockTriggerKind::UnorderedList));
    assert_eq!(trigger("- "), Some(BlockTriggerKind::UnorderedList));
    assert_eq!(trigger("+"), Some(BlockTriggerKind::UnorderedList));
    assert_eq!(trigger("* "), Some(BlockTriggerKind::UnorderedList));
    assert_eq!(trigger("-x"), None);
    assert_eq!(trigger("- item"), None);

    assert_eq!(trigger("1."), Some(BlockTriggerKind::OrderedList));
    assert_eq!(trigger("1) "), Some(BlockTriggerKind::OrderedList));
    assert_eq!(trigger("2."), None);
    assert_eq!(trigger("11."), None);
    assert_eq!(trigger("1. item"), None);

    assert_eq!(trigger(">"), Some(BlockTriggerKind::Quote));
    assert_eq!(trigger("> "), Some(BlockTriggerKind::Quote));
    assert_eq!(trigger(">>"), None);
    assert_eq!(trigger("> quote"), None);

    assert_eq!(trigger("|"), Some(BlockTriggerKind::Table));
    assert_eq!(trigger("  |"), Some(BlockTriggerKind::Table));
    assert_eq!(trigger("| "), None);
}

#[test]
fn fence_trigger_forms() {
    assert_eq!(trigger("```"), Some(BlockTriggerKind::CodeBlock));
    assert_eq!(trigger("````"), Some(BlockTriggerKind::CodeBlock));
    assert_eq!(trigger("~~~"), Some(BlockTriggerKind::CodeBlock));
    assert_eq!(trigger("```rust"), None);
    assert_eq!(trigger("``` "), None);
    assert_eq!(trigger("``"), None);
}

#[test]
fn cursor_must_be_at_line_end() {
    assert_eq!(block_trigger("#", 0), None);
    assert_eq!(block_trigger("# x", 1), None);
    assert_eq!(block_trigger("# x", 3), None);
    let text = "# a\n##";
    assert_eq!(
        block_trigger(text, text.len()),
        Some(BlockTrigger {
            from: 4,
            to: 6,
            kind: BlockTriggerKind::Heading,
        })
    );
}

#[test]
fn triggers_suppressed_inside_fence() {
    assert_eq!(trigger("```\n#"), None);
    assert_eq!(trigger("```\nhello\n-"), None);
    assert_eq!(trigger("```\ncode\n```"), None);
    assert_eq!(trigger("~~~\n|"), None);
}

#[test]
fn fence_content_after_close_triggers_again() {
    assert_eq!(
        trigger("```\ncode\n```\n#"),
        Some(BlockTriggerKind::Heading)
    );
}

#[test]
fn continuous_blocks_suppress_panel() {
    assert_eq!(trigger("- a\n-"), None);
    assert_eq!(trigger("- [ ] a\n- "), None);
    assert_eq!(trigger("1. a\n1."), None);
    assert_eq!(trigger("> a\n>"), None);
    assert_eq!(trigger("| a |\n|"), None);
    assert_eq!(trigger("  - a\n  -"), None);
}

#[test]
fn heading_and_fence_are_not_suppressed_by_previous_line() {
    assert_eq!(trigger("# a\n#"), Some(BlockTriggerKind::Heading));
    assert_eq!(
        trigger("```\ncode\n```\n```"),
        Some(BlockTriggerKind::CodeBlock)
    );
}

#[test]
fn commands_by_trigger_kind() {
    assert_eq!(
        block_commands(BlockTriggerKind::Heading),
        (1..=6).map(BlockCommandId::Heading).collect::<Vec<_>>()
    );
    assert_eq!(
        block_commands(BlockTriggerKind::UnorderedList),
        vec![BlockCommandId::UnorderedList, BlockCommandId::TaskList]
    );
    assert_eq!(
        block_commands(BlockTriggerKind::OrderedList),
        vec![BlockCommandId::OrderedList]
    );
    assert_eq!(
        block_commands(BlockTriggerKind::Quote),
        vec![BlockCommandId::Quote]
    );
    assert_eq!(
        block_commands(BlockTriggerKind::CodeBlock),
        vec![BlockCommandId::CodeBlock]
    );
    assert_eq!(
        block_commands(BlockTriggerKind::Table),
        vec![BlockCommandId::Table]
    );
}

#[test]
fn insertions_replace_trigger_with_ready_skeleton() {
    for (level, expected) in [(1_u8, "# "), (2, "## "), (6, "###### ")] {
        let insertion = block_insertion(BlockCommandId::Heading(level));
        assert_eq!(insertion.text, *expected);
        assert_eq!(insertion.cursor, expected.len());
    }
    for (id, text, cursor) in [
        (BlockCommandId::UnorderedList, "- ", 2),
        (BlockCommandId::TaskList, "- [ ] ", 6),
        (BlockCommandId::OrderedList, "1. ", 3),
        (BlockCommandId::Quote, "> ", 2),
        (BlockCommandId::CodeBlock, "```\n```", 4),
        (BlockCommandId::Table, "|  |  |\n| --- | --- |\n|  |  |", 2),
    ] {
        let insertion = block_insertion(id);
        assert_eq!(insertion.text, text);
        assert_eq!(insertion.cursor, cursor);
    }
}

#[test]
fn panel_move_wraps_both_directions() {
    let trigger = BlockTrigger {
        from: 0,
        to: 1,
        kind: BlockTriggerKind::Heading,
    };
    let mut panel = BlockPanel::new(trigger, block_commands(BlockTriggerKind::Heading), 0);
    assert_eq!(panel.active(), 0);
    panel.move_active(-1);
    assert_eq!(panel.active(), 5);
    panel.move_active(1);
    assert_eq!(panel.active(), 0);
    panel.move_active(2);
    assert_eq!(panel.active(), 2);
}

#[test]
fn panel_hover_keeps_anchor_and_wheel_scrolls_viewport() {
    let trigger = BlockTrigger {
        from: 0,
        to: 1,
        kind: BlockTriggerKind::Heading,
    };
    let mut panel = BlockPanel::new(trigger, block_commands(BlockTriggerKind::Heading), 0);
    assert_eq!(panel.anchor(), 0);
    assert_eq!(panel.viewport(), None);

    panel.set_active(4);
    assert_eq!(panel.active(), 4);
    assert_eq!(panel.anchor(), 0, "悬停只改高亮，窗口锚点不动");

    // 滚轮只滚视口：高亮与锚点不动。
    assert!(panel.scroll_viewport(1, 0));
    assert_eq!(panel.viewport(), Some(1));
    assert_eq!(panel.active(), 4);
    assert_eq!(panel.anchor(), 0);

    // 视口两端钳制不循环；未移动返回 false。
    assert!(panel.scroll_viewport(99, 1));
    assert_eq!(panel.viewport(), Some(5));
    assert!(!panel.scroll_viewport(99, 5));
    assert!(panel.scroll_viewport(-99, 5));
    assert_eq!(panel.viewport(), Some(0));

    panel.set_active(99);
    assert_eq!(panel.active(), 5, "悬停索引越界钳到末项");
    panel.move_active(1);
    assert_eq!(panel.active(), 0, "键盘移动保持循环");
    assert_eq!(panel.viewport(), None, "键盘移动后窗口重新跟随高亮");
    assert_eq!(panel.anchor(), 0);
}

/// 文本末尾的提及触发。
fn mention(text: &str) -> Option<MentionTrigger> {
    mention_trigger(text, text.len())
}

#[test]
fn mention_trigger_forms() {
    assert_eq!(
        mention("@").map(|trigger| (trigger.from, trigger.query)),
        Some((0, String::new()))
    );
    assert_eq!(
        mention("a @sr").map(|trigger| (trigger.from, trigger.to, trigger.query)),
        Some((2, 5, "sr".into()))
    );
    assert_eq!(mention("[@sr").map(|trigger| trigger.from), Some(1));
    assert_eq!(
        mention("@src/app.rs").map(|trigger| trigger.query),
        Some("src/app.rs".into())
    );
    assert_eq!(mention("a@sr"), None);
    assert_eq!(mention("@a,b"), None);
    assert_eq!(mention("@a b"), None);
    assert_eq!(mention("@@a"), None);
}

#[test]
fn mention_trigger_allows_cjk_query() {
    assert_eq!(
        mention("@主页").map(|trigger| trigger.query),
        Some("主页".into())
    );
}

#[test]
fn mention_trigger_suppressed_inside_fence() {
    assert_eq!(mention("```\n@a"), None);
    assert_eq!(
        mention("```\ncode\n```\n@a").map(|trigger| trigger.query),
        Some("a".into())
    );
}

#[test]
fn mention_insertion_appends_trailing_space() {
    let file = MentionEntry {
        path: "src/app.rs".into(),
        is_directory: false,
    };
    assert_eq!(mention_insertion(&file), "@src/app.rs ");
    let directory = MentionEntry {
        path: "src".into(),
        is_directory: true,
    };
    assert_eq!(mention_insertion(&directory), "@src/ ");
}

#[test]
fn filter_mentions_ranks_name_matches_first() {
    let entries = vec![
        MentionEntry {
            path: "src/main.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "docs/appendix.md".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "app.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "src/app.rs".into(),
            is_directory: false,
        },
    ];
    let filtered = filter_mentions(&entries, "app.rs");
    let paths: Vec<&str> = filtered.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(paths, vec!["app.rs", "src/app.rs"]);
}

#[test]
fn filter_mentions_empty_query_sorts_by_path() {
    let entries = vec![
        MentionEntry {
            path: "z.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "a.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "m.rs".into(),
            is_directory: false,
        },
    ];
    let filtered = filter_mentions(&entries, "");
    let paths: Vec<&str> = filtered.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(paths, vec!["a.rs", "m.rs", "z.rs"]);
}

#[test]
fn filter_mentions_matches_subsequence_and_caps_items() {
    let entries = vec![MentionEntry {
        path: "src/app.rs".into(),
        is_directory: false,
    }];
    assert_eq!(filter_mentions(&entries, "sar").len(), 1);
    assert!(filter_mentions(&entries, "qzx").is_empty());

    let many: Vec<MentionEntry> = (0..150)
        .map(|index| MentionEntry {
            path: format!("file{index:03}.rs"),
            is_directory: false,
        })
        .collect();
    assert_eq!(filter_mentions(&many, "").len(), MENTION_LIMIT);
}

#[test]
fn mention_panel_wheel_scrolls_viewport_and_clamps() {
    let entries: Vec<MentionEntry> = (0..4)
        .map(|index| MentionEntry {
            path: format!("f{index}.rs"),
            is_directory: false,
        })
        .collect();
    let trigger = MentionTrigger {
        from: 0,
        to: 1,
        query: String::new(),
    };
    let mut panel = MentionPanel::new(trigger, entries, 0);
    assert!(panel.scroll_viewport(2, 0));
    assert_eq!(panel.viewport(), Some(2));
    assert_eq!(panel.active(), 0, "滚轮不动高亮");

    assert!(panel.scroll_viewport(99, 2));
    assert_eq!(panel.viewport(), Some(3), "越界钳到末项");
    assert!(!panel.scroll_viewport(99, 3));
    assert!(panel.scroll_viewport(-99, 3));
    assert_eq!(panel.viewport(), Some(0));
    assert!(!panel.scroll_viewport(-99, 0));
}

#[test]
fn mention_panel_hover_keeps_window_anchor() {
    let entries: Vec<MentionEntry> = (0..4)
        .map(|index| MentionEntry {
            path: format!("f{index}.rs"),
            is_directory: false,
        })
        .collect();
    let trigger = MentionTrigger {
        from: 0,
        to: 1,
        query: String::new(),
    };
    let mut panel = MentionPanel::new(trigger, entries, 0);
    panel.move_active(3);
    assert_eq!(panel.anchor(), 3);
    panel.set_active(1);
    assert_eq!(panel.active(), 1);
    assert_eq!(panel.anchor(), 3, "悬停不应移动窗口锚点");

    // 滚轮滚动后的悬停同样不移动显式视口。
    assert!(panel.scroll_viewport(1, 0));
    panel.set_active(0);
    assert_eq!(panel.viewport(), Some(1), "悬停不应移动显式视口");
    panel.move_active(1);
    assert_eq!(panel.viewport(), None);
    assert_eq!(panel.anchor(), panel.active());
}

#[test]
fn mention_panel_move_wraps_both_directions() {
    let entries = vec![
        MentionEntry {
            path: "a.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "b.rs".into(),
            is_directory: false,
        },
    ];
    let trigger = MentionTrigger {
        from: 0,
        to: 1,
        query: String::new(),
    };
    let mut panel = MentionPanel::new(trigger, entries, 0);
    panel.move_active(-1);
    assert_eq!(panel.active(), 1);
    panel.move_active(1);
    assert_eq!(panel.active(), 0);
}

#[test]
fn reference_deletion_range_covers_marker_and_trailing_blank() {
    let text = "@[refer-file](/tmp/a.txt) ";
    assert_eq!(
        reference_deletion_range(text, text.len()),
        Some(0..text.len())
    );
    let text = "看 @[refer-image](/tmp/a b.png) 图";
    let start = text.find("@[refer-image]").expect("marker");
    let cursor = text.find(") 图").expect("trailing blank") + 2;
    assert_eq!(reference_deletion_range(text, cursor), Some(start..cursor));
}

#[test]
fn reference_deletion_range_requires_trailing_blank_and_valid_marker() {
    let text = "@[refer-file](/tmp/a.txt)";
    assert_eq!(
        reference_deletion_range(text, text.len()),
        None,
        "无尾随空白"
    );
    assert_eq!(
        reference_deletion_range("@[refer-file](/tmp/a.txt) x", 29),
        None
    );
    assert_eq!(
        reference_deletion_range("@[refer-bogus](/tmp/a) ", 23),
        None,
        "类型非法"
    );
    assert_eq!(reference_deletion_range("", 0), None);
}

#[test]
fn reference_deletion_range_ignores_fenced_markers() {
    let text = "```\n@[refer-file](/tmp/a.txt) \n```\n";
    let cursor = text.find(") \n").expect("marker") + 2;
    assert_eq!(reference_deletion_range(text, cursor), None);
}

#[test]
fn mention_deletion_range_covers_token_and_trailing_blank() {
    let text = "@src/app.rs ";
    assert_eq!(mention_deletion_range(text, text.len()), Some(0..12));
    let text = "a @f.rs ";
    assert_eq!(mention_deletion_range(text, text.len()), Some(2..8));
    let text = "@主页 ";
    assert_eq!(mention_deletion_range(text, text.len()), Some(0..8));
}

#[test]
fn mention_deletion_range_requires_boundary_and_trailing_blank() {
    assert_eq!(mention_deletion_range("@f.rs", 5), None);
    assert_eq!(mention_deletion_range("@f.rs x", 7), None);
    assert_eq!(mention_deletion_range("[@f.rs] ", 8), None);
    assert_eq!(mention_deletion_range("a@f.rs ", 7), None);
    assert_eq!(mention_deletion_range("@@f.rs ", 7), None);
    assert_eq!(mention_deletion_range("", 0), None);
}

/// 目录范围过滤用条目：`src` 下的直接子项与更深层文件、范围外文件。
fn scoped_entries() -> Vec<MentionEntry> {
    vec![
        MentionEntry {
            path: "src".into(),
            is_directory: true,
        },
        MentionEntry {
            path: "src/app.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "src/main.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "src/ui".into(),
            is_directory: true,
        },
        MentionEntry {
            path: "src/ui/mod.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "src/ui/main_map.rs".into(),
            is_directory: false,
        },
        MentionEntry {
            path: "docs/main.md".into(),
            is_directory: false,
        },
    ]
}

#[test]
fn filter_mentions_scope_lists_direct_children_only() {
    let filtered = filter_mentions(&scoped_entries(), "src/");
    let paths: Vec<&str> = filtered.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["src/app.rs", "src/main.rs", "src/ui"],
        "空过滤词只列直接子项且不含范围自身"
    );
}

#[test]
fn filter_mentions_scope_recurses_within_subtree_on_filter() {
    let filtered = filter_mentions(&scoped_entries(), "src/ma");
    let paths: Vec<&str> = filtered.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["src/main.rs", "src/ui/main_map.rs"],
        "范围下递归匹配且排除范围外同名文件"
    );
    assert!(filter_mentions(&scoped_entries(), "src/zzz").is_empty());
}

#[test]
fn filter_mentions_scope_prefix_is_case_insensitive() {
    let entries = vec![MentionEntry {
        path: "Src/App.rs".into(),
        is_directory: false,
    }];
    assert_eq!(filter_mentions(&entries, "src/").len(), 1);
    assert_eq!(filter_mentions(&entries, "SRC/APP").len(), 1);
}

#[test]
fn filter_mentions_scope_returns_entries_with_original_paths() {
    let filtered = filter_mentions(&scoped_entries(), "src/ui/");
    assert_eq!(
        filtered
            .iter()
            .map(|entry| (entry.path.as_str(), entry.is_directory))
            .collect::<Vec<_>>(),
        vec![("src/ui/main_map.rs", false), ("src/ui/mod.rs", false)],
        "空过滤词按路径排序且路径保持根相对原样"
    );
}

#[test]
fn mention_panel_scope_name_reads_last_directory_segment() {
    let entries = vec![MentionEntry {
        path: "src/app.rs".into(),
        is_directory: false,
    }];
    let panel_with = |query: &str| {
        MentionPanel::new(
            MentionTrigger {
                from: 0,
                to: 1 + query.len(),
                query: query.to_string(),
            },
            entries.clone(),
            0,
        )
    };
    assert_eq!(panel_with("src/ui/").scope_name(), Some("ui"));
    assert_eq!(panel_with("src/ui/ma").scope_name(), Some("ui"));
    assert_eq!(panel_with("src/").scope_name(), Some("src"));
    assert_eq!(panel_with("src").scope_name(), None, "无 / 表示未进入目录");
    assert_eq!(panel_with("/").scope_name(), None);
    assert_eq!(panel_with("ma").scope_name(), None);
    assert_eq!(panel_with("").scope_name(), None);
}

// ---- 斜杠模板命令与模板块 ----

#[test]
fn slash_trigger_forms_and_rejections() {
    let trigger = |text: &str| slash_trigger(text, text.len());
    assert_eq!(trigger("/").map(|t| t.query), Some(String::new()));
    assert_eq!(trigger("/add").map(|t| t.query), Some("add".into()));
    assert_eq!(
        trigger("/addTemplate").map(|t| t.query),
        Some("addTemplate".into())
    );
    assert_eq!(trigger("  /bug").map(|t| t.query), Some("bug".into()));
    assert_eq!(trigger("/a-b_c").map(|t| t.query), Some("a-b_c".into()));

    assert_eq!(trigger("text /add"), None, "必须行首触发");
    assert_eq!(trigger("/add more"), None, "行内出现空白不再触发");
    assert_eq!(trigger("//"), None);
    assert_eq!(trigger("/add@"), None);
    assert_eq!(slash_trigger("/add", 2), None, "光标必须在行尾");

    let text = "/add";
    let trigger = slash_trigger(text, text.len()).expect("trigger");
    assert_eq!(trigger.line_start, 0);
    assert_eq!(trigger.line_end, 4);
    assert_eq!(trigger.cursor, 4);
}

#[test]
fn slash_trigger_ignores_the_echoed_worktree_command() {
    // 回显的命令行（末尾空格）不再构成命令触发：选择器关闭后不会退回命令候选面板。
    let echoed = "&&& x --start\n/gitWorkTree \n&&& x --end\n";
    let cursor = echoed.find("/gitWorkTree ").expect("echoed line") + "/gitWorkTree ".len();
    assert_eq!(slash_trigger(echoed, cursor), None, "回显行不触发");
    let typed = "&&& x --start\n/gitWorkTree\n&&& x --end\n";
    let cursor = typed.find("/gitWorkTree\n").expect("typed line") + "/gitWorkTree".len();
    assert!(slash_trigger(typed, cursor).is_some(), "无末尾空格仍是触发");
}

/// 空上下文：块外、无 git、无模板块。
fn outside_context() -> SlashContext {
    SlashContext {
        context: BlockContext::Normal,
        worktree_available: false,
        template_present: false,
        unclosed_block: false,
    }
}

/// 过滤结果的短名序列（内置命令）。
fn filter_names(query: &str) -> Vec<String> {
    slash_filter(&[], outside_context(), query)
        .into_iter()
        .map(|item| item.name)
        .collect()
}

#[test]
fn slash_filter_ranks_short_and_long_aliases() {
    assert_eq!(filter_names("").len(), 6, "5 个内置模板 + supplement");
    assert_eq!(filter_names("add"), vec!["add"]);
    assert_eq!(filter_names("addt"), vec!["add"]);
    assert_eq!(filter_names("ADD"), vec!["add"]);
    assert_eq!(filter_names("addtemplate"), vec!["add"]);
    assert_eq!(filter_names("ref"), vec!["refactor"]);
    assert_eq!(filter_names("style"), vec!["style"]);
    assert_eq!(filter_names("zzz"), Vec::<String>::new());
    let all = filter_names("template");
    assert_eq!(all.len(), 5, "长名都含 template，稳定保留全部候选");
    assert_eq!(all[0], "add", "稳定排序按内置顺序");
}

#[test]
fn slash_filter_matches_subsequence_like_mentions() {
    assert_eq!(filter_names("cmm"), vec!["common"]);
    assert_eq!(filter_names("cmn"), vec!["common"]);
    assert_eq!(filter_names("rf"), vec!["refactor"]);
    assert_eq!(filter_names("st"), vec!["style", "supplement"]);
    assert_eq!(filter_names("sty"), vec!["style"]);
    assert_eq!(filter_names("common"), vec!["common"], "精确短名优先");
}

/// 自定义命令构造：Markdown、模板块与补充块。
fn custom_items() -> Vec<SlashItem> {
    vec![
        SlashItem::custom_markdown(
            "lint",
            CommandSource::User,
            SlashScope::Global,
            "run lint",
            "lint the change",
        ),
        SlashItem::custom_markdown(
            "notes",
            CommandSource::Project,
            SlashScope::Template,
            "add notes",
            "template notes",
        ),
        SlashItem::custom_block(
            "triage",
            CommandSource::User,
            Some("Triage"),
            "body",
            "triage the bug",
        ),
        SlashItem::custom_supple(
            "extra",
            CommandSource::User,
            Some("Extra"),
            "supple body",
            "extra supplement",
        ),
    ]
}

#[test]
fn slash_filter_scopes_custom_commands_by_block_context() {
    // 块外（尚无 &&&）：global Markdown + 模板块命令 + 补充块命令；template 作用域隐藏。
    let outside = slash_filter(&custom_items(), outside_context(), "");
    let names: Vec<&str> = outside.iter().map(|item| item.name.as_str()).collect();
    assert!(names.contains(&"lint"), "{names:?}");
    assert!(names.contains(&"triage"), "{names:?}");
    assert!(names.contains(&"extra"), "{names:?}");
    assert!(
        !names.contains(&"notes"),
        "template markdown hidden outside: {names:?}"
    );
    // 块外但本页已有 &&&：内置模板与自定义 AgentBlock 隐藏，/supplement 与 SuppleBlock 保留。
    let with_template = slash_filter(
        &custom_items(),
        SlashContext {
            context: BlockContext::Normal,
            worktree_available: false,
            template_present: true,
            unclosed_block: false,
        },
        "",
    );
    let names: Vec<&str> = with_template
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    assert!(
        !names.contains(&"add"),
        "template_present 时隐藏内置模板: {names:?}"
    );
    assert!(
        !names.contains(&"triage"),
        "template_present 时隐藏 AgentBlock: {names:?}"
    );
    assert!(names.contains(&"supplement"), "{names:?}");
    assert!(names.contains(&"extra"), "{names:?}");
    assert!(names.contains(&"lint"), "{names:?}");
    // 块内：内置模板隐藏，template Markdown 出现，模板块命令隐藏，补充块命令保留。
    let inside = slash_filter(
        &custom_items(),
        SlashContext {
            context: BlockContext::Template,
            worktree_available: false,
            template_present: true,
            unclosed_block: false,
        },
        "",
    );
    let names: Vec<&str> = inside.iter().map(|item| item.name.as_str()).collect();
    assert!(!names.contains(&"add"), "builtins hidden inside: {names:?}");
    assert!(names.contains(&"notes"), "{names:?}");
    assert!(names.contains(&"extra"), "{names:?}");
    assert!(
        !names.contains(&"triage"),
        "block command hidden inside: {names:?}"
    );
}

#[test]
fn slash_filter_hides_supple_commands_when_unclosed_block_present() {
    let names = slash_filter(
        &custom_items(),
        SlashContext {
            context: BlockContext::Normal,
            worktree_available: false,
            template_present: true,
            unclosed_block: true,
        },
        "",
    )
    .into_iter()
    .map(|item| item.name)
    .collect::<Vec<_>>();
    assert!(
        !names.contains(&"supplement".to_string()),
        "未闭合块下隐藏内置 /supplement: {names:?}"
    );
    assert!(
        !names.contains(&"extra".to_string()),
        "未闭合块下隐藏自定义 SuppleBlock: {names:?}"
    );
    assert!(names.contains(&"lint".to_string()), "{names:?}");
}

#[test]
fn slash_filter_adds_worktree_candidate_only_with_git_inside_blocks() {
    let inside = slash_filter(
        &[],
        SlashContext {
            context: BlockContext::Template,
            worktree_available: true,
            template_present: true,
            unclosed_block: false,
        },
        "work",
    );
    assert_eq!(inside.len(), 1);
    assert_eq!(inside[0].name, "gitWorkTree");
    let without_git = slash_filter(
        &[],
        SlashContext {
            context: BlockContext::Template,
            worktree_available: false,
            template_present: true,
            unclosed_block: false,
        },
        "work",
    );
    assert!(without_git.is_empty(), "no worktree command without git");
    let outside = slash_filter(
        &[],
        SlashContext {
            context: BlockContext::Normal,
            worktree_available: true,
            template_present: false,
            unclosed_block: false,
        },
        "work",
    );
    assert!(outside.is_empty(), "no worktree command outside a block");
}

#[test]
fn agent_block_content_wraps_body_and_places_cursor_in_title() {
    let (content, cursor) = agent_block_content(
        "triage",
        None,
        "&&& triage --start\ninner\n&&& triage --end",
    );
    assert!(
        content.starts_with("&&& triage --start\n{title: }"),
        "{content}"
    );
    assert!(content.ends_with("&&& triage --end"), "{content}");
    assert!(content.contains("inner"));
    assert!(
        content.contains("inner\n\n&&& triage --end"),
        "正文与结束行之间空一行: {content}"
    );
    assert!(
        !content.contains("&&& triage --start\ninner"),
        "no nesting: {content}"
    );
    assert!(content[cursor..].starts_with('}'));
    let (content, _) = agent_block_content("x", Some("My Title"), "body");
    assert!(
        content.starts_with("&&& x --start\n{title: My Title}"),
        "{content}"
    );
    assert!(
        content.ends_with("body\n\n&&& x --end"),
        "自定义块底部空行: {content}"
    );
    // 正文为空时标题下空行同时充当底部空行，不叠出多行空行。
    let (content, _) = agent_block_content("x", None, "");
    assert!(content.ends_with("{title: }\n\n&&& x --end"), "{content}");
}

#[test]
fn slash_insertion_maps_item_variants() {
    let builtin = SlashItem::builtin(SlashCommandId::Add);
    let (content, cursor) = slash_insertion(&builtin).expect("builtin insertion");
    assert_eq!(content, slash_template_content(SlashCommandId::Add).0);
    assert_eq!(cursor, slash_template_content(SlashCommandId::Add).1);
    let raw = SlashItem::custom_markdown(
        "lint",
        CommandSource::User,
        SlashScope::Global,
        "run lint",
        "",
    );
    assert_eq!(
        slash_insertion(&raw),
        Some(("run lint".to_string(), "run lint".len()))
    );
    assert!(slash_insertion(&SlashItem::worktree()).is_none());
    assert!(slash_insertion(&SlashItem::placeholder("loading…")).is_none());
}

#[test]
fn worktree_marker_line_insert_replace_and_read() {
    // 未绑定：绑定行插到结束行之前，上方补一个空行与正文分隔。
    let text = "&&& addTemplate --start\nbody\n&&& addTemplate --end done {id:42}\n";
    let (range, replacement) =
        set_line_worktree(text, 2, "feature/x", None).expect("insert marker");
    let mut bound = text.to_string();
    bound.replace_range(range.start..range.end, &replacement);
    assert_eq!(
        bound,
        "&&& addTemplate --start\nbody\n\n{wt:feature/x}\n&&& addTemplate --end done {id:42}\n"
    );
    assert_eq!(
        block_worktree_branch_at_line(&bound, 4),
        Some((3, "feature/x".to_string()))
    );
    // 已绑定：整行替换（上方空行保留，不重复补）。
    let (range, replacement) = set_line_worktree(&bound, 4, "main", None).expect("replace marker");
    let mut rebound = bound.clone();
    rebound.replace_range(range.start..range.end, &replacement);
    assert_eq!(
        rebound,
        "&&& addTemplate --start\nbody\n\n{wt:main}\n&&& addTemplate --end done {id:42}\n"
    );
    // 标题行与正文已隔空行：绑定行贴住结束行。
    let titled = "&&& addTemplate --start\n{title: T}\n\nbody\n&&& addTemplate --end\n";
    let (range, replacement) = set_line_worktree(titled, 4, "dev", None).expect("insert near end");
    let mut with_title = titled.to_string();
    with_title.replace_range(range.start..range.end, &replacement);
    assert_eq!(
        with_title,
        "&&& addTemplate --start\n{title: T}\n\nbody\n\n{wt:dev}\n&&& addTemplate --end\n"
    );
    assert_eq!(
        block_worktree_branch_at_line(&with_title, 6),
        Some((5, "dev".to_string()))
    );
    // 命令行占用绑定位置时整行替换（不新增行）。
    let occupied = "&&& addTemplate --start\nbody\n/gitW\n&&& addTemplate --end\n";
    let (range, replacement) =
        set_line_worktree(occupied, 3, "dev", Some(2)).expect("occupy marker line");
    let mut occupied_text = occupied.to_string();
    occupied_text.replace_range(range.start..range.end, &replacement);
    assert_eq!(
        occupied_text,
        "&&& addTemplate --start\nbody\n\n{wt:dev}\n&&& addTemplate --end\n"
    );
    // 结束行上的旧版 `{wt:}` 不再是绑定来源。
    let legacy = "&&& addTemplate --start\nbody\n&&& addTemplate --end {wt:main}\n";
    assert_eq!(block_worktree_branch_at_line(legacy, 2), None);
}

#[test]
fn clear_line_worktree_keeps_bottom_gap() {
    let text = "&&& addTemplate --start\nbody\n\n{wt:feature/x}\n&&& addTemplate --end\n";
    let (range, replacement) = clear_line_worktree(text, 4).expect("clear binding");
    let mut cleared = text.to_string();
    cleared.replace_range(range, &replacement);
    assert_eq!(
        cleared, "&&& addTemplate --start\nbody\n\n&&& addTemplate --end\n",
        "解绑只删绑定行，保留上方空行"
    );
    assert_eq!(clear_line_worktree(&cleared, 3), None, "无绑定返回 None");
}

#[test]
fn edit_touches_worktree_line_guards_binding_row() {
    let text = "&&& x --start\nbody\n\n{wt:main}\n&&& x --end\n";
    let start = text.find("{wt:main}").expect("marker");
    let end = start + "{wt:main}".len();
    assert!(
        edit_touches_worktree_line(text, start, end),
        "整行选中删除被拒"
    );
    assert!(
        edit_touches_worktree_line(text, start, start + 1),
        "局部改动被拒"
    );
    assert!(
        edit_touches_worktree_line(text, start - 1, start),
        "行首合并被拒"
    );
    assert!(
        edit_touches_worktree_line(text, end, end + 1),
        "结束行首退格被拒"
    );
    assert!(
        edit_touches_worktree_line(text, start, start),
        "行内插入被拒"
    );
    assert!(
        !edit_touches_worktree_line(text, end + 1, end + 3),
        "结束行内容可编辑"
    );
    // 上方空行的换行紧贴绑定行，删除被拒；空行本身可并入上一行。
    assert!(
        edit_touches_worktree_line(text, start - 1, start),
        "分割线换行删除被拒"
    );
    assert!(
        !edit_touches_worktree_line(text, 0, text.len()),
        "整段替换放行"
    );
}

// ---- 补充块解析 ----

#[test]
fn parse_supple_start_line_forms() {
    let parsed = parse_supple_start_line("+++ supplement --start").expect("valid");
    assert_eq!(parsed.command, "supplement");
    assert_eq!(parsed.title, None);
    assert!(parse_supple_start_line("  +++ suppleTemplate --start {title: T}").is_some());
    assert!(parse_supple_start_line("+++ supplement --start 「title: 标题」").is_some());
    // 缺 --start 或保留词均为非法起始。
    assert!(parse_supple_start_line("+++ supplement").is_none());
    assert!(parse_supple_start_line("+++ log --start").is_none());
    assert!(parse_supple_start_line("+++ logTemplate --start").is_none());
    assert!(parse_supple_start_line("++ supplement --start").is_none());
    assert!(parse_supple_start_line("+++ supplement --end").is_none());
}

#[test]
fn parse_supple_end_line_forms() {
    let parsed = parse_supple_end_line("+++ supplement --end").expect("valid");
    assert_eq!(parsed.command, "supplement");
    assert!(parse_supple_end_line("+++ x --end {id:abc} {wt:main}").is_some());
    assert!(parse_supple_end_line("+++ log --end").is_none());
    assert!(parse_supple_end_line("+++ x --start").is_none());
}

/// 指定上下文的候选短名序列。
fn filter_names_in(ctx: BlockContext, query: &str) -> Vec<String> {
    slash_filter(
        &[],
        SlashContext {
            context: ctx,
            worktree_available: false,
            template_present: false,
            unclosed_block: false,
        },
        query,
    )
    .into_iter()
    .map(|item| item.name)
    .collect()
}

#[test]
fn slash_filter_scopes_builtin_by_context() {
    let normal = filter_names_in(BlockContext::Normal, "");
    assert!(normal.contains(&"add".to_string()));
    assert!(normal.contains(&"supplement".to_string()));

    let template = filter_names_in(BlockContext::Template, "");
    assert_eq!(template, vec!["supplement"]);

    let supple = filter_names_in(BlockContext::Supple, "");
    assert_eq!(supple, vec!["supplement"]);
}

#[test]
fn supple_block_content_wraps_and_places_cursor_in_title() {
    let (content, cursor) = supple_block_content("supplement", None, "## Head");
    assert!(
        content.starts_with("+++ supplement --start\n{title: }"),
        "{content}"
    );
    assert!(
        content.ends_with("## Head\n\n+++ supplement --end"),
        "正文与结束行之间空一行: {content}"
    );
    assert!(content.contains("## Head"));
    assert!(content[cursor..].starts_with('}'));
    let (content, _) = supple_block_content("note", Some("标题"), "body");
    assert!(
        content.starts_with("+++ note --start\n{title: 标题}"),
        "{content}"
    );
}

#[test]
fn slash_insertion_maps_supple_builtin() {
    let supple = SlashItem::builtin_supplement();
    let (text, _) = slash_insertion(&supple).expect("supple insertion");
    assert!(text.starts_with("+++ supplement --start"));
}

#[test]
fn supple_block_worktree_binding_round_trips() {
    let text = "&&& x --start\n&&& x --end\n\n+++ supplement --start\nbody\n+++ supplement --end\n";
    let cursor = text.find("body").unwrap();
    assert_eq!(
        block_end_line_at(text, cursor),
        Some(5),
        "光标在顶层 +++ 内定位到补充块结束行"
    );
    let (range, new) = set_line_worktree(text, 5, "main", None).expect("set supple binding");
    let mut text = text.to_string();
    text.replace_range(range, &new);
    assert!(text.contains("{wt:main}"));
    let end = block_end_line_at(&text, text.find("body").unwrap()).expect("end");
    assert_eq!(
        block_worktree_branch(&text, text.find("body").unwrap()).map(|(_, branch)| branch),
        Some("main".to_string())
    );
    let (range, new) = clear_line_worktree(&text, end).expect("clear");
    text.replace_range(range, &new);
    assert!(!text.contains("{wt:"), "{text}");
    assert!(
        text.contains("body\n\n+++ supplement --end"),
        "解绑保留补充块底部空行: {text}"
    );
}

#[test]
fn slash_filter_offers_worktree_and_supplement_in_supple_context() {
    let supple = slash_filter(
        &[],
        SlashContext {
            context: BlockContext::Supple,
            worktree_available: true,
            template_present: false,
            unclosed_block: false,
        },
        "",
    );
    assert_eq!(
        supple
            .iter()
            .map(|item| item.name.clone())
            .collect::<Vec<_>>(),
        vec!["supplement", "gitWorkTree"]
    );
}

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
fn slash_trigger_suppressed_in_fence_and_template_block() {
    assert_eq!(slash_trigger("```\n/add", 8), None);
    let block = "&&& addTemplate --start 「title: 」\n/add\n&&& addTemplate --end";
    let cursor = block.find("\n/add").expect("line") + 4;
    assert_eq!(slash_trigger(block, cursor), None);
    let after = format!("{block}\n/add");
    assert_eq!(
        slash_trigger(&after, after.len()).map(|t| t.query),
        Some("add".into())
    );
}

#[test]
fn slash_filter_ranks_short_and_long_aliases() {
    assert_eq!(slash_filter("").len(), 5);
    assert_eq!(slash_filter("add"), vec![SlashCommandId::Add]);
    assert_eq!(slash_filter("addt"), vec![SlashCommandId::Add]);
    assert_eq!(slash_filter("ADD"), vec![SlashCommandId::Add]);
    assert_eq!(slash_filter("addtemplate"), vec![SlashCommandId::Add]);
    assert_eq!(slash_filter("ref"), vec![SlashCommandId::Refactor]);
    assert_eq!(slash_filter("style"), vec![SlashCommandId::Style]);
    assert_eq!(slash_filter("zzz"), Vec::new());
    let all = slash_filter("template");
    assert_eq!(all.len(), 5, "长名都含 template，稳定保留全部候选");
    assert_eq!(all[0], SlashCommandId::Add, "稳定排序按内置顺序");
}

#[test]
fn slash_filter_matches_subsequence_like_mentions() {
    assert_eq!(slash_filter("cmm"), vec![SlashCommandId::Common]);
    assert_eq!(slash_filter("cmn"), vec![SlashCommandId::Common]);
    assert_eq!(slash_filter("rf"), vec![SlashCommandId::Refactor]);
    assert_eq!(slash_filter("st"), vec![SlashCommandId::Style]);
    assert_eq!(
        slash_filter("common"),
        vec![SlashCommandId::Common],
        "精确短名优先"
    );
}

#[test]
fn slash_template_content_matches_protocol_and_cursor() {
    for id in SlashCommandId::ALL {
        let (content, cursor) = slash_template_content(id);
        let start = format!("&&& {} --start\n「title: 」", id.long_name());
        let end = format!("&&& {} --end", id.long_name());
        assert!(content.starts_with(&start), "{content}");
        assert!(content.ends_with(&end), "{content}");
        assert_eq!(content.as_bytes().get(cursor - 1), Some(&b' '));
        assert!(content[cursor..].starts_with('」'), "光标落在 」 之前");
        assert!(
            parse_template_start_line(content.lines().next().expect("start line")).is_some(),
            "起止行仍是合法开始行"
        );
    }
    let (add, _) = slash_template_content(SlashCommandId::Add);
    assert!(add.contains("# Add Requirement"));
    assert!(add.contains("- Requirements: \n  - "));
}

#[test]
fn parse_template_start_line_forms() {
    let parsed =
        parse_template_start_line("&&& addTemplate --start 「title: 标题」").expect("start line");
    assert_eq!(parsed.command, "addTemplate");
    assert_eq!(parsed.title, Some(" 标题"));
    let parsed = parse_template_start_line("  &&& bugTemplate").expect("short form");
    assert_eq!(parsed.indent, "  ");
    assert_eq!(parsed.title, None);
    assert_eq!(
        parse_template_start_line("&&& commonTemplate --start").map(|p| p.title),
        Some(None)
    );

    assert_eq!(parse_template_start_line("&&& addTemplate --end"), None);
    assert_eq!(
        parse_template_start_line("&&& done"),
        None,
        "保留词不算起始行"
    );
    assert_eq!(
        parse_template_start_line("&&& suppleTemplate --start"),
        None
    );
    assert_eq!(parse_template_start_line("&&&addTemplate"), None);
    assert_eq!(parse_template_start_line("&&& 1bad --start"), None);
    assert_eq!(
        parse_template_start_line("&&& addTemplate --start junk"),
        None
    );
    assert_eq!(
        parse_template_start_line("&&& addTemplate --start 「title: a」 trailing"),
        None
    );
}

#[test]
fn parse_template_end_line_forms() {
    let parsed =
        parse_template_end_line("&&& addTemplate --end done {id:abc} {wt:main}").expect("end line");
    assert_eq!(parsed.command, Some("addTemplate"));
    assert!(parsed.end_flag);
    assert_eq!(parsed.status, TemplateStatus::Done);
    assert_eq!(parsed.id, Some("abc"));
    assert_eq!(parsed.wt, Some("main"));
    assert_eq!(
        parse_template_end_line("&&& --end in_progress")
            .expect("no command")
            .status,
        TemplateStatus::InProgress
    );
    assert_eq!(
        parse_template_end_line("&&&").expect("bare marker").status,
        TemplateStatus::Todo
    );
    assert_eq!(
        parse_template_end_line("&&& done").map(|p| p.status),
        Some(TemplateStatus::Done)
    );

    assert_eq!(
        parse_template_end_line("&&& addTemplate"),
        None,
        "缺 --end 不是结束行"
    );
    assert_eq!(parse_template_end_line("&&& addTemplate --start"), None);
    assert_eq!(parse_template_end_line("&&& --end junk"), None);
    assert_eq!(parse_template_end_line("&&& --end --end"), None);
}

#[test]
fn parse_template_block_finds_range_and_unclosed_tail() {
    let text = "a\n&&& addTemplate --start 「title: 」\n# Add\n&&& addTemplate --end\nb";
    assert_eq!(
        parse_template_block_at_line(text, 1),
        Some(TemplateBlockRange { start: 1, end: 3 })
    );
    assert_eq!(parse_template_block_at_line(text, 0), None);
    assert_eq!(parse_template_block_at_line(text, 2), None);
    let unclosed = "&&& bugTemplate --start 「title: 」\n# Fix";
    assert_eq!(
        parse_template_block_at_line(unclosed, 0),
        Some(TemplateBlockRange { start: 0, end: 1 })
    );
}

#[test]
fn cycle_template_status_cycles_and_preserves_metadata() {
    assert_eq!(
        cycle_template_status("&&& addTemplate --end").as_deref(),
        Some("&&& addTemplate --end in_progress")
    );
    assert_eq!(
        cycle_template_status("&&& addTemplate --end in_progress").as_deref(),
        Some("&&& addTemplate --end done")
    );
    assert_eq!(
        cycle_template_status("&&& addTemplate --end done").as_deref(),
        Some("&&& addTemplate --end")
    );
    assert_eq!(
        cycle_template_status("  &&& bugTemplate --end done {id:ff} {wt:dev}").as_deref(),
        Some("  &&& bugTemplate --end {id:ff} {wt:dev}")
    );
    assert_eq!(
        cycle_template_status("&&& done").as_deref(),
        Some("&&&"),
        "done 循环回 todo 时移除状态后缀"
    );
    assert_eq!(cycle_template_status("&&& addTemplate"), None);
    assert_eq!(cycle_template_status("plain"), None);
}

#[test]
fn clean_template_content_drops_unfilled_items() {
    let content = "# Add Requirement\n\n- Reference: \n- Location: \n- Description: \n- Requirements: \n  - \n- Notes: \n  - ";
    let cleaned = clean_template_content(content);
    assert_eq!(cleaned, "# Add Requirement");
}

#[test]
fn clean_template_content_keeps_filled_children() {
    let content = "- Requirements: \n  - keep me\n- Notes: \n  - \n- Location: here";
    let cleaned = clean_template_content(content);
    assert_eq!(cleaned, "- Requirements: \n  - keep me\n- Location: here");
}

#[test]
fn clean_template_content_preserves_subblocks_and_collapses_blanks() {
    let content = "- Reference: \n+++ suppleTemplate --start 「title: 」\n- Requirements: \n  - \n+++ suppleTemplate --end\n\n\n\n- Location: x";
    let cleaned = clean_template_content(content);
    assert!(cleaned.contains("+++ suppleTemplate --start"));
    assert!(cleaned.contains("- Requirements: \n  - \n+++ suppleTemplate --end"));
    assert!(!cleaned.contains("\n\n\n"), "折叠多余空行");
    assert!(cleaned.ends_with("- Location: x"));
}

#[test]
fn copy_template_content_strips_supple_and_log_markers() {
    let content = "「title: 标题」\n# Add Requirement\n\n+++ suppleTemplate --start 「title: 」\n- Extra: keep out\n+++ suppleTemplate --end\n%%% logTemplate --start 「title: 」\n- Time: 10:00\n%%% logTemplate --end\n- Location: here";
    let copied = copy_template_content(content);
    assert!(copied.starts_with("「title: 标题」\n# Add Requirement"));
    assert!(!copied.contains("suppleTemplate"), "{copied}");
    assert!(!copied.contains("Extra"), "补充子块内容整体剔除: {copied}");
    assert!(!copied.contains("%%%"), "记录子块标记剔除: {copied}");
    assert!(
        copied.contains("- Time: 10:00"),
        "记录子块内容保留: {copied}"
    );
    assert!(copied.ends_with("- Location: here"));
}

#[test]
fn copy_template_content_drops_comments_and_unfilled_items() {
    let content =
        "# Add\n// 注释行\n- Reference: \n  // 子注释\n- Location: here\n- Requirements: \n  - \n";
    let copied = copy_template_content(content);
    assert_eq!(copied, "# Add\n- Location: here");
}

#[test]
fn copy_template_content_keeps_legacy_log_blocks() {
    let content = "- Location: x\n+++ log --start\n- Records: a\n+++ log --end";
    let copied = copy_template_content(content);
    assert_eq!(copied, "- Location: x\n- Records: a");
}

#[test]
fn inside_template_block_tracks_offsets() {
    let text = "a\n&&& addTemplate --start 「title: 」\nbody\n&&& addTemplate --end\nb";
    let inside = text.find("body").expect("body");
    let outside = text.rfind("\nb").expect("b") + 1;
    assert!(inside_template_block(text, inside));
    assert!(!inside_template_block(text, outside));
    assert!(!inside_template_block(text, 0));
}

#[test]
fn template_line_infos_mark_roles_and_status() {
    let text = "a\n&&& addTemplate --start 「title: 」\n# Add\n&&& addTemplate --end done\nb";
    let infos = template_line_infos(text);
    assert_eq!(infos.len(), 5);
    assert_eq!(infos[0], None);
    assert_eq!(
        infos[1],
        Some(TemplateLineInfo {
            role: TemplateLineRole::Start,
            status: TemplateStatus::Done,
        })
    );
    assert_eq!(
        infos[2],
        Some(TemplateLineInfo {
            role: TemplateLineRole::Middle,
            status: TemplateStatus::Done,
        })
    );
    assert_eq!(
        infos[3],
        Some(TemplateLineInfo {
            role: TemplateLineRole::End,
            status: TemplateStatus::Done,
        })
    );
    assert_eq!(infos[4], None);
}

#[test]
fn template_line_infos_unclosed_block_extends_to_end() {
    let text = "&&& bugTemplate --start 「title: 」\n# Fix";
    let infos = template_line_infos(text);
    assert_eq!(
        infos[0],
        Some(TemplateLineInfo {
            role: TemplateLineRole::Start,
            status: TemplateStatus::Todo,
        })
    );
    assert_eq!(
        infos[1],
        Some(TemplateLineInfo {
            role: TemplateLineRole::Middle,
            status: TemplateStatus::Todo,
        })
    );
}

#[test]
fn is_template_block_line_checks_membership_and_bounds() {
    let text = "a\n&&& addTemplate --start 「title: 」\nbody\n&&& addTemplate --end\nb";
    assert!(!is_template_block_line(text, 0));
    assert!(is_template_block_line(text, 1));
    assert!(is_template_block_line(text, 2));
    assert!(is_template_block_line(text, 3));
    assert!(!is_template_block_line(text, 4));
    assert!(!is_template_block_line(text, 99), "越界行不算块内");
    let unclosed = "&&& bugTemplate --start 「title: 」\n# Fix";
    assert!(is_template_block_line(unclosed, 1));
}

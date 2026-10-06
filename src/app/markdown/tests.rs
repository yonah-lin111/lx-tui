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
fn mention_panel_clamped_move_stops_at_ends() {
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
    panel.move_active_clamped(-1);
    assert_eq!(panel.active(), 0);
    panel.move_active_clamped(1);
    assert_eq!(panel.active(), 1);
    panel.move_active_clamped(1);
    assert_eq!(panel.active(), 1);
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

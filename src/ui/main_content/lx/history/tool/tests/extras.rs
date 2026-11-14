//! 文件工具摘要树：`read` 的种类/范围、`search` 的命中数、`edit` 计数与截断事实。

use super::*;
use crate::chat::tools::ReadKind;

/// 展开态详情（节点全部可见），用于断言字段顺序。
fn expanded_detail(call: &ToolCallView, width: u16) -> String {
    file_summary::detail(call, width, true)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|segment| segment.text)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_call(kind: Option<ReadKind>, summary: ToolSummary) -> ToolCallView {
    let mut summary = summary;
    summary.read_kind = kind;
    tool_call("read", ToolStatus::Completed, Some(summary), None)
}

/// 文件 `read`：标题 `Read · file`，路径后追加结果体确认的区间与实际行数。
#[test]
fn read_file_shows_kind_in_title_and_range_in_body() {
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some("src/ui/style.rs".into()),
            range: Some((12, 40)),
            read_lines: Some(29),
            total_lines: Some(1000),
            ..ToolSummary::default()
        },
    );
    let header = header_text(&call, 80);
    assert!(header.contains(text::agent::VERB_TOOL_READ), "{header}");
    assert!(header.contains(text::agent::READ_KIND_FILE), "{header}");
    assert!(!header.contains("src/ui/style.rs"), "路径在树体：{header}");
    let detail = detail_text(&call, 80);
    assert!(
        detail.contains("src/ui/style.rs:12-40 · 29 lines"),
        "路径后显示区间与行数：{detail}"
    );
    assert!(!detail.contains("1000"), "不显示文件总行数：{detail}");
    let suffix = detail_segments(&call, 80)
        .into_iter()
        .find(|(text, _)| text.starts_with(":12-40"))
        .expect("区间后缀段");
    assert_eq!(suffix.1, style::agent_step_detail(), "后缀压暗");
}

/// 单行区间只写一个行号，单行用 `line`。
#[test]
fn single_line_read_uses_one_line_number_and_singular_label() {
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some("src/a.rs".into()),
            range: Some((12, 12)),
            read_lines: Some(1),
            total_lines: Some(40),
            ..ToolSummary::default()
        },
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("src/a.rs:12 · 1 line"), "{detail}");
}

/// 运行中没有结果体行数：只显示路径，不拿参数区间预估。
#[test]
fn running_read_shows_path_only() {
    let call = tool_call(
        "read",
        ToolStatus::Running,
        Some(ToolSummary {
            path: Some("src/a.rs".into()),
            range: Some((12, 40)),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("src/a.rs"), "{detail}");
    assert!(!detail.contains(":12-40"), "无结果体不得显示区间：{detail}");
    assert!(!detail.contains("line"), "无结果体不得显示行数：{detail}");
}

/// 旧记录只有参数派生摘要（有 range、无 read_lines）：同样只显示路径。
#[test]
fn read_without_result_line_count_shows_path_only() {
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src/a.rs".into()),
            range: Some((12, 40)),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("src/a.rs"), "{detail}");
    assert!(!detail.contains(":12-40"), "{detail}");
    assert!(!detail.contains("line"), "{detail}");
}

/// 长路径优先保留区间/行数后缀，路径自身尾部省略。
#[test]
fn long_read_path_keeps_suffix_and_truncates_path() {
    let path = "a".repeat(120);
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some(path),
            range: Some((12, 40)),
            read_lines: Some(29),
            total_lines: Some(1000),
            ..ToolSummary::default()
        },
    );
    let detail = detail_text(&call, 40);
    assert!(detail.contains("…"), "长路径必须省略：{detail}");
    assert!(
        detail.contains(":12-40 · 29 lines"),
        "后缀必须完整保留：{detail}"
    );
}

/// 后缀放不下时整段丢弃，只显示路径。
#[test]
fn narrow_read_drops_suffix_before_path() {
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some("src/a.rs".into()),
            range: Some((12, 40)),
            read_lines: Some(29),
            total_lines: Some(1000),
            ..ToolSummary::default()
        },
    );
    let detail = detail_text(&call, 12);
    assert!(detail.contains("sr"), "路径仍需可见：{detail}");
    assert!(!detail.contains("lines"), "放不下时丢弃后缀：{detail}");
}

/// 路径与后缀恰好占满内容宽时仍保留后缀，不因预留省略符而丢弃。
#[test]
fn read_suffix_is_kept_when_it_exactly_fits() {
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some("a".into()),
            range: Some((12, 40)),
            read_lines: Some(29),
            total_lines: Some(1000),
            ..ToolSummary::default()
        },
    );
    // 内容宽 18 = 路径 1 + 后缀 `:12-40 · 29 lines` 17。
    let detail = detail_text(&call, 23);
    assert!(detail.contains("a:12-40 · 29 lines"), "{detail}");
}

/// 组合字符按 0 列计宽，`read` 路径仍必须保持单行。
#[test]
fn combining_marks_do_not_wrap_read_path() {
    let path = "e\u{301}".repeat(20);
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some(path),
            range: Some((1, 1)),
            read_lines: Some(1),
            total_lines: Some(1),
            ..ToolSummary::default()
        },
    );
    let detail = detail_text(&call, 18);
    assert_eq!(detail.lines().count(), 1, "路径恒单行：{detail}");
    assert!(detail.contains('…'), "超长路径应尾部省略：{detail}");
}

/// 目录 `read`：标题 `Read · folder · 18 files`，树体只有路径。
#[test]
fn read_directory_shows_count_in_title_and_path_in_body() {
    let call = read_call(
        Some(ReadKind::Directory),
        ToolSummary {
            path: Some("/project/src".into()),
            total_lines: Some(18),
            ..ToolSummary::default()
        },
    );
    let header = header_text(&call, 80);
    assert!(header.contains(text::agent::READ_KIND_DIR), "{header}");
    assert!(header.contains("18 files"), "{header}");
    assert!(!header.contains("match"), "{header}");
    let detail = detail_text(&call, 80);
    assert!(detail.contains("/project/src"), "{detail}");
    assert!(!detail.contains("18"), "{detail}");
    assert!(!detail.contains("line"), "目录读取不显示行数：{detail}");
}

/// 图片 `read`：标题 `Read · image`，树体显示路径。
#[test]
fn read_image_shows_image_kind_in_title_and_path_in_body() {
    let call = read_call(
        Some(ReadKind::Image),
        ToolSummary {
            path: Some("assets/diagram.png".into()),
            ..ToolSummary::default()
        },
    );
    let header = header_text(&call, 80);
    assert!(header.contains(text::agent::VERB_TOOL_READ), "{header}");
    assert!(header.contains(text::agent::READ_KIND_IMAGE), "{header}");
    let detail = detail_text(&call, 80);
    assert!(detail.contains("assets/diagram.png"), "{detail}");
    assert!(!detail.contains("line"), "图片读取不显示行数：{detail}");
}

/// 旧历史没有 `kind`：标题保持 `Read`，路径仍在树体。
#[test]
fn unknown_read_kind_has_no_kind_status() {
    let call = read_call(
        None,
        ToolSummary {
            path: Some("src".into()),
            range: Some((1, 9)),
            ..ToolSummary::default()
        },
    );
    let header = header_text(&call, 80);
    assert!(!header.contains(text::agent::READ_KIND_FILE), "{header}");
    assert!(!header.contains(text::agent::READ_KIND_DIR), "{header}");
    assert!(!header.contains("lines 1-9"), "{header}");
    assert!(detail_text(&call, 80).contains("src"));
}

/// 空文件：标题仍只写 `file`，不编造 `0-0`。
#[test]
fn empty_file_has_no_range_noise() {
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some("empty.txt".into()),
            total_lines: Some(0),
            ..ToolSummary::default()
        },
    );
    let header = header_text(&call, 80);
    assert!(!header.contains("0-0"), "{header}");
    assert!(detail_text(&call, 80).contains("empty.txt"));
}

/// 执行阶段被截断：`read` 标题下方仍只显示一条路径，不显示 `output limited`。
#[test]
fn truncated_read_keeps_only_the_path() {
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some("big.rs".into()),
            range: Some((1, 40)),
            total_lines: Some(900),
            truncated: true,
            ..ToolSummary::default()
        },
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("big.rs"), "{detail}");
    assert!(!detail.contains(text::agent::OUTPUT_LIMITED), "{detail}");
    assert!(!detail.contains("lines"), "{detail}");
    assert_eq!(detail.lines().count(), 1, "只有一条路径：{detail:?}");
    assert_eq!(rows(&call, 80, Folds::default()), 2);
}

#[test]
fn search_shows_matches_in_title_and_omits_truncation_outside_more_lines() {
    let call = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("tool_test.txt".into()),
            pattern: Some("测试".into()),
            matches: Some(3),
            truncated: true,
            ..ToolSummary::default()
        }),
        None,
    );
    let header = header_text(&call, 90);
    assert!(header.contains("3 matches"), "{header}");
    assert!(
        !header.contains(text::agent::SEARCH_TRUNCATED),
        "标题不再显示 results truncated：{header}"
    );
    let detail = expanded_detail(&call, 90);
    assert!(
        detail.contains(&text::agent::pattern_label("测试")),
        "{detail}"
    );
    assert!(detail.contains("in tool_test.txt"), "{detail}");
    assert!(
        !detail.contains(text::agent::INLINE_TRUNCATED),
        "无 more lines 时树体不单独显示 [truncated]：{detail}"
    );
    assert!(!detail.contains('·'), "独立字段不拼 `·`：{detail}");
}

#[test]
fn search_singular_match_uses_one_match() {
    let call = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src".into()),
            pattern: Some("fn".into()),
            matches: Some(1),
            ..ToolSummary::default()
        }),
        None,
    );
    assert!(header_text(&call, 80).contains("1 match"));
}

/// 增删计数合成一个节点（`+12 -4`），partial 另起节点。
#[test]
fn edit_counts_and_partial_are_separate_nodes() {
    let call = tool_call(
        "edit",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src/app/state.rs".into()),
            added: Some(12),
            removed: Some(4),
            partial: true,
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("src/app/state.rs"), "{detail}");
    assert!(detail.contains("(+12 -4)"), "计数紧跟路径：{detail}");
    assert!(detail.contains(text::agent::TOOL_PARTIAL), "{detail}");
    assert!(!detail.contains('·'), "{detail}");
    let segments = detail_segments(&call, 80);
    let added = segments
        .iter()
        .find(|(text, _)| text.contains("+12"))
        .cloned()
        .expect("+12");
    assert_eq!(added.1, style::agent_added());
    let removed = segments
        .iter()
        .find(|(text, _)| text.contains("-4"))
        .cloned()
        .expect("-4");
    assert_eq!(removed.1, style::agent_removed());
}

#[test]
fn write_counts_keep_zero_removed() {
    let call = tool_call(
        "write",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("docs/notes.md".into()),
            added: Some(18),
            removed: Some(0),
            ..ToolSummary::default()
        }),
        None,
    );
    assert!(detail_text(&call, 80).contains("+18 -0"));
}

/// 多文件 edit（无路径）不产生孤立分隔符。
#[test]
fn multi_file_edit_has_only_counts() {
    let call = tool_call(
        "edit",
        ToolStatus::Completed,
        Some(ToolSummary {
            added: Some(12),
            removed: Some(4),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert_eq!(
        detail,
        format!("  {} (+12 -4)", text::agent::MARKER_TREE_LEAF),
        "{detail}"
    );
}

/// 未知工具：名字在标题、路径在树体。
#[test]
fn unknown_tool_shows_name_and_summary_only() {
    let call = tool_call(
        "web_fetch",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("https://example.com".into()),
            ..ToolSummary::default()
        }),
        None,
    );
    let (painted, _) = render_row(&call, 80);
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert!(header_line.contains("web_fetch"), "{header_line:?}");
    assert!(
        detail_line.contains("https://example.com"),
        "{detail_line:?}"
    );
}

/// `read` 超长路径：单行尾部省略，不软换行成第二行，也不登记可展开点击面（§5.1）。
#[test]
fn long_path_lives_in_the_title_and_never_overflows() {
    let long = "a/very/long/nested/directory/structure/with/many/segments/file.rs".repeat(3);
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some(long.clone()),
            range: Some((1, 9)),
            ..ToolSummary::default()
        },
    );
    let wide = detail_text(&call, 400);
    assert!(wide.contains(&long), "宽卡片显示完整路径");
    for width in 8u16..120 {
        let painted = full_text(&call, width);
        let lines: Vec<&str> = painted.split('\n').collect();
        assert_eq!(
            lines.len(),
            2,
            "宽 {width} read 恒为标题 + 单行路径：{painted:?}"
        );
        for line in lines {
            assert!(
                line.width() <= usize::from(width),
                "宽 {width} 越界：{line:?}"
            );
        }
    }
    assert!(
        detail_text(&call, 80).ends_with('…'),
        "超宽路径单行尾部省略"
    );
    assert!(
        !has_expandable_content(&call, 80),
        "read 没有可展开内容，不登记点击面"
    );
    assert_eq!(
        file_summary::detail(&call, 80, true).len(),
        1,
        "展开态也只有一条路径"
    );
}

/// 短摘要恒为一个节点、不可展开。
#[test]
fn completed_read_is_a_single_title_row() {
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some("src/ui/style.rs".into()),
            range: Some((12, 40)),
            ..ToolSummary::default()
        },
    );
    assert_eq!(file_summary::detail(&call, 80, false).len(), 1);
    assert_eq!(file_summary::detail(&call, 80, true).len(), 1);
    assert_eq!(rows(&call, 80, Folds::default()), 2);
    assert!(!has_expandable_content(&call, 80));
}

/// `read` 完成态无摘要：保留一行空收口，不产生 0 行块。
#[test]
fn read_without_summary_keeps_one_empty_leaf() {
    let call = tool_call("read", ToolStatus::Completed, None, None);
    let detail = detail_text(&call, 60);
    assert_eq!(detail, text::agent::TREE_LEAF);
    assert_eq!(rows(&call, 60, Folds::default()), 2);
}

/// 失败：只显示错误，不显示路径/计数；错误仍用正常前景。
#[test]
fn failed_card_shows_only_the_error() {
    let call = tool_call(
        "read",
        ToolStatus::Failed,
        Some(ToolSummary {
            path: Some("src/ui/style.rs".into()),
            range: Some((12, 40)),
            ..ToolSummary::default()
        }),
        Some("permission denied\nsecond line"),
    );
    let detail = detail_text(&call, 100);
    assert!(detail.contains("permission denied second line"), "{detail}");
    assert!(!detail.contains("src/ui/style.rs"), "{detail}");
    assert!(!detail.contains("lines 12-40"), "{detail}");
}

/// 重载路径的 `kind` 决定标题：目录给 `dir · N files`，路径仍在树体。
#[test]
fn reloaded_directory_kind_has_no_range_in_the_title() {
    let summary = ToolSummary::from_arguments(r#"{"path":"src"}"#).absorb_result_json(Some(
        r#"{"tool":"read","path":"/w/src","kind":"directory","content":"[file] a\n","start_line":1,"end_line":1,"next_offset":null,"truncated":false,"total_lines":1}"#,
    ));
    let call = tool_call("read", ToolStatus::Completed, Some(summary), None);
    let header = header_text(&call, 80);
    assert!(header.contains(text::agent::READ_KIND_DIR), "{header}");
    assert!(header.contains("1 file"), "{header}");
    assert!(!header.contains("lines"), "{header}");
    assert!(detail_text(&call, 80).contains("src"));
}

/// 点击面几何必须落在卡片行数内，且卡片整体可点（长路径折行后不漂移）。
#[test]
fn card_regions_stay_inside_the_card() {
    let call = read_call(
        Some(ReadKind::File),
        ToolSummary {
            path: Some("src/deeply/nested/path/that/wraps/again/and/again/mod.rs".into()),
            range: Some((1, 40)),
            ..ToolSummary::default()
        },
    );
    for width in [10u16, 20, 30, 40, 80] {
        let total = rows(&call, width, Folds::default());
        let regions = card_regions(&call, width, Folds::default(), total);
        for region in &regions {
            assert!(
                region.card.offset + region.card.rows <= total,
                "宽 {width}：点击面越界 {region:?} / {total}"
            );
        }
        assert_eq!(
            regions.iter().any(|region| region.clickable),
            has_expandable_content(&call, width),
            "宽 {width}：只有确有隐藏内容时才登记点击面"
        );
    }
}

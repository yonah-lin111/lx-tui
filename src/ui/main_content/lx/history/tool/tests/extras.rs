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

/// 文件 `read`：标题 `Read · file`，路径在树体；不再显示行范围。
#[test]
fn read_file_shows_kind_in_title_and_path_in_body() {
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
    assert!(detail.contains("src/ui/style.rs"), "{detail}");
    assert!(!detail.contains("lines 12-40"), "不再显示范围：{detail}");
    assert!(!detail.contains("1000"), "不显示文件总行数：{detail}");
}

/// 目录 `read`：标题 `Read · dir · 18 files`，树体只有路径。
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

/// 执行阶段被截断：路径之后单列事实。
#[test]
fn truncated_read_reports_output_limited() {
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
    assert!(detail.contains(text::agent::OUTPUT_LIMITED), "{detail}");
}

#[test]
fn search_shows_matches_and_truncation_in_the_title() {
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
    assert!(header.contains(text::agent::SEARCH_TRUNCATED), "{header}");
    let detail = expanded_detail(&call, 90);
    assert!(
        detail.contains(&text::agent::pattern_label("测试")),
        "{detail}"
    );
    assert!(detail.contains("in tool_test.txt"), "{detail}");
    assert!(
        !detail.contains(text::agent::SEARCH_TRUNCATED),
        "截断事实不占树体：{detail}"
    );
    assert!(!detail.contains('·'), "独立字段不拼 `·`：{detail}");
    assert!(!detail.contains("…[truncated]"), "{detail}");
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

/// 长路径：标题右侧优先保留路径；面板放不下时只丢状态尾巴，标题仍完整。
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
        for line in painted.split('\n') {
            assert!(
                line.width() <= usize::from(width),
                "宽 {width} 越界：{line:?}"
            );
        }
    }
    assert!(has_expandable_content(&call, 80), "超长路径折叠后可以展开");
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

/// `read` 完成态无摘要：只有标题，不留空 `└─`。
#[test]
fn read_without_summary_leaves_no_dangling_leaf() {
    let call = tool_call("read", ToolStatus::Completed, None, None);
    assert_eq!(detail_text(&call, 60), "");
    assert_eq!(rows(&call, 60, Folds::default()), 1);
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

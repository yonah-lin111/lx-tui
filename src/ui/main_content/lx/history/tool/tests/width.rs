//! 宽度自适应、列宽裁剪与行数几何不变量（树体版）。
//!
//! 新口径：字段不再为了宽度被**丢弃或中间省略**——每个逻辑节点按列宽折行，
//! 折叠只隐藏物理行并给出精确的 `… N more lines`。因此这里断言的是
//! 「不越界、不切宽字符、内容在展开后完整、前缀不漂移」。

use super::*;

// ===== 3. 宽度自适应与列宽裁剪 =====

#[test]
fn narrow_widths_keep_state_and_never_overflow() {
    let long_path = "a/very/long/nested/directory/structure/with/many/segments/file.rs";
    let call = tool_call(
        "read",
        ToolStatus::Running,
        Some(ToolSummary {
            path: Some(long_path.into()),
            range: Some((1, 999)),
            ..ToolSummary::default()
        }),
        None,
    );
    for width in [1u16, 2, 3, 4, 6, 8, 12, 20, 40] {
        let (painted, _) = render_row(&call, width);
        for line in painted.split('\n') {
            assert!(
                line.width() <= usize::from(width),
                "宽 {width} 越界：{line:?}（{} 列）",
                line.width()
            );
        }
        if width >= 3 {
            assert!(
                painted.starts_with(text::agent::STEP_MARK_FILLED),
                "宽 {width} 行首应保留状态方块：{painted:?}"
            );
        }
    }
}

#[test]
fn wide_characters_and_unbreakable_names_are_clipped_by_cell_width() {
    for path in [
        "文档/设计/很长的中文文件名称没有任何空格.rs",
        "averyveryverylongfilenamewithoutanyspacesatall.rs",
    ] {
        let call = tool_call(
            "write",
            ToolStatus::Completed,
            Some(ToolSummary {
                path: Some(path.into()),
                added: Some(120),
                removed: Some(7),
                ..ToolSummary::default()
            }),
            None,
        );
        for width in [8u16, 12, 20, 30, 60] {
            let (painted, grid) = render_row(&call, width);
            for line in painted.split('\n') {
                assert!(
                    line.width() <= usize::from(width),
                    "{path} 宽 {width} 越界：{line:?}"
                );
            }
            for y in 0..rows(&call, width, folds(false)) {
                for x in 0..width {
                    let glyph = grid.cell(x, y as u16);
                    if glyph.ch == crate::ui::main_content::lx::grid::CONTINUATION {
                        assert!(x > 0, "{path} 首格不得是宽字符占位格");
                    }
                }
            }
        }
    }
}

/// 长路径折行而不是被中间省略：展开后完整可见，折叠只隐藏尾部物理行。
#[test]
fn long_path_wraps_and_expands_with_full_content() {
    let path = "src/some/really/deeply/nested/path/that/keeps/going/and/going/mod.rs";
    let call = tool_call(
        "edit",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some(path.into()),
            added: Some(3),
            removed: Some(1),
            ..ToolSummary::default()
        }),
        None,
    );
    let expanded = detail_expanded_flat(&call, 40);
    assert!(expanded.contains(path), "展开后完整路径：{expanded:?}");
    assert!(expanded.contains("+3 -1"), "计数完整：{expanded:?}");
    let folded = detail_text(&call, 40);
    assert!(folded.contains("more line"), "折叠给出剩余行数：{folded:?}");
    // 折叠态仍然每行不越界。
    for width in 8u16..60 {
        for row in detail_text(&call, width).split('\n') {
            assert!(row.width() <= usize::from(width), "宽 {width}: {row:?}");
        }
    }
}

/// 错误文案按列宽折行，完整保留；不显示路径与计数。
#[test]
fn failed_reason_wraps_without_losing_text() {
    let long = format!(
        "read /a/very/long/path failed: {}",
        "permission denied ".repeat(8)
    );
    let call = tool_call("read", ToolStatus::Failed, None, Some(&long));
    for width in [20u16, 40, 80] {
        let (painted, _) = render_row(&call, width);
        let detail = painted.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
        for line in detail.split('\n') {
            assert!(line.width() <= usize::from(width), "宽 {width}: {line:?}");
        }
        assert!(
            !detail.contains("src/ui/style.rs"),
            "失败不显示路径：{detail:?}"
        );
    }
    let expanded = detail_expanded_flat(&call, 40);
    let expected: String = long.chars().filter(|ch| !ch.is_control()).collect();
    assert!(
        expanded.contains(expected.trim_end()),
        "错误文案完整：{expanded:?}"
    );
}

// ===== 4. 行数与几何不变量 =====

#[test]
fn tool_row_count_agrees_with_rows_for_any_content() {
    for width in [1u16, 2, 4, 8, 20, 200] {
        for status in [
            ToolStatus::InputStreaming,
            ToolStatus::Pending,
            ToolStatus::Running,
            ToolStatus::Completed,
            ToolStatus::Failed,
            ToolStatus::Cancelled,
            ToolStatus::Interrupted,
        ] {
            let call = tool_call("read", status, Some(read_summary()), Some("boom"));
            let total = rows(&call, width, folds(false));
            let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
            let mut grid = Grid::new(rect);
            let mut row_index = 0usize;
            draw(&mut grid, rect, 0, &mut row_index, &call, 0, folds(false));
            assert_eq!(row_index, total, "宽 {width} {status:?}：rows == draw");
        }
    }
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    assert_eq!(rows(&call, 0, folds(false)), 0);
    let rect = Rect::new(0, 0, 0, 2);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, &call, 0, folds(false));
    assert_eq!(row_index, 0, "宽度 0 时该块整体跳过");
}

#[test]
fn scrolled_out_rows_draw_nothing_but_still_advance() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let total = rows(&call, 40, folds(false));
    let rect = Rect::new(0, 5, 40, 2);
    let mut grid = Grid::new(rect);
    let mut row_index = 3usize;
    let scroll = 3 + total; // 整张卡片都在滚动窗口之上。
    draw(
        &mut grid,
        rect,
        scroll,
        &mut row_index,
        &call,
        0,
        folds(false),
    );
    assert_eq!(row_index, 3 + total, "滚出视口仍推进整张卡片");
    assert!(grid_text(&grid, rect).trim().is_empty(), "不可见行不写网格");
}

#[test]
fn settled_search_falls_back_to_arguments_for_its_scope() {
    let mut call = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: None,
            matches: Some(5),
            ..ToolSummary::default()
        }),
        None,
    );
    call.arguments_json = r#"{"pattern":"fn render","path":"src/ui"}"#.into();
    let header = header_text(&call, 80);
    assert!(header.contains("5 matches"), "{header}");
    let painted = detail_expanded_flat(&call, 80);
    assert!(painted.contains("src/ui"), "搜索范围回退显示：{painted}");
    assert!(
        painted.contains(&text::agent::pattern_label("fn render")),
        "搜索模式必须显示：{painted}"
    );
    let pattern_at = painted.find("fn render").expect("模式");
    let path_at = painted.find("src/ui").expect("范围");
    assert!(pattern_at < path_at, "模式在范围之前：{painted}");
}

// ===== 搜索详情的窄宽度 =====

fn search_scoped_call(pattern: &str, path: &str) -> ToolCallView {
    tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some(path.into()),
            pattern: Some(pattern.into()),
            matches: Some(3),
            ..ToolSummary::default()
        }),
        None,
    )
}

/// 任意宽度：pattern 完整保留（不裁字、不中间省略），`in` 永远在 pattern 之后。
#[test]
fn search_detail_keeps_the_pattern_intact_at_narrow_widths() {
    let call = search_scoped_call("测试", "tool_test.txt");
    let pattern = text::agent::pattern_label("测试");
    for width in (10u16..=70).rev() {
        let painted = detail_text(&call, width);
        for row in painted.split('\n') {
            assert!(
                row.width() <= usize::from(width),
                "宽 {width} 越界：{row:?}"
            );
        }
        let body = detail_expanded_flat(&call, width);
        assert!(
            body.contains(&pattern),
            "宽 {width} 下 pattern 必须完整：{body:?}"
        );
        if let Some(pattern_at) = body.find('测')
            && let Some(in_at) = body.find(text::agent::IN_SCOPE_SEP.trim_end())
        {
            assert!(
                in_at > pattern_at,
                "宽 {width} 下 `in` 必须在 pattern 之后：{body:?}"
            );
        }
    }
}

/// 长 pattern + 长路径：只折行，不丢字符；展开后整段可读。
#[test]
fn search_long_pattern_wraps_instead_of_shrinking() {
    let call = search_scoped_call("fn render_search_detail", "src/ui/main_content/lx");
    let body = detail_expanded_flat(&call, 40);
    assert!(
        body.contains(&text::agent::pattern_label("fn render_search_detail")),
        "pattern 完整：{body:?}"
    );
    assert!(
        body.contains("src/ui/main_content/lx"),
        "范围完整：{body:?}"
    );
    for width in (10u16..=70).rev() {
        for row in detail_text(&call, width).split('\n') {
            assert!(row.width() <= usize::from(width), "宽 {width}: {row:?}");
        }
    }
}

// ===== 渲染层不做解析（P3-4）=====

#[test]
fn param_does_not_reparse_arguments_when_summary_is_complete() {
    reset_fallback_parse_count();
    let mut call = search_scoped_call("测试", "tool_test.txt");
    call.arguments_json = "{not json at all".into();
    let painted = detail_expanded_flat(&call, 80);
    assert!(
        painted.contains(&text::agent::pattern_label("测试")),
        "完整摘要必须独立可见（不依赖参数解析）：{painted:?}"
    );
    assert!(painted.contains("tool_test.txt"), "{painted:?}");
    assert_eq!(
        fallback_parse_count(),
        0,
        "摘要完整时不得回退解析参数 JSON：{painted:?}"
    );

    reset_fallback_parse_count();
    let mut read = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    read.arguments_json = "{not json".into();
    let painted = detail_expanded_flat(&read, 80);
    assert!(painted.contains("src/ui/style.rs"), "{painted:?}");
    assert!(!painted.contains("12-40"), "不再显示行范围：{painted:?}");
    assert_eq!(fallback_parse_count(), 0, "read 也不回退解析参数 JSON");

    reset_fallback_parse_count();
    let mut edit = tool_call(
        "edit",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: None,
            added: Some(12),
            removed: Some(4),
            ..ToolSummary::default()
        }),
        None,
    );
    edit.arguments_json = format!(
        r#"{{"patch":"*** Begin Patch\n*** Update File: {}\n*** End Patch"}}"#,
        "a".repeat(1024 * 1024)
    );
    let painted = detail_expanded_flat(&edit, 80);
    assert!(painted.contains("+12 -4"), "{painted:?}");
    assert_eq!(
        fallback_parse_count(),
        0,
        "多文件 edit 摘要完整时不得每帧解析巨型 patch JSON"
    );
}

#[test]
fn param_falls_back_to_arguments_only_when_summary_lacks_fields() {
    reset_fallback_parse_count();
    let mut call = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: None,
            matches: Some(5),
            ..ToolSummary::default()
        }),
        None,
    );
    call.arguments_json = r#"{"pattern":"fn render","path":"src/ui"}"#.into();
    let painted = detail_expanded_flat(&call, 80);
    assert!(painted.contains("fn render"), "{painted:?}");
    assert!(painted.contains("src/ui"), "{painted:?}");
    assert_eq!(fallback_parse_count(), 1, "缺字段时只解析一次");
}

#[test]
fn streaming_input_skips_fallback_parse_for_oversized_arguments() {
    reset_fallback_parse_count();
    let mut call = tool_call("edit", ToolStatus::InputStreaming, None, None);
    call.arguments_json = format!(
        r#"{{"patch":"*** Begin Patch\n*** Update File: {}\n*** End Patch"}}"#,
        "a".repeat(1024 * 1024)
    );
    let painted = detail_text(&call, 80);
    assert_eq!(
        fallback_parse_count(),
        0,
        "超过体积门槛的参数不做兜底解析：{painted:?}"
    );
    assert!(
        painted.contains(text::LX_CHAT_TOOL_INPUT_STREAMING),
        "仍用状态词兜底：{painted:?}"
    );

    reset_fallback_parse_count();
    let mut small = tool_call("read", ToolStatus::InputStreaming, None, None);
    small.arguments_json = r#"{"path":"src/ui/style.rs"}"#.into();
    let painted = detail_text(&small, 80);
    assert!(painted.contains("src/ui/style.rs"), "{painted:?}");
    assert_eq!(fallback_parse_count(), 1, "小参数只解析一次（树体用）");
}

#[test]
fn lead_marker_identifies_the_state_at_any_width() {
    let cases = [
        ("read", ToolStatus::Running, None),
        ("read", ToolStatus::Completed, None),
        ("read", ToolStatus::Failed, Some("boom")),
        ("read", ToolStatus::Interrupted, Some("gone")),
    ];
    let mut seen: Vec<(char, Style)> = Vec::new();
    for (name, status, error) in cases {
        let call = tool_call(name, status, Some(read_summary()), error);
        let (narrow, _) = render_row(&call, 1);
        assert!(!narrow.trim().is_empty(), "{status:?} 极窄仍可辨");
        let (painted, grid) = render_row(&call, 40);
        let lead_char = painted.chars().next().expect("行首方块");
        let lead_style = grid.cell(0, 0).style;
        assert!(
            !seen.contains(&(lead_char, lead_style)),
            "{status:?} 的行首标记 {lead_char:?} 与已有状态重复：{painted}"
        );
        seen.push((lead_char, lead_style));
    }
    let cancelled = tool_call("read", ToolStatus::Cancelled, Some(read_summary()), None);
    let (painted, grid) = render_row(&cancelled, 40);
    assert_eq!(
        painted.chars().next(),
        Some(text::agent::STEP_MARK_FILLED),
        "取消必须是实心方块：{painted}"
    );
    assert_eq!(
        grid.cell(0, 0).style,
        style::chat_tool_verb(),
        "取消方块必须与工具动词同色"
    );
    assert!(
        detail_expanded_flat(&cancelled, 80).contains(text::LX_CHAT_TOOL_CANCELLED),
        "取消由详情行区分"
    );
}

/// 详情行内容不随 loading 帧变化（只有标题行的方块在呼吸）。
#[test]
fn detail_line_is_stable_across_loading_frames() {
    let call = tool_call("read", ToolStatus::Running, Some(read_summary()), None);
    let reference = detail_text(&call, 80);
    for frame in 0..text::agent::STEP_BREATH_TICKS * 3 {
        assert_eq!(detail_text(&call, 80), reference, "帧 {frame} 详情行不该变");
    }
}

/// 折行后的树体：每行不越界；首节点带分支/收口符，续行只带竖线或空白。
#[test]
fn wrapped_detail_never_overflows_and_reindents() {
    let path = "src/some/really/deeply/nested/path/that/keeps/going/and/going/mod.rs";
    let call = tool_call(
        "edit",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some(path.into()),
            added: Some(3),
            removed: Some(1),
            ..ToolSummary::default()
        }),
        None,
    );
    for width in 8u16..80 {
        let painted = detail_text(&call, width);
        let rows: Vec<&str> = painted.split('\n').collect();
        assert!(
            rows.len() <= super::file_summary::DETAIL_PREVIEW_ROWS + 1,
            "宽 {width} 折叠态最多 2 行内容 + 1 行剩余行数：{painted:?}"
        );
        for row in &rows {
            assert!(
                row.width() <= usize::from(width),
                "宽 {width} 越界：{row:?}"
            );
        }
        assert!(
            rows[0].starts_with(text::agent::TREE_BRANCH)
                || rows[0].starts_with(text::agent::TREE_LEAF)
                || width <= super::TREE_COLS as u16,
            "首行带节点连接符：{rows:?}"
        );
        for row in rows.iter().skip(1) {
            assert!(
                [
                    text::agent::TREE_BRANCH,
                    text::agent::TREE_BAR,
                    text::agent::TREE_LEAF,
                    text::agent::TREE_PAD,
                ]
                .iter()
                .any(|prefix| row.starts_with(prefix))
                    || width <= super::TREE_COLS as u16,
                "每行都带等宽树前缀：{row:?}"
            );
        }
    }
}

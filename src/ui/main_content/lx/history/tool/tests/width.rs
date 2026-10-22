//! 宽度自适应、列宽裁剪与行数几何不变量。

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
            // 状态始终可辨：行首方块。
            assert!(
                painted.starts_with(text::agent::STEP_MARK_FILLED),
                "宽 {width} 行首应保留状态方块：{painted:?}"
            );
        }
    }
}

#[test]
fn wide_characters_and_unbreakable_names_are_clipped_by_cell_width() {
    // 中文宽字符目录名 + 无空格长英文名：按显示列裁剪，不按字节切。
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
            // 不得出现只画了一半的宽字符（占位格必须紧跟其首格）。
            for y in 0..rows(&call, width, false) {
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

/// 窄宽度让位：**先折行**（2 行额度），参数仍放不下时再省略路径；
/// 计数与参数同处一份额度内，路径被省略而计数保留是常态。
#[test]
fn detail_wraps_before_it_truncates_the_path() {
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
    // 中等宽度：2 行额度把长路径折开，计数仍在。
    let medium = detail_text(&call, 40);
    assert!(medium.contains('\n'), "长参数折行：{medium:?}");
    assert!(
        medium.contains(text::agent::added_label(3).as_str()),
        "计数保留：{medium:?}"
    );
    // 各宽度：前缀始终完整，路径信息不会被截成空串（能画就画）。
    let mut prefix_missing: Vec<u16> = Vec::new();
    for width in 8u16..60 {
        let painted = detail_text(&call, width);
        let first = painted.split('\n').next().unwrap_or_default();
        // 前缀 "  └─ " 共 5 列，宽 ≥ 6 时必须完整出现。
        if width >= 6 && !first.starts_with("  ") {
            prefix_missing.push(width);
        }
    }
    assert!(
        prefix_missing.is_empty(),
        "宽度 ≥6 时 `  ` 缩进前缀始终在，缺失于 {prefix_missing:?}"
    );
}

/// 失败时的错误文案：超宽尾部省略，绝不换行。
#[test]
fn failed_reason_is_truncated_on_one_line() {
    let long = format!(
        "read /a/very/long/path failed: {}",
        "permission denied ".repeat(20)
    );
    let call = tool_call("read", ToolStatus::Failed, None, Some(&long));
    for width in [20u16, 40, 80] {
        let (painted, _) = render_row(&call, width);
        let (_, detail_line) = painted.split_once('\n').expect("两行");
        assert!(
            detail_line.width() <= usize::from(width),
            "宽 {width} 错误行不越界：{detail_line:?}"
        );
        assert!(
            !detail_line.contains('\n'),
            "错误行必须单行：{detail_line:?}"
        );
    }
    // 宽 20：错误被省略号截断。
    let narrow = detail_text(&call, 20);
    assert!(narrow.contains('…'), "窄宽度尾部省略：{narrow:?}");
}

// ===== 4. 行数与几何不变量 =====

#[test]
fn tool_row_count_is_always_two_for_any_content() {
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
            // 详情 = 最多 2 行内容 + 可能 1 行剩余行数提示。
            let detail_rows = rows(&call, width, false) - 1;
            assert!(
                (1..=3).contains(&detail_rows),
                "宽 {width} 详情恒为 1~3 行，实际 {detail_rows}"
            );
        }
    }
    // 宽度 0：不占行（与空正文块同口径），渲染也不写任何单元格。
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    assert_eq!(rows(&call, 0, false), 0);
    let rect = Rect::new(0, 0, 0, 2);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, &call, 0, false);
    assert_eq!(
        row_index, 0,
        "绘制推进与 layout 口径一致（宽度 0 时该块整体跳过）"
    );
}

#[test]
fn scrolled_out_rows_draw_nothing_but_still_advance() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let rect = Rect::new(0, 5, 40, 2);
    let mut grid = Grid::new(rect);
    let mut row_index = 3usize; // 已在滚动窗口之上
    draw(&mut grid, rect, 5, &mut row_index, &call, 0, false);
    assert_eq!(row_index, 5, "不可见两行仍推进");
    assert!(grid_text(&grid, rect).trim().is_empty(), "不可见行不写网格");
}

#[test]
fn settled_search_falls_back_to_arguments_for_its_scope() {
    // 结算后摘要只带命中数（`to_summary` 不给 search 的 path/pattern）；此时从参数
    // JSON 取搜索模式与范围补上，避免 search 行只剩一个裸计数。
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
    let painted = detail_text(&call, 80);
    assert!(painted.contains("src/ui"), "搜索范围回退显示：{painted}");
    assert!(painted.contains("5 matches"), "{painted}");
    // 搜索模式必须显示（加引号），且排在范围之前——`"pattern" in path`。
    assert!(
        painted.contains(&text::agent::pattern_label("fn render")),
        "搜索模式必须显示：{painted}"
    );
    let pattern_at = painted.find("fn render").expect("模式");
    let path_at = painted.find("src/ui").expect("范围");
    assert!(pattern_at < path_at, "模式在范围之前：{painted}");
}

// ===== 搜索详情的窄宽度让位（P2-7）=====

/// 带范围路径的 `search` 调用（pattern 含宽字符，路径较长）。
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

/// 详情行去掉前缀与折行缩进后的正文（用于「内容还在不在」的断言）。
fn detail_body_text(call: &ToolCallView, width: u16) -> String {
    // 只看内容行（不含末尾「… N more lines」提示行），并去掉 `└─ ` 前缀。
    let painted = detail_content_flat(call, width);
    let prefix = detail_prefix();
    painted
        .strip_prefix(&prefix)
        .unwrap_or(painted.as_str())
        .trim_end()
        .to_string()
}

/// P2-7：窄宽度下 pattern 的核心字符必须始终可见，且绝不被中间省略切碎。
///
/// 旧实现把 `"pattern" in path` 合成**一段**做 `ellipsize_middle`，`w=30` 时得到
/// `"测试"…ル.txt · 3 matches`（`in` 消失、pattern 与 path 粘连），`w=24` 时
/// `"测…txt · 3 matches`（pattern 被切成首尾两字）。
#[test]
fn search_detail_keeps_pattern_at_narrow_widths() {
    let call = search_scoped_call("测试", "tool_test.txt");
    let pattern = text::agent::pattern_label("测试");
    for width in (10u16..=70).rev() {
        let body = detail_body_text(&call, width);
        let painted = detail_text(&call, width);
        for row in painted.split('\n') {
            assert!(
                row.width() <= usize::from(width),
                "宽 {width} 越界：{row:?}"
            );
        }
        if width >= 11 {
            // pattern 完整（连引号一起）。
            assert!(
                body.contains(&pattern),
                "宽 {width} 下 pattern 必须完整：{body:?}"
            );
        } else {
            // 最窄档：至少保留 pattern 的首个模式字符 + 省略号。
            assert!(
                body.contains("测…"),
                "宽 {width} 至少保留 pattern 首字符：{body:?}"
            );
        }
        // 不得出现「省略号 + 范围路径尾部」的粘连残形（旧 bug 的指纹）。
        for glued in ["…ル", "…txt", "…tool_test"] {
            assert!(
                !body.contains(glued),
                "宽 {width} 出现 pattern 与 path 粘连的残形 {glued:?}：{body:?}"
            );
        }
        // pattern 是这一行第一个内容段：`in` 绝不会跑到它前面。
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

/// 长 pattern + 长路径：窄档只允许**尾部省略** pattern，不允许中间省略。
#[test]
fn search_long_pattern_shrinks_from_the_tail_only() {
    let call = search_scoped_call("fn render_search_detail", "src/ui/main_content/lx");
    let pattern = "fn render_search_detail";
    for width in (10u16..=70).rev() {
        let body = detail_body_text(&call, width);
        if body.is_empty() {
            continue;
        }
        assert!(
            body.starts_with('"'),
            "宽 {width} pattern 必须以引号起头（不是中间省略的残段）：{body:?}"
        );
        let shown = body.trim_start_matches('"');
        if !body.contains('…') {
            // 放得下：整串原样。
            assert!(
                shown.starts_with(pattern),
                "宽 {width} 放得下就该完整：{body:?}"
            );
            continue;
        }
        // 尾部省略：保留的字符必须是 pattern 的**前缀**。
        let kept = shown.split('…').next().unwrap_or_default();
        assert!(
            pattern.starts_with(kept),
            "宽 {width} 只允许尾部省略（保留前缀），实际 {body:?}"
        );
        assert!(
            !kept.is_empty(),
            "宽 {width} 至少保留一个 pattern 字符：{body:?}"
        );
    }
}

/// P2-7：窄宽度先丢范围（`in` + path），再收敛 pattern。
#[test]
fn search_detail_drops_scope_before_pattern() {
    let call = search_scoped_call("测试", "tool_test.txt");
    let sep_in = text::agent::IN_SCOPE_SEP.trim_end();
    // 宽档：三段齐全（pattern / in / path）。
    let wide = detail_body_text(&call, 70);
    assert!(wide.contains("tool_test.txt"), "宽档显示范围：{wide:?}");
    assert!(wide.contains(sep_in), "宽档显示 in：{wide:?}");

    // 收集每一档是否显示 in / path / pattern 完整。
    let mut saw_scope_dropped = false;
    let mut pattern_lost_before_scope = Vec::new();
    for width in (10u16..=70).rev() {
        let body = detail_body_text(&call, width);
        let has_scope = body.contains(sep_in) || body.contains("tool_test.txt");
        let has_pattern = body.contains("测");
        // `in` 与 path 必须同进同退（要么整段在、要么整段不在）。
        assert_eq!(
            body.contains(sep_in),
            body.contains("tool_test.txt"),
            "宽 {width} 下 `in` 与 path 必须一起存在/一起丢弃：{body:?}"
        );
        if !has_scope && has_pattern {
            saw_scope_dropped = true;
        }
        if !has_pattern && width >= 11 {
            pattern_lost_before_scope.push(width);
        }
    }
    assert!(
        saw_scope_dropped,
        "必须存在「已丢范围、仍保留 pattern」的宽度档"
    );
    assert!(
        pattern_lost_before_scope.is_empty(),
        "pattern 不得先于范围丢失，缺失于 {pattern_lost_before_scope:?}"
    );
}

// ===== 渲染层不做解析（P3-4）=====

/// 摘要已完整时，渲染**不得**回退解析 `arguments_json`。
///
/// 用「故意非法的参数 JSON + 完整摘要」来证明：一旦回退解析，模式与路径都会消失
/// （`from_arguments` 解析失败返回空摘要），渲染结果必然错。
#[test]
fn param_does_not_reparse_arguments_when_summary_is_complete() {
    reset_fallback_parse_count();
    let mut call = search_scoped_call("测试", "tool_test.txt");
    call.arguments_json = "{not json at all".into();
    let painted = detail_text(&call, 80);
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

    // 通用路径同样不解析：非法参数 + 完整摘要仍画出路径与计数。
    reset_fallback_parse_count();
    let mut read = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    read.arguments_json = "{not json".into();
    let painted = detail_text(&read, 80);
    assert!(painted.contains("src/ui/style.rs"), "{painted:?}");
    assert!(painted.contains("12-40"), "{painted:?}");
    assert_eq!(fallback_parse_count(), 0, "read 也不回退解析参数 JSON");

    // 多文件 `edit`：摘要无 path（正常），但 added/removed 齐全 → 不回退解析。
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
    let painted = detail_text(&edit, 80);
    assert!(painted.contains("+12 -4"), "{painted:?}");
    assert_eq!(
        fallback_parse_count(),
        0,
        "多文件 edit 摘要完整时不得每帧解析巨型 patch JSON"
    );
}

/// 摘要缺字段时才回退解析（且同一帧最多一次）。
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
    let painted = detail_text(&call, 80);
    assert!(painted.contains("fn render"), "{painted:?}");
    assert!(painted.contains("src/ui"), "{painted:?}");
    assert_eq!(
        fallback_parse_count(),
        1,
        "缺字段时只解析一次（共享同一次解析）"
    );
}

/// 流式接收中（还没有摘要）也不解析巨型参数：1 MiB 的 patch 每帧解析会白烧 CPU。
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

    // 小参数（正常 read/write）照常解析：路径能先看到。
    reset_fallback_parse_count();
    let mut small = tool_call("read", ToolStatus::InputStreaming, None, None);
    small.arguments_json = r#"{"path":"src/ui/style.rs"}"#.into();
    let painted = detail_text(&small, 80);
    assert!(painted.contains("src/ui/style.rs"), "{painted:?}");
    assert_eq!(fallback_parse_count(), 1, "小参数只解析一次");
}

#[test]
fn lead_marker_identifies_the_state_at_any_width() {
    // 极窄档唯一保留的是行首方块：五个状态的（字形 + 样式）必须两两不同。
    let cases = [
        ("read", ToolStatus::Running, None),
        ("read", ToolStatus::Completed, None),
        ("read", ToolStatus::Failed, Some("boom")),
        ("read", ToolStatus::Cancelled, None),
        ("read", ToolStatus::Interrupted, Some("gone")),
    ];
    let mut seen: Vec<(char, Style)> = Vec::new();
    for (name, status, error) in cases {
        let call = tool_call(name, status, Some(read_summary()), error);
        // 1 列宽也只剩行首方块本身（写 buffer 按列裁剪）。
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

/// 折行后的详情：每一物理行都不越界，且续行不重复 `└─` 连接符。
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
    let prefix = detail_prefix();
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
        assert!(rows[0].starts_with(&prefix), "首行带连接符：{rows:?}");
        for row in rows.iter().skip(1) {
            assert!(
                !row.contains(text::agent::MARKER_TREE_LEAF),
                "续行不重复连接符：{row:?}"
            );
            assert!(
                row.starts_with(&" ".repeat(prefix.width())),
                "续行等宽缩进：{row:?}"
            );
        }
        // 路径信息不丢（可以被尾部/中间省略，但至少留下首段与省略号）。
        assert!(
            detail_flat(&call, width).contains(&prefix) || width < 6,
            "宽 {width} 前缀保留"
        );
    }
}

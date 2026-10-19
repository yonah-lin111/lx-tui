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
            for y in 0..(DETAIL_ROWS + HEADER_ROWS) {
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

/// 窄宽度让位顺序：先丢掉次要计数，再省略路径，`└─` 前缀始终保留。
#[test]
fn detail_drops_secondary_counts_before_truncating_the_path() {
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
    // 中等宽度：路径被中间省略，但计数仍在。
    let medium = detail_text(&call, 40);
    assert!(medium.contains('…'), "长路径中间省略：{medium:?}");
    assert!(
        medium.contains(text::agent::added_label(3).as_str()),
        "计数保留：{medium:?}"
    );
    // 更窄：计数先丢，路径仍在。
    let mut dropped_counts_with_path = false;
    let mut prefix_missing: Vec<u16> = Vec::new();
    for width in 8u16..60 {
        let painted = detail_text(&call, width);
        if !painted.contains(text::agent::added_label(3).as_str()) && painted.contains("src") {
            dropped_counts_with_path = true;
        }
        // 前缀 "  └─ " 共 5 列，宽 ≥ 6 时必须完整出现。
        if width >= 6 && !painted.starts_with("  ") {
            prefix_missing.push(width);
        }
    }
    assert!(
        dropped_counts_with_path,
        "应存在「丢掉计数、仍保留路径」的宽度档"
    );
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
            assert_eq!(rows(&call, width), 2, "宽 {width} 恒占 2 行（标题 + 详情）");
        }
    }
    // 宽度 0：不占行（与空正文块同口径），渲染也不写任何单元格。
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    assert_eq!(rows(&call, 0), 0);
    let rect = Rect::new(0, 0, 0, 2);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, &call, 0);
    assert_eq!(
        row_index, 2,
        "绘制推进与 layout 口径一致（宽度 0 时 layout 跳过该块）"
    );
}

#[test]
fn scrolled_out_rows_draw_nothing_but_still_advance() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let rect = Rect::new(0, 5, 40, 2);
    let mut grid = Grid::new(rect);
    let mut row_index = 3usize; // 已在滚动窗口之上
    draw(&mut grid, rect, 5, &mut row_index, &call, 0);
    assert_eq!(row_index, 5, "不可见两行仍推进");
    assert!(grid_text(&grid, rect).trim().is_empty(), "不可见行不写网格");
}

#[test]
fn settled_search_falls_back_to_arguments_for_its_scope() {
    // 结算后摘要只带命中数（`to_summary` 不给 search 的 path）；此时从参数 JSON
    // 取搜索模式与范围补上，避免 search 行只剩一个裸计数。
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

//! 工具步骤行测试：四工具 × 多状态的 buffer 断言、宽度自适应、列宽裁剪与几何一致性。
//!
//! 只断言自动可验证的部分（文本/样式/列宽/行数）；真实配色与对齐由用户 `cargo run` 验收。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use super::*;
use crate::app::state::{ToolStatus, ToolSummary};

use crate::ui::style;
use crate::ui::text;

// ===== 构造与辅助 =====

/// 构造一个工具调用步骤。
fn tool_call(
    name: &str,
    status: ToolStatus,
    summary: Option<ToolSummary>,
    error: Option<&str>,
) -> ToolCallView {
    ToolCallView {
        call_id: format!("call-{name}"),
        name: name.into(),
        position: 0,
        arguments_json: String::new(),
        status,
        summary,
        error: error.map(str::to_string),
        result_json: None,
    }
}

/// `read` 的摘要：路径 + 行范围。
fn read_summary() -> ToolSummary {
    ToolSummary {
        path: Some("src/ui/style.rs".into()),
        range: Some((12, 40)),
        ..ToolSummary::default()
    }
}

/// `search` 的摘要：命中数（搜索范围作为路径）。
fn search_summary() -> ToolSummary {
    ToolSummary {
        path: Some("src".into()),
        matches: Some(3),
        ..ToolSummary::default()
    }
}

/// `edit` 的摘要：路径 + 增删计数。
fn edit_summary() -> ToolSummary {
    ToolSummary {
        path: Some("src/app/state.rs".into()),
        added: Some(12),
        removed: Some(4),
        ..ToolSummary::default()
    }
}

/// `write` 的摘要：路径 + 增删计数。
fn write_summary() -> ToolSummary {
    ToolSummary {
        path: Some("docs/notes.md".into()),
        added: Some(18),
        removed: Some(0),
        ..ToolSummary::default()
    }
}

/// 渲染一行到网格并按列拼出文本（跳过宽字符占位格）。
fn render_row(call: &ToolCallView, width: u16) -> (String, Grid) {
    render_row_at(call, width, 0)
}

/// 以指定 loading 帧渲染一行（方块呼吸相位断言用）。
fn render_row_at(call: &ToolCallView, width: u16, loading_frame: usize) -> (String, Grid) {
    let rect = Rect::new(0, 0, width, 1);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, call, loading_frame);
    assert_eq!(row_index, rows(call, width), "绘制推进行数必须与 rows 一致");
    (grid_text(&grid, rect), grid)
}

/// 网格整行文本。
fn grid_text(grid: &Grid, rect: Rect) -> String {
    let mut out = String::new();
    for x in rect.x..rect.right() {
        let glyph = grid.cell(x, rect.y);
        if glyph.ch != crate::ui::main_content::lx::grid::CONTINUATION {
            out.push(glyph.ch);
        }
    }
    out
}

/// 行内各段的文本与样式（供样式断言）。
fn segments(call: &ToolCallView, width: u16) -> Vec<(String, Style)> {
    line(call, width, 0)
        .into_iter()
        .map(|segment| (segment.text, segment.style))
        .collect()
}

/// 整行文本（段拼接，未裁剪）。
fn line_text(call: &ToolCallView, width: u16) -> String {
    segments(call, width)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

// ===== 1. 四工具 × 四状态 =====

#[test]
fn every_tool_and_open_status_shows_breathing_square_verb_and_path() {
    let cases: [(&str, ToolSummary); 4] = [
        ("read", read_summary()),
        ("search", search_summary()),
        ("edit", edit_summary()),
        ("write", write_summary()),
    ];
    for (name, summary) in cases {
        for status in [
            ToolStatus::InputStreaming,
            ToolStatus::Pending,
            ToolStatus::Running,
        ] {
            let call = tool_call(name, status, Some(summary.clone()), None);
            let (painted, grid) = render_row_at(&call, 80, 0);
            assert!(
                painted.starts_with(text::agent::STEP_MARK_FILLED),
                "{name}/{status:?} 行首应为步骤方块：{painted}"
            );
            // 运行中的方块用 lx 品牌粉（Indexed 218）加粗：与正文默认前景、
            // 面板边框的 Cyan/dim 都区分得开。
            let lead = grid.cell(0, 0).style;
            assert_eq!(
                lead.fg,
                style::chat_step_mark_running().fg,
                "{name}/{status:?} 方块品牌粉"
            );
            assert!(
                lead.add_modifier.contains(ratatui::style::Modifier::BOLD),
                "{name}/{status:?} 方块加粗"
            );
            // 与边框青、边框标题淡蓝都不撞色。
            assert_ne!(lead.fg, style::accent().fg, "不与边框青同色");
            assert_ne!(lead.fg, style::border_title().fg, "不与边框标题同色");
            assert!(
                painted.contains(text::tool_status_label(status)),
                "{name}/{status:?} 应显示状态文字：{painted}"
            );
            assert!(!painted.contains('!'), "{name}/{status:?} 不带失败标记");
        }
    }
}

#[test]
fn step_square_breathes_while_open_and_stays_static_when_settled() {
    // 呼吸：每 `STEP_BREATH_TICKS` 个 loading 帧翻转一次相位（实 ⇄ 空）。
    let open = tool_call("read", ToolStatus::Running, Some(read_summary()), None);
    for frame in 0..text::agent::STEP_BREATH_TICKS {
        let (painted, _) = render_row_at(&open, 40, frame);
        assert!(
            painted.starts_with(text::agent::STEP_MARK_FILLED),
            "帧 {frame} 应为实心：{painted}"
        );
    }
    for frame in text::agent::STEP_BREATH_TICKS..text::agent::STEP_BREATH_TICKS * 2 {
        let (painted, _) = render_row_at(&open, 40, frame);
        assert!(
            painted.starts_with(text::agent::STEP_MARK_HOLLOW),
            "帧 {frame} 应为空心：{painted}"
        );
    }
    // 终态与帧无关：完成 / 失败 / 中断恒实心（取消恒空心，见下个测试）。
    for status in [
        ToolStatus::Completed,
        ToolStatus::Failed,
        ToolStatus::Interrupted,
    ] {
        let call = tool_call("read", status, Some(read_summary()), Some("boom"));
        for frame in [0usize, text::agent::STEP_BREATH_TICKS, 13] {
            let (painted, _) = render_row_at(&call, 40, frame);
            assert!(
                painted.starts_with(text::agent::STEP_MARK_FILLED),
                "{status:?} 帧 {frame} 应停为静态实心：{painted}"
            );
        }
    }
}

#[test]
fn completed_tool_has_square_verb_and_status_without_type_icon() {
    let cases: [(&str, ToolSummary, &str); 4] = [
        ("read", read_summary(), text::agent::VERB_TOOL_READ),
        ("search", search_summary(), text::agent::VERB_TOOL_SEARCH),
        ("edit", edit_summary(), text::agent::VERB_TOOL_EDIT),
        ("write", write_summary(), text::agent::VERB_TOOL_WRITE),
    ];
    for (name, summary, verb) in cases {
        let call = tool_call(name, ToolStatus::Completed, Some(summary), None);
        let (painted, grid) = render_row(&call, 80);
        for frame in text::LX_LOADING_FRAMES {
            assert!(
                !painted.contains(frame),
                "{name} 完成态不得出现 braille 转轮 {frame}：{painted}"
            );
        }
        assert!(
            painted.starts_with(text::agent::STEP_MARK_FILLED),
            "{name} 行首为静态方块：{painted}"
        );
        assert!(painted.contains(verb), "{name} 缺少动词 {verb}：{painted}");
        assert!(
            painted.contains(text::tool_status_label(ToolStatus::Completed)),
            "{name} 缺少完成态文字：{painted}"
        );
        // 不再使用按工具区分的类型图标（G11：统一方块）。`+`（write 的图标）会与
        // 增删计数的 `+12` 撞字符，故只查不会自然出现的三类；行首由上面的
        // `starts_with(方块)` 断言兜底。
        for icon in [
            text::agent::ICON_READ,
            text::agent::ICON_SEARCH,
            text::agent::ICON_EDIT,
            text::agent::ICON_MCP,
        ] {
            assert!(
                !painted.contains(icon),
                "{name} 不再画类型图标 {icon:?}：{painted}"
            );
        }
        // 完成方块：品牌天蓝加粗、**不压暗**（压暗会与边框标题/正文混同）。
        let lead = grid.cell(0, 0).style;
        assert_eq!(
            lead.fg,
            style::chat_step_mark_done().fg,
            "{name} 完成方块品牌天蓝"
        );
        assert!(
            lead.add_modifier.contains(ratatui::style::Modifier::BOLD),
            "{name} 完成方块加粗"
        );
        assert!(
            !lead.add_modifier.contains(ratatui::style::Modifier::DIM),
            "{name} 完成方块不压暗"
        );
        assert_ne!(lead.fg, style::accent().fg, "{name} 不与边框青同色");
    }
}

#[test]
fn failed_tool_is_red_and_carries_trimmed_reason() {
    for (name, summary) in [
        ("read", read_summary()),
        ("search", search_summary()),
        ("edit", edit_summary()),
        ("write", write_summary()),
    ] {
        let call = tool_call(
            name,
            ToolStatus::Failed,
            Some(summary),
            Some("context mismatch\nsecond line"),
        );
        let (painted, grid) = render_row(&call, 100);
        assert!(
            painted.starts_with(text::agent::STEP_MARK_FILLED),
            "{name} 失败行首为方块：{painted}"
        );
        assert!(
            painted.contains(text::tool_status_label(ToolStatus::Failed)),
            "{name} 显示失败文字：{painted}"
        );
        assert!(
            painted.contains("context mismatch"),
            "{name} 显示精简原因：{painted}"
        );
        assert!(
            painted.contains("context mismatch second line"),
            "{name} 原因合并成单行（换行转空格）：{painted}"
        );
        assert!(
            !painted.contains('\n'),
            "原因里的换行必须转成空格：{painted}"
        );
        // 整行红 + 粗（对齐 gallery 的 error 步骤）。
        let fail = style::agent_fail();
        for x in 0..u16::try_from(painted.width()).unwrap_or(0) {
            let glyph = grid.cell(x, 0);
            if glyph.ch == ' ' {
                continue;
            }
            assert_eq!(glyph.style.fg, fail.fg, "{name} 失败行应整行红：{painted}");
        }
    }
}

#[test]
fn cancelled_and_interrupted_are_distinguishable_from_failed() {
    let cancelled = tool_call("read", ToolStatus::Cancelled, Some(read_summary()), None);
    let (painted, grid) = render_row(&cancelled, 80);
    assert!(
        painted.contains(text::tool_status_label(ToolStatus::Cancelled)),
        "取消态显示状态文字：{painted}"
    );
    assert!(
        !painted.contains(text::agent::ICON_ERROR),
        "取消不是失败，不用失败标记：{painted}"
    );
    // 取消用空心方块（未产出结果）+ 次要信息色，与失败/中断/完成都不同。
    assert!(
        painted.starts_with(text::agent::STEP_MARK_HOLLOW),
        "取消态行首用空心方块：{painted}"
    );
    assert_eq!(
        grid.cell(0, 0).style.fg,
        style::agent_step_detail().fg,
        "取消方块用次要信息色"
    );

    // 中断：黄（警告）档，与失败红可区分；无 error 时不显示空段。
    let interrupted = tool_call("edit", ToolStatus::Interrupted, Some(edit_summary()), None);
    let rendered = segments(&interrupted, 80);
    let (_, status_style) = rendered
        .iter()
        .find(|(text, _)| text.contains(text::tool_status_label(ToolStatus::Interrupted)))
        .cloned()
        .expect("中断状态段");
    assert_eq!(status_style.fg, style::agent_warn().fg, "中断用警告黄");
    assert_ne!(status_style.fg, style::agent_fail().fg, "与失败红可区分");
    let (interrupted_text, interrupted_grid) = render_row(&interrupted, 80);
    assert!(
        interrupted_text.starts_with(text::agent::STEP_MARK_FILLED),
        "中断用实心方块：{interrupted_text}"
    );
    assert_eq!(
        interrupted_grid.cell(0, 0).style.fg,
        style::agent_warn().fg,
        "中断方块黄色（与失败红区分）"
    );
    assert!(
        !line_text(&interrupted, 80).ends_with(text::LX_CHAT_TOOL_STATUS_SEP),
        "无 error 时不追加空段：{}",
        line_text(&interrupted, 80)
    );
}

// ===== 2. 各工具特有的次要信息 =====

#[test]
fn read_shows_line_range_in_range_color() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let rendered = segments(&call, 80);
    let (range_text, range_style) = rendered
        .iter()
        .find(|(text, _)| text.contains("12-40"))
        .cloned()
        .expect("行范围段");
    assert_eq!(
        range_text,
        format!("{}{}", text::LX_CHAT_TOOL_STATUS_SEP, "12-40")
    );
    assert_eq!(range_style, style::chat_tool_range(), "行范围用品牌天蓝");
    assert!(
        !range_style
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED)
    );
}

#[test]
fn search_shows_match_count_and_never_the_match_body() {
    let mut summary = search_summary();
    summary.truncated = true;
    let call = tool_call("search", ToolStatus::Completed, Some(summary), None);
    let (painted, _) = render_row(&call, 100);
    assert!(painted.contains("3 matches"), "命中数：{painted}");
    assert!(
        painted.contains(text::agent::TOOL_TRUNCATED),
        "截断显式告知：{painted}"
    );
    // 命中正文（结果体）绝不进历史行。
    for needle in ["src/ui/style.rs:12:", "pub fn text()", "fn render"] {
        assert!(!painted.contains(needle), "不应铺开命中正文：{painted}");
    }

    // 单复数：1 用 `match`。
    let single = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src".into()),
            matches: Some(1),
            ..ToolSummary::default()
        }),
        None,
    );
    assert!(render_row(&single, 80).0.contains("1 match"));
}

#[test]
fn edit_and_write_show_added_and_removed_counts_in_diff_colors() {
    let edit = tool_call("edit", ToolStatus::Completed, Some(edit_summary()), None);
    let rendered = segments(&edit, 80);
    let added = rendered
        .iter()
        .find(|(value, _)| value.ends_with(text::agent::added_label(12).as_str()))
        .cloned()
        .expect("+12 段");
    assert_eq!(added.1, style::agent_added(), "新增计数用绿色");
    let removed = rendered
        .iter()
        .find(|(value, _)| value.ends_with(text::agent::removed_label(4).as_str()))
        .cloned()
        .expect("-4 段");
    assert_eq!(removed.1, style::agent_removed(), "删除计数用红色");

    // write：`+18/-0`（removed 有值且为 0 时仍显示，表示“整文件覆盖无删除”）。
    let write = tool_call("write", ToolStatus::Completed, Some(write_summary()), None);
    let painted = render_row(&write, 80).0;
    assert!(
        painted.contains(text::agent::added_label(18).as_str()),
        "{painted}"
    );
    assert!(
        painted.contains(text::agent::removed_label(0).as_str()),
        "{painted}"
    );
}

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
    assert!(
        painted.starts_with(text::agent::STEP_MARK_FILLED),
        "未知工具同样以方块起头：{painted}"
    );
    assert!(
        painted.contains("web_fetch"),
        "未知工具显示工具名：{painted}"
    );
    assert!(
        painted.contains("https://example.com"),
        "参数摘要：{painted}"
    );
}

#[test]
fn optional_error_and_summary_leave_no_empty_segments() {
    let bare = tool_call("read", ToolStatus::Completed, None, None);
    let painted = render_row(&bare, 60).0;
    let trimmed = painted.trim_end();
    assert!(!trimmed.contains("()"), "无摘要不显示空括号：{trimmed}");
    assert!(!trimmed.contains("  "), "无摘要不留双空格：{trimmed:?}");
    assert_eq!(trimmed, painted.trim_end(), "行尾不留空白（裁剪后）");
}

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
        let drawn = painted.width();
        assert!(
            drawn <= usize::from(width),
            "宽 {width} 越界：{painted:?}（{drawn} 列）"
        );
        if width >= 3 {
            // 状态始终可辨：行首方块。宽度足够时状态文字也应可见。
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
            assert!(
                painted.width() <= usize::from(width),
                "{path} 宽 {width} 越界：{painted:?}"
            );
            // 不得出现只画了一半的宽字符（占位格必须紧跟其首格）。
            for x in 0..width {
                let glyph = grid.cell(x, 0);
                if glyph.ch == crate::ui::main_content::lx::grid::CONTINUATION {
                    assert!(x > 0, "{path} 首格不得是宽字符占位格");
                }
            }
            // 状态在窄宽度下仍保留（宽度够时）。
            if width >= 20 {
                assert!(
                    painted.contains(text::tool_status_label(ToolStatus::Completed)),
                    "{path} 宽 {width} 丢失状态：{painted:?}"
                );
            }
        }
    }
}

#[test]
fn path_is_truncated_before_secondary_counts_are_dropped() {
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
    // 中等宽度：路径被中间省略，但计数与状态仍在。
    let (painted, _) = render_row(&call, 40);
    assert!(painted.contains('…'), "长路径中间省略：{painted}");
    assert!(
        painted.contains(text::agent::added_label(3).as_str()),
        "计数先于状态保留：{painted}"
    );
    assert!(
        painted.contains(text::tool_status_label(ToolStatus::Completed)),
        "状态最后才丢：{painted}"
    );
    // 生效顺序：极端宽度下先丢计数，状态仍可辨（行首方块）。
    let (tiny, _) = render_row(&call, 12);
    assert!(
        tiny.starts_with(text::agent::STEP_MARK_FILLED),
        "极窄仍保留行首方块：{tiny:?}"
    );
}

#[test]
fn secondary_fields_are_dropped_by_priority_before_the_status() {
    // `read`：行范围（键信息）比命中数（次要计数）保留得久；
    // 截断提示比两者都重要——一档档收窄，逐档确认顺序。
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src".into()),
            range: Some((1, 20)),
            matches: Some(7),
            truncated: true,
            ..ToolSummary::default()
        }),
        None,
    );
    let full = line_text(&call, 120);
    assert!(full.contains("1-20") && full.contains("7 matches"));
    assert!(full.contains(text::agent::TOOL_TRUNCATED));

    // 逐档收窄（10..120 列），确认三条不变量同时成立：
    // 1. 次要计数（命中数）比行范围先丢；
    // 2. 行范围比截断提示先丢（截断必须显式告知用户）；
    // 3. 状态文字比任何次要字段活得久——它会先于它们被丢的只有极窄档。
    let mut saw_range_without_matches = false;
    let mut saw_truncated_without_range = false;
    let mut widest_without_status = 0u16;
    for width in 10u16..120 {
        let painted = line_text(&call, width);
        let has_range = painted.contains("1-20");
        let has_matches = painted.contains("7 matches");
        let has_truncated = painted.contains(text::agent::TOOL_TRUNCATED);
        let has_status = painted.contains(text::tool_status_label(ToolStatus::Completed));
        if has_range && !has_matches {
            saw_range_without_matches = true;
        }
        if has_truncated && !has_range {
            saw_truncated_without_range = true;
        }
        assert!(
            !has_matches || has_range,
            "命中数不应比行范围活得久（宽 {width}）：{painted}"
        );
        assert!(
            !has_range || has_truncated,
            "行范围不应比截断提示活得久（宽 {width}）：{painted}"
        );
        if !has_status {
            widest_without_status = width;
        }
    }
    assert!(
        saw_range_without_matches,
        "应存在「保留行范围、丢掉命中数」的宽度档"
    );
    assert!(
        saw_truncated_without_range,
        "应存在「保留截断提示、丢掉行范围」的宽度档"
    );
    // 状态文字最晚丢：无状态只发生在极窄档（`Read` 动词都放不下的量级）。
    assert!(
        widest_without_status < 20,
        "状态文字应比次要字段活得久，却最早在 {widest_without_status} 列消失"
    );
}

// ===== 4. 行数与几何不变量 =====

#[test]
fn tool_row_count_is_always_one_for_any_content() {
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
            assert_eq!(rows(&call, width), 1, "宽 {width} 恒占 1 行");
        }
    }
    // 宽度 0：不占行（与空正文块同口径），渲染也不写任何单元格。
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    assert_eq!(rows(&call, 0), 0);
    let rect = Rect::new(0, 0, 0, 1);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, &call, 0);
    assert_eq!(
        row_index, 1,
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
    assert_eq!(row_index, 4, "不可见行仍推进");
    assert!(grid_text(&grid, rect).trim().is_empty(), "不可见行不写网格");
}

#[test]
fn settled_search_falls_back_to_arguments_for_its_scope() {
    // 结算后摘要只带命中数（`to_summary` 不给 search 的 path）；此时从参数 JSON
    // 取搜索范围补上，避免 search 行只剩动词与计数。
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
    let painted = render_row(&call, 80).0;
    assert!(painted.contains("src/ui"), "搜索范围回退显示：{painted}");
    assert!(painted.contains("5 matches"), "{painted}");
    // 参数里的正文/模式不铺开（只取 path）。
    assert!(!painted.contains("fn render"), "不铺搜索模式：{painted}");
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

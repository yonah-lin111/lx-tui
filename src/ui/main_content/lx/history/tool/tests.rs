//! 工具步骤行测试：两行式（`■ 动词` + `  └─ 详情`）× 多状态的 buffer 断言、
//! 宽度自适应、列宽裁剪与几何一致性。
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

/// 渲染两行到网格，按行拼出文本（跳过宽字符占位格），用 `\n` 连接。
fn render_row(call: &ToolCallView, width: u16) -> (String, Grid) {
    render_row_at(call, width, 0)
}

/// 以指定 loading 帧渲染（方块呼吸相位断言用）。
fn render_row_at(call: &ToolCallView, width: u16, loading_frame: usize) -> (String, Grid) {
    let rect = Rect::new(0, 0, width, (DETAIL_ROWS + HEADER_ROWS) as u16);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, call, loading_frame);
    assert_eq!(row_index, rows(call, width), "绘制推进行数必须与 rows 一致");
    (grid_text(&grid, rect), grid)
}

/// 网格整块文本（两行以 `\n` 连接）。
fn grid_text(grid: &Grid, rect: Rect) -> String {
    let mut lines: Vec<String> = Vec::new();
    for y in rect.y..rect.bottom() {
        let mut out = String::new();
        for x in rect.x..rect.right() {
            let glyph = grid.cell(x, y);
            if glyph.ch != crate::ui::main_content::lx::grid::CONTINUATION {
                out.push(glyph.ch);
            }
        }
        lines.push(out);
    }
    lines.join("\n")
}

/// 标题行的段（第 1 行：`■ 动词`）。
fn header_segments(call: &ToolCallView, width: u16) -> Vec<(String, Style)> {
    header(call, width, 0)
        .into_iter()
        .map(|segment| (segment.text, segment.style))
        .collect()
}

/// 详情行的段（第 2 行：`  └─ 详情`）。
fn detail_segments(call: &ToolCallView, width: u16) -> Vec<(String, Style)> {
    detail(call, width)
        .into_iter()
        .map(|segment| (segment.text, segment.style))
        .collect()
}

/// 第 1 行文本（未裁剪）。
fn header_text(call: &ToolCallView, width: u16) -> String {
    header_segments(call, width)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

/// 第 2 行文本（未裁剪）。
fn detail_text(call: &ToolCallView, width: u16) -> String {
    detail_segments(call, width)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

/// 两行用 `\n` 连接（段拼接，未裁剪）。
fn full_text(call: &ToolCallView, width: u16) -> String {
    format!("{}\n{}", header_text(call, width), detail_text(call, width))
}

// ===== 1. 两行结构与状态编码 =====

/// 第 1 行只有「方块 + 动词」：**没有任何状态文字**（`done` 已删除）。
#[test]
fn header_is_only_square_and_verb_without_status_text() {
    let cases: [(&str, ToolSummary, &str); 4] = [
        ("read", read_summary(), text::agent::VERB_TOOL_READ),
        ("search", search_summary(), text::agent::VERB_TOOL_SEARCH),
        ("edit", edit_summary(), text::agent::VERB_TOOL_EDIT),
        ("write", write_summary(), text::agent::VERB_TOOL_WRITE),
    ];
    for (name, summary, verb) in cases {
        for status in [
            ToolStatus::InputStreaming,
            ToolStatus::Pending,
            ToolStatus::Running,
            ToolStatus::Completed,
            ToolStatus::Failed,
            ToolStatus::Cancelled,
            ToolStatus::Interrupted,
        ] {
            let call = tool_call(name, status, Some(summary.clone()), Some("boom"));
            let painted = header_text(&call, 80);
            assert!(
                painted.starts_with(text::agent::STEP_MARK_FILLED)
                    || painted.starts_with(text::agent::STEP_MARK_HOLLOW),
                "{name}/{status:?} 行首应为步骤方块：{painted:?}"
            );
            assert!(
                painted.contains(verb),
                "{name}/{status:?} 第 1 行应含动词 {verb}：{painted:?}"
            );
            // 关键：第 1 行不再出现任何状态文字（尤其 `done`）。
            for status_text in [
                text::LX_CHAT_TOOL_INPUT_STREAMING,
                text::LX_CHAT_TOOL_PENDING,
                text::LX_CHAT_TOOL_RUNNING,
                text::LX_CHAT_TOOL_COMPLETED,
                text::LX_CHAT_TOOL_FAILED,
                text::LX_CHAT_TOOL_CANCELLED,
                text::LX_CHAT_TOOL_INTERRUPTED,
            ] {
                assert!(
                    !painted.contains(status_text),
                    "{name}/{status:?} 第 1 行不该有状态文字 {status_text:?}：{painted:?}"
                );
            }
            // 第 1 行也不带参数：路径只在第 2 行。
            if let Some(summary) = call.summary.as_ref().and_then(|s| s.path.as_deref()) {
                assert!(
                    !painted.contains(summary),
                    "{name}/{status:?} 第 1 行不该有路径：{painted:?}"
                );
            }
            // 也没有 `·` 分隔符（状态/参数都在第 2 行）。
            assert!(
                !painted.contains(text::LX_CHAT_TOOL_STATUS_SEP.trim()),
                "{name}/{status:?} 第 1 行不该有分隔符：{painted:?}"
            );
        }
    }
}

/// 完成态、失败态都不显示 `done` / `failed` 这类状态词；只有第 2 行承载细节。
#[test]
fn completed_and_failed_never_show_status_words_on_any_line() {
    let completed = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let painted = full_text(&completed, 80);
    assert!(
        !painted.contains(text::LX_CHAT_TOOL_COMPLETED),
        "完成态不再显示 done：{painted:?}"
    );
    // 完成态第 2 行是参数。
    assert!(painted.contains("src/ui/style.rs"), "{painted:?}");

    let failed = tool_call(
        "read",
        ToolStatus::Failed,
        Some(read_summary()),
        Some("read /etc/shadow failed: permission denied"),
    );
    // 第 1 行不该有状态词；第 2 行是错误文案本身（可能天然含 "failed" 字样，
    // 那是错误文本的一部分，不是状态字段）。
    let header_line = header_text(&failed, 100);
    assert!(
        !header_line.contains(text::LX_CHAT_TOOL_FAILED),
        "失败态第 1 行不再显示 failed：{header_line:?}"
    );
    assert!(
        header_line.trim_end()
            == format!(
                "{} {}",
                text::agent::STEP_MARK_FILLED,
                text::agent::VERB_TOOL_READ
            ),
        "失败态第 1 行只有方块 + 动词：{header_line:?}"
    );
}

/// 第 2 行恒以 `  └─ ` 起头（缩进 2 列 + 连接符 + 空格），与通知组件同口径。
#[test]
fn detail_line_starts_with_tree_leaf_prefix() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let prefix = format!("  {} ", text::agent::MARKER_TREE_LEAF);
    let painted = detail_text(&call, 80);
    assert!(painted.starts_with(&prefix), "第 2 行前缀：{painted:?}");
    // 连接符本身用通知同款灰色 + 加粗。
    let segments = detail_segments(&call, 80);
    assert_eq!(
        segments[0].0, prefix,
        "首段必须是完整前缀（缩进+连接符+空格）"
    );
    assert_eq!(
        segments[0].1,
        style::chat_notice_connector(),
        "连接符用通知同款灰色+粗"
    );
}

/// 第 2 行的参数：路径金黄下划线、行范围 dim、增删绿/红。
#[test]
fn detail_line_carries_params_with_their_own_colors() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let segments = detail_segments(&call, 80);
    let path = segments
        .iter()
        .find(|(text, _)| text == "src/ui/style.rs")
        .cloned()
        .expect("路径段");
    assert_eq!(path.1, style::chat_tool_param(), "路径金黄下划线");
    let range = segments
        .iter()
        .find(|(text, _)| text.contains("12-40"))
        .cloned()
        .expect("行范围段");
    assert_eq!(range.1, style::chat_tool_range(), "行范围 dim");

    // edit 的增删计数。
    let edit = tool_call("edit", ToolStatus::Completed, Some(edit_summary()), None);
    let segments = detail_segments(&edit, 80);
    let added = segments
        .iter()
        .find(|(text, _)| text.ends_with(text::agent::added_label(12).as_str()))
        .cloned()
        .expect("+12 段");
    assert_eq!(added.1, style::agent_added(), "新增绿");
    let removed = segments
        .iter()
        .find(|(text, _)| text.ends_with(text::agent::removed_label(4).as_str()))
        .cloned()
        .expect("-4 段");
    assert_eq!(removed.1, style::agent_removed(), "删除红");
}

/// 失败：**整行第 1 行红**（方块 + 动词），第 2 行**只有错误**，不显示任何参数。
#[test]
fn failed_header_is_red_and_detail_shows_only_the_error() {
    for (name, summary) in [
        ("read", read_summary()),
        ("search", search_summary()),
        ("edit", edit_summary()),
        ("write", write_summary()),
    ] {
        let summary_path = summary.path.clone().expect("有路径");
        let call = tool_call(
            name,
            ToolStatus::Failed,
            Some(summary),
            Some("read /etc/shadow failed: permission denied\nsecond line"),
        );
        let (painted, grid) = render_row(&call, 100);
        let (header_line, detail_line) = painted.split_once('\n').expect("两行");
        // 第 1 行整行红 + 粗。
        let fail = style::agent_fail();
        for x in 0..u16::try_from(header_line.width()).unwrap_or(0) {
            let glyph = grid.cell(x, 0);
            if glyph.ch == ' ' {
                continue;
            }
            assert_eq!(
                glyph.style.fg, fail.fg,
                "{name} 第 1 行应整行红：{header_line:?}"
            );
        }
        // 第 2 行只放错误：不含路径、不含计数。
        assert!(
            detail_line.contains("permission denied"),
            "{name} 第 2 行显示错误：{detail_line:?}"
        );
        assert!(
            detail_line.contains("permission denied second line"),
            "{name} 错误合并成单行（换行转空格）：{detail_line:?}"
        );
        assert!(
            !detail_line.contains(&summary_path),
            "{name} 失败时第 2 行不该再显示路径参数：{detail_line:?}"
        );
        for extra in ["12-40", "+12", "-4", "3 matches", "+18"] {
            assert!(
                !detail_line.contains(extra),
                "{name} 失败时第 2 行不该显示计数 {extra:?}：{detail_line:?}"
            );
        }
        // 错误文案用正常前景（不是红），红只在第 1 行。
        let error_segment = detail_segments(&call, 100)
            .into_iter()
            .find(|(text, _)| text.contains("permission denied"))
            .expect("错误段");
        assert_eq!(error_segment.1, style::text(), "{name} 错误文案用正常前景");
    }
}

/// 中断：第 1 行黄，第 2 行只放原因；无 error 时第 2 行为空（行数仍算 2）。
#[test]
fn interrupted_header_is_yellow_and_detail_shows_only_the_reason() {
    let call = tool_call(
        "edit",
        ToolStatus::Interrupted,
        Some(edit_summary()),
        Some("interrupted before completion"),
    );
    let (painted, grid) = render_row(&call, 100);
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert_eq!(
        grid.cell(0, 0).style.fg,
        style::agent_warn().fg,
        "中断方块黄色"
    );
    assert!(
        header_line.contains(text::agent::VERB_TOOL_EDIT),
        "第 1 行动词：{header_line:?}"
    );
    assert!(
        detail_line.contains("interrupted before completion"),
        "第 2 行原因：{detail_line:?}"
    );
    assert!(
        !detail_line.contains("src/app/state.rs"),
        "中断时不显示路径参数：{detail_line:?}"
    );

    // 无 error：第 2 行只有前缀（不留状态词）。
    let bare = tool_call("edit", ToolStatus::Interrupted, Some(edit_summary()), None);
    let detail = detail_text(&bare, 80);
    assert_eq!(
        detail,
        format!("  {} ", text::agent::MARKER_TREE_LEAF),
        "无原因时第 2 行只剩前缀：{detail:?}"
    );
}

/// 取消：空心方块 + 第 2 行 `cancelled` 兜底（状态词从第 1 行搬到第 2 行）。
#[test]
fn cancelled_uses_hollow_square_and_puts_the_word_in_the_detail_line() {
    let call = tool_call("read", ToolStatus::Cancelled, Some(read_summary()), None);
    let (painted, grid) = render_row(&call, 80);
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert!(
        header_line.starts_with(text::agent::STEP_MARK_HOLLOW),
        "取消态行首用空心方块：{header_line:?}"
    );
    assert_eq!(
        grid.cell(0, 0).style.fg,
        style::agent_step_detail().fg,
        "取消方块用次要信息色"
    );
    assert!(
        detail_line.contains(text::LX_CHAT_TOOL_CANCELLED),
        "第 2 行显示 cancelled：{detail_line:?}"
    );
    let segment = detail_segments(&call, 80)
        .into_iter()
        .find(|(text, _)| text.contains(text::LX_CHAT_TOOL_CANCELLED))
        .expect("兜底段");
    assert_eq!(segment.1, style::agent_step_detail(), "兜底词 dim");
}

/// 没有参数可展示的进行中状态：第 2 行用 dim 状态词兜底，永不为空。
#[test]
fn open_states_without_params_fall_back_to_a_status_word() {
    for (status, expected) in [
        (
            ToolStatus::InputStreaming,
            text::LX_CHAT_TOOL_INPUT_STREAMING,
        ),
        (ToolStatus::Pending, text::LX_CHAT_TOOL_PENDING),
        (ToolStatus::Running, text::LX_CHAT_TOOL_RUNNING),
    ] {
        let call = tool_call("read", status, None, None);
        let detail = detail_text(&call, 80);
        assert!(
            detail.contains(expected),
            "{status:?} 第 2 行应兜底 {expected:?}：{detail:?}"
        );
        assert!(
            detail.starts_with(text::agent::MARKER_TREE_LEAF.trim_end().trim_start())
                || detail.contains(text::agent::MARKER_TREE_LEAF),
            "{status:?} 兜底同样带连接符：{detail:?}"
        );
    }
    // 完成态没有兜底：`done` 已删除，第二行留空。
    let done = tool_call("read", ToolStatus::Completed, None, None);
    assert_eq!(
        detail_text(&done, 80),
        format!("  {} ", text::agent::MARKER_TREE_LEAF),
        "完成态无摘要时第 2 行留空（不显示 done）"
    );
}

/// 方块呼吸：运行中两帧交替，终态静态。
#[test]
fn step_square_breathes_while_open_and_stays_static_when_settled() {
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

/// 完成态：静态翡翠绿方块，不出现 braille 转轮，也不画类型图标。
#[test]
fn completed_tool_has_square_verb_and_detail_without_type_icon() {
    let cases: [(&str, ToolSummary, &str); 4] = [
        ("read", read_summary(), text::agent::VERB_TOOL_READ),
        ("search", search_summary(), text::agent::VERB_TOOL_SEARCH),
        ("edit", edit_summary(), text::agent::VERB_TOOL_EDIT),
        ("write", write_summary(), text::agent::VERB_TOOL_WRITE),
    ];
    for (name, summary, verb) in cases {
        let call = tool_call(name, ToolStatus::Completed, Some(summary), None);
        let (painted, grid) = render_row(&call, 80);
        assert!(
            !painted
                .chars()
                .any(|ch| ('\u{2800}'..='\u{28ff}').contains(&ch)),
            "{name} 完成态不得出现 braille 转轮：{painted}"
        );
        assert!(
            painted.starts_with(text::agent::STEP_MARK_FILLED),
            "{name} 行首为静态方块：{painted}"
        );
        let (header_line, _) = painted.split_once('\n').expect("两行");
        assert!(
            header_line.contains(verb),
            "{name} 缺少动词 {verb}：{header_line:?}"
        );
        // 不再使用按工具区分的类型图标（G11：统一方块）。
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
        // 完成方块：翡翠绿、不加粗、**不压暗**。
        let lead = grid.cell(0, 0).style;
        assert_eq!(
            lead.fg,
            style::chat_step_mark_done().fg,
            "{name} 完成方块翡翠绿"
        );
        assert!(
            !lead.add_modifier.contains(ratatui::style::Modifier::BOLD),
            "{name} 完成方块不加粗（与运行中区分）"
        );
        assert!(
            !lead.add_modifier.contains(ratatui::style::Modifier::DIM),
            "{name} 完成方块不压暗"
        );
        assert_ne!(lead.fg, style::accent().fg, "{name} 不与边框青同色");
    }
}

// ===== 2. 各工具特有的次要信息 =====

#[test]
fn search_shows_match_count_and_never_the_match_body() {
    let mut summary = search_summary();
    summary.truncated = true;
    let call = tool_call("search", ToolStatus::Completed, Some(summary), None);
    let painted = detail_text(&call, 100);
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
    assert!(detail_text(&single, 80).contains("1 match"));
}

#[test]
fn edit_and_write_show_added_and_removed_counts_in_diff_colors() {
    let write = tool_call("write", ToolStatus::Completed, Some(write_summary()), None);
    let painted = detail_text(&write, 80);
    assert!(
        painted.contains(text::agent::added_label(18).as_str()),
        "{painted}"
    );
    // `removed: Some(0)` 仍显示 `-0`（表示整文件覆盖无删除）。
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
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert!(
        header_line.contains("web_fetch"),
        "未知工具在第 1 行显示工具名：{header_line:?}"
    );
    assert!(
        detail_line.contains("https://example.com"),
        "参数摘要在第 2 行：{detail_line:?}"
    );
}

#[test]
fn optional_error_and_summary_leave_no_empty_segments() {
    let bare = tool_call("read", ToolStatus::Completed, None, None);
    let painted = render_row(&bare, 60).0;
    assert!(!painted.contains("()"), "无摘要不显示空括号：{painted:?}");
    // 第 1 行不留尾随空格以外的痕迹；第 2 行只有前缀。
    let (header_line, detail_line) = painted.split_once('\n').expect("两行");
    assert_eq!(
        header_line.trim_end(),
        format!(
            "{} {}",
            text::agent::STEP_MARK_FILLED,
            text::agent::VERB_TOOL_READ
        ),
        "第 1 行只有方块 + 动词：{header_line:?}"
    );
    assert_eq!(
        detail_line.trim_end(),
        format!("  {}", text::agent::MARKER_TREE_LEAF),
        "第 2 行只有前缀：{detail_line:?}"
    );
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

// ===== 5. 搜索模式 / 实际行数 / 计数组装 =====

/// 搜索行必须显示 pattern（加引号）且在路径之前：`"测试" in path · 3 matches`。
#[test]
fn search_shows_the_pattern_before_the_path() {
    let call = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("tool_test.txt".into()),
            pattern: Some("测试".into()),
            matches: Some(3),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    let expected = format!(
        "  {} {} {}tool_test.txt{}{}",
        text::agent::MARKER_TREE_LEAF,
        text::agent::pattern_label("测试"),
        text::agent::IN_SCOPE_SEP,
        text::LX_CHAT_TOOL_STATUS_SEP,
        text::agent::matches_label(3)
    );
    assert_eq!(detail, expected, "搜索行 = 「模式 in 路径 · 命中数」");

    // 模式与路径都用参数色（金黄、无下划线）。
    let segments = detail_segments(&call, 80);
    let scope = segments
        .iter()
        .find(|(text, _)| text.contains("测试"))
        .cloned()
        .expect("模式段");
    assert_eq!(scope.1, style::chat_tool_param());
    assert!(
        !scope
            .1
            .add_modifier
            .contains(ratatui::style::Modifier::UNDERLINED),
        "参数不再加下划线"
    );
}

/// 只有模式没有路径时仍显示模式（`"fn render" · 5 matches`）。
#[test]
fn search_without_path_still_shows_the_pattern() {
    let call = tool_call(
        "search",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: None,
            pattern: Some("fn render".into()),
            matches: Some(5),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("fn render"), "模式必须显示：{detail:?}");
    assert!(detail.contains("5 matches"), "{detail:?}");
    assert!(
        !detail.contains(text::agent::IN_SCOPE_SEP.trim()),
        "无路径时不该有 in：{detail:?}"
    );
}

/// `read` 同时显示请求区间与实际行数：`12-40 · (31 lines)`。
#[test]
fn read_shows_range_and_actual_line_count() {
    let call = tool_call(
        "read",
        ToolStatus::Completed,
        Some(ToolSummary {
            path: Some("src/ui/style.rs".into()),
            range: Some((12, 40)),
            total_lines: Some(31),
            ..ToolSummary::default()
        }),
        None,
    );
    let detail = detail_text(&call, 80);
    assert!(detail.contains("12-40"), "区间：{detail:?}");
    assert!(
        detail.contains(&text::agent::lines_label(31)),
        "实际行数：{detail:?}"
    );
    // 行数与区间同色（dim 元信息）。
    let segments = detail_segments(&call, 80);
    let lines = segments
        .iter()
        .find(|(text, _)| text.contains("31 lines"))
        .cloned()
        .expect("行数段");
    assert_eq!(lines.1, style::chat_tool_range());
}

/// 没有实际行数时只显示区间（老数据/中断调用不编造）。
#[test]
fn read_without_total_lines_shows_the_range_only() {
    let call = tool_call("read", ToolStatus::Completed, Some(read_summary()), None);
    let detail = detail_text(&call, 80);
    assert!(detail.contains("12-40"));
    assert!(!detail.contains("lines)"), "不编造行数：{detail:?}");
}

/// 增删计数合成一段 `+12 -4`（中间不插 `·`），且各自保留绿/红。
#[test]
fn edit_counts_are_one_group_without_a_separator() {
    let call = tool_call("edit", ToolStatus::Completed, Some(edit_summary()), None);
    let detail = detail_text(&call, 80);
    assert!(
        detail.ends_with("+12 -4"),
        "应是 `+12 -4` 一个整体：{detail:?}"
    );
    assert!(
        !detail.contains(&format!("+12{}", text::LX_CHAT_TOOL_STATUS_SEP)),
        "两个计数之间不该有 `·`：{detail:?}"
    );
    // 颜色仍分开：`+12` 绿、`-4` 红。
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

/// `write` 的 `+18 -0` 同样合体（0 删除仍显示）。
#[test]
fn write_counts_are_one_group_too() {
    let call = tool_call("write", ToolStatus::Completed, Some(write_summary()), None);
    let detail = detail_text(&call, 80);
    assert!(detail.ends_with("+18 -0"), "{detail:?}");
}

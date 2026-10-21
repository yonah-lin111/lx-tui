//! 文件差异卡片测试：布局/绘制行数一致、预览 3 行、展开全部、宽度安全、样式与符号。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use super::*;
use crate::app::state::ToolStatus;
use crate::chat::tools::{
    DiffCounts, DiffHunk, DiffLine, DiffLineKind, DiffUnavailableReason, FileDiff,
};

/// 工具层差异 → 详情视图（生产路径是 JSON 解码，这里走同源的 From）。
fn detail_of(diffs: Vec<(FileDiff, usize, usize)>) -> FileDiffDetailView {
    let files: Vec<FileChangeView> = diffs
        .into_iter()
        .enumerate()
        .map(|(index, (diff, added, removed))| FileChangeView {
            path: format!("/tmp/file{index}.rs"),
            to: None,
            kind: FileChangeKind::Updated,
            added: Some(added),
            removed: Some(removed),
            counts_known: diff.counts_known(),
            simplified: diff.simplified(),
            diff: Some(FileDiffView::from(&diff)),
        })
        .collect();
    FileDiffDetailView {
        files,
        files_total: 1,
        added: Some(1),
        removed: Some(1),
        simplified: false,
        omitted_files: 0,
        single: false,
    }
}

/// 一行差异。
fn line(kind: DiffLineKind, old: Option<usize>, new: Option<usize>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old_line: old,
        new_line: new,
        text: text.into(),
        no_final_newline: false,
    }
}

/// 单个 hunk 的完整差异。
fn complete(hunks: Vec<Vec<DiffLine>>) -> FileDiff {
    FileDiff::Complete {
        hunks: hunks.into_iter().map(|lines| DiffHunk { lines }).collect(),
        simplified: false,
    }
}

/// 两行替换（1 上下文 + 删除 + 新增）。
fn replace_diff() -> FileDiff {
    complete(vec![vec![
        line(DiffLineKind::Context, Some(1), Some(1), "keep"),
        line(DiffLineKind::Removed, Some(2), None, "old"),
        line(DiffLineKind::Added, None, Some(2), "new"),
    ]])
}

/// 构造文件工具的调用步骤。
fn tool_call(name: &str, view: FileDiffDetailView) -> ToolCallView {
    ToolCallView {
        call_id: "call-1".into(),
        name: name.into(),
        position: 0,
        arguments_json: String::new(),
        status: ToolStatus::Completed,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(crate::app::state::ToolDetailView::File(view))),
    }
}

/// 渲染到网格并返回逐行文本。
fn render(call: &ToolCallView, width: u16, expanded: bool) -> (Vec<String>, usize) {
    let total = rows(call, width, expanded);
    let height = u16::try_from(total.max(1)).unwrap_or(u16::MAX);
    let rect = Rect::new(0, 0, width, height);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, call, 0, expanded, false);
    assert_eq!(row_index, total, "draw 推进行数必须等于 rows（宽 {width}）");
    let mut lines = Vec::new();
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
    (lines, total)
}

/// 单元格样式（按列取）。
fn style_at(call: &ToolCallView, width: u16, expanded: bool, x: u16, y: u16) -> Style {
    let total = rows(call, width, expanded);
    let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 0, &mut row_index, call, 0, expanded, false);
    grid.cell(x, y).style
}

/// 20 行里两处远距修改：展开后 hunk 之间有 `⋮`。
#[test]
fn expanded_shows_all_hunks_with_separator() {
    let mut first = vec![line(DiffLineKind::Context, Some(1), Some(1), "a")];
    first.push(line(DiffLineKind::Removed, Some(2), None, "b"));
    first.push(line(DiffLineKind::Added, None, Some(2), "B"));
    let mut second = vec![line(DiffLineKind::Context, Some(9), Some(9), "i")];
    second.push(line(DiffLineKind::Removed, Some(10), None, "j"));
    second.push(line(DiffLineKind::Added, None, Some(10), "J"));
    let view = detail_of(vec![(complete(vec![first, second]), 2, 2)]);
    let call = tool_call("edit", view);
    let (lines, _) = render(&call, 80, true);
    let text = lines.join("\n");
    assert!(text.contains(text::agent::FILE_DIFF_HUNK_SEP), "{text}");
    assert!(text.contains("B") && text.contains("J"), "{text}");
    // 上下文行也进入展开正文。
    assert!(text.contains('a') && text.contains('i'));
    // 行号取自真实文件行号。
    assert!(text.contains(" 2 ") || text.contains("2 -"), "{text}");
}

/// 折叠：每文件最多 3 个正文物理行；第 4 行起出省略提示（提示不占额度）。
#[test]
fn preview_is_capped_at_three_physical_rows() {
    let diff = complete(vec![vec![
        line(DiffLineKind::Removed, Some(1), None, "one"),
        line(DiffLineKind::Added, None, Some(1), "ONE"),
        line(DiffLineKind::Removed, Some(2), None, "two"),
        line(DiffLineKind::Added, None, Some(2), "TWO"),
        line(DiffLineKind::Removed, Some(3), None, "three"),
    ]]);
    let view = detail_of(vec![(diff, 2, 3)]);
    let call = tool_call("edit", view);
    let (lines, _) = render(&call, 80, false);
    // 标题 + 文件头 + 3 正文行 + 省略提示。
    assert_eq!(lines.len(), 1 + 1 + PREVIEW_LINES + 1, "{lines:#?}");
    assert!(
        lines
            .last()
            .is_some_and(|l| l.contains(text::agent::FILE_DIFF_OMITTED))
    );
    // 上下文行不占预览额度：预览里完全不出现上下文文本。
    assert!(!lines.join("\n").contains("keep"));
    // 只有 3 个正文行（不含提示）。
    let body = &lines[2..2 + PREVIEW_LINES];
    assert_eq!(body.len(), 3);
    assert!(body[0].contains("one"));
    assert!(body[1].contains("ONE"));
    assert!(body[2].contains("two"));
}

/// 内容刚好 3 行以内：不显示省略提示。
#[test]
fn short_preview_has_no_omitted_hint() {
    let diff = complete(vec![vec![
        line(DiffLineKind::Removed, Some(1), None, "a"),
        line(DiffLineKind::Added, None, Some(1), "A"),
    ]]);
    let call = tool_call("edit", detail_of(vec![(diff, 1, 1)]));
    let (lines, _) = render(&call, 80, false);
    // 标题 + 文件头 + 2 个正文行（不足 3 行，不加提示）。
    assert_eq!(lines.len(), 1 + 1 + 2, "{lines:#?}");
    assert!(!lines.join("\n").contains(text::agent::FILE_DIFF_OMITTED));
}

/// 长行按列宽换行：续行保留空 gutter、不重复符号与行号。
#[test]
fn long_line_wraps_without_repeating_gutter() {
    let long = "x".repeat(50);
    let diff = complete(vec![vec![line(DiffLineKind::Added, None, Some(7), &long)]]);
    let call = tool_call("write", detail_of(vec![(diff, 1, 0)]));
    let (lines, _) = render(&call, 20, false);
    // 标题 + 文件头 + 多行正文。
    assert!(lines.len() > 3);
    let body: Vec<&String> = lines[2..].iter().collect();
    assert!(body[0].contains('7') && body[0].contains('+'), "{body:#?}");
    for continuation in &body[1..] {
        assert!(
            !continuation.contains('+'),
            "续行不得重复 +：{continuation:?}"
        );
        assert!(
            !continuation.contains('7'),
            "续行不得重复行号：{continuation:?}"
        );
    }
    // 每行都在列宽内。
    for line in &lines {
        assert!(line.width() <= 20, "越界：{line:?}（{} 列）", line.width());
    }
}

/// 中文按显示列换行，不按字节切。
#[test]
fn wide_characters_wrap_by_display_columns() {
    let long = "中文内容测试".repeat(6);
    let diff = complete(vec![vec![line(DiffLineKind::Added, None, Some(1), &long)]]);
    let call = tool_call("write", detail_of(vec![(diff, 1, 0)]));
    for width in [8u16, 12, 20, 40] {
        let (lines, _) = render(&call, width, false);
        for line in &lines {
            assert!(
                line.width() <= usize::from(width),
                "宽 {width} 越界：{line:?}（{} 列）",
                line.width()
            );
        }
        assert!(!lines.join("").contains('\u{fffd}'));
    }
}

/// 宽度 0/1/2/8 不死循环、不越界；0 宽返回 0 行。
#[test]
fn narrow_widths_are_safe() {
    let call = tool_call("edit", detail_of(vec![(replace_diff(), 1, 1)]));
    assert_eq!(rows(&call, 0, false), 0);
    for width in [1u16, 2, 3, 8, 20] {
        for expanded in [false, true] {
            let (lines, total) = render(&call, width, expanded);
            assert!(total >= 1);
            for line in &lines {
                assert!(
                    line.width() <= usize::from(width),
                    "宽 {width} 越界：{line:?}"
                );
            }
        }
    }
    // 极窄仍保留变更符号 `+` / `-`。
    let (lines, _) = render(&call, 2, false);
    let text = lines.join("\n");
    assert!(text.contains('+') || text.contains('-'), "{text}");
}

/// 新增绿、删除红；行号/上下文为次要信息色。
#[test]
fn added_and_removed_use_semantic_styles() {
    let call = tool_call("edit", detail_of(vec![(replace_diff(), 1, 1)]));
    let (lines, _) = render(&call, 40, false);
    // 第 3 行是删除、第 4 行是新增。
    let removed_row = 2u16;
    let added_row = 3u16;
    let removed_x =
        u16::try_from(lines[usize::from(removed_row)].find('-').expect("删除符号")).expect("列");
    let added_x =
        u16::try_from(lines[usize::from(added_row)].find('+').expect("新增符号")).expect("列");
    assert_eq!(
        style_at(&call, 40, false, removed_x, removed_row),
        style::agent_removed()
    );
    assert_eq!(
        style_at(&call, 40, false, added_x, added_row),
        style::agent_added()
    );
    // 正文首字符（`old`/`new`）同色。
    let old_x =
        u16::try_from(lines[usize::from(removed_row)].find("old").expect("正文")).expect("列");
    assert_eq!(
        style_at(&call, 40, false, old_x, removed_row),
        style::agent_removed()
    );
}

/// 无内容变化：显示 `No content changes`，仍占一行且可展开（不制造无限入口）。
#[test]
fn empty_diff_shows_no_content_changes() {
    let call = tool_call("edit", detail_of(vec![(complete(Vec::new()), 0, 0)]));
    let (lines, _) = render(&call, 60, false);
    assert!(lines.join("\n").contains(text::agent::FILE_DIFF_NO_CHANGES));
    let (expanded_lines, _) = render(&call, 60, true);
    assert!(
        expanded_lines
            .join("\n")
            .contains(text::agent::FILE_DIFF_NO_CHANGES)
    );
}

/// 不可用原因映射为明确文案。
#[test]
fn unavailable_reason_is_explicit() {
    for (reason, needle) in [
        (DiffUnavailableReason::Binary, "binary"),
        (DiffUnavailableReason::NotUtf8, "not UTF-8"),
        (DiffUnavailableReason::ReadFailed, "read failed"),
    ] {
        let call = tool_call(
            "edit",
            detail_of(vec![(FileDiff::Unavailable { reason }, 0, 0)]),
        );
        let (lines, _) = render(&call, 80, true);
        let text = lines.join("\n");
        assert!(text.contains(text::agent::FILE_DIFF_UNAVAILABLE), "{text}");
        assert!(text.contains(needle), "{text:?}");
    }
}

/// 截断与简化提示只在展开时出现，且不影响折叠预览。
#[test]
fn truncated_and_simplified_hints_show_when_expanded() {
    let diff = FileDiff::Truncated {
        hunks: vec![DiffHunk {
            lines: vec![line(DiffLineKind::Added, None, Some(1), "x")],
        }],
        simplified: true,
    };
    let call = tool_call("edit", detail_of(vec![(diff, 1, 0)]));
    let (folded, _) = render(&call, 60, false);
    let folded = folded.join("\n");
    assert!(
        !folded.contains(text::agent::FILE_DIFF_TRUNCATED),
        "{folded}"
    );
    assert!(
        !folded.contains(text::agent::FILE_DIFF_SIMPLIFIED),
        "{folded}"
    );
    let (expanded, _) = render(&call, 60, true);
    let expanded = expanded.join("\n");
    assert!(
        expanded.contains(text::agent::FILE_DIFF_TRUNCATED),
        "{expanded}"
    );
    assert!(
        expanded.contains(text::agent::FILE_DIFF_SIMPLIFIED),
        "{expanded}"
    );
}

/// EOF 无换行：元信息行只在展开时出现。
#[test]
fn eof_marker_is_meta_row() {
    let mut last = line(DiffLineKind::Added, None, Some(1), "hello");
    last.no_final_newline = true;
    let call = tool_call("write", detail_of(vec![(complete(vec![vec![last]]), 1, 0)]));
    let (folded, _) = render(&call, 60, false);
    assert!(
        !folded
            .join("\n")
            .contains(text::agent::FILE_DIFF_NO_FINAL_NEWLINE)
    );
    let (expanded, _) = render(&call, 60, true);
    assert!(
        expanded
            .join("\n")
            .contains(text::agent::FILE_DIFF_NO_FINAL_NEWLINE),
        "{expanded:#?}"
    );
}

/// Tab 展开为 4 列制表位，控制字符不进入终端。
#[test]
fn tabs_and_control_chars_are_safe() {
    let mut tabs = line(DiffLineKind::Added, None, Some(1), "a\tb\u{1b}[31m");
    tabs.no_final_newline = false;
    let call = tool_call("write", detail_of(vec![(complete(vec![vec![tabs]]), 1, 0)]));
    let (lines, _) = render(&call, 80, false);
    let text = lines.join("\n");
    assert!(!text.contains('\u{1b}'), "不得写入 ESC");
    assert!(!text.contains('\t'), "Tab 必须展开");
    assert!(text.contains("a   b"), "Tab 到下一个 4 列制表位：{text:?}");
}

/// 多文件：每个文件都有头与预览；移动路径显示 `旧 → 新`。
#[test]
fn multiple_files_and_move_path() {
    let mut view = detail_of(vec![(replace_diff(), 1, 1), (complete(Vec::new()), 0, 0)]);
    view.files[1].path = "/tmp/old.rs".into();
    view.files[1].to = Some("/tmp/new.rs".into());
    view.files[1].kind = FileChangeKind::Moved {
        to: "/tmp/new.rs".into(),
    };
    view.files_total = 2;
    let call = tool_call("edit", view);
    let (lines, _) = render(&call, 80, false);
    let text = lines.join("\n");
    assert!(text.contains("/tmp/file0.rs"), "{text}");
    assert!(text.contains("/tmp/old.rs → /tmp/new.rs"), "{text}");
    assert!(text.contains(text::agent::FILE_DIFF_NO_CHANGES), "{text}");
    // 第一个文件是中间节点（`├─`），最后一个收口（`└─`）。
    assert!(text.contains(text::agent::TREE_BRANCH), "{text}");
    assert!(text.contains(text::agent::TREE_LEAF), "{text}");
}

/// 省略文件数：调用级提示单独一行。
#[test]
fn omitted_files_are_reported() {
    let mut view = detail_of(vec![(replace_diff(), 1, 1)]);
    view.omitted_files = 3;
    let call = tool_call("edit", view);
    let (lines, _) = render(&call, 80, false);
    let text = lines.join("\n");
    assert!(
        text.contains(&text::agent::file_diff_omitted_files(3)),
        "{text}"
    );
}

/// 计数未知：显示未知标记，不写 0。
#[test]
fn unknown_counts_are_marked() {
    let mut view = detail_of(vec![(
        FileDiff::Unavailable {
            reason: DiffUnavailableReason::Binary,
        },
        0,
        0,
    )]);
    view.files[0].added = None;
    view.files[0].removed = None;
    view.files[0].counts_known = false;
    let call = tool_call("edit", view);
    let (lines, _) = render(&call, 80, false);
    let text = lines.join("\n");
    assert!(
        text.contains(text::agent::FILE_DIFF_COUNTS_UNKNOWN),
        "{text}"
    );
    assert!(!text.contains("+0"), "未知计数不得写成 0：{text}");
}

/// 没有差异正文（旧记录 / 非文件详情）时 `has_diff` 为假，走原两行摘要。
#[test]
fn no_diff_falls_back_to_summary() {
    let mut view = detail_of(vec![(replace_diff(), 1, 1)]);
    view.files[0].diff = None;
    let call = tool_call("edit", view);
    assert!(!has_diff(&call));
    assert_eq!(rows(&call, 80, false), title_rows() + 1);
    // 没有详情分支时同样不产生行。
    let mut bare = tool_call("edit", detail_of(Vec::new()));
    bare.detail = None;
    assert!(!has_diff(&bare));
}

/// 滚动：滚过卡片首行时，推进的行数不变。
#[test]
fn scroll_does_not_change_advanced_rows() {
    let call = tool_call("edit", detail_of(vec![(replace_diff(), 1, 1)]));
    let total = rows(&call, 80, true);
    let rect = Rect::new(0, 0, 80, 2);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(&mut grid, rect, 1, &mut row_index, &call, 0, true, false);
    assert_eq!(row_index, total);
}

/// `DiffCounts` 与视图计数保持同口径（工具层 → UI 无二次计算）。
#[test]
fn counts_come_from_diff_not_recomputed() {
    let diff = replace_diff();
    let counts = DiffCounts {
        added: 1,
        removed: 1,
        truncated: false,
        simplified: false,
    };
    assert!(diff.counts_known() || counts.truncated);
    let call = tool_call(
        "edit",
        detail_of(vec![(diff, counts.added, counts.removed)]),
    );
    let (lines, _) = render(&call, 80, false);
    assert!(
        lines[1].contains("+1") && lines[1].contains("-1"),
        "{:?}",
        lines[1]
    );
}

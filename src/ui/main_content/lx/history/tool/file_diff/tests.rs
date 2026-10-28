//! 文件差异卡片测试：布局/绘制行数一致、预览 3 行、展开全部、宽度安全、样式与符号。

use ratatui::layout::Rect;

use super::*;
use crate::app::state::{FileDiffDetailView, ToolStatus};
use crate::chat::tools::{DiffCounts, DiffHunk, DiffLine, DiffLineKind, FileDiff};

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

/// 5 条变更行（超过 3 行预览额度）：折叠会省略、展开会多行。
fn long_diff() -> FileDiff {
    complete(vec![
        (1..=5)
            .flat_map(|number| {
                [
                    line(
                        DiffLineKind::Removed,
                        Some(number),
                        None,
                        &format!("old {number}"),
                    ),
                    line(
                        DiffLineKind::Added,
                        None,
                        Some(number),
                        &format!("new {number}"),
                    ),
                ]
            })
            .collect(),
    ])
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
    tool_call_id("call-1", name, view, ToolStatus::Completed)
}

/// 带指定 call id / 状态的调用步骤（合并卡测试用）。
fn tool_call_id(
    call_id: &str,
    name: &str,
    view: FileDiffDetailView,
    status: ToolStatus,
) -> ToolCallView {
    ToolCallView {
        call_id: call_id.into(),
        name: name.into(),
        position: 0,
        arguments_json: String::new(),
        status,
        summary: None,
        error: None,
        result_json: None,
        detail: Some(Box::new(crate::app::state::ToolDetailView::File(view))),
    }
}

/// 渲染单张卡片到网格并返回逐行文本。
fn render(call: &ToolCallView, width: u16, expanded: bool) -> (Vec<String>, usize) {
    render_group(&[call], width, &[expanded])
}

/// 渲染一张（可能是合并的）卡片到网格并返回逐行文本。
///
/// `expanded` 与 `calls` 一一对应：合并卡里每个调用各自的展开状态。
fn render_group(calls: &[&ToolCallView], width: u16, expanded: &[bool]) -> (Vec<String>, usize) {
    let total = rows(calls, width, expanded);
    let height = u16::try_from(total.max(1)).unwrap_or(u16::MAX);
    let rect = Rect::new(0, 0, width, height);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        calls,
        0,
        expanded,
        false,
    );
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
    let total = rows(&[call], width, &[expanded]);
    let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        &[call],
        0,
        &[expanded],
        false,
    );
    grid.cell(x, y).style
}

/// 折叠态在内容**下一行**写剩余行数；展开后这一行消失（用户要求）。
#[test]
fn folded_card_shows_remaining_lines_below_the_content() {
    let call = tool_call("edit", detail_of(vec![(long_diff(), 5, 5)]));
    let lines = render(&call, 80, false).0;
    let joined = lines.join("\n");
    assert!(
        joined.contains("more line"),
        "折叠态下一行显示剩余行数：{joined}"
    );
    assert!(
        !joined.contains(text::agent::FILE_DIFF_COLLAPSE),
        "没有收起按钮文案"
    );
    // 折叠 3 行预览、展开铺开：行数差就是被折起来的行。
    let expanded = render(&call, 80, true).0;
    assert!(!expanded.join("\n").contains("more line"), "展开后不再提示");
    // 展开后多出的是真实内容行（hunks 全铺）。
    assert!(expanded.len() > lines.len());
}

/// 滚动：滚过卡片首行时，推进的行数不变。
#[test]
fn scroll_does_not_change_advanced_rows() {
    let call = tool_call("edit", detail_of(vec![(replace_diff(), 1, 1)]));
    let total = rows(&[&call], 80, &[true]);
    let rect = Rect::new(0, 0, 80, 2);
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        1,
        &mut row_index,
        &[&call],
        0,
        &[true],
        false,
    );
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

// ===== 合并卡：连续同种文件调用 =====

/// 两次 `edit` 调用的两个文件：合并后是**一张卡**，标题给出文件数与合计。
#[test]
fn consecutive_edits_merge_into_one_card() {
    let first = tool_call_id(
        "call-a",
        "edit",
        detail_of(vec![(replace_diff(), 1, 1)]),
        ToolStatus::Completed,
    );
    let second = tool_call_id(
        "call-b",
        "edit",
        detail_of(vec![(replace_diff(), 1, 1)]),
        ToolStatus::Completed,
    );
    let group = [&first, &second];
    let (lines, total) = render_group(&group, 80, &[false, false]);
    let text = lines.join("\n");
    // 标题只出现一次，且写明文件数。
    let titles: Vec<&String> = lines
        .iter()
        .filter(|line| line.contains(text::agent::VERB_TOOL_EDIT))
        .collect();
    assert_eq!(titles.len(), 1, "两次调用只有一张卡：{lines:#?}");
    assert!(
        titles[0].contains(&text::agent::tool_files_label(2)),
        "标题写明文件数：{:?}",
        titles[0]
    );
    // 合计计数：两个文件各 +1 -1。
    assert!(
        titles[0].contains("+2") && titles[0].contains("-2"),
        "{text}"
    );
    // 每个调用各自一块，块首都是 `└─`（合并标题之下不再有 `├─` 串联）。
    assert_eq!(
        text.matches("/tmp/file0.rs").count(),
        2,
        "两个文件同名但各自一行：{text}"
    );
    assert_eq!(
        text.matches(text::agent::TREE_LEAF).count(),
        2,
        "每个调用一块，各自以 `└─` 起头：{text}"
    );
    assert!(!text.contains(text::agent::TREE_BRANCH), "{text}");
    // 每块折叠态各有一行剩余行数提示（在各自内容下方）；没有收起按钮文案。
    assert_eq!(
        text.matches("more line").count(),
        2,
        "每块各一行提示：{text}"
    );
    assert!(
        !text.contains(text::agent::FILE_DIFF_COLLAPSE),
        "无收起文案"
    );
    assert_eq!(total, lines.len());
}

/// 合并卡是**一个**展开目标：展开后所有文件一起展开。
#[test]
fn merged_card_expands_as_a_whole() {
    let first = tool_call_id(
        "call-a",
        "edit",
        detail_of(vec![(long_diff(), 5, 5)]),
        ToolStatus::Completed,
    );
    let second = tool_call_id(
        "call-b",
        "edit",
        detail_of(vec![(long_diff(), 5, 5)]),
        ToolStatus::Completed,
    );
    let group = [&first, &second];
    let (folded, _) = render_group(&group, 80, &[false, false]);
    // 只展开第一个调用：两块互不影响。
    let (expanded, _) = render_group(&group, 80, &[true, false]);
    assert!(expanded.len() > folded.len(), "展开比折叠多行");
    // 折叠态各块给剩余行数提示，展开后铺开 hunks。
    let folded_text = folded.join("\n");
    assert!(
        folded_text.contains("more line"),
        "折叠态给剩余行数：{folded_text}"
    );
    assert!(
        !expanded
            .join("\n")
            .contains(text::agent::FILE_DIFF_COLLAPSE)
    );
    // 被展开的那一块露出更多变更行（另一块仍停在 3 行预览）。
    let expanded_text = expanded.join("\n");
    assert!(
        expanded_text.matches("new 5").count() == 1,
        "展开块露出第 5 条变更，另一块没有：{expanded_text}"
    );
}

/// 合并卡方块汇总状态：只要有一个失败，整卡就是失败色。
#[test]
fn merged_card_mark_follows_the_worst_status() {
    let ok = tool_call_id(
        "call-a",
        "edit",
        detail_of(vec![(replace_diff(), 1, 1)]),
        ToolStatus::Completed,
    );
    let mut bad = tool_call_id(
        "call-b",
        "edit",
        detail_of(vec![(replace_diff(), 1, 1)]),
        ToolStatus::Failed,
    );
    bad.error = Some("boom".into());
    let group = [&ok, &bad];
    let total = rows(&group, 80, &[false, false]);
    let rect = Rect::new(0, 0, 80, u16::try_from(total).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        &group,
        0,
        &[false, false],
        false,
    );
    assert_eq!(
        grid.cell(0, 0).style,
        style::agent_fail(),
        "整卡标题转失败色"
    );
}

/// `edit` 与 `write` 不合并（标题只能写一个动词）。
#[test]
fn different_file_verbs_do_not_merge() {
    let edit = tool_call_id(
        "call-a",
        "edit",
        detail_of(vec![(replace_diff(), 1, 1)]),
        ToolStatus::Completed,
    );
    let write = tool_call_id(
        "call-b",
        "write",
        detail_of(vec![(replace_diff(), 1, 1)]),
        ToolStatus::Completed,
    );
    let steps = [
        crate::app::state::AssistantStepView::Tool {
            position: 1,
            call: edit,
        },
        crate::app::state::AssistantStepView::Tool {
            position: 2,
            call: write,
        },
    ];
    assert_eq!(super::super::merge_group(&steps, 0).len(), 1);
}

// ===== 语法高亮 =====

/// `.rs` 正文做关键字分色：`let` 不是增删语义色。
#[test]
fn rust_body_is_syntax_highlighted() {
    let diff = complete(vec![vec![line(
        DiffLineKind::Added,
        None,
        Some(1),
        "let value = 1;",
    )]]);
    let call = tool_call("write", detail_of(vec![(diff, 1, 0)]));
    let (lines, _) = render(&call, 80, false);
    let row = u16::try_from(
        lines
            .iter()
            .position(|line| line.contains("let value"))
            .expect("新增行"),
    )
    .expect("行号");
    let keyword_x =
        u16::try_from(lines[usize::from(row)].find("let").expect("关键字")).expect("列");
    assert_eq!(
        style_at(&call, 80, false, keyword_x, row),
        style::code_keyword().patch(style::agent_added_bg()),
        "关键字保持语法色，但行底色仍是新增绿"
    );
    // 数字也是语法色（不是增删色）。
    // 取正文里的 `1`（行号槽里的数字是弱化元信息，不是语法高亮）。
    let number_x = u16::try_from(lines[usize::from(row)].rfind('1').expect("数字")).expect("列");
    assert_eq!(
        style_at(&call, 80, false, number_x, row),
        style::code_number().patch(style::agent_added_bg())
    );
}

/// `.md` 正文走 markdown 词法：标题与行内代码分色。
#[test]
fn markdown_body_uses_markdown_highlighting() {
    let diff = complete(vec![vec![
        line(DiffLineKind::Added, None, Some(1), "# 标题"),
        line(DiffLineKind::Added, None, Some(2), "普通 `code` 文本"),
    ]]);
    let mut call = tool_call("edit", detail_of(vec![(diff, 2, 0)]));
    if let Some(crate::app::state::ToolDetailView::File(view)) = call.detail.as_deref_mut() {
        view.files[0].path = "docs/design.md".into();
    }
    let (lines, _) = render(&call, 80, false);
    let head_row = u16::try_from(
        lines
            .iter()
            .position(|line| line.contains("标题"))
            .expect("标题行"),
    )
    .expect("行号");
    let head_x =
        u16::try_from(lines[usize::from(head_row)].find('标').expect("标题文字")).expect("列");
    assert_eq!(
        style_at(&call, 80, false, head_x, head_row),
        style::markdown_heading().patch(style::agent_added_bg()),
        "markdown 标题保留标题色"
    );
    let code_row = u16::try_from(
        lines
            .iter()
            .position(|line| line.contains("普通"))
            .expect("正文行"),
    )
    .expect("行号");
    let code_x =
        u16::try_from(lines[usize::from(code_row)].find("code").expect("行内代码")).expect("列");
    assert_eq!(
        style_at(&call, 80, false, code_x, code_row),
        style::markdown_inline_code().patch(style::agent_added_bg())
    );
}

/// 未知扩展名不做假高亮：正文保持增删语义色。
#[test]
fn unknown_extension_keeps_plain_diff_colors() {
    let diff = complete(vec![vec![line(
        DiffLineKind::Removed,
        Some(1),
        None,
        "let value = 1;",
    )]]);
    let mut call = tool_call("edit", detail_of(vec![(diff, 0, 1)]));
    if let Some(crate::app::state::ToolDetailView::File(view)) = call.detail.as_deref_mut() {
        view.files[0].path = "notes.unknownext".into();
    }
    let (lines, _) = render(&call, 80, false);
    let row = u16::try_from(
        lines
            .iter()
            .position(|line| line.contains("let value"))
            .expect("删除行"),
    )
    .expect("行号");
    let keyword_x = u16::try_from(lines[usize::from(row)].find("let").expect("文字")).expect("列");
    assert_eq!(
        style_at(&call, 80, false, keyword_x, row),
        style::agent_removed().patch(style::agent_removed_bg()),
        "不认识的语言不得把 let 染成关键字色"
    );
}

/// 合并卡的每一块都有自己的展开入口：点第一个不影响第二个（展开互相独立）。
#[test]
fn merged_blocks_report_independent_action_rows() {
    let first = tool_call_id(
        "call-a",
        "edit",
        detail_of(vec![(long_diff(), 5, 5)]),
        ToolStatus::Completed,
    );
    let second = tool_call_id(
        "call-b",
        "edit",
        detail_of(vec![(long_diff(), 5, 5)]),
        ToolStatus::Completed,
    );
    let group = [&first, &second];
    let folded = blocks(&group, 80, &[false, false]);
    assert_eq!(folded.len(), 2, "两个调用各一块");
    assert_eq!(folded[0].start, 1, "首块紧跟合并标题");
    assert_eq!(
        folded[1].start,
        folded[0].start + folded[0].len + CALL_GAP_ROWS,
        "块间空一行分隔"
    );
    assert_eq!(
        rows(&group, 80, &[false, false]),
        folded[1].start + folded[1].len,
        "卡片总行数覆盖块间空行"
    );
    // 只展开第一块：它的行变多；第二块行数与起点跟着后移（展开互不影响）。
    let expanded = blocks(&group, 80, &[true, false]);
    assert!(expanded[0].len > folded[0].len, "第一块展开多行");
    assert_eq!(expanded[1].len, folded[1].len, "第二块行数不变");
    assert_eq!(
        expanded[1].start,
        expanded[0].start + expanded[0].len + CALL_GAP_ROWS,
        "第二块起点随第一块后移"
    );
}
/// 同一工具的连续调用之间空一行：上一块末行与下一块文件头之间是空行。
#[test]
fn consecutive_calls_are_separated_by_a_blank_line() {
    let first = tool_call_id(
        "call-a",
        "edit",
        detail_of(vec![(replace_diff(), 1, 1)]),
        ToolStatus::Completed,
    );
    let second = tool_call_id(
        "call-b",
        "edit",
        detail_of(vec![(replace_diff(), 1, 1)]),
        ToolStatus::Completed,
    );
    let group = [&first, &second];
    let (lines, _) = render_group(&group, 80, &[false, false]);
    let folded = blocks(&group, 80, &[false, false]);
    let start = folded[1].start;
    assert_eq!(
        start,
        folded[0].start + folded[0].len + CALL_GAP_ROWS,
        "块间空一行"
    );
    assert!(
        lines[start - 1].trim().is_empty(),
        "块间那一行必须是空行：{:?}",
        lines
    );
    assert!(lines[start].contains("file0"), "空行之后紧跟下一块的文件头");
}

/// 展开块：**正文行铺灰底、文件头不铺**，diff 增删行保留自己的红/绿（优先级更高）。
#[test]
fn expanded_body_is_grey_except_the_file_header_and_diff_lines() {
    // 带上下文行（`keep`）的 diff：上下文行也必须铺灰底。
    let call = tool_call("write", detail_of(vec![(replace_diff(), 1, 1)]));
    let width = 60u16;
    let (_, total) = render(&call, width, true);
    let rect = Rect::new(0, 0, width, u16::try_from(total).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        &[&call],
        0,
        &[true],
        false,
    );
    let grey = style::chat_expanded_bg().bg;
    let added = style::agent_added_bg().bg;
    let removed = style::agent_removed_bg().bg;
    // 行 0 = 标题，行 1 = 文件头，行 2.. 才是正文。
    for x in 0..width {
        assert_eq!(grid.cell(x, 1).style.bg, None, "({x},1) 文件头不铺底色");
    }
    let mut greys = 0;
    let mut diffs = 0;
    for y in 2..rect.height {
        let bg = grid.cell(0, y).style.bg;
        for x in 0..width {
            assert_eq!(
                grid.cell(x, y).style.bg,
                bg,
                "({x},{y}) 整行必须同一底色（满宽）"
            );
        }
        match bg {
            Some(color) if Some(color) == grey => greys += 1,
            Some(color) if Some(color) == added || Some(color) == removed => diffs += 1,
            other => panic!("({y}) 意外的底色：{other:?}"),
        }
    }
    assert!(greys > 0, "上下文行铺展开灰底");
    assert!(diffs > 0, "增删行保留自己的底色");
}

/// diff 卡的折叠内容**不随鼠标悬停提亮**：同一次绘制里路径、正文与提示行都恒色。
///
/// 悬停面已经取消（thinking 块也不再因 hover 变色），所以这里断言折叠态的三处样式
/// 都是固定档位，且「剩余行数」提示行恒为 dim。
#[test]
fn file_diff_folded_content_never_lifts() {
    let call = tool_call("write", detail_of(vec![(long_diff(), 5, 5)]));
    let width = 60u16;
    let folded = rows(&[&call], width, &[false]);
    let content_rows = layout(&call, width, false).len();
    assert_eq!(folded, 1 + content_rows, "块 = 标题 + 折叠内容");
    let rect = Rect::new(
        0,
        0,
        width,
        u16::try_from(folded.max(1)).unwrap_or(u16::MAX),
    );
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        &[&call],
        0,
        &[false],
        false,
    );
    let hint_y = u16::try_from(folded.saturating_sub(1)).unwrap_or(u16::MAX);
    assert_eq!(
        grid.cell(6, 1).style,
        style::chat_tool_param(),
        "文件头路径恒为浅黄"
    );
    assert_eq!(
        grid.cell(6, hint_y).style,
        style::agent_step_detail(),
        "剩余行数提示恒为 dim"
    );
    assert_ne!(
        grid.cell(6, hint_y).style,
        style::text(),
        "提示行不得升为默认前景"
    );
    assert!(has_more_content(&call, width), "内容确实可以展开");
}

/// 文件头路径**恒为 dim**：折叠与展开都不提亮（它是块的标题，不是折叠内容）。
///
/// 用户明确要求：展开参数保持高亮只适用于普通工具的 `└─` 参数行与命令正文，
/// diff 的文件头路径不属于「被折叠起来的参数」。
#[test]
fn file_header_path_stays_dim_when_expanded() {
    let call = tool_call("write", detail_of(vec![(long_diff(), 5, 5)]));
    let width = 60u16;
    let header_path_style = |expanded: bool| {
        let total = rows(&[&call], width, &[expanded]);
        let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
        let mut grid = Grid::new(rect);
        let mut row_index = 0usize;
        draw(
            &mut grid,
            rect,
            0,
            &mut row_index,
            &[&call],
            0,
            &[expanded],
            false,
        );
        // 行 0 = 标题，行 1 = 文件头；路径从连接符（5 列）之后开始。
        grid.cell(6, 1).style
    };
    assert_eq!(
        header_path_style(false),
        style::chat_tool_param(),
        "折叠态路径是注脚级 dim"
    );
    assert_eq!(
        header_path_style(true),
        style::chat_tool_param(),
        "展开态路径仍为 dim（不随展开提亮）"
    );
}

/// 合并卡里各块的路径互不牵连：展开一块不会让另一块的路径变亮。
#[test]
fn merging_blocks_keeps_every_header_dim() {
    let first = tool_call_id(
        "call-a",
        "edit",
        detail_of(vec![(long_diff(), 5, 5)]),
        ToolStatus::Completed,
    );
    let second = tool_call_id(
        "call-b",
        "edit",
        detail_of(vec![(long_diff(), 5, 5)]),
        ToolStatus::Completed,
    );
    let group = [&first, &second];
    let expanded = [true, false];
    let width = 60u16;
    let folded_blocks = blocks(&group, width, &expanded);
    let total = rows(&group, width, &expanded);
    let rect = Rect::new(0, 0, width, u16::try_from(total.max(1)).unwrap_or(u16::MAX));
    let mut grid = Grid::new(rect);
    let mut row_index = 0usize;
    draw(
        &mut grid,
        rect,
        0,
        &mut row_index,
        &group,
        0,
        &expanded,
        false,
    );
    for block in &folded_blocks {
        let y = u16::try_from(block.start).unwrap_or(u16::MAX);
        assert_eq!(
            grid.cell(6, y).style,
            style::chat_tool_param(),
            "块 {y} 的文件头路径必须保持 dim"
        );
    }
}

//! 汇报块：子 agent 结束后在主消息流里显示的**工具卡片**。
//!
//! ```text
//! ▲ Subagent report · completed
//!   ├─ explorer · compact 的触发点
//!   └─ Report:
//!      report body preview line 1
//!      report body preview line 2
//!      … 8 more lines
//! ```
//!
//! 数据来自 `ChatMessageView.reports`；同一条内部 user 消息里的所有汇报各成一块。
//! 折叠预览固定 2 行灰色正文，展开用默认前景；点击整块切换展开状态，槽位
//! `report:<agent_id>`。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::SubagentReportView;
use crate::ui::markdown_render;
use crate::ui::{style, text};

use super::super::Grid;
use super::{StepKind, StepMark, visible_row};
use crate::ui::main_content::lx::history::tool::{Segment, put_segments, segments_cols};

/// 汇报块**每次展开**追加的正文行数；展开上限按固定批次递增，
/// `… N more lines` 点击一次只追加一批（避免一次把整段报告铺满历史）。
const REPORT_EXPAND_ROWS: usize = 24;
/// 折叠态正文预览行数。
const REPORT_PREVIEW_ROWS: usize = 2;
/// 树体正文缩进（与 `└─` 前缀等宽）。
const BODY_INDENT: &str = "     ";

/// 汇报状态 → 标题方块。
fn report_mark(status: &str) -> StepMark {
    match status {
        "failed" => StepMark::Failed,
        "cancelled" => StepMark::Cancelled,
        _ => StepMark::Done,
    }
}

/// 标题动词颜色：失败红、取消黄，其余工具绿。
fn verb_style(status: &str) -> ratatui::style::Style {
    match status {
        "failed" => style::agent_fail(),
        "cancelled" => style::agent_warn(),
        _ => style::chat_tool_verb(),
    }
}

/// `agent名 · title`（不显示 task 提示词）。
fn agent_line(report: &SubagentReportView) -> String {
    let role = report.role.as_str();
    if report.title.is_empty() {
        role.to_string()
    } else {
        format!("{role}{}{}", text::agent::SPAWN_ROLE_SEP, report.title)
    }
}

/// 极窄宽度下让出树体缩进，保证 `body_lines` 与 `draw` 的可用列宽恒一致。
fn body_indent(width: u16) -> u16 {
    let indent = u16::try_from(BODY_INDENT.len()).unwrap_or(5);
    if width > indent { indent } else { 0 }
}

/// 正文的物理行（markdown 已按宽度折行）。
fn body_lines(report: &SubagentReportView, width: u16) -> Vec<String> {
    let body_width = width.saturating_sub(body_indent(width)).max(1);
    markdown_render::render_wrapped(&report.body, body_width)
        .into_iter()
        .map(|line| {
            let mut text = " ".repeat(usize::from(line.indent));
            for (value, _) in line.spans {
                text.push_str(&value);
            }
            text
        })
        .collect()
}

/// 当前折叠态/展开态实际显示的正文行与剩余行数。
struct BodyView {
    lines: Vec<String>,
    remaining: usize,
}

fn body_view(
    report: &SubagentReportView,
    width: u16,
    expanded: bool,
    more_steps: usize,
) -> BodyView {
    let all = body_lines(report, width);
    let cap = if expanded {
        REPORT_EXPAND_ROWS.saturating_mul(more_steps.saturating_add(1))
    } else {
        REPORT_PREVIEW_ROWS
    };
    let shown = all.len().min(cap);
    BodyView {
        remaining: all.len().saturating_sub(shown),
        lines: all.into_iter().take(shown).collect(),
    }
}

/// 汇报块一次排版的结果：行数、提示行偏移与是否还能展开。
///
/// 快照构建原先分别调用 `rows` / `hint_row` / `has_more`，同一份 markdown 会被
/// 排版三次；这里一次算完，避免汇报较多的会话每次增量都重复全量折行。
pub(super) struct Measure {
    pub(super) rows: usize,
    pub(super) hint_row: Option<usize>,
    pub(super) has_more: bool,
}

/// 汇报块的行数（宽度 0 时为 0）。
#[cfg(test)]
pub(super) fn rows(
    report: &SubagentReportView,
    width: u16,
    expanded: bool,
    more_steps: usize,
) -> usize {
    measure(report, width, expanded, more_steps).rows
}

/// `… N more lines` 在块内的相对行号；没有剩余或折叠态返回 None。
/// 供快照把这个提示行单独登记成「再展开一批」的点击面。
#[cfg(test)]
pub(super) fn hint_row(
    report: &SubagentReportView,
    width: u16,
    expanded: bool,
    more_steps: usize,
) -> Option<usize> {
    measure(report, width, expanded, more_steps).hint_row
}

pub(super) fn measure(
    report: &SubagentReportView,
    width: u16,
    expanded: bool,
    more_steps: usize,
) -> Measure {
    if width == 0 {
        return Measure {
            rows: 0,
            hint_row: None,
            has_more: false,
        };
    }
    let all = body_lines(report, width);
    let cap = if expanded {
        REPORT_EXPAND_ROWS.saturating_mul(more_steps.saturating_add(1))
    } else {
        REPORT_PREVIEW_ROWS
    };
    let shown = all.len().min(cap);
    let remaining = all.len().saturating_sub(shown);
    let body_label = usize::from(!all.is_empty());
    let hint_row = (expanded && remaining > 0).then_some(2 + body_label + shown);
    Measure {
        rows: 2 + body_label + shown + usize::from(remaining > 0),
        hint_row,
        has_more: all.len() > REPORT_PREVIEW_ROWS,
    }
}

/// 绘制汇报块；推进行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    report: &SubagentReportView,
    scroll: usize,
    row: &mut usize,
    _loading_frame: usize,
    _animating: bool,
    expanded: bool,
    more_steps: usize,
) {
    if rect.width == 0 {
        return;
    }
    let mark = report_mark(&report.status);
    let mut header = vec![
        Segment::new(
            mark.segment(StepKind::Report, 0, false),
            mark.style(StepKind::Report, false),
        ),
        Segment::new(
            format!("{} ", text::agent::VERB_TOOL_SUBAGENT_REPORT),
            verb_style(&report.status),
        ),
    ];
    let status = format!("{}{}", text::LX_CHAT_TOOL_STATUS_SEP, report.status);
    let status_segment = Segment::new(status, style::agent_step_detail());
    if segments_cols(&header) + status_segment.cols() <= usize::from(rect.width) {
        header.push(status_segment);
    }
    if let Some(y) = visible_row(rect, scroll, *row) {
        put_segments(grid, rect.x, y, rect.width, &header);
    }
    *row += 1;

    // `├─ agent名 · title`：后面还有 Report 正文时用分支，否则收口。
    let body = body_view(report, rect.width, expanded, more_steps);
    let line = agent_line(report);
    let connector = if body.lines.is_empty() {
        text::agent::TREE_LEAF
    } else {
        text::agent::TREE_BRANCH
    };
    let mut detail = vec![
        Segment::new(connector, style::chat_notice_connector()),
        Segment::new(line.clone(), style::text()),
    ];
    let room = usize::from(rect.width).saturating_sub(connector.width());
    if line.width() > room {
        detail[1] = Segment::new(
            crate::ui::text::truncate_to_width(&line, room),
            style::text(),
        );
    }
    if let Some(y) = visible_row(rect, scroll, *row) {
        put_segments(grid, rect.x, y, rect.width, &detail);
    }
    *row += 1;

    let body_style = if expanded {
        style::text()
    } else {
        style::agent_step_detail()
    };
    if !body.lines.is_empty() {
        let label = vec![
            Segment::new(
                text::agent::TREE_LEAF.to_string(),
                style::chat_notice_connector(),
            ),
            Segment::new(text::agent::REPORT_LABEL, body_style),
        ];
        if let Some(y) = visible_row(rect, scroll, *row) {
            put_segments(grid, rect.x, y, rect.width, &label);
        }
        *row += 1;
    }

    let indent = body_indent(rect.width);
    let body_x = rect.x.saturating_add(indent);
    let body_width = rect.width.saturating_sub(indent);
    for line in &body.lines {
        if let Some(y) = visible_row(rect, scroll, *row) {
            grid.put_clipped(body_x, y, line, body_style, body_width);
        }
        *row += 1;
    }
    if body.remaining > 0 {
        if let Some(y) = visible_row(rect, scroll, *row) {
            let hint = text::agent::file_diff_more_lines(body.remaining);
            grid.put_clipped(body_x, y, &hint, style::agent_step_detail(), body_width);
        }
        *row += 1;
    }
}

/// 正文是否还能展开（超过 2 行预览）。
#[cfg(test)]
pub(super) fn has_more(report: &SubagentReportView, width: u16) -> bool {
    measure(report, width, false, 0).has_more
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(body: &str) -> SubagentReportView {
        SubagentReportView {
            agent_id: "sub-1".into(),
            title: "test title".into(),
            role: crate::chat::agent::AgentRole::Explorer,
            status: "completed".into(),
            files_changed: 2,
            duration_ms: Some(14_200),
            body: body.into(),
            delivered_at_ms: 1,
        }
    }

    #[test]
    fn zero_width_has_no_rows() {
        assert_eq!(rows(&report("body"), 0, false, 0), 0);
    }

    #[test]
    fn collapsed_preview_is_two_lines_and_shows_remainder() {
        let report = report("line one\nline two\nline three\nline four");
        assert_eq!(
            rows(&report, 60, false, 0),
            6,
            "标题 + 链接 + Report + 2 行 + 剩余行"
        );
        assert!(rows(&report, 60, true, 0) > 6);
        assert!(has_more(&report, 60));
    }

    #[test]
    fn expanded_report_expands_in_fixed_batches() {
        let body = (0..60)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let report = report(&body);
        assert_eq!(
            hint_row(&report, 60, false, 0),
            None,
            "折叠态没有更多点击面"
        );
        assert_eq!(hint_row(&report, 60, true, 0), Some(27), "首批 24 行");
        assert_eq!(rows(&report, 60, true, 0), 28);
        assert_eq!(hint_row(&report, 60, true, 1), Some(51), "第二批再 24 行");
        assert_eq!(rows(&report, 60, true, 1), 52);
        assert_eq!(hint_row(&report, 60, true, 2), None, "全部展开后提示消失");
        assert_eq!(rows(&report, 60, true, 2), 63);
    }

    #[test]
    fn short_body_has_no_remainder_hint() {
        let report = report("line one");
        assert_eq!(rows(&report, 60, false, 0), 4);
        assert!(!has_more(&report, 60));
    }

    #[test]
    fn empty_body_is_title_and_link_only() {
        assert_eq!(rows(&report(""), 60, false, 0), 2);
    }

    #[test]
    fn summary_uses_the_subagent_title() {
        let line = agent_line(&report("body"));
        assert!(
            line.contains("test title"),
            "汇报头必须带自己的标题：{line}"
        );
        assert!(!line.contains("任务提示词"), "{line}");
    }

    #[test]
    fn renders_as_a_report_card_with_triangle_title() {
        let report = report("line one\nline two\nline three");
        let area = Rect::new(0, 0, 60, 20);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(&mut grid, area, &report, 0, &mut row, 0, false, false, 0);
        let text: String = (0..60).map(|x| grid.cell(x, 0).ch).collect();
        assert!(
            text.contains(text::agent::VERB_TOOL_SUBAGENT_REPORT),
            "{text}"
        );
        assert!(
            text.contains(text::agent::STEP_MARK_TRIANGLE_FILLED),
            "{text}"
        );
        assert!(
            !text.contains(text::agent::STEP_MARK_FILLED),
            "汇报块不再使用工具方块：{text}"
        );
        let detail: String = (0..60).map(|x| grid.cell(x, 1).ch).collect();
        assert!(detail.contains("explorer · test title"), "{detail}");
        // `Report:` 标签颜色与正文内容保持一致（折叠态灰、展开态默认前景），不加额外参数色。
        assert_eq!(grid.cell(5, 2).style, grid.cell(5, 3).style);
        assert_eq!(grid.cell(5, 2).style, style::agent_step_detail());
        let mut expanded_grid = Grid::new(area);
        let mut expanded_row = 0;
        draw(
            &mut expanded_grid,
            area,
            &report,
            0,
            &mut expanded_row,
            0,
            false,
            true,
            0,
        );
        assert_eq!(
            expanded_grid.cell(5, 2).style,
            expanded_grid.cell(5, 3).style
        );
        assert_eq!(expanded_grid.cell(5, 2).style, style::text());
        // 极窄宽度（<= 5 列）让出缩进，正文仍写进网格且行数与 measure 一致。
        let narrow_area = Rect::new(0, 0, 4, 20);
        let mut narrow_grid = Grid::new(narrow_area);
        let mut narrow_row = 0;
        draw(
            &mut narrow_grid,
            narrow_area,
            &report,
            0,
            &mut narrow_row,
            0,
            false,
            false,
            0,
        );
        assert_eq!(narrow_row, rows(&report, 4, false, 0));
        let narrow_body: String = (0..4).map(|x| narrow_grid.cell(x, 3).ch).collect();
        assert!(
            !narrow_body.trim().is_empty(),
            "极窄宽度正文可见：{narrow_body:?}"
        );
    }
}

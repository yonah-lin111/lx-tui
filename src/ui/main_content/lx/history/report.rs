//! 汇报块：子 agent 结束后在主消息流里显示的**文字块**（圆点 `●`，不是工具卡片）。
//!
//! 数据来自 `ChatMessageView.reports`；同一条内部 user 消息里的所有汇报各成一块，
//! 块间空行由 `snapshot` 累加。折叠恒 2 行：
//!
//! ```text
//! ● explorer · completed · 2 files changed · 14.2s
//!   └─ compact 的触发点在 src/chat/context/mod.rs:167 …
//! ```
//!
//! 展开用 markdown 渲染正文（与 assistant 正文同一套排版）。展开状态复用
//! `LxChat::detail_expanded`，槽位 `report:<agent_id>`。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::state::SubagentReportView;
use crate::ui::markdown_render::{self, MdLine};
use crate::ui::{style, text};

use super::super::Grid;
use super::{STEP_GUTTER, StepKind, StepMark, visible_row, wrap};

/// 汇报块展开后的正文行上限（避免超长报告撑爆滚动几何）。
const REPORT_MAX_ROWS: usize = 24;
/// 折叠态正文预览行数。
const REPORT_PREVIEW_ROWS: usize = 1;

/// 汇总行：`role · status · N files changed · duration`。
fn summary_line(report: &SubagentReportView) -> String {
    let mut parts = vec![report.role.as_str().to_string(), report.status.clone()];
    if report.files_changed > 0 {
        parts.push(format!(
            "{} {}",
            report.files_changed,
            text::agent::REPORT_FILES_CHANGED
        ));
    }
    if let Some(duration) = report.duration_ms {
        parts.push(super::format_duration(duration));
    }
    parts.join(text::LX_CHAT_TOOL_STATUS_SEP)
}

/// 正文行（markdown 或纯文本回退）。
fn body_lines(report: &SubagentReportView, width: u16) -> Vec<MdLine> {
    markdown_render::render_wrapped(&report.body, width.saturating_sub(STEP_GUTTER))
}

/// 内容正文的物理行数上限内的行。
fn visible_body(report: &SubagentReportView, width: u16, expanded: bool) -> Vec<MdLine> {
    let mut lines = body_lines(report, width);
    let cap = if expanded {
        REPORT_MAX_ROWS
    } else {
        REPORT_PREVIEW_ROWS
    };
    lines.truncate(cap);
    lines
}

/// 汇报块的行数（宽度 0 时为 0）。
pub(super) fn rows(report: &SubagentReportView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    // 汇总行（可能换行）+ 正文行。
    let summary = wrap(&summary_line(report), width.saturating_sub(STEP_GUTTER));
    let body = visible_body(report, width, expanded);
    1 + summary.len().saturating_sub(1) + body.len().max(1)
}

/// 绘制汇报块；推进行数必须与 [`rows`] 严格相等。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    report: &SubagentReportView,
    scroll: usize,
    row: &mut usize,
    loading_frame: usize,
    animating: bool,
    expanded: bool,
) {
    if rect.width == 0 {
        return;
    }
    let mark = match report.status.as_str() {
        "failed" => StepMark::Failed,
        "cancelled" => StepMark::Cancelled,
        _ => StepMark::Done,
    };
    let summary = wrap(
        &summary_line(report),
        rect.width.saturating_sub(STEP_GUTTER),
    );
    // 汇总行：首行带圆点，续行缩进。
    for (index, line) in summary.iter().enumerate() {
        if let Some(y) = visible_row(rect, scroll, *row) {
            if index == 0 {
                grid.put_char(
                    rect.x,
                    y,
                    mark.glyph(StepKind::Body, loading_frame, animating),
                    mark.style(StepKind::Body, false, false),
                );
            }
            grid.put_clipped(
                rect.x.saturating_add(STEP_GUTTER),
                y,
                line,
                style::text(),
                rect.width.saturating_sub(STEP_GUTTER),
            );
        }
        *row += 1;
    }
    // 正文：`└─` 前缀 + 预览/展开内容。
    let body = visible_body(report, rect.width, expanded);
    let prefix = format!("{} ", text::agent::MARKER_TREE_LEAF);
    let prefix_cols = prefix
        .chars()
        .map(|ch| ch.width().unwrap_or(1))
        .sum::<usize>();
    for (index, line) in body.iter().enumerate() {
        if let Some(y) = visible_row(rect, scroll, *row) {
            if index == 0 {
                grid.put(
                    rect.x,
                    y,
                    &format!("  {prefix}"),
                    style::agent_step_detail(),
                );
            }
            let mut cx = rect
                .x
                .saturating_add(STEP_GUTTER)
                .saturating_add(u16::try_from(prefix_cols).unwrap_or(2))
                .saturating_add(line.indent);
            for (value, span_style) in &line.spans {
                for ch in value.chars() {
                    let cell = ch.width().unwrap_or(1).max(1) as u16;
                    if cx.saturating_add(cell) > rect.right() {
                        break;
                    }
                    grid.put_char(cx, y, ch, *span_style);
                    cx = cx.saturating_add(cell);
                }
            }
        }
        *row += 1;
    }
    if body.is_empty() {
        *row += 1;
    }
}

/// 正文是否还能展开（展开前后行数不同）。
pub(super) fn has_more(report: &SubagentReportView, width: u16) -> bool {
    rows(report, width, true) > rows(report, width, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(body: &str) -> SubagentReportView {
        SubagentReportView {
            agent_id: "sub-1".into(),
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
        assert_eq!(rows(&report("body"), 0, false), 0);
    }

    #[test]
    fn collapsed_is_two_lines_and_expand_adds_body() {
        let report = report("line one\nline two\nline three");
        assert_eq!(rows(&report, 80, false), 2);
        assert!(rows(&report, 80, true) > 2);
        assert!(has_more(&report, 80));
    }

    #[test]
    fn empty_body_still_renders_two_rows() {
        let report = report("");
        assert_eq!(rows(&report, 80, false), 2);
    }

    #[test]
    fn narrow_width_wraps_the_summary() {
        let report = report("body");
        assert!(rows(&report, 8, false) > 2, "汇总行在窄宽度折行");
    }
}

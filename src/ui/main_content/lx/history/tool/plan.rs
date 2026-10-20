//! `update_plan` 卡片：三态勾选列表 + 完成计数。
//!
//! 版式（对应 design §10.4）：
//!
//! ```text
//! ■ Plan                                 1/3 completed
//!   │  [x] Implement command tools
//!   │  [>] Verify process cleanup
//!   │  [ ] Review terminal display
//!   └─ Implementation finished; checking behavior.
//! ```
//!
//! - 与命令卡片共用同一套树形语法：步骤是中间行（`  │  `），**最后一行收口**
//!   （`  └─ `）——有解释时收在解释行，没有解释时收在最后一条步骤；
//! - 三种状态都用 ASCII 标记（`[ ]` / `[>]` / `[x]`），不只靠颜色；
//! - 计数只认 `completed`，`in_progress` 不算完成；
//! - 长步骤按宽度换行（用显示列，中文/emoji 不按字节切）；
//! - 空计划显示 `Plan cleared`；
//! - 不提供用户点击改状态的功能：这里是模型列出的待办，不是可勾选的用户任务。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{PlanSnapshotView, PlanStepView, ToolCallView};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::TREE_COLS;
use super::{
    Segment, header_with_verb, put_segments, single_line, title_rows, truncate_with_ellipsis,
};

/// 计划解释文字的单行取样上限。
const EXPLANATION_SAMPLE_CHARS: usize = 512;

/// 计划卡片占用的渲染行数（标题 1 + 步骤换行行数 + 解释 1）。
#[allow(clippy::too_many_arguments)]
pub(super) fn rows(call: &ToolCallView, width: u16, _expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    let Some(crate::app::state::ToolDetailView::Plan(view)) = call.detail.as_deref() else {
        return title_rows();
    };
    title_rows() + step_rows(view, width) + usize::from(explanation_of(view).is_some())
}

/// 全部步骤（含换行与占位）占用的行数。
///
/// 与 [`draw`] 共用同一份换行口径：可用列 = 宽度 − 树形前缀 − 状态标记前缀（`[x] `）。
fn step_rows(view: &PlanSnapshotView, width: u16) -> usize {
    if view.is_empty() {
        return 1;
    }
    let available = usize::from(width.saturating_sub(TREE_COLS as u16));
    let text_limit = available.saturating_sub(MARKER_COLS).max(1);
    view.steps
        .iter()
        .map(|(step, _)| wrap(step, text_limit).len())
        .sum()
}

/// 步骤标记前缀的显示列数（`[x] ` 四个 ASCII 字符）。
const MARKER_COLS: usize = 4;
/// 步骤标记前缀宽度（`u16`，与绘制共用）。
const MARKER_COLS_U16: u16 = MARKER_COLS as u16;

/// 绘制计划卡片；推进行数必须与 [`rows`] 严格相等。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
) {
    let width = rect.width;
    if width == 0 {
        return;
    }
    let Some(crate::app::state::ToolDetailView::Plan(view)) = call.detail.as_deref() else {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            let segments = header_with_verb(
                call,
                width,
                loading_frame,
                text::agent::VERB_UPDATE_PLAN,
                None,
            );
            put_segments(grid, rect.x, y, width, &segments);
        }
        *row_index += 1;
        return;
    };
    // 标题行：`■ Plan` + 右对齐完成计数。
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let progress = if view.is_empty() {
            None
        } else {
            Some(text::agent::plan_progress(
                view.completed(),
                view.steps.len(),
            ))
        };
        let segments = header_with_verb(
            call,
            width,
            loading_frame,
            text::agent::VERB_UPDATE_PLAN,
            progress,
        );
        put_segments(grid, rect.x, y, width, &segments);
    }
    *row_index += 1;
    let explanation = explanation_of(view);
    let available = usize::from(width.saturating_sub(TREE_COLS as u16));
    if view.is_empty() {
        // 空计划：`Plan cleared` 是唯一一行，故用收口连接符。
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(
                rect.x,
                y,
                text::agent::TREE_LEAF,
                style::chat_notice_connector(),
                width,
            );
            grid.put_clipped(
                rect.x.saturating_add(TREE_COLS as u16),
                y,
                text::agent::PLAN_EMPTY,
                style::agent_step_detail(),
                width.saturating_sub(TREE_COLS as u16),
            );
        }
        *row_index += 1;
        return;
    }
    let last_step = view.steps.len().saturating_sub(1);
    for (index, (step, status)) in view.steps.iter().enumerate() {
        let marker = text::agent::plan_marker(*status);
        let step_style = match status {
            PlanStepView::Completed => style::agent_step_detail(),
            PlanStepView::InProgress => style::chat_tool_verb(),
            PlanStepView::Pending => style::text(),
        };
        // 步骤是中间行；没有解释行时最后一条步骤自己收口。
        let connector = if index == last_step && explanation.is_none() {
            text::agent::TREE_LEAF
        } else {
            text::agent::TREE_BAR
        };
        let text_limit = available.saturating_sub(MARKER_COLS).max(1);
        let prefix_x = rect.x.saturating_add(TREE_COLS as u16);
        let text_x = prefix_x.saturating_add(MARKER_COLS_U16);
        for (line_index, line) in wrap(step, text_limit).iter().enumerate() {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                if line_index == 0 {
                    grid.put_clipped(rect.x, y, connector, style::chat_notice_connector(), width);
                    grid.put_clipped(
                        prefix_x,
                        y,
                        &format!("{marker} "),
                        style::agent_step_detail(),
                        width.saturating_sub(TREE_COLS as u16),
                    );
                }
                grid.put_clipped(
                    text_x,
                    y,
                    line,
                    step_style,
                    rect.right().saturating_sub(text_x),
                );
            }
            *row_index += 1;
        }
    }
    if let Some(explanation) = explanation
        && let Some(y) = visible_row(rect, scroll, *row_index)
    {
        let segments = explanation_line(&explanation, width);
        put_segments(grid, rect.x, y, width, &segments);
        *row_index += 1;
    }
}

/// 解释行：`  └─ <explanation>`（单行，超出尾部省略）。
fn explanation_line(explanation: &str, width: u16) -> Vec<Segment> {
    let limit = usize::from(width);
    let prefix = text::agent::TREE_LEAF;
    let prefix_cols = prefix.width();
    if prefix_cols >= limit {
        return vec![Segment::new(
            truncate_with_ellipsis(explanation, limit),
            style::agent_step_detail(),
        )];
    }
    let available = limit.saturating_sub(prefix_cols);
    let text = truncate_with_ellipsis(explanation, available);
    let mut segments = vec![Segment::new(prefix, style::chat_notice_connector())];
    if !text.is_empty() {
        segments.push(Segment::new(text, style::agent_step_detail()));
    }
    segments
}

/// 解释文本（单行化、限长）；空解释返回 None。
fn explanation_of(view: &PlanSnapshotView) -> Option<String> {
    let explanation = view.explanation.as_deref()?;
    let line = single_line(explanation, EXPLANATION_SAMPLE_CHARS);
    (!line.is_empty()).then_some(line)
}

/// 按显示列换行（宽字符按两列；保留原空格）。
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut current = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        let cell = ch.width().unwrap_or(1).max(1);
        if col + cell > width && col > 0 {
            out.push(std::mem::take(&mut current));
            col = 0;
        }
        current.push(ch);
        col += cell;
    }
    out.push(current);
    out
}

//! `ask` 卡片：标题（方块 + 动词 + 状态）+ 每题一棵树（design §8.4）。
//!
//! ```text
//! ■ Ask · 2 questions
//!   │ 布局 · 双栏
//!   └─ 主题 · 深色, 用青色
//! ```
//!
//! - 运行中只写 `waiting for you`：参数还在流式接收，不解析、不猜题数；
//! - 折叠态每题一行 `header · answers`；展开态追加完整题干；
//! - **永不展示 preview**：字符画只服务于决策现场，历史里是噪音；
//! - `rows` 与 `draw` 必须严格同源（`history::layout` 与 `render` 共用）。

use ratatui::layout::Rect;

use crate::app::state::{AskDetailView, ToolCallView, ToolDetailView, ToolStatus};
use crate::ui::text::agent as a;
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::{TREE_COLS, header_with_verb, put_segments, single_line, title_rows};

/// 折叠行文本取样上限（限住超长答案的每帧成本）。
const SAMPLE_CHARS: usize = 512;

/// 卡片占用的渲染行数；与 [`draw`] 严格相等。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    let Some(ToolDetailView::Ask(view)) = call.detail.as_deref() else {
        return title_rows();
    };
    let line_width = usize::from(width.saturating_sub(TREE_COLS as u16)).max(1);
    let mut total = title_rows();
    for question in &view.questions {
        total += 1;
        if expanded {
            total += wrap(&question.question, line_width).len();
        }
    }
    total
}

/// 绘制卡片；推进行数必须与 [`rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    animating: bool,
    expanded: bool,
) {
    let width = rect.width;
    if width == 0 {
        return;
    }
    let view = match call.detail.as_deref() {
        Some(ToolDetailView::Ask(view)) => Some(view),
        _ => None,
    };
    let running = matches!(
        call.status,
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running
    );
    let status = if running {
        Some(a::ASK_WAITING.to_string())
    } else {
        view.map(|view| a::ask_question_count(view.questions.len()))
    };
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header_with_verb(
            call,
            width,
            loading_frame,
            animating,
            a::VERB_TOOL_ASK,
            status,
        );
        put_segments(grid, rect.x, y, width, &segments);
    }
    *row_index += 1;
    let Some(view) = view else {
        return;
    };
    draw_questions(grid, rect, scroll, row_index, view, expanded);
}

/// 每题一行摘要（折叠）或摘要 + 题干（展开）。
fn draw_questions(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    view: &AskDetailView,
    expanded: bool,
) {
    let width = rect.width;
    let line_width = usize::from(width.saturating_sub(TREE_COLS as u16)).max(1);
    let last = view.questions.len().saturating_sub(1);
    for (index, question) in view.questions.iter().enumerate() {
        let connector = if index == last {
            text::agent::TREE_LEAF
        } else {
            text::agent::TREE_BAR
        };
        let answers = if question.answers.is_empty() {
            a::ASK_NO_ANSWER.to_string()
        } else {
            question.answers.join(", ")
        };
        let line = single_line(&format!("{} · {answers}", question.header), SAMPLE_CHARS);
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(rect.x, y, connector, style::chat_notice_connector(), width);
            grid.put_clipped(
                rect.x.saturating_add(TREE_COLS as u16),
                y,
                &line,
                style::agent_step_detail(),
                width.saturating_sub(TREE_COLS as u16),
            );
        }
        *row_index += 1;
        if !expanded {
            continue;
        }
        for line in wrap(&question.question, line_width) {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                grid.put_clipped(
                    rect.x,
                    y,
                    text::agent::TREE_BAR,
                    style::chat_notice_connector(),
                    width,
                );
                grid.put_clipped(
                    rect.x.saturating_add(TREE_COLS as u16),
                    y,
                    &line,
                    style::text(),
                    width.saturating_sub(TREE_COLS as u16),
                );
            }
            *row_index += 1;
        }
    }
}

/// 按显示列换行（宽字符两列）。
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let mut current = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        let cell = unicode_width::UnicodeWidthChar::width(ch)
            .unwrap_or(1)
            .max(1);
        if col + cell > width && col > 0 {
            out.push(std::mem::take(&mut current));
            col = 0;
        }
        current.push(ch);
        col += cell;
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests;

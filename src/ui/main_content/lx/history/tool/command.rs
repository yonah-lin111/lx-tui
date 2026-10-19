//! `exec_command` / `write_stdin` 卡片：紧凑预览 + 按需原位展开。
//!
//! 版式（对应 design §10.2 / §10.3）：
//!
//! ```text
//! ■ Run command                     Running · 12s
//!   └─ cargo test chat::tools
//!      cwd: /workspace/lx
//!      running 24 tests
//!      test read::tests::empty_file ... ok
//! ```
//!
//! - 标题行沿用文件工具同一套 `■ 动词` 语法，状态文字 + 时长靠右；
//! - 详情行放 `└─` + 命令（这一行唯一必须保住的信息）；
//! - 默认预览最近六行输出，展开后显示全部保留文本与 cwd；
//! - 没有输出时显式占位（`Waiting for output`），绝不画空行；
//! - 卡片只读快照：不解析 JSON、不读进程、不读数据库。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{
    CommandView, ProcessInteractionView, ProcessStateView, ToolCallView, ToolStatus,
};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::{Segment, detail_prefix, header_with_verb, put_segments, single_line, title_rows};

/// 命令卡片折叠时的输出预览行数。
pub(super) const PREVIEW_LINES: usize = 6;
/// 展开时单卡片最多显示的输出行数（滚动交给外层历史区）。
const EXPANDED_LINES: usize = 200;
/// 单行取样上限（限住每帧成本）。
const SAMPLE_CHARS: usize = 512;

/// 命令 / 交互卡片占用的渲染行数；`expanded` 由窗格内展开状态决定。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => {
            title_rows() + 1 + command_body_rows(view, expanded)
        }
        Some(crate::app::state::ToolDetailView::Interaction(view)) => {
            title_rows() + 1 + interaction_body_rows(view)
        }
        _ => title_rows(),
    }
}

/// 命令卡片正文行数（不含标题行与命令详情行）。
///
/// 与 [`draw_command`] 共用同一份口径：展开时多一行 cwd，输出为空时占一行。
fn command_body_rows(view: &CommandView, expanded: bool) -> usize {
    let lines = output_lines(&view.output_tail);
    if expanded {
        let shown = lines.len().min(EXPANDED_LINES);
        return usize::from(true) + shown.max(1) + usize::from(view.dropped_bytes > 0);
    }
    let shown = lines.len().min(PREVIEW_LINES);
    // 没有输出时留一行占位；截断证据靠显式标记，不额外占行。
    shown.max(1)
}

/// 交互卡片正文行数（输入预览 + 输出快照）。
fn interaction_body_rows(view: &ProcessInteractionView) -> usize {
    let input = usize::from(view.input_preview.is_some());
    let lines = output_lines(&view.output);
    input + lines.len().clamp(1, PREVIEW_LINES)
}

/// 输出文本 → 显示行（保留中间空行；末尾换行不额外占行）。
fn output_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

/// 绘制命令或交互卡片；推进的行数必须与 [`rows`] 严格相等。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    expanded: bool,
) {
    if rect.width == 0 {
        // 宽度 0：与 [`rows`] 一致地不占行（外层布局已跳过该块）。
        return;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => {
            draw_command(
                grid,
                rect,
                scroll,
                row_index,
                call,
                view,
                loading_frame,
                expanded,
            );
        }
        Some(crate::app::state::ToolDetailView::Interaction(view)) => {
            draw_interaction(grid, rect, scroll, row_index, call, view, loading_frame);
        }
        _ => {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                let status = plain_status_label(call.status);
                let segments = header_with_verb(
                    call,
                    rect.width,
                    loading_frame,
                    text::agent::VERB_EXEC_COMMAND,
                    status,
                    expanded,
                );
                put_segments(grid, rect.x, y, rect.width, &segments);
            }
            *row_index += 1;
        }
    }
}

/// 命令卡片：标题 + 命令/状态 +（展开时 cwd）+ 输出预览。
#[allow(clippy::too_many_arguments)]
fn draw_command(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &CommandView,
    loading_frame: usize,
    expanded: bool,
) {
    let width = rect.width;
    let status = process_status(view.state, view.started_at_ms);
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header_with_verb(
            call,
            width,
            loading_frame,
            text::agent::VERB_EXEC_COMMAND,
            status,
            expanded,
        );
        put_segments(grid, rect.x, y, width, &segments);
    }
    *row_index += 1;
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        put_segments(grid, rect.x, y, width, &command_line(view, width));
    }
    *row_index += 1;
    let indent = body_indent();
    let body_limit = width.saturating_sub(indent);
    if expanded && let Some(y) = visible_row(rect, scroll, *row_index) {
        let line = text::agent::cwd_label(&view.cwd);
        grid.put_clipped(
            rect.x.saturating_add(indent),
            y,
            &single_line(&line, SAMPLE_CHARS),
            style::agent_step_detail(),
            body_limit,
        );
        *row_index += 1;
    }
    let lines = output_lines(&view.output_tail);
    if lines.is_empty() {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(
                rect.x.saturating_add(indent),
                y,
                text::agent::WAITING_FOR_OUTPUT,
                style::agent_step_detail(),
                body_limit,
            );
        }
        *row_index += 1;
    } else {
        let shown: Vec<&String> = if expanded {
            lines.iter().take(EXPANDED_LINES).collect()
        } else {
            let start = lines.len().saturating_sub(PREVIEW_LINES);
            lines.iter().skip(start).collect()
        };
        for line in shown {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                grid.put_clipped(
                    rect.x.saturating_add(indent),
                    y,
                    &single_line(line, SAMPLE_CHARS),
                    style::text(),
                    body_limit,
                );
            }
            *row_index += 1;
        }
    }
    if expanded && view.dropped_bytes > 0 {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(
                rect.x.saturating_add(indent),
                y,
                text::agent::EARLIER_OUTPUT_DISCARDED,
                style::agent_warn(),
                body_limit,
            );
        }
        *row_index += 1;
    }
    let _ = (view.truncated, view.ended_at_ms);
}

/// 交互卡片：标题 + 命令/状态 + 输入预览 + 本次输出快照。
fn draw_interaction(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    view: &ProcessInteractionView,
    loading_frame: usize,
) {
    let width = rect.width;
    let status = process_status(view.state, None);
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header_with_verb(
            call,
            width,
            loading_frame,
            text::agent::interaction_verb(view.action),
            status,
            false,
        );
        put_segments(grid, rect.x, y, width, &segments);
    }
    *row_index += 1;
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        put_segments(grid, rect.x, y, width, &interaction_line(view, width));
    }
    *row_index += 1;
    let indent = body_indent();
    let body_limit = width.saturating_sub(indent);
    if let Some(input) = &view.input_preview
        && let Some(y) = visible_row(rect, scroll, *row_index)
    {
        let line = format!("{} {input}", text::agent::INPUT_PREFIX);
        grid.put_clipped(
            rect.x.saturating_add(indent),
            y,
            &single_line(&line, SAMPLE_CHARS),
            style::agent_step_detail(),
            body_limit,
        );
        *row_index += 1;
    }
    let lines = output_lines(&view.output);
    if lines.is_empty() {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            grid.put_clipped(
                rect.x.saturating_add(indent),
                y,
                text::agent::NO_NEW_OUTPUT,
                style::agent_step_detail(),
                body_limit,
            );
        }
        *row_index += 1;
    } else {
        for line in lines.iter().take(PREVIEW_LINES) {
            if let Some(y) = visible_row(rect, scroll, *row_index) {
                grid.put_clipped(
                    rect.x.saturating_add(indent),
                    y,
                    &single_line(line, SAMPLE_CHARS),
                    style::text(),
                    body_limit,
                );
            }
            *row_index += 1;
        }
    }
    let _ = &view.truncated;
}

/// 正文缩进列（`└─ ` 之下对齐）。
fn body_indent() -> u16 {
    super::super::STEP_GUTTER + 3
}

/// 详情行：`  └─ <cmd>`。
fn command_line(view: &CommandView, width: u16) -> Vec<Segment> {
    labeled_line(&single_line(&view.command, SAMPLE_CHARS), width)
}

/// 交互详情行：命令，缺失时退回 `session: N`。
fn interaction_line(view: &ProcessInteractionView, width: u16) -> Vec<Segment> {
    let label = if view.command.is_empty() {
        text::agent::session_label(view.process_session_id)
    } else {
        single_line(&view.command, SAMPLE_CHARS)
    };
    labeled_line(&label, width)
}

/// `  └─ <label>`：前缀放不下时退化为整宽内容。
fn labeled_line(label: &str, width: u16) -> Vec<Segment> {
    let limit = usize::from(width);
    let prefix = detail_prefix();
    let prefix_cols = prefix.width();
    if prefix_cols >= limit {
        return vec![Segment::new(
            crate::ui::main_content::lx::history::truncate_with_ellipsis(label, limit),
            style::chat_tool_param(),
        )];
    }
    let available = limit.saturating_sub(prefix_cols);
    let text = crate::ui::main_content::lx::history::truncate_with_ellipsis(label, available);
    let mut segments = vec![Segment::new(prefix, style::chat_notice_connector())];
    if !text.is_empty() {
        segments.push(Segment::new(text, style::chat_tool_param()));
    }
    segments
}

/// 进程状态文案 + 运行时长：`Running · 12s` / `Exited 0` / `exit 2`。
fn process_status(state: ProcessStateView, started_at_ms: Option<i64>) -> Option<String> {
    let label = text::agent::proc_exit_label(state);
    if state.is_running()
        && let Some(started) = started_at_ms
    {
        let elapsed = crate::db::now_ms().saturating_sub(started).max(0);
        return Some(format!(
            "{label}{}{}",
            text::LX_CHAT_TOOL_STATUS_SEP,
            format_elapsed(elapsed)
        ));
    }
    Some(label)
}

/// 时长短格式：`820ms` / `4s` / `1m 12s`。
fn format_elapsed(ms: i64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1_000.0;
    if seconds < 60.0 {
        return format!("{seconds:.0}s");
    }
    let minutes = (seconds / 60.0).floor() as i64;
    let rest = (seconds % 60.0).round() as i64;
    format!("{minutes}m {rest}s")
}

/// 没有类型化详情时的状态词（例如只有失败错误的卡片）。
fn plain_status_label(status: ToolStatus) -> Option<String> {
    match status {
        ToolStatus::Completed | ToolStatus::Failed => None,
        other => Some(text::tool_status_label(other).to_string()),
    }
}

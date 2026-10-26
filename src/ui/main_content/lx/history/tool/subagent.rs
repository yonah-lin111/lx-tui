//! 委派卡片：`spawn_agent` / `wait_agent` / `close_agent` 的工具行。
//!
//! 语法与其余工具一致：第 1 行 `■ Spawn agent`（方块 = 工具调用），第 2 行
//! `  └─ explorer · <task 首行>`。折叠恒 2 行；展开显示 task 全文、agent_id、
//! 目标与每个目标的状态。行数只有一个来源（[`detail_lines`]），`rows` / `draw`
//! 与 `layout` 共用。

use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::state::{SubagentDetailView, ToolCallView, ToolDetailView, ToolStatus};
use crate::ui::{style, text};

use super::super::{Grid, visible_row};
use super::{
    DETAIL_MAX_ROWS, HEADER_ROWS, Segment, TEXT_SAMPLE_CHARS, header_with_verb, put_segments,
    single_line, wrap_spans,
};

/// 标题动词（按工具名）。
fn verb(name: &str) -> &'static str {
    match name {
        "spawn_agent" => text::agent::VERB_TOOL_SPAWN_AGENT,
        "wait_agent" => text::agent::VERB_TOOL_WAIT_AGENTS,
        "close_agent" => text::agent::VERB_TOOL_CLOSE_AGENT,
        _ => text::agent::VERB_TOOL_SPAWN_AGENT,
    }
}

/// 取详情视图（非委派卡片返回 None）。
fn view(call: &ToolCallView) -> Option<&SubagentDetailView> {
    match call.detail.as_deref()? {
        ToolDetailView::Subagent(view) => Some(view),
        _ => None,
    }
}

/// 折叠态的摘要行文本。
fn summary(view: &SubagentDetailView, call: &ToolCallView) -> String {
    match call.name.as_str() {
        "wait_agent" => format!(
            "{} {} · {}",
            view.agents.len(),
            text::agent::AGENTS_LABEL,
            if view.timed_out {
                text::agent::WAIT_TIMED_OUT
            } else {
                text::agent::WAIT_SETTLED
            }
        ),
        "close_agent" => {
            let agent = view.agent_id.as_deref().unwrap_or_default();
            let status = view.closed_status.as_deref().unwrap_or_default();
            if status.is_empty() {
                single_line(agent, TEXT_SAMPLE_CHARS)
            } else {
                format!("{agent} · {status}")
            }
        }
        _ => {
            let role = view.role.map_or("", |role| role.as_str());
            let task = view.task.as_deref().unwrap_or_default();
            let head = single_line(task.lines().next().unwrap_or_default(), TEXT_SAMPLE_CHARS);
            if head.is_empty() {
                role.to_string()
            } else if role.is_empty() {
                head
            } else {
                format!("{role} {}{head}", text::agent::SPAWN_ROLE_SEP)
            }
        }
    }
}

/// 展开态的内容（不含 `└─` 前缀）。
fn expanded_lines(view: &SubagentDetailView, call: &ToolCallView) -> Vec<(Style, String)> {
    let dim = style::agent_step_detail();
    let param = style::chat_tool_param();
    let mut lines: Vec<(Style, String)> = Vec::new();
    match call.name.as_str() {
        "spawn_agent" => {
            if let Some(task) = &view.task {
                for line in task.lines() {
                    lines.push((param, line.to_string()));
                }
            }
            if let Some(agent_id) = &view.agent_id {
                lines.push((dim, format!("{} {agent_id}", text::agent::AGENT_ID_LABEL)));
            }
        }
        "wait_agent" => {
            lines.push((
                dim,
                format!(
                    "{} {}",
                    text::agent::WAIT_TARGETS_LABEL,
                    view.targets.join(", ")
                ),
            ));
            for agent in &view.agents {
                lines.push((
                    dim,
                    format!(
                        "- {} {} {}",
                        agent.agent_id,
                        agent.role.as_str(),
                        agent.status
                    ),
                ));
            }
        }
        _ => {
            if let Some(agent_id) = &view.agent_id {
                lines.push((dim, format!("{} {agent_id}", text::agent::AGENT_ID_LABEL)));
            }
            if let Some(status) = &view.closed_status {
                lines.push((param, format!("{} {status}", text::agent::PREVIOUS_STATUS_LABEL)));
            }
        }
    }
    lines
}

/// `└─` 之后的物理行（含前缀）；第一个元素是摘要行。
fn detail_lines(call: &ToolCallView, width: u16, expanded: bool) -> Vec<Vec<Segment>> {
    let Some(view) = view(call) else {
        return Vec::new();
    };
    if width == 0 {
        return Vec::new();
    }
    let prefix = Segment::new(
        format!("  {} ", text::agent::MARKER_TREE_LEAF),
        style::agent_step_detail(),
    );
    let prefix_cols = prefix.cols();
    let available = usize::from(width).saturating_sub(prefix_cols).max(1);
    let mut out: Vec<Vec<Segment>> = Vec::new();
    let head = vec![Segment::new(summary(view, call), style::chat_tool_param())];
    for (index, mut line) in wrap_spans(&head, available).into_iter().enumerate() {
        if index == 0 {
            line.insert(0, prefix.clone());
        } else {
            line.insert(0, Segment::new(" ".repeat(prefix_cols), Style::default()));
        }
        out.push(line);
    }
    if !expanded {
        return out;
    }
    for (line_style, content) in expanded_lines(view, call) {
        let spans = vec![Segment::new(content, line_style)];
        for (index, mut line) in wrap_spans(&spans, available).into_iter().enumerate() {
            if index == 0 {
                line.insert(0, prefix.clone());
            } else {
                line.insert(0, Segment::new(" ".repeat(prefix_cols), Style::default()));
            }
            if out.len() >= DETAIL_MAX_ROWS {
                return out;
            }
            out.push(line);
        }
        if out.len() >= DETAIL_MAX_ROWS {
            return out;
        }
    }
    out
}

/// 行数（宽度 0 时为 0）。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    HEADER_ROWS + detail_lines(call, width, expanded).len()
}

/// 展开是否还有更多内容。
pub(super) fn has_more(call: &ToolCallView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    detail_lines(call, width, true) != detail_lines(call, width, false)
}

/// 绘制标题与详情行。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    expanded: bool,
    animating: bool,
) {
    if rect.width == 0 {
        return;
    }
    let status = matches!(
        call.status,
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running
    )
    .then(|| text::tool_status_label(call.status).to_string());
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header_with_verb(call, rect.width, loading_frame, animating, verb(&call.name), status);
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    for line in detail_lines(call, rect.width, expanded) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            put_segments(grid, rect.x, y, rect.width, &line);
        }
        *row_index += 1;
    }
}

/// 卡片可由点击/键盘展开的区域（整块 Body）。
pub(super) fn card_rows(expanded: bool, call: &ToolCallView, width: u16) -> usize {
    rows(call, width, expanded)
}

/// 是否需要登记点击面。
pub(super) fn clickable(call: &ToolCallView, width: u16) -> bool {
    has_more(call, width)
}

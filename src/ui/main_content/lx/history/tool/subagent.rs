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
                lines.push((
                    param,
                    format!("{} {status}", text::agent::PREVIOUS_STATUS_LABEL),
                ));
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
        let segments = header_with_verb(
            call,
            rect.width,
            loading_frame,
            animating,
            verb(&call.name),
            status,
        );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{SubagentDetailView, ToolCallView};

    fn call(name: &str, detail: SubagentDetailView) -> ToolCallView {
        ToolCallView {
            call_id: "call-1".into(),
            name: name.into(),
            position: 0,
            arguments_json: "{}".into(),
            status: ToolStatus::Completed,
            summary: None,
            error: None,
            result_json: None,
            detail: Some(Box::new(ToolDetailView::Subagent(detail))),
        }
    }

    fn spawn_view() -> SubagentDetailView {
        SubagentDetailView {
            role: Some(crate::chat::agent::AgentRole::Explorer),
            task: Some("find the compact entry".into()),
            agent_id: Some("sub-1".into()),
            targets: Vec::new(),
            timed_out: false,
            agents: Vec::new(),
            closed_status: None,
        }
    }

    #[test]
    fn zero_width_draws_nothing() {
        let call = call("spawn_agent", spawn_view());
        assert_eq!(rows(&call, 0, false), 0);
        assert_eq!(rows(&call, 0, true), 0);
    }

    #[test]
    fn collapsed_spawn_is_header_plus_one_line() {
        let call = call("spawn_agent", spawn_view());
        assert_eq!(rows(&call, 80, false), 2);
        assert!(has_more(&call, 80), "展开能看到 task 全文与 agent_id");
    }

    #[test]
    fn wide_characters_do_not_overflow() {
        let mut view = spawn_view();
        view.task = Some("查出 compact 的触发链在哪里".repeat(8));
        let call = call("spawn_agent", view);
        // 行数按宽度增长，但每行内容都按列裁剪（不 assert 具体行数，只验证不 panic）。
        let narrow = rows(&call, 12, true);
        let wide = rows(&call, 200, true);
        assert!(narrow >= wide, "窄宽度需要更多物理行");
        assert_eq!(rows(&call, 0, true), 0);
    }

    #[test]
    fn wait_and_close_summaries_use_their_own_fields() {
        let wait = call(
            "wait_agent",
            SubagentDetailView {
                role: None,
                task: None,
                agent_id: None,
                targets: vec!["sub-1".into()],
                timed_out: true,
                agents: vec![crate::chat::agent::AgentStatusView {
                    agent_id: "sub-1".into(),
                    role: crate::chat::agent::AgentRole::Explorer,
                    status: "running".into(),
                    report: None,
                }],
                closed_status: None,
            },
        );
        assert_eq!(rows(&wait, 80, false), 2);
        let close = call(
            "close_agent",
            SubagentDetailView {
                role: None,
                task: None,
                agent_id: Some("sub-2".into()),
                targets: Vec::new(),
                timed_out: false,
                agents: Vec::new(),
                closed_status: Some("running".into()),
            },
        );
        assert_eq!(rows(&close, 80, false), 2);
        assert!(has_more(&close, 80), "展开能看到 agent_id 与关闭前状态");
    }
}

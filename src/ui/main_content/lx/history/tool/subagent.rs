//! 委派卡片：`spawn_agent` / `wait_agent` / `close_agent` 的工具行。
//!
//! 语法与其余工具一致：第 1 行 `■ Spawn agent · running`，第 2 行
//! `  └─ explorer · <title>`。Agent 行是链接，点击直接打开 Agents 面板并定位。
//! `wait_agent` 的树体逐目标列出，状态只用静态绿/黄/红方块表达：
//!
//! ```text
//! ■ Wait agents · 2 targets · all · until done
//!   ├─ ■ explorer · 调查事件流
//!   └─ ■ explorer · 检查存储层
//! ```
//!
//! 行数只有一个来源（[`detail_lines`]），`rows` / `draw` / 链接几何共用。

use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::state::{SubagentDetailView, ToolCallView, ToolDetailView, ToolStatus};
use crate::chat::agent::WaitMode;
use crate::ui::markdown_render;
use crate::ui::{style, text};

use super::super::{Grid, StepMark, visible_row};
use super::{
    DETAIL_MAX_ROWS, HEADER_ROWS, Segment, TEXT_SAMPLE_CHARS, header_with_mark, header_with_verb,
    mark_of, put_segments, single_line, wrap_spans,
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

/// `agent名 · title`；缺字段时按可用信息回退，不展示 task 原文。
fn agent_line(view: &SubagentDetailView) -> String {
    let role = view.role.map_or("", |role| role.as_str());
    let title = view
        .title
        .as_deref()
        .map(|title| single_line(title, TEXT_SAMPLE_CHARS))
        .unwrap_or_default();
    let agent_id = view.agent_id.as_deref().unwrap_or_default();
    let name = if role.is_empty() {
        if agent_id.is_empty() {
            text::agent::AGENTS_LABEL
        } else {
            agent_id
        }
    } else {
        role
    };
    if title.is_empty() {
        name.to_string()
    } else {
        format!("{name}{}{title}", text::agent::SPAWN_ROLE_SEP)
    }
}

/// wait 目标状态色：只保留绿 / 黄 / 红三档，不使用呼吸动画。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitColor {
    Green,
    Yellow,
    Red,
}

impl WaitColor {
    fn style(self) -> Style {
        match self {
            Self::Green => style::chat_step_mark_done(),
            Self::Yellow => style::agent_warn(),
            Self::Red => style::agent_fail(),
        }
    }
}

/// 等待树体的一行。
struct WaitRow {
    connector: &'static str,
    color: WaitColor,
    text: String,
    /// `All` 行没有具体 agent，不登记链接。
    agent_id: Option<String>,
}

/// 报告段落标题：`Report · explorer · title`。
fn report_heading(report: &crate::chat::agent::SubagentReportView) -> String {
    let role = report.role.as_str();
    if report.title.is_empty() {
        format!("{} · {role}", text::agent::WAIT_REPORT)
    } else {
        format!(
            "{} · {role}{}{}",
            text::agent::WAIT_REPORT,
            text::agent::SPAWN_ROLE_SEP,
            report.title
        )
    }
}

/// 在详情行末尾追加一段报告正文（展开态）。
fn push_report_body(
    out: &mut Vec<Vec<Segment>>,
    report: &crate::chat::agent::SubagentReportView,
    width: u16,
) {
    const INDENT: usize = 5;
    if out.len() >= DETAIL_MAX_ROWS || report.body.trim().is_empty() {
        return;
    }
    out.push(vec![Segment::new(
        format!("  {}", report_heading(report)),
        style::chat_tool_param(),
    )]);
    let body_width = usize::from(width).saturating_sub(INDENT).max(1);
    for line in report_body_lines(&report.body, u16::try_from(body_width).unwrap_or(u16::MAX)) {
        if out.len() >= DETAIL_MAX_ROWS {
            return;
        }
        out.push(vec![
            Segment::new(" ".repeat(INDENT), Style::default()),
            Segment::new(line, style::text()),
        ]);
    }
}

/// 报告正文按宽度折行；与汇报块共用 markdown 渲染口径。
fn report_body_lines(body: &str, width: u16) -> Vec<String> {
    markdown_render::render_wrapped(body, width.max(1))
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

/// 目标状态 → 静态颜色（绿=完成，黄=等待/取消，红=失败）。
fn status_color(status: &str) -> WaitColor {
    match status {
        "completed" => WaitColor::Green,
        "failed" => WaitColor::Red,
        _ => WaitColor::Yellow,
    }
}

/// 全部等待时的一行：所有完成才绿；有失败/取消沿用对应颜色。
fn all_row(view: &SubagentDetailView, call_status: ToolStatus) -> WaitRow {
    let color = if view.agents.is_empty() {
        match call_status {
            ToolStatus::Completed => WaitColor::Green,
            ToolStatus::Failed => WaitColor::Red,
            ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running => {
                WaitColor::Yellow
            }
            ToolStatus::Interrupted | ToolStatus::Cancelled => WaitColor::Yellow,
        }
    } else if view.agents.iter().any(|agent| agent.status == "failed") {
        WaitColor::Red
    } else if view.agents.iter().all(|agent| agent.status == "completed")
        || (view.mode == WaitMode::Any
            && view.agents.iter().any(|agent| agent.status == "completed"))
    {
        WaitColor::Green
    } else {
        WaitColor::Yellow
    };
    WaitRow {
        connector: text::agent::TREE_LEAF,
        color,
        text: text::agent::WAIT_ALL.to_string(),
        agent_id: None,
    }
}

/// wait 卡片的树体行；目标列表为空时只画 `All`。
fn wait_rows(view: &SubagentDetailView, call_status: ToolStatus) -> Vec<WaitRow> {
    if view.all || view.targets.is_empty() {
        return vec![all_row(view, call_status)];
    }
    let last = view.targets.len().saturating_sub(1);
    view.targets
        .iter()
        .enumerate()
        .map(|(index, agent_id)| {
            let status = view.agents.iter().find(|agent| &agent.agent_id == agent_id);
            let color = status.map_or(WaitColor::Yellow, |agent| status_color(&agent.status));
            let text = match status {
                Some(agent) => {
                    let mut parts = vec![agent.role.as_str().to_string()];
                    if !agent.title.is_empty() {
                        parts.push(agent.title.clone());
                    }
                    parts.join(text::agent::SPAWN_ROLE_SEP)
                }
                None => agent_id.clone(),
            };
            WaitRow {
                connector: if index == last {
                    text::agent::TREE_LEAF
                } else {
                    text::agent::TREE_BRANCH
                },
                color,
                text,
                agent_id: Some(agent_id.clone()),
            }
        })
        .collect()
}

/// close 卡片的折叠摘要（仍保留展开详情）。
fn close_summary(view: &SubagentDetailView) -> String {
    let agent = view.agent_id.as_deref().unwrap_or_default();
    let status = view.closed_status.as_deref().unwrap_or_default();
    if status.is_empty() {
        single_line(agent, TEXT_SAMPLE_CHARS)
    } else {
        format!("{agent}{}{status}", text::agent::SPAWN_ROLE_SEP)
    }
}

/// close 卡片的展开内容（不含 `└─` 前缀）。
fn close_expanded_lines(view: &SubagentDetailView) -> Vec<(Style, String)> {
    let dim = style::agent_step_detail();
    let param = style::chat_tool_param();
    let mut lines: Vec<(Style, String)> = Vec::new();
    if let Some(agent_id) = &view.agent_id {
        lines.push((dim, format!("{} {agent_id}", text::agent::AGENT_ID_LABEL)));
    }
    if let Some(status) = &view.closed_status {
        lines.push((
            param,
            format!("{} {status}", text::agent::PREVIOUS_STATUS_LABEL),
        ));
    }
    lines
}

/// `└─` 之后的物理行；第一个元素是摘要/链接行。
fn detail_lines(call: &ToolCallView, width: u16, expanded: bool) -> Vec<Vec<Segment>> {
    let Some(view) = view(call) else {
        return Vec::new();
    };
    if width == 0 {
        return Vec::new();
    }
    match call.name.as_str() {
        "spawn_agent" => {
            let mut out = vec![vec![
                Segment::new(
                    text::agent::TREE_LEAF.to_string(),
                    style::agent_step_detail(),
                ),
                Segment::new(agent_line(view), style::text()),
            ]];
            if expanded && let Some(report) = &view.report {
                push_report_body(&mut out, report, width);
            }
            out
        }
        "wait_agent" => {
            let connector_style = style::chat_notice_connector();
            let mut out: Vec<Vec<Segment>> = wait_rows(view, call.status)
                .into_iter()
                .map(|row| {
                    vec![
                        Segment::new(row.connector, connector_style),
                        Segment::new(
                            format!("{} ", text::agent::STEP_MARK_FILLED),
                            row.color.style(),
                        ),
                        Segment::new(row.text, style::text()),
                    ]
                })
                .collect();
            if expanded {
                // 按 agent 顺序追加正文；标题用 role/title，正文用 `report`。
                for agent in view.agents.iter().filter(|agent| {
                    agent
                        .report
                        .as_deref()
                        .is_some_and(|body| !body.trim().is_empty())
                }) {
                    if out.len() >= DETAIL_MAX_ROWS {
                        break;
                    }
                    let heading = if agent.title.is_empty() {
                        format!("  {} · {}", text::agent::WAIT_REPORT, agent.role.as_str())
                    } else {
                        format!(
                            "  {} · {}{}{}",
                            text::agent::WAIT_REPORT,
                            agent.role.as_str(),
                            text::agent::SPAWN_ROLE_SEP,
                            agent.title
                        )
                    };
                    out.push(vec![Segment::new(heading, style::chat_tool_param())]);
                    if let Some(body) = agent.report.as_deref() {
                        let body_width = usize::from(width).saturating_sub(5).max(1);
                        for line in
                            report_body_lines(body, u16::try_from(body_width).unwrap_or(u16::MAX))
                        {
                            if out.len() >= DETAIL_MAX_ROWS {
                                break;
                            }
                            out.push(vec![
                                Segment::new(" ".repeat(5), Style::default()),
                                Segment::new(line, style::text()),
                            ]);
                        }
                    }
                }
            }
            out
        }
        _ => {
            let prefix = Segment::new(
                format!("  {} ", text::agent::MARKER_TREE_LEAF),
                style::agent_step_detail(),
            );
            let prefix_cols = prefix.cols();
            let available = usize::from(width).saturating_sub(prefix_cols).max(1);
            let mut out: Vec<Vec<Segment>> = Vec::new();
            let head = vec![Segment::new(close_summary(view), style::chat_tool_param())];
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
            for (line_style, content) in close_expanded_lines(view) {
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
    }
}

/// 行数（宽度 0 时为 0）。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    HEADER_ROWS + detail_lines(call, width, expanded).len()
}

/// 报告正文/close 详情展开后行数增加即可展开。
pub(super) fn has_more(call: &ToolCallView, width: u16) -> bool {
    width > 0 && detail_lines(call, width, true) != detail_lines(call, width, false)
}

/// 卡片内 agent 链接的（相对卡片首行偏移, agent_id）。
pub(super) fn agent_links(call: &ToolCallView, width: u16) -> Vec<(usize, String)> {
    if width == 0 {
        return Vec::new();
    }
    let Some(view) = view(call) else {
        return Vec::new();
    };
    match call.name.as_str() {
        "spawn_agent" => view
            .agent_id
            .as_ref()
            .map(|agent_id| vec![(HEADER_ROWS, agent_id.clone())])
            .unwrap_or_default(),
        "wait_agent" => wait_rows(view, call.status)
            .into_iter()
            .enumerate()
            .filter_map(|(index, row)| row.agent_id.map(|agent_id| (HEADER_ROWS + index, agent_id)))
            .collect(),
        _ => Vec::new(),
    }
}

/// spawn 标题方块跟子 agent 状态走，而不是“spawn 工具已返回”的调用状态。
fn agent_mark(view: &SubagentDetailView, call: &ToolCallView, loading_frame: usize) -> StepMark {
    match view.agent_status.as_deref() {
        // spawn 是“已经派出去了”的动作：子 agent 仍在跑也用完成色（绿色），
        // 不在主历史里持续黄色呼吸；运行状态由 Agents 面板负责展示。
        Some("completed" | "running" | "pending" | "blocked") => StepMark::Done,
        Some("failed") => StepMark::Failed,
        Some("cancelled") => StepMark::Cancelled,
        _ => mark_of(call.status, loading_frame),
    }
}

/// 标题状态：spawn 显示子 agent 状态；wait 显示等待停止参数；close 没有状态标题。
fn header_status(call: &ToolCallView, view: &SubagentDetailView) -> Option<String> {
    if call.name == "spawn_agent" {
        return Some(
            view.agent_status
                .clone()
                .unwrap_or_else(|| text::tool_status_label(call.status).to_string()),
        );
    }
    (call.name == "wait_agent").then(|| wait_condition(view))
}

/// wait 头部展示的停止参数：范围 + 模式 + 等待时间（all 模式显示 until done）。
fn wait_condition(view: &SubagentDetailView) -> String {
    let explicit = !view.all && !view.targets.is_empty();
    let mut parts = Vec::new();
    if explicit {
        let count = view.targets.len();
        parts.push(if count == 1 {
            text::agent::WAIT_TARGET_ONE.to_string()
        } else {
            format!("{count} {}", text::agent::WAIT_TARGETS_SUFFIX)
        });
        parts.push(view.mode.as_str().to_string());
    } else {
        parts.push(match view.mode {
            WaitMode::All => text::agent::WAIT_SCOPE_ALL.to_string(),
            WaitMode::Any => text::agent::WAIT_SCOPE_ANY.to_string(),
        });
    }
    if view.mode == WaitMode::All {
        parts.push(text::agent::WAIT_UNTIL_DONE.to_string());
    } else if let Some(timeout) = view.timeout_ms {
        parts.push(timeout_label(timeout));
    }
    if view.timed_out {
        parts.push(text::agent::WAIT_TIMED_OUT.to_string());
    }
    parts.join(text::agent::SPAWN_ROLE_SEP)
}

/// wait 超时参数的短格式：`30s` / `5m` / `1h 30m`。
fn timeout_label(ms: u64) -> String {
    let seconds = ms / 1_000;
    let minutes = seconds / 60;
    let rest = seconds % 60;
    if minutes >= 60 {
        let hours = minutes / 60;
        let minutes = minutes % 60;
        if minutes == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {minutes}m")
        }
    } else if minutes > 0 && rest == 0 {
        format!("{minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {rest}s")
    } else if seconds > 0 {
        format!("{seconds}s")
    } else {
        format!("{ms}ms")
    }
}

/// 绘制标题与详情行。
// 与工具卡片通用绘制签名保持一致（滚动/帧/展开/动画均来自渲染上下文）。
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
    let detail = view(call);
    let status = detail.and_then(|view| header_status(call, view));
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        // wait 卡片不走呼吸动画：内部目标只用静态绿/黄/红状态色。
        let header_animating = animating && call.name != "wait_agent";
        let segments = match detail {
            Some(view) if call.name == "spawn_agent" => header_with_mark(
                call,
                rect.width,
                loading_frame,
                header_animating,
                verb(&call.name),
                status,
                agent_mark(view, call, loading_frame),
            ),
            _ => header_with_verb(
                call,
                rect.width,
                loading_frame,
                header_animating,
                verb(&call.name),
                status,
            ),
        };
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
    use crate::app::state::{SubagentDetailView, ToolCallView, ToolStatus};

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
            title: Some("compact entry".into()),
            task: Some("find the compact entry".into()),
            agent_id: Some("sub-1".into()),
            agent_status: Some("running".into()),
            targets: Vec::new(),
            all: false,
            mode: WaitMode::All,
            timeout_ms: None,
            timed_out: false,
            agents: Vec::new(),
            report: None,
            closed_status: None,
        }
    }

    fn wait_view() -> SubagentDetailView {
        SubagentDetailView {
            role: None,
            title: None,
            task: None,
            agent_id: None,
            agent_status: None,
            targets: vec!["sub-1".into(), "sub-2".into()],
            all: false,
            mode: WaitMode::All,
            timeout_ms: Some(300_000),
            timed_out: false,
            agents: vec![
                crate::chat::agent::AgentStatusView {
                    agent_id: "sub-1".into(),
                    title: "compact entry".into(),
                    role: crate::chat::agent::AgentRole::Explorer,
                    status: "completed".into(),
                    report: None,
                },
                crate::chat::agent::AgentStatusView {
                    agent_id: "sub-2".into(),
                    title: "storage layer".into(),
                    role: crate::chat::agent::AgentRole::Executor,
                    status: "running".into(),
                    report: None,
                },
            ],
            report: None,
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
    fn spawn_is_header_plus_agent_link_without_task_prompt() {
        let call = call("spawn_agent", spawn_view());
        assert_eq!(rows(&call, 80, false), 2);
        assert!(!has_more(&call, 80), "spawn 不展开 task");
        let detail = detail_lines(&call, 80, false);
        let text: String = detail
            .iter()
            .flat_map(|line| line.iter().map(|segment| segment.text.as_str()))
            .collect();
        assert!(text.contains("explorer · compact entry"), "{text}");
        assert!(!text.contains("find the compact entry"), "{text}");
        assert_eq!(agent_links(&call, 80), vec![(1, "sub-1".into())]);
    }

    #[test]
    fn spawn_header_is_green_and_static_while_the_agent_runs() {
        let mut view = spawn_view();
        view.agent_status = Some("running".into());
        let call = call("spawn_agent", view);
        let area = Rect::new(0, 0, 80, 2);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(
            &mut grid,
            area,
            0,
            &mut row,
            &call,
            crate::ui::text::agent::STEP_BREATH_TICKS,
            false,
            true,
        );
        let header: String = (0..80).map(|x| grid.cell(x, 0).ch).collect();
        assert!(!header.contains('□'), "spawn 派出后不呼吸：{header}");
        assert_eq!(
            grid.cell(0, 0).style.fg,
            style::chat_step_mark_done().fg,
            "派出即绿色：{header}"
        );
    }

    #[test]
    fn spawn_header_status_tracks_the_agent_not_the_tool_call() {
        let mut view = spawn_view();
        view.agent_status = Some("running".into());
        let mut call = call("spawn_agent", view);
        call.status = ToolStatus::Completed;
        let area = Rect::new(0, 0, 80, 4);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(&mut grid, area, 0, &mut row, &call, 0, false, false);
        let header: String = (0..80).map(|x| grid.cell(x, 0).ch).collect();
        assert!(header.contains("Spawn agent"), "{header}");
        assert!(header.contains("running"), "{header}");
        assert!(
            !header.contains("done"),
            "spawn 工具已返回不能冒充子 agent 终态：{header}"
        );
    }

    #[test]
    fn wait_rows_show_only_color_and_links() {
        let call = call("wait_agent", wait_view());
        assert_eq!(rows(&call, 80, false), 3, "标题 + 两目标");
        assert!(!has_more(&call, 80));
        let links = agent_links(&call, 80);
        assert_eq!(links, vec![(1, "sub-1".into()), (2, "sub-2".into())]);
        let detail = detail_lines(&call, 80, false);
        let text: String = detail
            .iter()
            .flat_map(|line| line.iter().map(|segment| segment.text.as_str()))
            .collect();
        assert!(text.contains("explorer · compact entry"), "{text}");
        assert!(text.contains("executor · storage layer"), "{text}");
        for status in ["completed", "waiting", "failed"] {
            assert!(!text.contains(status), "目标行不再显示 {status}：{text}");
        }
    }

    #[test]
    fn wait_header_shows_stop_condition() {
        let call = call("wait_agent", wait_view());
        let area = Rect::new(0, 0, 80, 4);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(&mut grid, area, 0, &mut row, &call, 0, false, true);
        let header: String = (0..80).map(|x| grid.cell(x, 0).ch).collect();
        assert!(header.contains("Wait agents"), "{header}");
        assert!(header.contains("2 targets · all · until done"), "{header}");
        assert!(!header.contains("running"), "{header}");
        assert!(!header.contains("completed"), "{header}");
    }

    #[test]
    fn wait_header_shows_any_timeout_and_timed_out() {
        let mut view = wait_view();
        view.mode = WaitMode::Any;
        view.timeout_ms = Some(90_000);
        view.timed_out = true;
        let call = call("wait_agent", view);
        let area = Rect::new(0, 0, 100, 4);
        let mut grid = Grid::new(area);
        let mut row = 0;
        draw(&mut grid, area, 0, &mut row, &call, 0, false, true);
        let header: String = (0..100).map(|x| grid.cell(x, 0).ch).collect();
        assert!(
            header.contains("2 targets · any · 1m 30s · timed out"),
            "{header}"
        );
    }

    #[test]
    fn wait_arguments_carry_timeout_into_the_card() {
        let view = SubagentDetailView::from_arguments(
            "wait_agent",
            r#"{"wait_for":"any","timeout_ms":90000}"#,
        )
        .expect("wait view");
        assert_eq!(view.timeout_ms, Some(90_000));
        assert_eq!(wait_condition(&view), "any · 1m 30s");
    }

    #[test]
    fn wait_condition_labels_all_any_and_targets() {
        let mut view = wait_view();
        view.targets.clear();
        view.all = true;
        view.mode = WaitMode::All;
        view.timeout_ms = None;
        assert_eq!(wait_condition(&view), "all · until done");

        view.mode = WaitMode::Any;
        view.timeout_ms = Some(90_000);
        assert_eq!(wait_condition(&view), "any · 1m 30s");

        view.all = false;
        view.targets = vec!["sub-1".into()];
        view.mode = WaitMode::All;
        assert_eq!(wait_condition(&view), "1 target · all · until done");

        view.targets = vec!["sub-1".into(), "sub-2".into()];
        view.mode = WaitMode::Any;
        assert_eq!(wait_condition(&view), "2 targets · any · 1m 30s");
    }

    #[test]
    fn wait_rows_are_static_colored_squares() {
        let call = call("wait_agent", wait_view());
        let area = Rect::new(0, 0, 80, 5);
        let mut grid = Grid::new(area);
        let mut row = 0;
        // 即使主轮仍在动画推进，wait 内部目标也不呼吸。
        draw(
            &mut grid,
            area,
            0,
            &mut row,
            &call,
            crate::ui::text::agent::STEP_BREATH_TICKS,
            false,
            true,
        );
        let text: String = (0..80)
            .flat_map(|x| (0..5).map(move |y| (x, y)))
            .map(|(x, y)| grid.cell(x, y).ch)
            .collect();
        assert!(!text.contains('□'), "wait 内部不应出现空心呼吸方块：{text}");
        assert!(!text.contains("completed"), "{text}");
        assert!(!text.contains("waiting"), "{text}");
        let completed = grid.cell(5, 1);
        assert_eq!(completed.ch, crate::ui::text::agent::STEP_MARK_FILLED);
        assert_eq!(
            completed.style.fg,
            style::chat_step_mark_done().fg,
            "完成=绿"
        );
        let waiting = grid.cell(5, 2);
        assert_eq!(waiting.ch, crate::ui::text::agent::STEP_MARK_FILLED);
        assert_eq!(waiting.style.fg, style::agent_warn().fg, "等待=黄");
    }

    #[test]
    fn wait_report_body_expands_under_the_targets() {
        let mut view = wait_view();
        view.agents[0].report = Some("report body\nsecond line".into());
        let call = call("wait_agent", view);
        assert!(has_more(&call, 80));
        let collapsed: String = detail_lines(&call, 80, false)
            .iter()
            .flat_map(|line| line.iter().map(|segment| segment.text.as_str()))
            .collect();
        assert!(!collapsed.contains("report body"), "{collapsed}");
        let expanded: String = detail_lines(&call, 80, true)
            .iter()
            .flat_map(|line| line.iter().map(|segment| segment.text.as_str()))
            .collect();
        assert!(
            expanded.contains("Report · explorer · compact entry"),
            "{expanded}"
        );
        assert!(expanded.contains("report body"), "{expanded}");
        assert_eq!(
            agent_links(&call, 80),
            vec![(1, "sub-1".into()), (2, "sub-2".into())]
        );
    }

    #[test]
    fn spawn_report_body_expands_below_the_agent_link() {
        let mut view = spawn_view();
        view.report = Some(crate::chat::agent::SubagentReportView {
            agent_id: "sub-1".into(),
            title: "compact entry".into(),
            role: crate::chat::agent::AgentRole::Explorer,
            status: "completed".into(),
            files_changed: 1,
            duration_ms: Some(20),
            body: "found the entry".into(),
            delivered_at_ms: 1,
        });
        let call = call("spawn_agent", view);
        assert!(has_more(&call, 80));
        assert_eq!(rows(&call, 80, false), 2);
        let expanded: String = detail_lines(&call, 80, true)
            .iter()
            .flat_map(|line| line.iter().map(|segment| segment.text.as_str()))
            .collect();
        assert!(expanded.contains("found the entry"), "{expanded}");
        assert_eq!(agent_links(&call, 80), vec![(1, "sub-1".into())]);
    }

    #[test]
    fn wait_all_is_one_row_without_a_link() {
        let mut view = wait_view();
        view.targets.clear();
        view.all = true;
        let call = call("wait_agent", view);
        assert_eq!(rows(&call, 80, false), 2);
        assert!(agent_links(&call, 80).is_empty());
        let detail = detail_lines(&call, 80, false);
        let text: String = detail
            .iter()
            .flat_map(|line| line.iter().map(|segment| segment.text.as_str()))
            .collect();
        assert!(text.contains("All"), "{text}");
    }

    #[test]
    fn wide_characters_do_not_overflow() {
        let mut view = spawn_view();
        view.title = Some("查 compact 的触发链".repeat(8));
        let call = call("spawn_agent", view);
        let narrow = rows(&call, 12, false);
        let wide = rows(&call, 200, false);
        assert_eq!(narrow, 2);
        assert_eq!(wide, 2);
        assert_eq!(rows(&call, 0, false), 0);
    }

    #[test]
    fn close_summary_still_expands() {
        let close = call(
            "close_agent",
            SubagentDetailView {
                role: None,
                title: None,
                task: None,
                agent_id: Some("sub-2".into()),
                agent_status: None,
                targets: Vec::new(),
                all: false,
                mode: WaitMode::All,
                timeout_ms: None,
                timed_out: false,
                agents: Vec::new(),
                report: None,
                closed_status: Some("running".into()),
            },
        );
        assert_eq!(rows(&close, 80, false), 2);
        assert!(has_more(&close, 80), "close 仍能展开 agent_id 与关闭前状态");
    }
}

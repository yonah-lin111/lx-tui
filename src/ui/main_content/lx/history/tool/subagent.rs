//! 委派卡片：`spawn_agent` / `wait_agent` / `close_agent` 的工具行。
//!
//! ```text
//! ■ Wait agents · 2 targets · all · until done
//!   ├─ ■ explorer · 调查事件流
//!   └─ ■ explorer · 检查存储层
//! ```
//!
//! - 调用条件（目标数、等待模式、超时）在标题右侧，用 ` · ` 分隔；
//! - 目标行是树体节点，角色与标题之间用 ` · ` 分隔；
//! - 目标行是链接，点击打开 Agents 面板并定位；行偏移由同一份树体布局给出；
//! - 展开态追加报告（标题 + markdown 正文）或 close 详情，不再用 ` · ` 拼接；
//! - 行数只有一个来源（[`layout`]），`rows` / `draw` / 链接几何共用。

use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::state::{SubagentDetailView, ToolCallView, ToolDetailView, ToolStatus};
use crate::chat::agent::WaitMode;
use crate::ui::markdown_render;
use crate::ui::{style, text};

use super::super::{Grid, StepMark, visible_row};
use super::tree::{self, Node, Section};
use super::{
    Segment, TEXT_SAMPLE_CHARS, header_with_mark, header_with_verb, mark_of, put_segments,
    single_line,
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

/// `agent名  title`；缺字段时按可用信息回退，不展示 task 原文。
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

/// 目标状态 → 静态颜色（绿=完成，黄=等待/取消，红=失败）。
fn status_color(status: &str) -> WaitColor {
    match status {
        "completed" => WaitColor::Green,
        "failed" => WaitColor::Red,
        _ => WaitColor::Yellow,
    }
}

/// 全部等待时的一行：所有完成才绿；有失败/取消沿用对应颜色。
fn all_color(view: &SubagentDetailView, call_status: ToolStatus) -> WaitColor {
    if view.agents.is_empty() {
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
    }
}

/// 构建结果：节点 + 立即可见的链接（节点下标 → agent_id）。
struct Built {
    nodes: Vec<Node>,
    links: Vec<(usize, String)>,
}

impl Built {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            links: Vec::new(),
        }
    }
}

/// wait 头部展示的停止参数：范围 + 模式 + 等待时间（all 模式显示 until done）。
///
/// 这些字段属于**调用条件**，与 wait 的目标列表不是同一层：保留在标题右侧、
/// 用 ` · ` 分隔（与工具状态同一读法），目标行才是树体节点。
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

/// 目标行节点：`■ role  title`（静态状态色）。
fn target_node(view: &SubagentDetailView, agent_id: &str) -> Node {
    let status = view.agents.iter().find(|agent| agent.agent_id == agent_id);
    let color = status.map_or(WaitColor::Yellow, |agent| status_color(&agent.status));
    let body = match status {
        Some(agent) => {
            let mut parts = vec![agent.role.as_str().to_string()];
            if !agent.title.is_empty() {
                parts.push(agent.title.clone());
            }
            parts.join(text::agent::SPAWN_ROLE_SEP)
        }
        None => agent_id.to_string(),
    };
    Node::new(vec![
        Segment::new(format!("{} ", text::agent::STEP_MARK_FILLED), color.style()),
        Segment::new(body, style::text()),
    ])
}

/// 展开态报告：`Report:` + markdown 正文，整块一个多行节点。
fn report_node(body: &str, width: u16) -> Node {
    let mut lines: Vec<Vec<Segment>> =
        vec![vec![Segment::new(text::agent::REPORT_LABEL, style::text())]];
    for line in markdown_render::render_wrapped(
        body,
        u16::try_from(tree::content_width(width)).unwrap_or(1),
    ) {
        let mut text = " ".repeat(usize::from(line.indent));
        for (value, _) in line.spans {
            text.push_str(&value);
        }
        lines.push(vec![Segment::new(text, style::text())]);
    }
    Node::block(lines)
}

/// 卡片节点与链接。
fn built(call: &ToolCallView, width: u16, expanded: bool) -> Built {
    let Some(view) = view(call) else {
        return Built::new();
    };
    let mut built = Built::new();
    match call.name.as_str() {
        "spawn_agent" => {
            built
                .nodes
                .push(Node::text(agent_line(view), style::text()));
            if view.agent_id.is_some() {
                built
                    .links
                    .push((0, view.agent_id.clone().unwrap_or_default()));
            }
            if expanded && let Some(report) = &view.report {
                built.nodes.push(report_node(&report.body, width));
            }
        }
        "wait_agent" => {
            let targets: Vec<String> = if view.all || view.targets.is_empty() {
                Vec::new()
            } else {
                view.targets.clone()
            };
            if targets.is_empty() {
                let color = all_color(view, call.status);
                built.nodes.push(Node::new(vec![
                    Segment::new(format!("{} ", text::agent::STEP_MARK_FILLED), color.style()),
                    Segment::new(text::agent::WAIT_ALL.to_string(), style::text()),
                ]));
            } else {
                for target in &targets {
                    let index = built.nodes.len();
                    built.nodes.push(target_node(view, target));
                    built.links.push((index, target.clone()));
                }
            }
            if expanded {
                for agent in view.agents.iter().filter(|agent| {
                    agent
                        .report
                        .as_deref()
                        .is_some_and(|b| !b.trim().is_empty())
                }) {
                    if let Some(body) = agent.report.as_deref() {
                        built.nodes.push(report_node(body, width));
                    }
                }
            }
        }
        _ => {
            // close：目标行 + 之前的状态（不再把两者挤进同一行）。
            let target = if view.role.is_some() || view.title.is_some() {
                agent_line(view)
            } else {
                single_line(
                    view.agent_id.as_deref().unwrap_or_default(),
                    TEXT_SAMPLE_CHARS,
                )
            };
            built
                .nodes
                .push(Node::text(target, style::chat_tool_param()));
            if let Some(status) = &view.closed_status {
                built.nodes.push(Node::text(
                    format!("{} {status}", text::agent::PREVIOUS_STATUS_LABEL),
                    style::agent_step_detail(),
                ));
            }
            if expanded && let Some(agent_id) = &view.agent_id {
                built.nodes.push(Node::text(
                    format!("{} {agent_id}", text::agent::AGENT_ID_LABEL),
                    style::agent_step_detail(),
                ));
            }
        }
    }
    built
}

/// 排版（无折叠）：`rows`、`draw`、链接几何共用。
fn layout(call: &ToolCallView, width: u16, expanded: bool) -> (tree::Layout, Vec<(usize, String)>) {
    let built = built(call, width, expanded);
    let layout = tree::layout(&[Section::fixed(built.nodes)], width);
    let links = built
        .links
        .into_iter()
        .filter_map(|(node, agent_id)| {
            layout
                .node_rows
                .get(node)
                .map(|row| (row.saturating_add(super::HEADER_ROWS), agent_id))
        })
        .collect();
    (layout, links)
}

/// 行数（宽度 0 时为 0）。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    super::HEADER_ROWS + layout(call, width, expanded).0.rows.len()
}

/// 展开后行数增加即可展开（报告正文 / close 详情）。
pub(super) fn has_more(call: &ToolCallView, width: u16) -> bool {
    width > 0 && rows(call, width, true) > rows(call, width, false)
}

/// 卡片内 agent 链接的（相对卡片首行偏移, agent_id）。
pub(super) fn agent_links(call: &ToolCallView, width: u16) -> Vec<(usize, String)> {
    if width == 0 {
        return Vec::new();
    }
    layout(call, width, false).1
}

/// spawn 标题方块跟子 agent 状态走，而不是“spawn 工具已返回”的调用状态。
fn agent_mark(view: &SubagentDetailView, call: &ToolCallView, loading_frame: usize) -> StepMark {
    match view.agent_status.as_deref() {
        Some("completed" | "running" | "pending" | "blocked") => StepMark::Done,
        Some("failed") => StepMark::Failed,
        Some("cancelled") => StepMark::Cancelled,
        _ => mark_of(call.status, loading_frame),
    }
}

/// 标题状态：spawn 显示子 agent 状态；wait 显示调用条件；close 无。
fn header_status(call: &ToolCallView, view: &SubagentDetailView) -> Option<String> {
    match call.name.as_str() {
        "spawn_agent" => Some(
            view.agent_status
                .clone()
                .unwrap_or_else(|| text::tool_status_label(call.status).to_string()),
        ),
        "wait_agent" => Some(wait_condition(view)),
        _ => None,
    }
}

/// 绘制标题与树体。
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
    tree::draw(
        grid,
        rect,
        scroll,
        row_index,
        &layout(call, rect.width, expanded).0.rows,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::SubagentDetailView;
    use crate::chat::agent::{AgentRole, AgentStatusView};

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

    fn wait_view() -> SubagentDetailView {
        SubagentDetailView {
            role: None,
            title: None,
            task: None,
            agent_id: None,
            agent_status: None,
            targets: vec!["a1".into(), "a2".into()],
            all: false,
            mode: WaitMode::All,
            timeout_ms: None,
            timed_out: false,
            agents: vec![
                AgentStatusView {
                    agent_id: "a1".into(),
                    title: "调查事件流".into(),
                    role: AgentRole::Explorer,
                    status: "completed".into(),
                    report: None,
                },
                AgentStatusView {
                    agent_id: "a2".into(),
                    title: "检查存储层".into(),
                    role: AgentRole::Explorer,
                    status: "running".into(),
                    report: None,
                },
            ],
            report: None,
            closed_status: None,
        }
    }

    fn flat(call: &ToolCallView, width: u16, expanded: bool) -> String {
        let height = rows(call, width, expanded).max(1) as u16;
        let rect = Rect::new(0, 0, width, height);
        let mut grid = Grid::new(rect);
        let mut row = 0;
        draw(&mut grid, rect, 0, &mut row, call, 0, expanded, false);
        assert_eq!(row, rows(call, width, expanded));
        let mut out = String::new();
        for y in 0..height {
            for x in 0..width {
                let glyph = grid.cell(x, y);
                if glyph.ch != crate::ui::main_content::lx::grid::CONTINUATION {
                    out.push(glyph.ch);
                }
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn wait_conditions_stay_in_the_title() {
        let call = call("wait_agent", wait_view());
        let text = flat(&call, 60, false);
        let title = text.lines().next().unwrap_or("");
        assert!(title.contains("Wait agents"), "{text}");
        assert!(title.contains("2 targets"), "调用条件在标题右侧：{text}");
        assert!(title.contains("all"), "{text}");
        assert!(title.contains("until done"), "{text}");
        let body: String = text.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(body.contains("explorer · 调查事件流"), "{text}");
    }

    #[test]
    fn wait_links_point_at_target_rows() {
        let call = call("wait_agent", wait_view());
        let links = agent_links(&call, 60);
        assert_eq!(links.len(), 2);
        let text: Vec<String> = flat(&call, 60, false).lines().map(str::to_string).collect();
        assert!(text[links[0].0].contains("调查事件流"), "{text:?}");
        assert!(text[links[1].0].contains("检查存储层"), "{text:?}");
    }

    #[test]
    fn expanded_report_adds_report_nodes() {
        let mut view = wait_view();
        view.agents[0].report = Some("事件流已确认".into());
        let call = call("wait_agent", view);
        let collapsed = rows(&call, 60, false);
        let expanded = rows(&call, 60, true);
        assert!(expanded > collapsed);
        let text = flat(&call, 60, true);
        assert!(text.contains(text::agent::REPORT_LABEL), "{text}");
        assert!(text.contains("事件流已确认"), "{text}");
    }

    /// 报告长行按列宽折行，几何与绘制严格一致；不出现 `·` 拼接。
    #[test]
    fn long_report_wraps_and_geometry_holds() {
        let mut view = wait_view();
        view.agents[0].report = Some(format!("事件流确认{}", "长".repeat(200)));
        let call = call("wait_agent", view);
        for width in [10u16, 20, 40, 80] {
            let text = flat(&call, width, true);
            for line in text.lines() {
                assert!(
                    line.chars().count() <= usize::from(width),
                    "宽 {width} 越界：{line:?}"
                );
            }
            assert!(text.contains("Repor"), "宽 {width} 缺报告标题：{text}");
        }
    }

    #[test]
    fn spawn_and_close_use_role_dot_separator() {
        let spawn = SubagentDetailView {
            role: Some(AgentRole::Explorer),
            title: Some("调查".into()),
            task: None,
            agent_id: Some("a1".into()),
            agent_status: Some("running".into()),
            targets: Vec::new(),
            all: false,
            mode: WaitMode::All,
            timeout_ms: None,
            timed_out: false,
            agents: Vec::new(),
            report: None,
            closed_status: None,
        };
        let text = flat(&call("spawn_agent", spawn), 60, false);
        assert!(text.contains("explorer · 调查"), "{text}");

        let close = SubagentDetailView {
            role: None,
            title: None,
            task: None,
            agent_id: Some("a1".into()),
            agent_status: None,
            targets: Vec::new(),
            all: false,
            mode: WaitMode::All,
            timeout_ms: None,
            timed_out: false,
            agents: Vec::new(),
            report: None,
            closed_status: Some("running".into()),
        };
        let text = flat(&call("close_agent", close), 60, true);
        assert!(text.contains("a1"), "{text}");
        assert!(!text.contains('·'), "{text}");
        assert!(text.contains("previous: running"), "{text}");
    }
}

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
use crate::ui::{style, text};

use super::super::{Grid, StepMark, visible_row};
use super::tree::{self, Node, Section};
use super::{
    Segment, TEXT_SAMPLE_CHARS, header_with_mark, header_with_verb, mark_of, muted_markdown_lines,
    put_segments, single_line,
};

/// 标题动词（按工具名）。
pub(super) fn verb(name: &str) -> &'static str {
    match name {
        "spawn_agent" => text::agent::VERB_TOOL_SPAWN_AGENT,
        "send_input" => text::agent::VERB_TOOL_SEND_INPUT,
        "interrupt_agent" => text::agent::VERB_TOOL_INTERRUPT_AGENT,
        "resume_agent" => text::agent::VERB_TOOL_RESUME_AGENT,
        "wait_agent" => text::agent::VERB_TOOL_WAIT_AGENTS,
        "close_agent" => text::agent::VERB_TOOL_CLOSE_AGENT,
        "list_agents" => text::agent::VERB_TOOL_LIST_AGENTS,
        "cancel_agent_input" => text::agent::VERB_TOOL_CANCEL_AGENT_INPUT,
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

/// `Turing [explorer] · title`；无昵称时回退 `explorer · title`，不展示 task 原文。
fn agent_line(view: &SubagentDetailView) -> String {
    let role = view.role.map_or("", |role| role.as_str());
    let identity = crate::chat::agent::format_agent_identity(view.nickname.as_deref(), role);
    let title = view
        .title
        .as_deref()
        .map(|title| single_line(title, TEXT_SAMPLE_CHARS))
        .unwrap_or_default();
    let agent_id = view.agent_id.as_deref().unwrap_or_default();
    let name = if identity.is_empty() {
        if agent_id.is_empty() {
            text::agent::AGENTS_LABEL.to_string()
        } else {
            agent_id.to_string()
        }
    } else {
        identity
    };
    let mut parts = vec![name];
    if !title.is_empty() {
        parts.push(title);
    }
    if let Some(model) = view.model_label.as_deref().filter(|s| !s.is_empty()) {
        parts.push(model.to_string());
    }
    if view.fork_context {
        parts.push(text::agent::FORKED_CONTEXT_TAG.to_string());
    }
    parts.join(text::agent::SPAWN_ROLE_SEP)
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

/// 构建结果：节点 + 立即可见的链接（节点下标 → agent_id）+ 一级节点数。
struct Built {
    nodes: Vec<Node>,
    links: Vec<(usize, String)>,
    root_nodes: usize,
}

impl Built {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            links: Vec::new(),
            root_nodes: 1,
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

/// 目标行节点：`■ Turing [role] · title`（静态状态色）。
fn target_node(view: &SubagentDetailView, agent_id: &str) -> Node {
    let status = view.agents.iter().find(|agent| {
        crate::chat::agent::matches_agent_target(
            &agent.agent_id,
            agent.nickname.as_deref(),
            agent.agent_path.as_deref(),
            crate::chat::agent::ROOT_AGENT_PATH,
            agent_id,
        )
    });
    let color = status.map_or(WaitColor::Yellow, |agent| status_color(&agent.status));
    let body = match status {
        Some(agent) => {
            let identity = crate::chat::agent::format_agent_identity(
                agent.nickname.as_deref(),
                agent.role.as_str(),
            );
            let mut parts = vec![identity];
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

/// 展开态报告：`Report:` + 压暗 markdown 正文，整块一个多行节点。
fn report_node(body: &str, changed_paths: &[String], width: u16, nested: bool) -> Node {
    let cols = if nested {
        tree::nested_content_width(width)
    } else {
        tree::content_width(width)
    };
    let body_width = u16::try_from(cols).unwrap_or(1);
    let mut lines: Vec<Vec<Segment>> = vec![vec![Segment::new(
        text::agent::REPORT_LABEL,
        style::muted(),
    )]];
    if !changed_paths.is_empty() {
        lines.push(vec![Segment::new(
            format!(
                "{} {}",
                text::agent::CHANGED_FILES_LABEL,
                changed_paths.join(", ")
            ),
            style::muted(),
        )]);
    }
    lines.extend(muted_markdown_lines(body, body_width));
    Node::block(lines)
}

/// 失败/中断原因节点：工具结算异常且带错误摘要时补一行原因，避免只剩一个色块。
fn error_node(call: &ToolCallView) -> Option<Node> {
    if !matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
        return None;
    }
    let error = call
        .error
        .as_deref()
        .map(str::trim)
        .filter(|error| !error.is_empty())?;
    let style = match call.status {
        ToolStatus::Interrupted => style::agent_warn(),
        _ => style::agent_fail(),
    };
    Some(Node::text(single_line(error, TEXT_SAMPLE_CHARS), style))
}

/// 卡片节点与链接。
fn built(call: &ToolCallView, width: u16, expanded: bool, in_group: bool) -> Built {
    let Some(view) = view(call) else {
        let mut built = Built::new();
        if let Some(node) = error_node(call) {
            built.nodes.push(node);
        }
        return built;
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
                built.nodes.push(report_node(
                    &report.body,
                    &report.changed_paths,
                    width,
                    in_group,
                ));
            }
        }
        "wait_agent" => {
            let display_ids: Vec<String> = if !view.all && !view.targets.is_empty() {
                view.targets.clone()
            } else {
                view.agents
                    .iter()
                    .map(|agent| agent.agent_id.clone())
                    .collect()
            };
            if display_ids.is_empty() {
                let color = all_color(view, call.status);
                built.nodes.push(Node::new(vec![
                    Segment::new(format!("{} ", text::agent::STEP_MARK_FILLED), color.style()),
                    Segment::new(text::agent::WAIT_ALL.to_string(), style::text()),
                ]));
            } else {
                for target in &display_ids {
                    let index = built.nodes.len();
                    built.nodes.push(target_node(view, target));
                    built.links.push((index, target.clone()));
                }
            }
            built.root_nodes = built.nodes.len().max(1);
            if expanded {
                for agent in view.agents.iter().filter(|agent| {
                    agent
                        .report
                        .as_deref()
                        .is_some_and(|b| !b.trim().is_empty())
                }) {
                    if let Some(body) = agent.report.as_deref() {
                        built.nodes.push(report_node(body, &[], width, in_group));
                    }
                }
            }
        }
        "list_agents" => {
            if view.agents.is_empty() {
                built.nodes.push(Node::text(
                    text::LX_TASK_AGENTS_EMPTY,
                    style::agent_step_detail(),
                ));
            } else {
                for agent in &view.agents {
                    let index = built.nodes.len();
                    built.nodes.push(target_node(view, &agent.agent_id));
                    built.links.push((index, agent.agent_id.clone()));
                }
            }
            built.root_nodes = built.nodes.len().max(1);
        }
        _ => {
            // close / interrupt_agent / send_input / resume_agent / cancel_agent_input：目标行 + 状态。
            let target = if view.role.is_some() || view.title.is_some() || view.nickname.is_some() {
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
            if let Some(agent_id) = view.agent_id.as_ref().filter(|s| !s.is_empty())
                && call.name != "cancel_agent_input"
            {
                built.links.push((0, agent_id.clone()));
            }
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
    if let Some(node) = error_node(call) {
        built.nodes.push(node);
    }
    built
}

/// 排版（无折叠）：`rows`、`draw`、链接几何共用。
fn layout(call: &ToolCallView, width: u16, expanded: bool) -> (tree::Layout, Vec<(usize, String)>) {
    let (layout, links, _) = layout_with_group(call, width, expanded, false);
    (layout, links)
}

fn layout_with_group(
    call: &ToolCallView,
    width: u16,
    expanded: bool,
    in_group: bool,
) -> (tree::Layout, Vec<(usize, String)>, usize) {
    let in_nested_group = in_group && call.name != "list_agents";
    let built = built(call, width, expanded, in_nested_group);
    let root_nodes = built.root_nodes;
    let layout = if in_nested_group {
        tree::layout_in_group(&[Section::fixed(built.nodes)], width, root_nodes)
    } else {
        tree::layout(&[Section::fixed(built.nodes)], width)
    };
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
    (layout, links, root_nodes)
}

/// 行数（宽度 0 时为 0）。
#[cfg(test)]
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    rows_with_group(call, width, expanded, false)
}

pub(super) fn rows_with_group(
    call: &ToolCallView,
    width: u16,
    expanded: bool,
    in_group: bool,
) -> usize {
    if width == 0 {
        return 0;
    }
    super::HEADER_ROWS
        + layout_with_group(call, width, expanded, in_group)
            .0
            .rows
            .len()
}

/// 展开后行数增加即可展开（报告正文 / close 详情）。
pub(super) fn has_more_with_group(call: &ToolCallView, width: u16, in_group: bool) -> bool {
    width > 0
        && rows_with_group(call, width, true, in_group)
            > rows_with_group(call, width, false, in_group)
}

/// 卡片内 agent 链接的（相对卡片首行偏移, agent_id）。
#[cfg(test)]
pub(super) fn agent_links(call: &ToolCallView, width: u16) -> Vec<(usize, String)> {
    agent_links_with_group(call, width, false)
}

pub(super) fn agent_links_with_group(
    call: &ToolCallView,
    width: u16,
    in_group: bool,
) -> Vec<(usize, String)> {
    if width == 0 {
        return Vec::new();
    }
    layout_with_group(call, width, false, in_group).1
}

/// spawn 标题方块跟子 agent 状态走，而不是“spawn 工具已返回”的调用状态。
fn agent_mark(view: &SubagentDetailView, call: &ToolCallView, loading_frame: usize) -> StepMark {
    match view.agent_status.as_deref() {
        Some("completed" | "running" | "pending") => StepMark::Done,
        Some("blocked" | "interrupted") => StepMark::Interrupted,
        Some("failed") => StepMark::Failed,
        Some("cancelled") => StepMark::Cancelled,
        _ => mark_of(call.status, loading_frame),
    }
}

/// 标题状态：spawn/send_input/resume/cancel_agent_input 显示状态；wait 显示调用条件；close 无。
pub(super) fn header_status(call: &ToolCallView, view: &SubagentDetailView) -> Option<String> {
    match call.name.as_str() {
        "spawn_agent" | "send_input" | "resume_agent" | "cancel_agent_input" => Some(
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

/// 合并组内调用体：跳过标题，状态追加到首个树节点尾部。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_body(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    _loading_frame: usize,
    expanded: bool,
    _animating: bool,
    last: bool,
) {
    if rect.width == 0 {
        return;
    }
    let in_nested_group = call.name != "list_agents";
    let (mut layout, _, root_nodes) =
        layout_with_group(call, rect.width, expanded, in_nested_group);
    let status = view(call).and_then(|view| header_status(call, view));
    tree::decorate_group_call(&mut layout.rows, call, status, rect.width);
    if in_nested_group {
        tree::apply_group_connector_with_roots(&mut layout.rows, last, root_nodes);
    }
    tree::draw(grid, rect, scroll, row_index, &layout.rows);
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
            nickname: None,
            title: None,
            task: None,
            agent_id: None,
            agent_path: None,
            live_activity: None,
            agent_status: None,
            model_label: None,
            fork_context: false,
            targets: vec!["a1".into(), "a2".into()],
            all: false,
            mode: WaitMode::All,
            timeout_ms: None,
            timed_out: false,
            agents: vec![
                AgentStatusView {
                    agent_id: "a1".into(),
                    nickname: None,
                    agent_path: None,
                    title: "调查事件流".into(),
                    role: AgentRole::Explorer,
                    status: "completed".into(),
                    stopped_by_user: false,
                    report: None,
                },
                AgentStatusView {
                    agent_id: "a2".into(),
                    nickname: None,
                    agent_path: None,
                    title: "检查存储层".into(),
                    role: AgentRole::Explorer,
                    status: "running".into(),
                    stopped_by_user: false,
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
            nickname: None,
            title: Some("调查".into()),
            task: None,
            agent_id: Some("a1".into()),
            agent_path: None,
            live_activity: None,
            agent_status: Some("running".into()),
            model_label: None,
            fork_context: false,
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
            nickname: None,
            title: None,
            task: None,
            agent_id: Some("a1".into()),
            agent_path: None,
            live_activity: None,
            agent_status: None,
            model_label: None,
            fork_context: false,
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

    #[test]
    fn spawn_card_shows_nickname_model_and_fork_tag() {
        let spawn = SubagentDetailView {
            role: Some(AgentRole::Explorer),
            nickname: Some("Turing".into()),
            title: Some("investigate event flow".into()),
            task: None,
            agent_id: Some("a1".into()),
            agent_path: None,
            live_activity: None,
            agent_status: Some("running".into()),
            model_label: Some("openai/gpt-4.1-mini (low)".into()),
            fork_context: true,
            targets: Vec::new(),
            all: false,
            mode: WaitMode::All,
            timeout_ms: None,
            timed_out: false,
            agents: Vec::new(),
            report: None,
            closed_status: None,
        };
        let text = flat(&call("spawn_agent", spawn), 100, false);
        assert!(
            text.contains(
                "Turing [explorer] · investigate event flow · \
                 openai/gpt-4.1-mini (low) · forked"
            ),
            "{text}"
        );
    }

    #[test]
    fn interrupt_card_shows_previous_status_and_paused_inputs() {
        let interrupt = SubagentDetailView {
            role: None,
            nickname: Some("Turing".into()),
            title: None,
            task: None,
            agent_id: Some("a1".into()),
            agent_path: None,
            live_activity: None,
            agent_status: None,
            model_label: None,
            fork_context: false,
            targets: Vec::new(),
            all: false,
            mode: WaitMode::All,
            timeout_ms: None,
            timed_out: false,
            agents: Vec::new(),
            report: None,
            closed_status: Some("running · 1 paused".into()),
        };
        let text = flat(&call("interrupt_agent", interrupt), 60, false);
        assert!(text.contains("Interrupt agent"), "{text}");
        assert!(text.contains("Turing"), "{text}");
        assert!(text.contains("previous: running · 1 paused"), "{text}");
    }

    #[test]
    fn expanded_subagent_report_uses_muted_markdown_instead_of_bright_text() {
        let mut view = wait_view();
        view.agents[0].report = Some("found `target_fn` in module".into());
        let call = call("wait_agent", view);
        let (tree, _) = layout(&call, 80, true);
        let report_label = tree
            .rows
            .iter()
            .flat_map(|row| row.segments.iter())
            .find(|seg| seg.text.contains("Report:"))
            .expect("Report label");
        assert_eq!(report_label.style, style::muted());
        let plain_seg = tree
            .rows
            .iter()
            .flat_map(|row| row.segments.iter())
            .find(|seg| seg.text.contains("found "))
            .expect("plain segment");
        assert_eq!(plain_seg.style, style::muted());
        let code_seg = tree
            .rows
            .iter()
            .flat_map(|row| row.segments.iter())
            .find(|seg| seg.text == "target_fn")
            .expect("inline code segment");
        assert_eq!(
            code_seg.style,
            style::md_read_inline_code().patch(style::muted())
        );
    }

    #[test]
    fn failed_card_appends_the_error_reason() {
        let mut call = call("spawn_agent", wait_view());
        call.status = ToolStatus::Failed;
        call.error = Some("model not found: ghost".into());
        let text = flat(&call, 80, false);
        assert!(text.contains("model not found: ghost"), "{text}");
    }

    #[test]
    fn interrupted_agent_status_uses_the_interrupted_mark() {
        let mut view = wait_view();
        view.agent_status = Some("interrupted".into());
        let call = call("spawn_agent", view.clone());
        assert_eq!(agent_mark(&view, &call, 0), StepMark::Interrupted);
    }

    #[test]
    fn wait_all_renders_every_agent_row_and_link() {
        let mut view = wait_view();
        view.targets.clear();
        view.all = true;
        let call = call("wait_agent", view);
        let text = flat(&call, 60, false);
        assert!(text.contains("调查事件流"), "{text}");
        assert!(text.contains("检查存储层"), "{text}");
        assert!(
            !text.lines().any(|line| line.contains("All")),
            "有目标时必须逐行渲染，不再退回单行 All：{text}"
        );
        let links = agent_links(&call, 60);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].1, "a1");
        assert_eq!(links[1].1, "a2");
    }

    #[test]
    fn spawn_card_never_shows_live_activity() {
        let mut view = SubagentDetailView::from_arguments(
            "spawn_agent",
            r#"{"role":"explorer","task":"investigate","title":"investigate"}"#,
        )
        .expect("spawn detail");
        view.agent_id = Some("a1".into());
        view.agent_status = Some("running".into());
        view.live_activity = Some("$ cargo test".into());
        let text = flat(&call("spawn_agent", view.clone()), 80, false);
        assert!(
            !text.contains("$ cargo test"),
            "运行中不显示当前步骤：{text}"
        );
        assert!(text.contains("└─"), "单节点以 └─ 结尾：{text}");

        view.agent_status = Some("completed".into());
        let text = flat(&call("spawn_agent", view), 80, false);
        assert!(!text.contains("$ cargo test"), "完成后收起：{text}");
        assert!(text.contains("└─"), "完成态保持单节点 └─：{text}");
    }

    #[test]
    fn expanded_report_lists_changed_paths() {
        let mut view = SubagentDetailView::from_arguments(
            "spawn_agent",
            r#"{"role":"executor","task":"edit","title":"edit"}"#,
        )
        .expect("spawn detail");
        view.agent_id = Some("a1".into());
        view.agent_status = Some("completed".into());
        view.report = Some(crate::chat::agent::SubagentReportView {
            agent_id: "a1".into(),
            nickname: None,
            title: "edit".into(),
            role: AgentRole::Executor,
            status: "completed".into(),
            files_changed: 2,
            changed_paths: vec!["src/a.rs".into(), "src/b.rs".into()],
            duration_ms: Some(1_000),
            body: "done".into(),
            delivered_at_ms: 1,
        });
        let text = flat(&call("spawn_agent", view), 80, true);
        assert!(
            text.contains("Changed files: src/a.rs, src/b.rs"),
            "变更文件明细在报告正文上方：{text}"
        );
    }
}

//! 工具步骤标题行（`■ 动词 · 状态`）与分段折行/网格写入原语。

use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{ToolCallView, ToolStatus};
use crate::ui::{style, text};

use super::super::super::Grid;
use super::super::{StepKind, StepMark, truncate_with_ellipsis};
use super::{MAX_FALLBACK_PARSE_BYTES, TEXT_SAMPLE_CHARS, file_summary, subagent};

/// 行内一段着色文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) table: Option<crate::ui::markdown_render::TableLineRef>,
    pub(crate) link: Option<String>,
}

impl Segment {
    pub(in crate::ui::main_content::lx::history) fn new(
        text: impl Into<String>,
        style: Style,
    ) -> Self {
        Self {
            text: text.into(),
            style,
            table: None,
            link: None,
        }
    }

    /// 该段的显示列数（宽字符按两列）。
    pub(in crate::ui::main_content::lx::history) fn cols(&self) -> usize {
        self.text.width()
    }
}

/// 若干段的显示列数合计。
pub(in crate::ui::main_content::lx::history) fn segments_cols(segments: &[Segment]) -> usize {
    segments.iter().map(Segment::cols).sum()
}

/// 四文件工具的展示种类；未知工具名走 [`Kind::Other`]（动词位置显示工具名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui::main_content::lx::history) enum Kind {
    Read,
    Search,
    Edit,
    Write,
    Other,
}

impl Kind {
    /// 工具名 → 展示种类（只认四工具稳定英文名）。
    pub(in crate::ui::main_content::lx::history) fn of(name: &str) -> Self {
        match name {
            "read" => Self::Read,
            "search" => Self::Search,
            "edit" => Self::Edit,
            "write" => Self::Write,
            _ => Self::Other,
        }
    }
}

// ===== 第 1 行：`■ 动词` =====

/// 固定动词（卡片 title 主体）：运行中橙黄、已完成翡翠绿。
///
/// 用户要求「方块颜色和工具 title 统一为黄色」：动词必须与方块同色，
/// 否则一张卡片两种颜色。
pub(in crate::ui::main_content::lx::history) fn verb_label(call: &ToolCallView) -> String {
    use text::agent as a;
    match Kind::of(&call.name) {
        Kind::Read => a::VERB_TOOL_READ.to_string(),
        Kind::Search => a::VERB_TOOL_SEARCH.to_string(),
        Kind::Edit => a::VERB_TOOL_EDIT.to_string(),
        Kind::Write => a::VERB_TOOL_WRITE.to_string(),
        Kind::Other => match call.name.as_str() {
            "curr_time" => a::VERB_TOOL_CURR_TIME.to_string(),
            "exec_command" => a::VERB_EXEC_COMMAND.to_string(),
            "write_stdin" => write_stdin_fallback_verb(&call.arguments_json).to_string(),
            "update_plan" => a::VERB_UPDATE_PLAN.to_string(),
            "ask" => a::VERB_TOOL_ASK.to_string(),
            "web_search" => a::VERB_TOOL_WEB_SEARCH.to_string(),
            "spawn_agent" | "send_input" | "interrupt_agent" | "resume_agent" | "wait_agent"
            | "close_agent" | "list_agents" | "cancel_agent_input" => {
                subagent::verb(&call.name).to_string()
            }
            name if name.starts_with(crate::mcp::MCP_PREFIX) => {
                if let Some((server, tool)) = name
                    .strip_prefix(crate::mcp::MCP_PREFIX)
                    .and_then(|rest| rest.split_once("__"))
                {
                    format!("{server} · {tool}")
                } else {
                    single_line(name, TEXT_SAMPLE_CHARS)
                }
            }
            _ => single_line(&call.name, TEXT_SAMPLE_CHARS),
        },
    }
}

/// `write_stdin` 在尚未生成 `ToolDetailView::Interaction` 时，按参数 JSON 推导显示动词。
fn write_stdin_fallback_verb(arguments_json: &str) -> &'static str {
    if arguments_json.len() <= MAX_FALLBACK_PARSE_BYTES
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments_json)
    {
        if value
            .get("terminate")
            .or_else(|| value.get("kill"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return text::agent::VERB_STOP_COMMAND;
        }
        if value
            .get("chars")
            .or_else(|| value.get("input"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| !text.is_empty())
        {
            return text::agent::VERB_SEND_INPUT;
        }
    }
    text::agent::VERB_READ_OUTPUT
}

/// 组装第 1 行（`■ 动词 · 状态`）：状态紧跟动词，与进程/委派卡片同一读法。
///
/// 失败/中断整行红/黄；`read`/`search` 的种类、条目数与命中数由
/// [`file_summary::title_status`] 提供。
pub(in crate::ui::main_content::lx::history) fn header(
    call: &ToolCallView,
    width: u16,
    loading_frame: usize,
    animating: bool,
) -> Vec<Segment> {
    header_with_verb(
        call,
        width,
        loading_frame,
        animating,
        &verb_label(call),
        file_summary::title_status(call),
    )
}

// ===== 第 2 行：`  └─ 详情` =====

/// 工具状态 → 行首方块标记（子模块共用）。
pub(in crate::ui::main_content::lx::history) fn mark_of(
    status: ToolStatus,
    _loading_frame: usize,
) -> StepMark {
    match status {
        ToolStatus::Failed => StepMark::Failed,
        ToolStatus::Interrupted => StepMark::Interrupted,
        ToolStatus::Cancelled => StepMark::Cancelled,
        ToolStatus::Completed => StepMark::Done,
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running => StepMark::Running,
    }
}

/// 标题行：`■ 动词` +（宽度允许时）紧跟动词的状态文字。
///
/// 状态**不再右对齐**，而是紧贴动词、用 ` · ` 分隔（`■ Run command · Running`）：
/// 右上角原来的 `[details]` 已移除，展开/收起改由树体末尾的可点击行承担
/// （`command::toggle_row`）；状态若仍右对齐，就会孤零零贴在行的最右端、与动词断开。
///
/// 状态只在**整段放得下**时画（窄屏整项丢弃，绝不截半段）；极窄时保留方块与动词
/// ——方块是最窄档唯一的状态信号。
pub(in crate::ui::main_content::lx::history) fn header_with_verb(
    call: &ToolCallView,
    width: u16,
    loading_frame: usize,
    animating: bool,
    verb: &str,
    status: Option<String>,
) -> Vec<Segment> {
    let mark = mark_of(call.status, loading_frame);
    header_with_mark(call, width, loading_frame, animating, verb, status, mark)
}

/// 同上，但方块状态由调用方指定。
///
/// 进程卡片需要它：命令已经交回且工具结算为 `completed`，但**进程本身仍在跑**时，
/// 方块应继续呼吸（`Running`），不能被工具的终态覆盖。
pub(in crate::ui::main_content::lx::history) fn header_with_mark(
    call: &ToolCallView,
    width: u16,
    loading_frame: usize,
    animating: bool,
    verb: &str,
    status: Option<String>,
    mark: StepMark,
) -> Vec<Segment> {
    // 终态整行同色（方块 + 动词）：失败红、中断黄。取消不是错误态，保持工具色
    // （方块与 title 统一；详情行的 `cancelled` 已经说明它没有产出结果）。
    let tint = match call.status {
        ToolStatus::Failed => Some(style::agent_fail()),
        ToolStatus::Interrupted => Some(style::agent_warn()),
        _ => None,
    };
    header_with_tint(tint, width, loading_frame, animating, verb, status, mark)
}

/// 同上，但整行语义色由调用方指定（合并组头取组内最差状态）。
pub(in crate::ui::main_content::lx::history) fn header_with_tint(
    tint: Option<Style>,
    width: u16,
    loading_frame: usize,
    animating: bool,
    verb: &str,
    status: Option<String>,
    mark: StepMark,
) -> Vec<Segment> {
    let limit = usize::from(width);
    let paint = |segment: Segment| match tint {
        Some(tint) => Segment::new(segment.text, tint),
        None => segment,
    };
    let lead = paint(Segment::new(
        mark.segment(StepKind::Tool, loading_frame, animating),
        mark.style(StepKind::Tool, false),
    ));
    if segments_cols(std::slice::from_ref(&lead)) > limit {
        return vec![lead];
    }
    let running = matches!(mark, StepMark::Running | StepMark::Alive);
    let verb = paint(Segment::new(
        verb.to_string(),
        if running {
            style::chat_tool_verb_running()
        } else {
            style::chat_tool_verb()
        },
    ));
    let mut segments = vec![lead, verb];
    // 动词整段放不下时尾部省略（不挤掉行首方块）。
    while segments_cols(&segments) > limit && segments.len() > 1 {
        let remaining = limit.saturating_sub(segments[0].cols());
        let text = truncate_with_ellipsis(&segments[1].text, remaining);
        if text.is_empty() {
            segments.truncate(1);
        } else {
            segments[1] = Segment::new(text, segments[1].style);
        }
    }
    // 状态紧跟动词：`■ 动词 · 状态`（够宽才画，且必须落在动词之后）。
    if let Some(status) = status
        && segments.len() == 2
    {
        // 运行中的状态词与方块/动词同档橙黄（设计规范：进程存活时「动词与状态
        // 文字同色不加粗」）；终态保持 dim——失败/中断已由 `tint` 整行覆盖。
        let status_style = if running {
            style::chat_step_label_alive()
        } else {
            style::agent_step_detail()
        };
        let tail = Segment::new(
            format!("{}{status}", text::LX_CHAT_TOOL_STATUS_SEP),
            status_style,
        );
        if segments_cols(&segments) + tail.cols() <= limit {
            segments.push(tail);
        }
    }
    segments
}

/// 逐段写入网格，整体不越过 `width` 列（宽字符不半写）。
pub(in crate::ui::main_content::lx::history) fn put_segments(
    grid: &mut Grid,
    x: u16,
    y: u16,
    width: u16,
    segments: &[Segment],
) {
    let right = x.saturating_add(width);
    let mut cx = x;
    for segment in segments {
        let remaining = right.saturating_sub(cx);
        if remaining == 0 {
            return;
        }
        if let Some(ref line_ref) = segment.table {
            grid.set_table_row(cx, y, line_ref.clone());
        }
        let cols = u16::try_from(segment.cols()).unwrap_or(u16::MAX);
        if let Some(ref url) = segment.link {
            grid.set_link_span(cx, y, cols.min(remaining), url);
        }
        grid.put_clipped(cx, y, &segment.text, segment.style, remaining);
        cx = cx.saturating_add(cols);
    }
}

/// 按显示列换行，同时保留分段样式：跨行处切开当前分段，不丢样式。
///
/// 差异正文（语法高亮）与工具参数（`└─` 详情行）共用这一份实现：两处都要
/// 「按列折行且不打断着色」，各写一遍必然一边宽字符算错一边样式丢失。
pub(in crate::ui::main_content::lx::history) fn wrap_spans(
    spans: &[Segment],
    width: usize,
) -> Vec<Vec<Segment>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Segment>> = Vec::new();
    let mut current: Vec<Segment> = Vec::new();
    let mut col = 0usize;
    for source in spans {
        let mut buf = String::new();
        let mut pending_table = source.table.clone();
        for ch in source.text.chars() {
            let cell = ch.width().unwrap_or(1).max(1);
            if col + cell > width && col > 0 {
                if !buf.is_empty() {
                    let mut seg = Segment::new(std::mem::take(&mut buf), source.style);
                    seg.table = pending_table.take();
                    seg.link.clone_from(&source.link);
                    current.push(seg);
                }
                rows.push(std::mem::take(&mut current));
                col = 0;
            }
            buf.push(ch);
            col += cell;
        }
        if !buf.is_empty() {
            let mut seg = Segment::new(buf, source.style);
            seg.table = pending_table.take();
            seg.link.clone_from(&source.link);
            current.push(seg);
        }
    }
    rows.push(current);
    if rows.len() > 1 {
        for row in &mut rows {
            for seg in row {
                seg.table = None;
            }
        }
    }
    rows
}

/// 单行化并限长：控制字符（含换行/制表）转空格，去掉首尾空白。
pub(in crate::ui::main_content::lx::history) fn single_line(
    value: &str,
    max_chars: usize,
) -> String {
    value
        .chars()
        .take(max_chars)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .trim()
        .to_string()
}

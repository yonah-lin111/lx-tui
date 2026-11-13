//! 文件工具摘要树体：参数、计数与截断事实各自成节点。
//!
//! ```text
//! ■ Read · folder · 12 files
//!   └─ /project/src
//!
//! ■ Search · 3 matches
//!   └─ "package.json|Cargo.toml" in /project
//! ```
//!
//! `read` 的种类与目录条目数在标题右侧（`Read · file` / `Read · image` / `Read · folder · 12 files`），
//! 树体只放单行路径（或失败/中断原因）；`search` 的命中数也在标题右侧。
//!
//! - 独立字段之间不再用 ` · ` 拼接：每个字段是一个 [`tree::Node`]；
//! - `read` 的 file/folder 只来自**结果体**（[`ToolSummary::read_kind`]），不靠路径猜；
//! - `read` 标题下方只显示一条路径（超宽单行尾部省略）；结果体确认后追加
//!   `:12-40 · 29 lines`，不显示 `output limited` 或 `more lines`；
//! - 渲染热路径不做常规 JSON 解析：只有摘要缺字段时才懒回退解析 `arguments_json`。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{ToolCallView, ToolStatus, ToolSummary};
use crate::chat::tools::ReadKind;
use crate::ui::{style, text};

use super::super::Grid;
use super::tree::{self, Node, Preview, Section};
use super::{Kind, MAX_FALLBACK_PARSE_BYTES, Segment, TEXT_SAMPLE_CHARS, single_line};

/// 详情行**折叠态**最多显示的物理行数。
pub(crate) const DETAIL_PREVIEW_ROWS: usize = 2;
/// 命令输出等正文展开态的物理行上限（`edit` 参数与命令输出共用）。
pub(crate) const DETAIL_MAX_ROWS: usize = 24;

/// 第二行的内容类型：决定是否有参数可展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Detail {
    /// 失败 / 中断：**只放错误文案**，不显示任何参数。
    Error,
    /// 正常：参数 + 次要计数（可能为空 → 状态词兜底）。
    Params,
}

impl Detail {
    fn of(call: &ToolCallView) -> Self {
        if matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
            Self::Error
        } else {
            Self::Params
        }
    }
}

/// 单字段取值：摘要优先；缺字段且允许回退时才触发一次（共享的）解析。
fn field(
    call: &ToolCallView,
    may_parse: bool,
    cache: &mut Option<ToolSummary>,
    pick: fn(&ToolSummary) -> Option<String>,
) -> Option<String> {
    call.summary
        .as_deref()
        .and_then(pick)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            may_parse
                .then(|| pick(fallback_summary(call, cache)))
                .flatten()
                .filter(|value| !value.is_empty())
        })
}

/// 主参数：摘要里的路径 / 搜索模式；不铺文件内容或命中正文。
fn path_node(call: &ToolCallView, cache: &mut Option<ToolSummary>) -> Option<Node> {
    let may_parse = needs_argument_fallback(call, Kind::of(&call.name));
    let path = field(call, may_parse, cache, |summary| summary.path.clone())
        .map(|path| single_line(&path, TEXT_SAMPLE_CHARS))
        .filter(|path| !path.is_empty())?;
    Some(Node::text(path, style::chat_tool_param()))
}

/// `read` 的详情只有一条路径：按内容列单行尾部省略，不产生第二物理行。
///
/// 结果体确认过区间与实际行数时追加 `:12-40 · 29 lines`；运行中/旧记录只显示路径，
/// 参数侧的 `offset` / `limit` 不参与展示。
fn read_path_node(call: &ToolCallView, width: u16) -> Option<Node> {
    let mut cache: Option<ToolSummary> = None;
    let path = field(
        call,
        needs_argument_fallback(call, Kind::Read),
        &mut cache,
        |summary| summary.path.clone(),
    )
    .map(|path| single_line(&path, TEXT_SAMPLE_CHARS))
    .filter(|path| !path.is_empty())?;

    let available = tree::content_width(width);
    let suffix = read_line_suffix(call.summary.as_deref());
    let suffix_width = suffix.as_deref().map_or(0, UnicodeWidthStr::width);
    // 后缀放得下才保留；路径至少保留 1 列 + 省略符。
    let keep_suffix = suffix_width > 0 && available > suffix_width.saturating_add(1);
    let path_budget = if keep_suffix {
        available.saturating_sub(suffix_width)
    } else {
        available
    };
    let path = crate::ui::main_content::lx::history::truncate_with_ellipsis(&path, path_budget);
    if path.is_empty() {
        return None;
    }
    let mut spans = vec![Segment::new(path, style::chat_tool_param())];
    if keep_suffix && let Some(suffix) = suffix {
        spans.push(Segment::new(suffix, style::agent_step_detail()));
    }
    Some(Node::new(spans))
}

/// 结果体确认过的行区间与行数：`:12-40 · 29 lines`。
///
/// `read_lines` 只在结果体写入（参数侧不会产生），因此它缺失时连区间一起隐藏。
fn read_line_suffix(summary: Option<&ToolSummary>) -> Option<String> {
    let summary = summary?;
    let (start, end) = summary.range?;
    let lines = summary.read_lines?;
    if end < start {
        return None;
    }
    let range = if start == end {
        format!(":{start}")
    } else {
        format!(":{start}-{end}")
    };
    Some(format!(
        "{range}{}{lines} line{}",
        text::LX_CHAT_TOOL_STATUS_SEP,
        if lines == 1 { "" } else { "s" }
    ))
}

/// `search` 的模式 + 范围：一个节点（`"pattern" in path`）。
///
/// `in` 仍是独立着色段，但同属一个逻辑字段；窄宽度折行时按列切分，不丢样式。
fn search_scope_node(call: &ToolCallView, cache: &mut Option<ToolSummary>) -> Option<Node> {
    let may_parse = needs_argument_fallback(call, Kind::Search);
    let pattern = field(call, may_parse, cache, |summary| summary.pattern.clone())
        .map(|pattern| single_line(&pattern, TEXT_SAMPLE_CHARS))
        .filter(|pattern| !pattern.is_empty());
    let path = field(call, may_parse, cache, |summary| summary.path.clone())
        .map(|path| single_line(&path, TEXT_SAMPLE_CHARS))
        .filter(|path| !path.is_empty());
    match (pattern, path) {
        (Some(pattern), Some(path)) => Some(Node::new(vec![
            Segment::new(
                text::agent::pattern_label(&pattern),
                style::chat_tool_param(),
            ),
            Segment::new(
                format!(" {}", text::agent::IN_SCOPE_SEP),
                style::agent_step_detail(),
            ),
            Segment::new(path, style::chat_tool_param()),
        ])),
        (Some(pattern), None) => Some(Node::text(
            text::agent::pattern_label(&pattern),
            style::chat_tool_param(),
        )),
        // 没有 pattern（老数据）：退回单段路径。
        (None, Some(path)) => Some(Node::text(path, style::chat_tool_param())),
        (None, None) => None,
    }
}

/// 摘要是否答不出需要的主参数（只有这时才回退解析 `arguments_json`）。
fn needs_argument_fallback(call: &ToolCallView, kind: Kind) -> bool {
    let Some(summary) = call.summary.as_ref() else {
        return call.arguments_json.len() <= MAX_FALLBACK_PARSE_BYTES;
    };
    match kind {
        Kind::Search => summary.pattern.is_none(),
        Kind::Edit => summary.is_empty(),
        Kind::Read | Kind::Write | Kind::Other => summary.path.is_none() && summary.is_empty(),
    }
}

/// 摘要缺字段时的**懒**回退解析（同一帧内最多一次）。
fn fallback_summary<'a>(
    call: &ToolCallView,
    parsed: &'a mut Option<ToolSummary>,
) -> &'a ToolSummary {
    parsed.get_or_insert_with(|| {
        #[cfg(test)]
        FALLBACK_PARSES.with(|count| count.set(count.get() + 1));
        ToolSummary::from_arguments(&call.arguments_json)
    })
}

#[cfg(test)]
thread_local! {
    static FALLBACK_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn fallback_parse_count() -> usize {
    FALLBACK_PARSES.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(super) fn reset_fallback_parse_count() {
    FALLBACK_PARSES.with(|count| count.set(0));
}

/// 取消态：`cancelled`（dim）。
fn cancelled_node(call: &ToolCallView) -> Option<Node> {
    (call.status == ToolStatus::Cancelled).then(|| {
        Node::text(
            text::tool_status_label(ToolStatus::Cancelled),
            style::agent_step_detail(),
        )
    })
}

/// 兜底状态词（`receiving input` / `pending` / `running` / `cancelled`）。
fn fallback_node(call: &ToolCallView) -> Option<Node> {
    match call.status {
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running => Some(Node::text(
            text::tool_status_label(call.status),
            style::agent_step_detail(),
        )),
        // 取消/完成/失败/中断由事实节点或错误节点表达，不再重复状态词。
        ToolStatus::Cancelled
        | ToolStatus::Completed
        | ToolStatus::Failed
        | ToolStatus::Interrupted => None,
    }
}

/// 错误文案节点：仅失败/中断使用；失败时不显示参数与计数。
fn error_node(call: &ToolCallView) -> Option<Node> {
    if !matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
        return None;
    }
    let reason = single_line(call.error.as_deref()?, TEXT_SAMPLE_CHARS);
    (!reason.is_empty()).then(|| Node::text(reason, style::text()))
}

/// 文件工具标题右侧状态：`read` 给种类/目录条目数，`search` 给命中数。
///
/// 失败/中断只保留错误详情，标题不再重复参数。
pub(super) fn title_status(call: &ToolCallView) -> Option<String> {
    if matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
        return None;
    }
    match Kind::of(&call.name) {
        Kind::Read => read_kind_status(call.summary.as_deref()?),
        Kind::Search => {
            let summary = call.summary.as_deref()?;
            summary.matches.map(text::agent::matches_label)
        }
        Kind::Edit | Kind::Write | Kind::Other => None,
    }
}

/// `read` 结果种类：`file` / `image` / `folder · 12 files`。
fn read_kind_status(summary: &ToolSummary) -> Option<String> {
    match summary.read_kind? {
        ReadKind::File => Some(text::agent::READ_KIND_FILE.to_string()),
        ReadKind::Image => Some(text::agent::READ_KIND_IMAGE.to_string()),
        ReadKind::Directory => {
            let mut status = text::agent::READ_KIND_DIR.to_string();
            if let Some(count) = summary.total_lines {
                status.push_str(text::LX_CHAT_TOOL_STATUS_SEP);
                status.push_str(&text::agent::read_dir_count(count));
            }
            Some(status)
        }
    }
}

/// `search` 的参数节点（命中行由 `search.rs` 追加）。
fn search_param_nodes(call: &ToolCallView, cache: &mut Option<ToolSummary>) -> Vec<Node> {
    search_scope_node(call, cache).into_iter().collect()
}

/// `edit` / `write` 的主参数：`path (+12 -4)` 一个节点，计数紧跟路径。
fn change_param_nodes(call: &ToolCallView, cache: &mut Option<ToolSummary>) -> Vec<Node> {
    let path = field(
        call,
        needs_argument_fallback(call, Kind::of(&call.name)),
        cache,
        |summary| summary.path.clone(),
    )
    .map(|path| single_line(&path, TEXT_SAMPLE_CHARS))
    .filter(|path| !path.is_empty());
    let summary = call.summary.as_deref();
    let mut spans: Vec<Segment> = Vec::new();
    if let Some(path) = path {
        spans.push(Segment::new(path, style::chat_tool_param()));
    }
    if let Some(summary) = summary {
        if !summary.counts_known {
            if !spans.is_empty() {
                spans.push(Segment::new(" ", style::text()));
            }
            spans.push(Segment::new(
                format!("({})", text::agent::FILE_DIFF_COUNTS_UNKNOWN),
                style::agent_step_detail(),
            ));
        } else if let (Some(added), Some(removed)) = (summary.added, summary.removed) {
            if !spans.is_empty() {
                spans.push(Segment::new(" (", style::text()));
            } else {
                spans.push(Segment::new("(", style::text()));
            }
            spans.push(Segment::new(
                text::agent::added_label(added),
                style::agent_added(),
            ));
            spans.push(Segment::new(" ", style::text()));
            spans.push(Segment::new(
                text::agent::removed_label(removed),
                style::agent_removed(),
            ));
            spans.push(Segment::new(")", style::text()));
        }
    }
    if spans.is_empty() {
        Vec::new()
    } else {
        vec![Node::new(spans)]
    }
}

/// 参数节点（会被折叠的长字段）。
fn param_nodes(call: &ToolCallView, cache: &mut Option<ToolSummary>) -> Vec<Node> {
    let kind = Kind::of(&call.name);
    let mut nodes = match kind {
        Kind::Read => path_node(call, cache).into_iter().collect(),
        Kind::Search => search_param_nodes(call, cache),
        Kind::Edit | Kind::Write => change_param_nodes(call, cache),
        Kind::Other => path_node(call, cache).into_iter().collect(),
    };
    if nodes.is_empty()
        && let Some(node) = fallback_node(call)
    {
        nodes.push(node);
    }
    nodes
}

/// 执行阶段的**事实节点**：始终可见，不随预览折叠隐藏。
fn fact_nodes(call: &ToolCallView) -> Vec<Node> {
    let mut nodes = Vec::new();
    let kind = Kind::of(&call.name);
    let summary = call.summary.as_deref();
    match kind {
        Kind::Read => {}
        Kind::Search => {}
        Kind::Edit | Kind::Write => {
            if let Some(summary) = summary {
                if summary.partial {
                    nodes.push(Node::text(
                        text::agent::TOOL_PARTIAL,
                        style::agent_step_detail(),
                    ));
                }
                if summary.simplified {
                    nodes.push(Node::text(
                        text::agent::FILE_DIFF_SIMPLIFIED,
                        style::agent_step_detail(),
                    ));
                }
                if summary.truncated {
                    nodes.push(Node::text(
                        text::agent::FILE_DIFF_TRUNCATED,
                        style::agent_warn(),
                    ));
                }
                if summary.omitted_files > 0 {
                    nodes.push(Node::text(
                        text::agent::file_diff_omitted_files(summary.omitted_files),
                        style::agent_warn(),
                    ));
                }
            }
        }
        Kind::Other => {}
    }
    if let Some(node) = cancelled_node(call) {
        nodes.push(node);
    }
    nodes
}

/// 摘要节点（不含错误态与折叠）：各卡片共用同一份字段顺序。
pub(super) fn nodes(call: &ToolCallView) -> Vec<Node> {
    let mut cache: Option<ToolSummary> = None;
    if Detail::of(call) == Detail::Error {
        return error_node(call).into_iter().collect();
    }
    let mut nodes = param_nodes(call, &mut cache);
    nodes.extend(fact_nodes(call));
    nodes
}

/// 详情排版：长参数折叠 2 行，事实节点始终可见（展开显示全部已保存摘要）。
///
/// `read` 单体卡片遵循 `lx-message-block-display.md` §5.1：标题下方只显示一条路径（或一条错误/状态词），
/// 结果体确认后追加区间与行数；超宽时单行尾部省略，不软换行成第二行，不显示 `output limited`，
/// 也不登记可展开点击面。
fn detail_layout(call: &ToolCallView, width: u16, expanded: bool) -> tree::Layout {
    detail_layout_with_group(call, width, expanded, false)
}

fn detail_layout_with_group(
    call: &ToolCallView,
    width: u16,
    expanded: bool,
    in_group: bool,
) -> tree::Layout {
    if width == 0 {
        return tree::layout(&[], 0);
    }
    let build = |sections: &[Section]| {
        if in_group {
            tree::layout_in_group(sections, width, 1)
        } else {
            tree::layout(sections, width)
        }
    };
    let mut cache: Option<ToolSummary> = None;
    if Kind::of(&call.name) == Kind::Read {
        let body_width = tree::content_width(width);
        let node = if Detail::of(call) == Detail::Error {
            call.error
                .as_deref()
                .map(|reason| single_line(reason, TEXT_SAMPLE_CHARS))
                .filter(|reason| !reason.is_empty())
                .map(|reason| {
                    Node::text(
                        super::super::truncate_with_ellipsis(&reason, body_width),
                        style::text(),
                    )
                })
                .unwrap_or_else(|| Node::new(Vec::new()))
        } else if call.status == ToolStatus::Cancelled {
            let label = text::tool_status_label(ToolStatus::Cancelled);
            Node::text(
                super::super::truncate_with_ellipsis(label, body_width),
                style::agent_step_detail(),
            )
        } else if let Some(node) = read_path_node(call, width) {
            node
        } else if matches!(
            call.status,
            ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running
        ) {
            let label = text::tool_status_label(call.status);
            Node::text(
                super::super::truncate_with_ellipsis(label, body_width),
                style::agent_step_detail(),
            )
        } else {
            Node::new(Vec::new())
        };
        return build(&[Section::fixed(vec![node])]);
    }
    let mut sections = Vec::new();
    if Detail::of(call) == Detail::Error {
        let mut nodes: Vec<Node> = error_node(call).into_iter().collect();
        if nodes.is_empty() {
            nodes.push(Node::new(Vec::new()));
        }
        sections.push(Section::fixed(nodes));
    } else {
        let params = param_nodes(call, &mut cache);
        if !params.is_empty() {
            let preview = (!expanded).then(|| Preview::head(DETAIL_PREVIEW_ROWS));
            sections.push(Section::new(params, preview));
        }
        let facts = fact_nodes(call);
        if !facts.is_empty() {
            sections.push(Section::fixed(facts));
        }
    }
    if sections.is_empty() {
        // 完成态没有摘要：仍保留一行收口连接符（行数恒 ≥ 1）。
        sections.push(Section::fixed(vec![Node::new(Vec::new())]));
    }
    build(&sections)
}

/// 详情行的物理行（含连接符与省略提示行）；`rows` 与 `draw` 共用。
pub(super) fn detail(call: &ToolCallView, width: u16, expanded: bool) -> Vec<Vec<Segment>> {
    detail_layout(call, width, expanded).segments()
}

/// 详情行的物理行数（含省略提示行）。
pub(super) fn detail_rows_with_group(
    call: &ToolCallView,
    width: u16,
    expanded: bool,
    in_group: bool,
) -> usize {
    detail_layout_with_group(call, width, expanded, in_group)
        .rows
        .len()
}

/// 详情**内容**行数（不含省略提示行）；供「能否展开」判定。
pub(super) fn detail_content_rows_with_group(
    call: &ToolCallView,
    width: u16,
    expanded: bool,
    in_group: bool,
) -> usize {
    let layout = detail_layout_with_group(call, width, expanded, in_group);
    if expanded {
        layout.total
    } else {
        layout.total.saturating_sub(layout.hidden)
    }
}

/// 合并组内调用体：只画详情树，不画标题行。
pub(super) fn draw_body(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    expanded: bool,
    last: bool,
) {
    if rect.width == 0 {
        return;
    }
    let mut layout = detail_layout_with_group(call, rect.width, expanded, true);
    let is_read = Kind::of(&call.name) == Kind::Read;
    if is_read {
        let conn = tree::connector_style_of(call);
        if let Some(first) = layout.rows.first_mut() {
            first.prefix_style = Some(conn);
        }
    } else {
        tree::decorate_group_call(&mut layout.rows, call, title_status(call), rect.width);
    }
    tree::apply_group_connector(&mut layout.rows, last);
    tree::draw(grid, rect, scroll, row_index, &layout.rows);
}

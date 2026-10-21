//! 工具步骤渲染：**两行**「`■ 动词` + `  └─ 详情`」（对齐消息底部通知的语法）。
//!
//! 语法与 [`super::super::notice`] 完全一致，同一块历史里只有一套步骤读法：
//!
//! ```text
//! ■ Read                        ← 第 1 行：方块 + 动词（翡翠绿），**不显示任何状态文字**
//!   └─ src/ui/style.rs · 12-40  ← 第 2 行：`└─` 连接符 + 参数/计数/兜底状态词
//! ```
//!
//! - **第 1 行只回答「哪个工具」**：方块（运行中 `■`/`□` 两帧呼吸、终态静态）+
//!   固定动词。完成态没有 `done` 之类尾巴——方块静止本身就是「做完了」。
//! - **第 2 行回答「对什么做了什么 / 出了什么事」**：参数（路径金黄）、
//!   次要计数（行范围 dim、命中数 dim、增删绿红、截断提示黄）。失败时**只放错误
//!   文案**（路径等信息 `failure_text` 已带），不再铺任何参数；中断同理；
//!   没有参数可展示的瞬间（`receiving input` / `pending` / `running` / `cancelled`）
//!   用 dim 的状态词兜底。**完成态没有兜底**：没有任何摘要时第 2 行只有 `└─`
//!   前缀（行数仍算 2）。
//!
//! **行数口径 [`rows`] 恒为 2**（宽度 0 时为 0），第二行**单行绝不换行**——
//! `history::layout`、`history::render`、置顶用户行与滚动条全靠这条不变量对齐。
//! 超宽时按「参数内的次要段（`search` 的 `in path`、长路径尾段）→ 次要计数按优先级
//! 丢弃 → 错误尾部省略 → 丢参数」依次让位，`└─` 前缀始终可辨。`search` 的 pattern
//! 是这一行唯一必须保住的信息，**绝不中间省略**（只在它单独都放不下时尾部省略）。
//!
//! 参数一律取自 [`ToolCallView::summary`]（参数派生 + 结果体补全，见
//! `ToolSummary::from_arguments` / `absorb_result_json`）；只有摘要缺字段时才回退解析
//! `arguments_json`，渲染热路径不做常规 JSON 解析。
//!
//! 纯渲染：只读 [`ToolCallView`]，只写 lx 页网格；不执行 IO、不改状态、不 `unwrap`。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{ToolCallView, ToolStatus};
use crate::ui::{style, text};

use super::super::Grid;
use super::{StepKind, StepMark, truncate_with_ellipsis, visible_row};

mod command;
mod file_diff;
mod file_summary;
mod plan;

use file_summary::detail;
#[cfg(test)]
use file_summary::{detail_prefix, fallback_parse_count, reset_fallback_parse_count};

/// 树形前缀列数（`  │  ` 与 `  └─ ` 等宽，都是 5 列）；命令/交互/计划卡片共用。
pub(super) const TREE_COLS: usize = 5;

/// 工具步骤占用的渲染行数。
///
/// - 文件工具：**恒为 2 行**（标题行 + 详情行）；带差异正文时为标题 + 文件头 /
///   预览（折叠最多 3 个正文物理行，见 [`file_diff`]）；
/// - `exec_command` / `write_stdin`：标题 + 命令 + `[open]`；
/// - `update_plan`：标题 + 步骤（长步骤换行）+ 解释行。
///
/// `history::layout` 与 `history::render` 必须共用本函数：工具行一旦换行，滚动条、
/// 置顶行与 thinking 命中矩形都会漂移。
pub(super) fn rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    if width == 0 {
        return 0;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(_))
        | Some(crate::app::state::ToolDetailView::Interaction(_)) => {
            expanded_or_preview_rows(call, width, expanded)
        }
        Some(crate::app::state::ToolDetailView::Plan(_)) => plan::rows(call, width, expanded),
        Some(crate::app::state::ToolDetailView::File(_)) if file_diff::has_diff(call) => {
            file_diff::rows(call, width, expanded)
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => HEADER_ROWS + DETAIL_ROWS,
    }
}

/// 进程卡片行数（折叠/展开共用同一入口）。
fn expanded_or_preview_rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    command::rows(call, width, expanded)
}

/// 标题行行数。
const HEADER_ROWS: usize = 1;
/// 详情行行数（**单行，绝不换行**；见模块文档的让位顺序）。
const DETAIL_ROWS: usize = 1;

/// 参数可用的最小显示列数；低于它就把参数整段省略（避免只剩一个省略号）。
pub(super) const MIN_PARAM_COLS: usize = 6;
/// `search` 的 pattern 只需保留「至少一个模式字符 + 省略号」——它是这一行唯一必须
/// 保住的信息，因此门槛比路径低（review P2-7：窄宽度也必须看得出在搜什么）。
/// 门槛取 5 列（两列引号 + 一个宽字符 + `…`），再窄就整段不画，不留一个裸引号。
pub(super) const MIN_PATTERN_COLS: usize = 5;
/// 回退解析参数 JSON 的**体积门槛**：参数还在流式接收（还没有摘要）时，只对这么大的
/// 参数体做兜底解析。多文件 `edit` 的 patch 最坏 1 MiB，每帧解析会白烧 CPU（P3-4）；
/// 而路径/patch 目标都在参数开头，小体积足够撑起「先看到一点」的体验。
pub(super) const MAX_FALLBACK_PARSE_BYTES: usize = 8 * 1024;
/// 错误原因的最小显示列数；低于它整段省略（同理不放半个省略号）。
pub(super) const MIN_REASON_COLS: usize = 8;
/// 参数 / 工具名 / 错误原因的字符取样上限（限住超长串的每帧成本）。
pub(super) const TEXT_SAMPLE_CHARS: usize = 512;

/// 绘制工具步骤并推进内容行号；推进的行数与 [`rows`] 严格相等。
///
/// `animating` 是"呼吸相位是否还在推进"（调用方传 `lx.chat.status.streaming`）：
/// 动画停止时方块恒实心，避免定格成 `□`（见 [`crate::ui::main_content::lx::history::StepMark::glyph`]）。
///
/// 生产渲染走 [`draw_animated`]（显式传动画状态）；本函数只在测试里当便捷入口，
/// 因此门控在 `cfg(test)`，避免未使用代码进二进制。
#[cfg(test)]
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    expanded: bool,
) {
    draw_animated(
        grid,
        rect,
        scroll,
        row_index,
        call,
        loading_frame,
        expanded,
        true,
    );
}

/// 同上，但显式给出动画推进状态（渲染层按 `lx.chat.status.streaming` 传入）。
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_animated(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
    expanded: bool,
    animating: bool,
) {
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(_))
        | Some(crate::app::state::ToolDetailView::Interaction(_)) => {
            command::draw(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                expanded,
                animating,
            );
            return;
        }
        Some(crate::app::state::ToolDetailView::Plan(_)) => {
            plan::draw(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                animating,
            );
            return;
        }
        Some(crate::app::state::ToolDetailView::File(_)) if file_diff::has_diff(call) => {
            file_diff::draw(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                expanded,
                animating,
            );
            return;
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => {}
    }
    if rect.width == 0 {
        return;
    }
    // 文件工具：第 1 行方块 + 动词，第 2 行 `  └─ 详情`。
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header(call, rect.width, loading_frame, animating);
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = detail(call, rect.width);
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
}

/// 标题行行数（子模块共用）。
pub(super) fn title_rows() -> usize {
    HEADER_ROWS
}

/// `[open]` 行的卡片内行偏移（0 = 标题行）；没有这一行时返回 None。
///
/// 进程卡片（`exec_command` / `write_stdin`）把 `[open]` 画在树体最后一行，
/// 由 `history::layout` 收成**唯一**的可点击命中矩形（点它打开后台任务面板）。
/// 卡片不再有"展开/收起"语义，因此没有第二个命中区。
pub(super) fn open_offset(call: &ToolCallView) -> Option<usize> {
    command::open_offset(call)
}

/// 文件差异卡片**标题行**的卡片内偏移；没有可展开差异时返回 None。
///
/// 与进程 `[open]` 是两种不同的命中动作：标题切换整张文件卡片的展开状态，
/// 绝不打开后台任务面板（见 `history::ToolHitKind`）。
pub(super) fn toggle_offset(call: &ToolCallView) -> Option<usize> {
    file_diff::toggle_offset(call)
}

/// 行内一段着色文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Segment {
    text: String,
    style: Style,
}

impl Segment {
    pub(super) fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    /// 该段的显示列数（宽字符按两列）。
    pub(super) fn cols(&self) -> usize {
        self.text.width()
    }
}

/// 若干段的显示列数合计。
pub(super) fn segments_cols(segments: &[Segment]) -> usize {
    segments.iter().map(Segment::cols).sum()
}

/// 四文件工具的展示种类；未知工具名走 [`Kind::Other`]（动词位置显示工具名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Read,
    Search,
    Edit,
    Write,
    Other,
}

impl Kind {
    /// 工具名 → 展示种类（只认四工具稳定英文名）。
    fn of(name: &str) -> Self {
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

/// 行首方块标记：与思考 / 正文块共用同款步骤标记（运行中两帧呼吸、终态静态；
/// 颜色区分完成 / 失败 / 中断 / 取消）。
///
/// 行首标记是**最窄档唯一保留的状态信号**，因此每个状态都要能一眼区分。
fn lead(call: &ToolCallView, loading_frame: usize, animating: bool) -> Segment {
    let mark = match call.status {
        ToolStatus::Failed => StepMark::Failed,
        ToolStatus::Interrupted => StepMark::Interrupted,
        ToolStatus::Cancelled => StepMark::Cancelled,
        ToolStatus::Completed => StepMark::Done,
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running => StepMark::Running,
    };
    Segment::new(
        mark.segment(loading_frame, animating),
        mark.style(StepKind::Tool),
    )
}

/// 固定动词（卡片 title 主体）：运行中橙黄、已完成翡翠绿。
///
/// 用户要求「方块颜色和工具 title 统一为黄色」：动词必须与方块同色，
/// 否则一张卡片两种颜色。
fn verb(call: &ToolCallView, running: bool) -> Segment {
    use text::agent as a;
    let label = match Kind::of(&call.name) {
        Kind::Read => a::VERB_TOOL_READ.to_string(),
        Kind::Search => a::VERB_TOOL_SEARCH.to_string(),
        Kind::Edit => a::VERB_TOOL_EDIT.to_string(),
        Kind::Write => a::VERB_TOOL_WRITE.to_string(),
        Kind::Other => single_line(&call.name, TEXT_SAMPLE_CHARS),
    };
    Segment::new(
        format!("{label} "),
        if running {
            style::chat_tool_verb_running()
        } else {
            style::chat_tool_verb()
        },
    )
}

/// 组装第 1 行（`■ 动词`）：无状态文字、无参数、无计数。
///
/// 失败时整行红（方块 + 动词），中断整行黄——对应「左侧卡片和工具 title 变红」。
fn header(call: &ToolCallView, width: u16, loading_frame: usize, animating: bool) -> Vec<Segment> {
    let limit = usize::from(width);
    let tint = match call.status {
        ToolStatus::Failed => Some(style::agent_fail()),
        ToolStatus::Interrupted => Some(style::agent_warn()),
        _ => None,
    };
    let paint = |segment: Segment| match tint {
        Some(tint) => Segment::new(segment.text, tint),
        None => segment,
    };
    let lead = paint(lead(call, loading_frame, animating));
    let running = matches!(
        call.status,
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running
    );
    let verb = paint(verb(call, running));
    // 极窄时只留方块（写 buffer 时仍按列裁剪，绝不越界）。
    if segments_cols(std::slice::from_ref(&lead)) > limit {
        return vec![lead];
    }
    let mut segments = vec![lead, verb];
    while segments_cols(&segments) > limit {
        if segments.len() == 1 {
            break;
        }
        // 动词整段放不下：尾部省略到剩余列数。
        let remaining = limit.saturating_sub(segments[0].cols());
        let text = truncate_with_ellipsis(&segments[1].text, remaining);
        if text.is_empty() {
            segments.truncate(1);
        } else {
            segments[1] = Segment::new(text, segments[1].style);
        }
    }
    segments
}

// ===== 第 2 行：`  └─ 详情` =====

/// 工具状态 → 行首方块标记（子模块共用）。
pub(super) fn mark_of(status: ToolStatus, _loading_frame: usize) -> StepMark {
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
pub(super) fn header_with_verb(
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
pub(super) fn header_with_mark(
    call: &ToolCallView,
    width: u16,
    loading_frame: usize,
    animating: bool,
    verb: &str,
    status: Option<String>,
    mark: StepMark,
) -> Vec<Segment> {
    let limit = usize::from(width);
    let tint = match call.status {
        ToolStatus::Failed => Some(style::agent_fail()),
        ToolStatus::Interrupted => Some(style::agent_warn()),
        _ => None,
    };
    let paint = |segment: Segment| match tint {
        Some(tint) => Segment::new(segment.text, tint),
        None => segment,
    };
    let lead = paint(Segment::new(
        mark.segment(loading_frame, animating),
        mark.style(StepKind::Tool),
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
pub(super) fn put_segments(grid: &mut Grid, x: u16, y: u16, width: u16, segments: &[Segment]) {
    let right = x.saturating_add(width);
    let mut cx = x;
    for segment in segments {
        let remaining = right.saturating_sub(cx);
        if remaining == 0 {
            return;
        }
        grid.put_clipped(cx, y, &segment.text, segment.style, remaining);
        cx = cx.saturating_add(u16::try_from(segment.cols()).unwrap_or(u16::MAX));
    }
}

/// 单行化并限长：控制字符（含换行/制表）转空格，去掉首尾空白。
pub(super) fn single_line(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .take(max_chars)
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests;

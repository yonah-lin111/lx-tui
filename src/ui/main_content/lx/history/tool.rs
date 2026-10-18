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
//! - **第 2 行回答「对什么做了什么 / 出了什么事」**：参数（路径金黄下划线）、
//!   次要计数（行范围 dim、命中数 dim、增删绿红、截断提示黄）。失败时**只放错误
//!   文案**（路径等信息 `failure_text` 已带），不再铺任何参数；中断同理；
//!   没有参数可展示的瞬间（`receiving input` / `pending` / `running` / `cancelled`）
//!   用 dim 的状态词兜底，第 2 行**永不为空**。
//!
//! **行数口径 [`rows`] 恒为 2**（宽度 0 时为 0），第二行**单行绝不换行**——
//! `history::layout`、`history::render`、置顶用户行与滚动条全靠这条不变量对齐。
//! 超宽时按「路径中间省略 → 次要计数按优先级丢弃 → 错误尾部省略 → 丢参数」
//! 依次让位，`└─` 前缀始终可辨。
//!
//! 纯渲染：只读 [`ToolCallView`]，只写 lx 页网格；不执行 IO、不改状态、不 `unwrap`。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{ToolCallView, ToolStatus, ToolSummary};
use crate::ui::{style, text};

use super::super::Grid;
use super::{StepKind, StepMark, truncate_with_ellipsis, visible_row};

/// 工具步骤占用的渲染行数：**恒为 2 行**（标题行 + 详情行）；宽度 0 时不占行。
///
/// `history::layout` 与 `history::render` 必须共用本函数：工具行一旦换行，滚动条、
/// 置顶行与 thinking 命中矩形都会漂移。
pub(super) fn rows(_call: &ToolCallView, width: u16) -> usize {
    if width == 0 {
        0
    } else {
        HEADER_ROWS + DETAIL_ROWS
    }
}

/// 标题行行数。
const HEADER_ROWS: usize = 1;
/// 详情行行数（**单行，绝不换行**；见模块文档的让位顺序）。
const DETAIL_ROWS: usize = 1;

/// 参数可用的最小显示列数；低于它就把参数整段省略（避免只剩一个省略号）。
const MIN_PARAM_COLS: usize = 6;
/// 错误原因的最小显示列数；低于它整段省略（同理不放半个省略号）。
const MIN_REASON_COLS: usize = 8;
/// 参数 / 工具名 / 错误原因的字符取样上限（限住超长串的每帧成本）。
const TEXT_SAMPLE_CHARS: usize = 512;

/// 绘制工具步骤并推进内容行号；推进的行数与 [`rows`] 严格相等。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
) {
    // 第 1 行：方块 + 动词。
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header(call, rect.width, loading_frame);
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    // 第 2 行：`  └─ 详情`。
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = detail(call, rect.width);
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
}

/// 行内一段着色文本。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    text: String,
    style: Style,
}

impl Segment {
    fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }

    /// 该段的显示列数（宽字符按两列）。
    fn cols(&self) -> usize {
        self.text.width()
    }
}

/// 若干段的显示列数合计。
fn segments_cols(segments: &[Segment]) -> usize {
    segments.iter().map(Segment::cols).sum()
}

/// 四文件工具的展示种类；未知工具名走 [`Kind::Other`]（动词位置显示工具名）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
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
fn lead(call: &ToolCallView, loading_frame: usize) -> Segment {
    let mark = match call.status {
        ToolStatus::Failed => StepMark::Failed,
        ToolStatus::Interrupted => StepMark::Interrupted,
        ToolStatus::Cancelled => StepMark::Cancelled,
        ToolStatus::Completed => StepMark::Done,
        ToolStatus::InputStreaming | ToolStatus::Pending | ToolStatus::Running => StepMark::Running,
    };
    Segment::new(mark.segment(loading_frame), mark.style(StepKind::Tool))
}

/// 固定动词：品牌天蓝加粗（饱和、与正文默认前景和边框 Cyan/dim 都区分得开）。
fn verb(call: &ToolCallView) -> Segment {
    use text::agent as a;
    let label = match Kind::of(&call.name) {
        Kind::Read => a::VERB_TOOL_READ.to_string(),
        Kind::Search => a::VERB_TOOL_SEARCH.to_string(),
        Kind::Edit => a::VERB_TOOL_EDIT.to_string(),
        Kind::Write => a::VERB_TOOL_WRITE.to_string(),
        Kind::Other => single_line(&call.name, TEXT_SAMPLE_CHARS),
    };
    Segment::new(format!("{label} "), style::chat_tool_verb())
}

/// 组装第 1 行（`■ 动词`）：无状态文字、无参数、无计数。
///
/// 失败时整行红（方块 + 动词），中断整行黄——对应「左侧卡片和工具 title 变红」。
fn header(call: &ToolCallView, width: u16, loading_frame: usize) -> Vec<Segment> {
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
    let lead = paint(lead(call, loading_frame));
    let verb = paint(verb(call));
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

/// 详情行前缀（缩进 + 连接符 + 空格）：与消息底部通知的 `reason_prefix()` 同款，
/// 列宽固定，续行对齐口径一致。
fn detail_prefix() -> String {
    format!("  {} ", text::agent::MARKER_TREE_LEAF)
}

/// 第二行的内容类型：决定是否有参数可展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Detail {
    /// 失败 / 中断：**只放错误文案**，不显示任何参数。
    Error,
    /// 正常：参数 + 次要计数（可能为空 → 状态词兜底）。
    Params,
}

impl Detail {
    /// 按状态选择第二行的语义。
    fn of(call: &ToolCallView) -> Self {
        if matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
            Self::Error
        } else {
            Self::Params
        }
    }
}

/// 主参数：摘要里的路径（或 `search` 的搜索范围）；只显示摘要，
/// **绝不**铺文件内容或命中正文。
///
/// `search` 结算后的摘要只带命中数（不含搜索范围），此时退回参数 JSON 里的 `path`；
/// [`ToolSummary::from_arguments`] 是纯解析，无 IO、不改状态。
fn param(call: &ToolCallView) -> Option<Segment> {
    let from_args = || ToolSummary::from_arguments(&call.arguments_json);
    let summary_path = || {
        call.summary
            .as_ref()
            .and_then(|summary| summary.path.as_deref())
            .filter(|path| !path.is_empty())
            .map(str::to_string)
    };
    // `search`：`"pattern" in path`。pattern 只来自参数（结果体不带），
    // 路径在结算摘要里可能缺席，回退到参数 JSON 的 `path`。
    if Kind::of(&call.name) == Kind::Search {
        let pattern = call
            .summary
            .as_ref()
            .and_then(|summary| summary.pattern.clone())
            .or_else(|| from_args().pattern);
        let path = summary_path().or_else(|| from_args().path);
        let scope = text::agent::search_scope_label(
            pattern
                .as_deref()
                .map(|p| single_line(p, TEXT_SAMPLE_CHARS))
                .as_deref(),
            path.as_deref()
                .map(|p| single_line(p, TEXT_SAMPLE_CHARS))
                .as_deref(),
        )?;
        return Some(Segment::new(scope, style::chat_tool_param()));
    }
    let path = summary_path().or_else(|| from_args().path)?;
    let text = single_line(&path, TEXT_SAMPLE_CHARS);
    (!text.is_empty()).then(|| Segment::new(text, style::chat_tool_param()))
}

/// 次要信息段 + 它的**丢弃优先级**（值越小越先被窄宽度隐藏）。
struct Extra {
    segment: Segment,
    priority: u8,
}

/// 命中数 / 增删计数：最先隐藏的次要计数。
const PRIORITY_COUNT: u8 = 1;
/// 行范围（`read` 的键信息）保留得比命中数久。
const PRIORITY_RANGE: u8 = 2;
/// 截断提示（必须显式告知用户）最后才隐藏的次要信息。
const PRIORITY_TRUNCATED: u8 = 3;

/// 次要信息（显示顺序：行范围 → 命中数 → 增删计数 → 截断提示）。
fn extras(call: &ToolCallView) -> Vec<Extra> {
    let sep = text::LX_CHAT_TOOL_STATUS_SEP;
    let Some(summary) = call.summary.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<Extra> = Vec::new();
    /// 收集一段带 `· ` 前缀的次要信息（局部宏，避免闭包同时借用 `out`）。
    macro_rules! push {
        ($value:expr, $style:expr, $priority:expr $(,)?) => {
            out.push(Extra {
                segment: Segment::new(format!("{sep}{}", $value), $style),
                priority: $priority,
            })
        };
    }
    match Kind::of(&call.name) {
        Kind::Read => {
            if let Some((start, end)) = summary.range {
                push!(
                    text::agent::range_label(start, end),
                    style::chat_tool_range(),
                    PRIORITY_RANGE,
                );
            }
            // 实际读到的行数（结果体里的 total_lines）：与区间并列，
            // 括号区分"哪一段"与"多大一块"。
            if summary.range.is_some()
                && let Some(total) = summary.total_lines
            {
                push!(
                    text::agent::lines_label(total),
                    style::chat_tool_range(),
                    PRIORITY_RANGE,
                );
            }
            // 目录读取的条目数与 `search` 的命中数同口径显示。
            if let Some(matches) = summary.matches {
                push!(
                    text::agent::matches_label(matches),
                    style::agent_step_detail(),
                    PRIORITY_COUNT,
                );
            }
        }
        Kind::Search => {
            if let Some(matches) = summary.matches {
                push!(
                    text::agent::matches_label(matches),
                    style::agent_step_detail(),
                    PRIORITY_COUNT,
                );
            }
        }
        Kind::Edit | Kind::Write => {
            // 增删计数是**一个**判断（"加了 12 删了 4"），合成一段 `+12 -4`；
            // 仍各自上色（绿/红），只是中间不再插 `·` 分隔符。
            let mut diff = Vec::new();
            if let Some(added) = summary.added {
                diff.push((text::agent::added_label(added), style::agent_added()));
            }
            if let Some(removed) = summary.removed {
                diff.push((text::agent::removed_label(removed), style::agent_removed()));
            }
            if !diff.is_empty() {
                // 首段带 `· ` 分隔符；后续计数各自成段（保留绿/红），中间只留空格。
                // 注意不能走 `push`：它会给每段都补 `· `。
                out.push(Extra {
                    segment: Segment::new(format!("{sep}{}", diff[0].0), diff[0].1),
                    priority: PRIORITY_COUNT,
                });
                out.extend(diff.iter().skip(1).map(|(label, label_style)| Extra {
                    segment: Segment::new(format!(" {label}"), *label_style),
                    priority: PRIORITY_COUNT,
                }));
            }
        }
        Kind::Other => {}
    }
    if summary.truncated {
        out.push(Extra {
            segment: Segment::new(
                format!("{sep}{}", text::agent::TOOL_TRUNCATED),
                style::agent_warn(),
            ),
            priority: PRIORITY_TRUNCATED,
        });
    }
    out
}

/// 取消态尾巴：`· cancelled`（dim）。
///
/// 取消与失败/中断不同——它**有**参数可展示（模型已给出目标路径），但结果是
/// 「没跑」。因此第二行 = 参数 + `· cancelled`，既不丢目标信息，也把"没跑成"
/// 说清楚（状态词统一留在第 2 行，第 1 行永远只有 `■ 动词`）。
fn cancelled_tail(call: &ToolCallView) -> Option<Segment> {
    (call.status == ToolStatus::Cancelled).then(|| {
        Segment::new(
            format!(
                "{}{}",
                text::LX_CHAT_TOOL_STATUS_SEP,
                text::tool_status_label(ToolStatus::Cancelled)
            ),
            style::agent_step_detail(),
        )
    })
}

/// 兜底状态词（`receiving input` / `pending` / `running` / `cancelled`）：
/// 第二行没有参数也没有错误时的占位，保证「这个调用在动/已停」始终可见。
///
/// **完成态没有兜底**：`done` 是本轮明确删除的文案，方块静止已经表达了"做完了"；
/// 完成态没有摘要时第二行留空（行数仍算 2）。
fn fallback_label(call: &ToolCallView) -> Option<Segment> {
    match call.status {
        ToolStatus::InputStreaming
        | ToolStatus::Pending
        | ToolStatus::Running
        | ToolStatus::Cancelled => Some(Segment::new(
            text::tool_status_label(call.status).to_string(),
            style::agent_step_detail(),
        )),
        ToolStatus::Completed | ToolStatus::Failed | ToolStatus::Interrupted => None,
    }
}

/// 错误文案段：`error` 单行化；仅失败/中断使用。
///
/// 失败时第二行**只放它**——路径等信息 `failure_text` 已经写在里面，不再重复参数。
/// 中断时 `error` 是稳定错误码（`interrupted`），同样单行展示。
fn error_segment(call: &ToolCallView) -> Option<Segment> {
    if !matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
        return None;
    }
    let reason = single_line(call.error.as_deref()?, TEXT_SAMPLE_CHARS);
    (!reason.is_empty()).then(|| Segment::new(reason, style::text()))
}

/// 组装第 2 行（`  └─ 详情`）；`width` 为内容列数。
///
/// 让位顺序（与设计文档一致）：路径中间省略 → 次要计数按优先级丢弃 →
/// 错误尾部省略 → 参数让位给错误。前缀永远保留（除非窄到放不下前缀）。
fn detail(call: &ToolCallView, width: u16) -> Vec<Segment> {
    let limit = usize::from(width);
    let prefix = detail_prefix();
    let prefix_cols = prefix.width();
    let prefix_segment = Segment::new(prefix, style::chat_notice_connector());
    if prefix_cols > limit {
        // 极窄：连接符都放不下，退回整宽画内容（与通知的原因行同口径）。
        return detail_body(call, limit);
    }
    let mut segments = vec![prefix_segment];
    segments.extend(detail_body(call, limit.saturating_sub(prefix_cols)));
    segments
}

/// 第二行正文（不含 `└─ ` 前缀）：按 [`Detail`] 分支，并在 `limit` 列内收敛。
fn detail_body(call: &ToolCallView, limit: usize) -> Vec<Segment> {
    match Detail::of(call) {
        Detail::Error => {
            let Some(error) = error_segment(call) else {
                // 失败/中断但没有 error 文案：第 2 行留空（行数仍算 2）。
                return Vec::new();
            };
            if error.cols() <= limit {
                return vec![error];
            }
            if limit < MIN_REASON_COLS {
                return Vec::new();
            }
            vec![Segment::new(
                truncate_with_ellipsis(&error.text, limit),
                error.style,
            )]
        }
        Detail::Params => detail_params(call, limit),
    }
}

/// 正常态第二行：参数 + 次要计数，按优先级让位；没有可展示内容时用状态词兜底。
fn detail_params(call: &ToolCallView, limit: usize) -> Vec<Segment> {
    let param = param(call);
    let extras = extras(call);
    if param.is_none() && extras.is_empty() {
        return fit_fallback(call, limit);
    }

    let mut kept = vec![true; extras.len()];
    let mut keep_param = param.is_some();
    loop {
        let kept_cols: usize = extras
            .iter()
            .zip(&kept)
            .filter(|(_, keep)| **keep)
            .map(|(extra, _)| extra.segment.cols())
            .sum();
        let param_cols = param
            .as_ref()
            .filter(|_| keep_param)
            .map_or(0, Segment::cols);
        let reserved = param_cols.min(limit.saturating_sub(kept_cols));
        let fits_param = !keep_param || {
            let available = limit.saturating_sub(kept_cols);
            available >= MIN_PARAM_COLS
        };
        let fits = kept_cols.saturating_add(reserved) <= limit && fits_param;
        if fits {
            let mut segments = Vec::new();
            let used_by_extras = kept_cols.min(limit);
            if let Some(param) = param.as_ref().filter(|_| keep_param) {
                let available = limit.saturating_sub(used_by_extras);
                if param.cols() <= available {
                    segments.push(param.clone());
                } else {
                    let text = text::ellipsize_middle(&param.text, available);
                    if !text.is_empty() {
                        segments.push(Segment::new(text, param.style));
                    }
                }
            }
            segments.extend(
                extras
                    .iter()
                    .zip(&kept)
                    .filter(|(_, keep)| **keep)
                    .map(|(extra, _)| extra.segment.clone()),
            );
            if segments.is_empty() {
                segments = fit_fallback(call, limit);
            } else if let Some(tail) = cancelled_tail(call) {
                // 取消态：参数之后补 `· cancelled`（放不下就丢尾巴，参数优先）。
                if segments_cols(&segments) + tail.cols() <= limit {
                    segments.push(tail);
                }
            }
            return segments;
        }
        // 让位顺序：次要字段（按优先级）→ 参数。
        if let Some(index) = drop_index(&extras, &kept) {
            kept[index] = false;
            continue;
        }
        if keep_param {
            keep_param = false;
            continue;
        }
        break;
    }
    Vec::new()
}

/// 兜底状态词按列宽收敛（放不下且够最小宽度时尾部省略，否则整段不画）。
fn fit_fallback(call: &ToolCallView, limit: usize) -> Vec<Segment> {
    let Some(fallback) = fallback_label(call) else {
        return Vec::new();
    };
    if fallback.cols() <= limit {
        return vec![fallback];
    }
    if limit < MIN_REASON_COLS {
        return Vec::new();
    }
    vec![Segment::new(
        truncate_with_ellipsis(&fallback.text, limit),
        fallback.style,
    )]
}

/// 下一个要丢弃的次要字段：**优先级最低**者；同级取显示顺序靠后的（先丢后出现的
/// 次要值，例如 `-N` 先于 `+N`）。没有可丢的字段时返回 None。
fn drop_index(extras: &[Extra], kept: &[bool]) -> Option<usize> {
    extras
        .iter()
        .enumerate()
        .filter(|(index, _)| kept[*index])
        .min_by_key(|(index, extra)| (extra.priority, std::cmp::Reverse(*index)))
        .map(|(index, _)| index)
}

/// 逐段写入网格，整体不越过 `width` 列（宽字符不半写）。
fn put_segments(grid: &mut Grid, x: u16, y: u16, width: u16, segments: &[Segment]) {
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
fn single_line(value: &str, max_chars: usize) -> String {
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

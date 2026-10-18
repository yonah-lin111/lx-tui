//! 工具步骤渲染：单行 `<方块> 动词 参数 · 次要计数 · 状态 · 原因`。行首方块与思考、
//! 正文块共用同一款步骤标记（见 [`super::StepMark`]：运行中 `■`/`□` 两帧呼吸、
//! 终态静态并按语义着色），不再使用工具类型图标。
//!
//! 纯渲染：只读 [`ToolCallView`]，只写 lx 页网格；不执行 IO、不改状态、不 `unwrap`。
//! 行数口径 [`rows`] **恒为 1 行**（`history::layout` 与 `history::render` 共用），
//! 保证滚动条、置顶用户行与 thinking 命中几何不因工具步骤漂移。
//!
//! 宽度自适应（对齐设计 §10：**先截断路径，再隐藏次要计数，方块与状态始终可辨**）：
//! 1. 参数（路径 / 搜索范围）先按显示列中间省略（宽字符不半写）；
//! 2. 再按优先级隐藏次要字段：命中数 → 行范围 → 截断提示；
//! 3. 仍不够时先丢失败/中断原因（尾部省略，仍有最小保留宽度），
//!    再整段省略参数（路径让位给状态），最后才丢状态文字；
//! 4. 任何宽度下行首方块都在：运行中呼吸、完成 / 失败 / 中断 / 取消用颜色区分
//!    （取消另用空心方块），因此状态始终可辨。
//!
//! 失败步骤（`Failed`）整行走 [`style::agent_fail`] 并附精简原因；`Interrupted` 用警告黄、
//! `Cancelled` 只给状态文字，与失败红可区分。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{ToolCallView, ToolStatus, ToolSummary};
use crate::ui::{style, text};

use super::super::Grid;
use super::{StepMark, truncate_with_ellipsis, visible_row};

/// 参数可用的最小显示列数；低于它就把参数整段省略（避免只剩一个省略号）。
const MIN_PARAM_COLS: usize = 6;
/// 失败/中断原因的最小显示列数；低于它整段省略（同理不放半个省略号）。
const MIN_REASON_COLS: usize = 8;
/// 参数 / 工具名 / 错误原因的字符取样上限（限住超长串的每帧成本）。
const TEXT_SAMPLE_CHARS: usize = 512;

/// 工具步骤占用的渲染行数：**恒为 1 行**（宽度 0 时不占行，与空正文块同口径）。
///
/// `history::layout` 与 `history::render` 必须共用本函数：工具行一旦换行，滚动条、
/// 置顶行与 thinking 命中矩形都会漂移。
pub(super) fn rows(_call: &ToolCallView, width: u16) -> usize {
    usize::from(width > 0)
}

/// 绘制工具步骤并推进内容行号；推进的行数与 [`rows`] 严格相等。
pub(super) fn draw(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    call: &ToolCallView,
    loading_frame: usize,
) {
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = line(call, rect.width, loading_frame);
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

/// 四文件工具的展示种类；未知工具名走 [`Kind::Other`]（动词 + 参数摘要）。
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
    Segment::new(mark.segment(loading_frame), mark.style())
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

/// 主参数：摘要里的路径（或 `search` 的搜索范围）；只显示摘要，
/// **绝不**铺文件内容或命中正文。
///
/// `search` 结算后的摘要只带命中数（不含搜索范围），此时退回参数 JSON 里的 `path`；
/// [`ToolSummary::from_arguments`] 是纯解析，无 IO、不改状态。
fn param(call: &ToolCallView) -> Option<Segment> {
    let path = call
        .summary
        .as_ref()
        .and_then(|summary| summary.path.as_deref())
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .or_else(|| {
            (Kind::of(&call.name) == Kind::Search)
                .then(|| ToolSummary::from_arguments(&call.arguments_json))
                .and_then(|summary| summary.path)
        })?;
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
    let mut push = |value: String, style: Style, priority: u8| {
        out.push(Extra {
            segment: Segment::new(format!("{sep}{value}"), style),
            priority,
        });
    };
    match Kind::of(&call.name) {
        Kind::Read => {
            if let Some((start, end)) = summary.range {
                push(
                    text::agent::range_label(start, end),
                    style::chat_tool_range(),
                    PRIORITY_RANGE,
                );
            }
            // 目录读取的条目数与 `search` 的命中数同口径显示。
            if let Some(matches) = summary.matches {
                push(
                    text::agent::matches_label(matches),
                    style::agent_step_detail(),
                    PRIORITY_COUNT,
                );
            }
        }
        Kind::Search => {
            if let Some(matches) = summary.matches {
                push(
                    text::agent::matches_label(matches),
                    style::agent_step_detail(),
                    PRIORITY_COUNT,
                );
            }
        }
        Kind::Edit | Kind::Write => {
            if let Some(added) = summary.added {
                push(
                    text::agent::added_label(added),
                    style::agent_added(),
                    PRIORITY_COUNT,
                );
            }
            if let Some(removed) = summary.removed {
                push(
                    text::agent::removed_label(removed),
                    style::agent_removed(),
                    PRIORITY_COUNT,
                );
            }
        }
        Kind::Other => {}
    }
    if summary.truncated {
        push(
            text::agent::TOOL_TRUNCATED.to_string(),
            style::agent_warn(),
            PRIORITY_TRUNCATED,
        );
    }
    out
}

/// 状态段：`· <状态文字>`（最晚让位的一段）。
fn status_segment(call: &ToolCallView) -> Segment {
    Segment::new(
        format!(
            "{}{}",
            text::LX_CHAT_TOOL_STATUS_SEP,
            text::tool_status_label(call.status)
        ),
        tail_style(call.status),
    )
}

/// 精简原因段：`· <error 单行>`；仅失败/中断且 `error` 非空时存在。
fn reason_segment(call: &ToolCallView) -> Option<Segment> {
    if !matches!(call.status, ToolStatus::Failed | ToolStatus::Interrupted) {
        return None;
    }
    let reason = single_line(call.error.as_deref()?, TEXT_SAMPLE_CHARS);
    (!reason.is_empty()).then(|| {
        Segment::new(
            format!("{}{reason}", text::LX_CHAT_TOOL_STATUS_SEP),
            tail_style(call.status),
        )
    })
}

/// 状态/原因样式：失败红、中断黄、其余次要信息（完成态不抢注意力）。
fn tail_style(status: ToolStatus) -> Style {
    match status {
        ToolStatus::Failed => style::agent_fail(),
        ToolStatus::Interrupted => style::agent_warn(),
        _ => style::agent_step_detail(),
    }
}

/// 组装一行的全部段（含宽度自适应收敛）；`width` 为内容列数。
fn line(call: &ToolCallView, width: u16, loading_frame: usize) -> Vec<Segment> {
    let limit = usize::from(width);
    // 失败步骤整行红 + 粗（对齐 gallery 的 error 步骤）；其余保留各自配色。
    let failed = call.status == ToolStatus::Failed;
    let tint = |value: Style| if failed { style::agent_fail() } else { value };
    let paint = |segment: Segment| Segment::new(segment.text, tint(segment.style));

    let lead = paint(lead(call, loading_frame));
    let verb = paint(verb(call));
    let param = param(call).map(paint);
    let extras: Vec<Extra> = extras(call)
        .into_iter()
        .map(|extra| Extra {
            segment: paint(extra.segment),
            priority: extra.priority,
        })
        .collect();
    let status = paint(status_segment(call));
    let reason = reason_segment(call).map(paint);

    let head_cols = lead.cols() + verb.cols();
    let mut kept = vec![true; extras.len()];
    let mut keep_param = param.is_some();
    let mut keep_reason = reason.is_some();
    let mut keep_status = true;
    loop {
        let fit = assemble(
            &lead,
            &verb,
            param.as_ref().filter(|_| keep_param),
            &extras,
            &kept,
            keep_status.then_some(&status),
            reason.as_ref().filter(|_| keep_reason),
            limit,
            head_cols,
        );
        if fit.settled {
            return tidy(fit.segments);
        }
        // 让位顺序：次要字段（按优先级）→ 原因 → 参数（路径）→ 状态文字。
        if let Some(index) = drop_index(&extras, &kept) {
            kept[index] = false;
            continue;
        }
        if keep_reason {
            keep_reason = false;
            continue;
        }
        if keep_param {
            keep_param = false;
            continue;
        }
        if keep_status {
            keep_status = false;
            continue;
        }
        break;
    }
    // 极窄：只剩行首符号与动词（写 buffer 时按列裁剪，绝不越界）。
    tidy(vec![lead, verb])
}

/// 一档候选行的组装结果。
struct Fit {
    segments: Vec<Segment>,
    /// 该档拿到的东西是否都真的画出了（参数/原因各需满足最小列宽）。
    settled: bool,
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

/// 组装一档候选行：定长段先占位，两个**可伸缩**段（参数、原因）按剩余列分账。
///
/// 分账规则：参数优先拿满，但为原因预留 [`MIN_REASON_COLS`] 列；参数不足
/// [`MIN_PARAM_COLS`] 视为放不下（由调用方让位）；原因拿剩余列并尾部省略。
/// 任何一段拿不到最小列宽时 `settled = false`，调用方据此继续让位。
#[allow(clippy::too_many_arguments)]
fn assemble(
    lead: &Segment,
    verb: &Segment,
    param: Option<&Segment>,
    extras: &[Extra],
    kept: &[bool],
    status: Option<&Segment>,
    reason: Option<&Segment>,
    limit: usize,
    head_cols: usize,
) -> Fit {
    let kept_cols: usize = extras
        .iter()
        .zip(kept)
        .filter(|(_, keep)| **keep)
        .map(|(extra, _)| extra.segment.cols())
        .sum();
    let fixed = head_cols
        .saturating_add(kept_cols)
        .saturating_add(status.map_or(0, Segment::cols));
    let budget = limit.saturating_sub(fixed);

    let mut settled = true;
    let param_text = param.and_then(|param| {
        let reserve = reason.map_or(0, |reason| reason.cols().min(MIN_REASON_COLS));
        let available = budget.saturating_sub(reserve);
        if available < MIN_PARAM_COLS {
            settled = false;
            return None;
        }
        let text = if param.cols() <= available {
            param.text.clone()
        } else {
            text::ellipsize_middle(&param.text, available)
        };
        settled &= !text.is_empty();
        Some(text)
    });
    let used = param_text.as_ref().map_or(0, |text| text.width());
    let reason_text = reason.and_then(|reason| {
        let available = budget.saturating_sub(used);
        if reason.cols() <= available {
            return Some(reason.text.clone());
        }
        if available < MIN_REASON_COLS {
            settled = false;
            return None;
        }
        Some(truncate_with_ellipsis(&reason.text, available))
    });
    settled &= param.is_none_or(|_| param_text.is_some());
    settled &= reason.is_none_or(|_| reason_text.is_some());

    let mut segments = vec![lead.clone(), verb.clone()];
    if let (Some(param), Some(text)) = (param, param_text) {
        segments.push(Segment::new(text, param.style));
    }
    segments.extend(
        extras
            .iter()
            .zip(kept)
            .filter(|(_, keep)| **keep)
            .map(|(extra, _)| extra.segment.clone()),
    );
    if let Some(status) = status {
        segments.push(status.clone());
    }
    if let (Some(reason), Some(text)) = (reason, reason_text) {
        segments.push(Segment::new(text, reason.style));
    }
    settled &= segments_cols(&segments) <= limit;
    Fit { segments, settled }
}

/// 收敛段间空白：前一段以空格收尾、后一段以空格（分隔符）起头时，
/// 去掉前一段的收尾空格，避免出现 `Read  · done` 这类双空格。
fn tidy(mut segments: Vec<Segment>) -> Vec<Segment> {
    for index in 1..segments.len() {
        if !segments[index].text.starts_with(' ') {
            continue;
        }
        let previous = &mut segments[index - 1];
        let trimmed = previous.text.trim_end();
        if trimmed.len() != previous.text.len() {
            previous.text = trimmed.to_string();
        }
    }
    segments
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

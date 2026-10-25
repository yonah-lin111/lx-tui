//! 文件工具摘要的第二行组装：参数 + 次要计数 + 错误/兜底状态词。
//!
//! 从 `tool.rs` 拆出（原文件已接近 1000 行上限）；行为与输出保持不变：
//! 让位顺序仍是「次要计数（按优先级）→ 参数尾段 → 参数首段 → 兜底状态词」。
//!
//! 渲染热路径不做常规 JSON 解析：摘要由事件层填全，只有摘要缺字段时才懒回退
//! 解析 `arguments_json`（见 [`needs_argument_fallback`]）。

use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{ToolCallView, ToolStatus, ToolSummary};
use crate::ui::{style, text};

use super::super::truncate_with_ellipsis;
use super::{
    Kind, MAX_FALLBACK_PARSE_BYTES, MIN_PARAM_COLS, MIN_PATTERN_COLS, MIN_REASON_COLS, Segment,
    TEXT_SAMPLE_CHARS, segments_cols, single_line, wrap_spans,
};

/// 详情行**折叠态**最多显示的物理行数（用户要求「默认显示 2 行，超过 2 行折叠」）。
pub(crate) const DETAIL_PREVIEW_ROWS: usize = 2;
/// 详情行**展开态**的物理行上限。
///
/// 展开不是「无限展开」：`edit` 的参数里可能塞进 1 MiB 的补丁，逐行铺开会把整屏
/// 历史挤没。上限之内完整显示，超过上限仍以 `…` 收尾——与折叠态同一种收敛方式。
pub(crate) const DETAIL_MAX_ROWS: usize = 24;

/// 详情行前缀（缩进 + 连接符 + 空格）：与消息底部通知的 `reason_prefix()` 同款，
/// 列宽固定，续行对齐口径一致。
pub(super) fn detail_prefix() -> String {
    // 两空格缩进后接直角连接符（与消息底部通知的 `reason_prefix()` 同款）：
    // `└` 落在图标列，详情行读作头部那一行的延续。
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
/// 摘要（`ToolSummary`：参数派生 + 结果体补全）是常规唯一来源；**只有摘要答不出
/// 需要的字段时**才懒回退解析 `arguments_json`（见 [`needs_argument_fallback`]），
/// 渲染热路径因此不做常规 JSON 解析（review P3-4）。
fn param(
    call: &ToolCallView,
    param_style: Style,
    cache: &mut Option<ToolSummary>,
) -> Option<Param> {
    let kind = Kind::of(&call.name);
    let may_parse = needs_argument_fallback(call, kind);
    // 单字段取值：摘要优先；缺字段且允许回退时才触发一次（共享的）解析。
    // `cache` 由调用方跨多次组装共享——一帧内最多解析一次。
    let mut field = |pick: fn(&ToolSummary) -> Option<String>| {
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
    };
    let path = field(|summary| summary.path.clone())
        .map(|path| single_line(&path, TEXT_SAMPLE_CHARS))
        .filter(|path| !path.is_empty());
    if kind != Kind::Search {
        return path.map(|path| path_param(path, param_style));
    }
    // `search`：三段独立成分 —— pattern（金黄）+ ` in `（dim）+ 范围路径（金黄）。
    // 拆段是为了窄宽度时「`in` 与 path 一起丢、pattern 完整保留」（review P2-7）。
    let pattern = field(|summary| summary.pattern.clone())
        .map(|pattern| single_line(&pattern, TEXT_SAMPLE_CHARS))
        .filter(|pattern| !pattern.is_empty());
    let Some(pattern) = pattern else {
        // 没有 pattern（老数据或参数缺失）：退回单段路径（中间省略）。
        return path.map(|path| path_param(path, param_style));
    };
    let mut tail = Vec::new();
    if let Some(path) = path {
        // `in` 不引入新色相：沿用 `agent_step_detail()`（dim 元信息色），
        // 与行范围/计数同档；路径仍用参数金黄（`chat_tool_param`）。
        tail.push(Segment::new(
            format!(" {}", text::agent::IN_SCOPE_SEP),
            style::agent_step_detail(),
        ));
        tail.push(Segment::new(path, param_style));
    }
    Some(Param {
        head: Segment::new(text::agent::pattern_label(&pattern), param_style),
        head_shrink: Shrink::Tail,
        tail,
    })
}

/// 摘要是否答不出需要的主参数（只有这时才回退解析 `arguments_json`）。
///
/// P2-8 之后「实时结算」与「历史重载」都用 `from_arguments(...).absorb_*()` 把参数
/// 派生值全量合并进摘要，因此**摘要完整时这里一律返回 false**，渲染层不做每帧解析。
/// 仍保留三条兜底路径：
/// - 还没有摘要（`Streaming`/`Pending` 的瞬态）：解析一次即空手（参数可能不完整）；
/// - `search` 缺 pattern：pattern 只存在于参数里，摘要没有就只能回参数找；
/// - 其余工具缺 path 且摘要其余字段也空（老数据/未合并）：回退解析补路径。
///
/// `edit` 的 path **缺失是正常形态**（多文件补丁没有唯一路径，见 P2-1），不算缺字段。
fn needs_argument_fallback(call: &ToolCallView, kind: Kind) -> bool {
    let Some(summary) = call.summary.as_ref() else {
        // 还没有摘要（流式接收中）：参数可能还不完整，且最坏是 1 MiB 的 patch 文本。
        // 只在参数体足够小时才做兜底解析，避免「流式期间每帧解析巨型 JSON」（P3-4）。
        return call.arguments_json.len() <= MAX_FALLBACK_PARSE_BYTES;
    };
    match kind {
        Kind::Search => summary.pattern.is_none(),
        Kind::Edit => summary.is_empty(),
        Kind::Read | Kind::Write | Kind::Other => summary.path.is_none() && summary.is_empty(),
    }
}

/// 单段路径参数（`read` / `edit` / `write` / 未知工具）：中间省略。
fn path_param(path: String, param_style: Style) -> Param {
    Param {
        head: Segment::new(path, param_style),
        head_shrink: Shrink::Middle,
        tail: Vec::new(),
    }
}

/// 摘要缺字段时的**懒**回退解析（同一帧内最多一次）。
///
/// 渲染层不该做解析：实时结算（`settle_tool_result`）与历史重载（`tool_view_of`）都已
/// 用「参数派生 + 结果体补全」把摘要填全，`param()` 因此不会走到这里。这段回退只服务
/// 极少数缺字段的瞬态/老数据，且**绝不**覆盖摘要已给出的值。
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
    /// 回退解析计数（仅测试、按线程隔离）：守住「摘要完整时渲染层不解析参数 JSON」。
    ///
    /// 用线程局部而非全局原子：测试并行跑在不同线程上，互不干扰。
    static FALLBACK_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// 当前线程的回退解析次数（测试用）。
#[cfg(test)]
pub(super) fn fallback_parse_count() -> usize {
    FALLBACK_PARSES.with(std::cell::Cell::get)
}

/// 清零当前线程的回退解析计数（测试用）。
#[cfg(test)]
pub(super) fn reset_fallback_parse_count() {
    FALLBACK_PARSES.with(|count| count.set(0));
}

/// 首段放不下时的收敛方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shrink {
    /// 路径：**中间省略**（保留首尾，仍认得出是哪一层级）。
    Middle,
    /// `search` 的 pattern：**尾部省略**——它是这一行唯一必须保住的信息，
    /// 中间省略会把它切成首尾两字（review P2-7 实测 `"测…txt`）。
    Tail,
}

/// 主参数：**必须保住**的首段 + 可整体让位的尾段。
///
/// 其余工具只有首段路径；`search` 首段是 pattern，尾段是 `in` + 范围路径。
/// 尾段要么整体保留、要么整体丢弃（不做半截中间省略），保证 `in` 不会与
/// pattern 或 path 粘成一段残字。
struct Param {
    head: Segment,
    head_shrink: Shrink,
    tail: Vec<Segment>,
}

impl Param {
    /// 总列数（`keep_tail == false` 时只算首段）。
    fn cols(&self, keep_tail: bool) -> usize {
        self.head.cols()
            + if keep_tail {
                segments_cols(&self.tail)
            } else {
                0
            }
    }

    /// 是否有可丢的尾段。
    fn has_tail(&self) -> bool {
        !self.tail.is_empty()
    }
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

/// 次要信息（显示顺序：行范围 + 实际行数 → 命中数 → 增删计数 → `partial` → 截断提示）。
///
/// `has_param` 表示第二行是否已有可见的主参数段：**只有前段存在时才给次要段加
/// ` · ` 分隔符**，否则会渲染出孤立的 `└─  · +12 -4`（多文件 `edit` 的
/// `path == None`，已实测过的 P2-1）。
fn extras(call: &ToolCallView, has_param: bool) -> Vec<Extra> {
    let sep = text::LX_CHAT_TOOL_STATUS_SEP;
    let Some(summary) = call.summary.as_ref() else {
        return Vec::new();
    };
    let mut out: Vec<Extra> = Vec::new();
    // 「当前详情行已有可见前段」：初始 = 主参数是否存在，之后每收集一段就置 true。
    let mut has_lead = has_param;
    /// 收集一段次要信息：前段存在时补 ` · `，否则直接以内容开头。
    /// （局部宏，避免闭包同时借用 `out` 与 `has_lead`。）
    macro_rules! push {
        ($value:expr, $style:expr, $priority:expr $(,)?) => {{
            let lead = if has_lead { sep } else { "" };
            out.push(Extra {
                segment: Segment::new(format!("{lead}{}", $value), $style),
                priority: $priority,
            });
            has_lead = true;
        }};
    }
    match Kind::of(&call.name) {
        Kind::Read => {
            // 区间与「实际读到的行数」合成**一段**：`12-40 (31 lines)`——
            // 括号区分「哪一段」与「多大一块」。行数只用 `read_lines`，
            // **绝不用 `total_lines` 兜底**（那是文件总行数，会把 `12-40` 读成
            // 「读了 1000 行」）。目录读取没有区间，只走下面的 `matches` 分支。
            let range = summary
                .range
                .map(|(start, end)| text::agent::range_label(start, end));
            let lines = summary.read_lines.map(text::agent::lines_label);
            let label = match (range, lines) {
                (Some(range), Some(lines)) => Some(format!("{range} {lines}")),
                (Some(range), None) => Some(range),
                (None, Some(lines)) => Some(lines),
                (None, None) => None,
            };
            if let Some(label) = label {
                push!(label, style::chat_tool_range(), PRIORITY_RANGE);
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
            // 增删计数是**一个**判断（"加了 12 删了 4"），合成一段 `+12 -4`：
            // 首段按需带 `· ` 前缀，后续计数之间只留空格；仍各自上色（绿/红）。
            let mut diff = Vec::new();
            if let Some(added) = summary.added {
                diff.push((text::agent::added_label(added), style::agent_added()));
            }
            if let Some(removed) = summary.removed {
                diff.push((text::agent::removed_label(removed), style::agent_removed()));
            }
            if let Some((first, first_style)) = diff.first() {
                let lead = if has_lead { sep } else { "" };
                out.push(Extra {
                    segment: Segment::new(format!("{lead}{first}"), *first_style),
                    priority: PRIORITY_COUNT,
                });
                out.extend(diff.iter().skip(1).map(|(label, label_style)| Extra {
                    segment: Segment::new(format!(" {label}"), *label_style),
                    priority: PRIORITY_COUNT,
                }));
                has_lead = true;
            }
            // 部分成功：本次补丁里有 hunk 应用后内容没变，与截断提示同档显式告知，
            // 否则模型看到的 `partial: true` 在 UI 上完全消失（review P3-10）。
            if summary.partial {
                push!(
                    text::agent::TOOL_PARTIAL,
                    style::agent_step_detail(),
                    PRIORITY_TRUNCATED,
                );
            }
        }
        Kind::Other => {}
    }
    if summary.truncated {
        let lead = if has_lead { sep } else { "" };
        out.push(Extra {
            segment: Segment::new(
                format!("{lead}{}", text::agent::TOOL_TRUNCATED),
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

/// 详情行（`  └─ …`）的物理行：首行带连接符，续行用等宽空白缩进。
///
/// - 折叠态：最多 [`DETAIL_PREVIEW_ROWS`] 行正文，**下面再单独一行**写剩余行数
///   （`… 421 more lines`，用户要求「在折叠内容下一行显示剩余行数」）；
/// - 展开态：最多 [`DETAIL_MAX_ROWS`] 行（参数文字始终是同一档浅黄），
///   且不再有剩余行数行；`└─` 行是工具参数本身，不铺展开灰底。
///
/// `rows` 与 `draw` 共用本函数，行数口径只有一份。
pub(super) fn detail(call: &ToolCallView, width: u16, expanded: bool) -> Vec<Vec<Segment>> {
    let limit = usize::from(width);
    if limit == 0 {
        return Vec::new();
    }
    let prefix = detail_prefix();
    let prefix_cols = prefix.width();
    // 极窄：连接符都放不下，退回整宽画内容（与通知的原因行同口径）。
    let (head, head_cols) = if prefix_cols > limit {
        (String::new(), 0)
    } else {
        (prefix, prefix_cols)
    };
    let body_limit = limit.saturating_sub(head_cols).max(1);
    // 参数文字（路径 / 搜索模式）：**恒为浅黄**，不随 hover 或展开变色。
    let param_style = style::chat_tool_param();
    // 两次组装、共享同一份解析缓存（一帧内仍只解析一次 `arguments_json`）：
    // - **展示**按折叠额度组装，让位顺序照旧（次要计数先丢、路径再省略）——
    //   用展开额度展示会让长路径铺满两行、把计数挤到看不见；
    // - **总行数**按展开上限组装，差值就是「还剩多少行」。
    //   只组装一次时这两件事互斥（早先「剩余行数恒为 0」就是这么来的）。
    let narrow = Detail::of(call) == Detail::Error || body_limit < super::MIN_PARAM_COLS;
    let fold_budget = if narrow { 1 } else { DETAIL_PREVIEW_ROWS };
    let full_budget = if narrow { 1 } else { DETAIL_MAX_ROWS };
    let mut cache: Option<ToolSummary> = None;
    // 展开态直接按上限组装并展示全部；折叠态按折叠额度组装（保住让位顺序），
    // 另按上限算总行数得到「还剩多少行」。两次组装共享 `cache`，解析仍只发生一次。
    let fold_body = detail_body(
        call,
        body_limit.saturating_mul(fold_budget),
        param_style,
        &mut cache,
    );
    if fold_body.is_empty() {
        // 没有可展示内容：仍占一行（连接符留着，行数恒 ≥ 1）。
        return vec![vec![Segment::new(head, style::chat_notice_connector())]];
    }
    let full_body = if expanded {
        detail_body(
            call,
            body_limit.saturating_mul(full_budget),
            param_style,
            &mut cache,
        )
    } else {
        Vec::new()
    };
    let mut wrapped = wrap_spans(if expanded { &full_body } else { &fold_body }, body_limit);
    wrapped.truncate(if expanded { full_budget } else { fold_budget });
    let shown = wrapped.len();
    let total = if expanded {
        shown
    } else {
        let full = detail_body(
            call,
            body_limit.saturating_mul(full_budget),
            param_style,
            &mut cache,
        );
        wrap_spans(&full, body_limit).len().max(shown)
    };
    let mut rows: Vec<Vec<Segment>> = Vec::with_capacity(wrapped.len() + 1);
    for (index, spans) in wrapped.into_iter().enumerate() {
        let lead = if index == 0 {
            head.clone()
        } else {
            " ".repeat(head_cols)
        };
        let mut row = Vec::with_capacity(spans.len() + 2);
        if !lead.is_empty() {
            row.push(Segment::new(lead, style::chat_notice_connector()));
        }
        row.extend(spans);
        rows.push(row);
    }
    // 折叠态：下一行单独写「还剩多少行」（不占折叠额度，展开后消失）。
    // 窄到放不下就整行不画——半截 `… 1 more li` 比没有更糟。
    let remaining = total.saturating_sub(shown);
    if remaining > 0 {
        let hint = text::agent::file_diff_more_lines(remaining);
        let indent = " ".repeat(head_cols);
        let room = limit.saturating_sub(head_cols);
        if hint.width() <= room {
            // 提示行恒为 dim：折叠内容不再随鼠标悬停提亮。
            rows.push(vec![Segment::new(
                format!("{indent}{hint}"),
                style::agent_step_detail(),
            )]);
        }
    }
    rows
}

/// 详情行的物理行数（`layout` 用；与 [`detail`] 同一份换行口径，含「剩余行数」提示行）。
pub(super) fn detail_rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    detail(call, width, expanded).len()
}

/// 折叠态的详情行数（**含**末尾的「剩余行数」提示行）；供折叠区几何用。
pub(super) fn foldable_rows(call: &ToolCallView, width: u16) -> usize {
    detail(call, width, false).len()
}

/// 详情**内容**行数（不含「剩余行数」提示行）；供「能否展开」判定用。
pub(super) fn detail_content_rows(call: &ToolCallView, width: u16, expanded: bool) -> usize {
    detail_rows(call, width, expanded)
        .saturating_sub(usize::from(!expanded && omitted_rows(call, width) > 0))
}

/// 折叠态是否还有内容没画出来。
fn omitted_rows(call: &ToolCallView, width: u16) -> usize {
    detail_rows(call, width, true).saturating_sub(detail_rows(call, width, false))
}

/// 第二行正文（不含 `└─ ` 前缀）：按 [`Detail`] 分支，并在 `limit` 列内收敛。
fn detail_body(
    call: &ToolCallView,
    limit: usize,
    param_style: Style,
    cache: &mut Option<ToolSummary>,
) -> Vec<Segment> {
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
        Detail::Params => detail_params(call, limit, param_style, cache),
    }
}

/// 正常态第二行：参数 + 次要计数，按优先级让位；没有可展示内容时用状态词兜底。
///
/// 让位顺序：**次要计数（按优先级）→ 参数的尾段（`search` 的 `in path` 整体）→
/// 参数首段**。首段放不下时按 [`Shrink`] 收敛（路径中间省略、pattern 尾部省略）。
/// 这条顺序保证「范围只在 pattern 完整时才显示」，pattern 绝不被切碎。
fn detail_params(
    call: &ToolCallView,
    limit: usize,
    param_style: Style,
    cache: &mut Option<ToolSummary>,
) -> Vec<Segment> {
    let param = param(call, param_style, cache);
    // 次要段是否带 ` · ` 前缀取决于主参数是否存在（否则会出现孤立分隔符）。
    let extras = extras(call, param.is_some());
    if param.is_none() && extras.is_empty() {
        return fit_fallback(call, limit);
    }

    let mut kept = vec![true; extras.len()];
    let mut keep_param = param.is_some();
    let mut keep_tail = param.as_ref().is_some_and(Param::has_tail);
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
            .map_or(0, |param| param.cols(keep_tail));
        let reserved = param_cols.min(limit.saturating_sub(kept_cols));
        // 参数至少要能放下「首段的可收敛形态」；pattern 只需一个字符 + 省略号。
        let min_head = match param.as_ref().filter(|_| keep_param).map(|p| p.head_shrink) {
            Some(Shrink::Tail) => MIN_PATTERN_COLS,
            _ => MIN_PARAM_COLS,
        };
        let fits_param = !keep_param || limit.saturating_sub(kept_cols) >= min_head;
        // `search` 的 pattern 不得为了给次要计数让位而被切碎（review P2-7）：
        // 首段需要收敛且是「尾部省略」形态时，先把次要计数丢光，再动 pattern。
        let head_room = limit.saturating_sub(kept_cols);
        let head_intact = !keep_param
            || kept_cols == 0
            || param
                .as_ref()
                .is_some_and(|param| param.head_shrink == Shrink::Middle)
            || param
                .as_ref()
                .is_some_and(|param| param.head.cols() <= head_room);
        let fits = kept_cols.saturating_add(reserved) <= limit && fits_param && head_intact;
        if fits {
            let mut segments = Vec::new();
            let used_by_extras = kept_cols.min(limit);
            if let Some(param) = param.as_ref().filter(|_| keep_param) {
                let available = limit.saturating_sub(used_by_extras);
                if param.cols(keep_tail) <= available {
                    segments.push(param.head.clone());
                    if keep_tail {
                        segments.extend(param.tail.iter().cloned());
                    }
                } else if param.head.cols() <= available {
                    // 首段完整、尾段放不下：**只丢尾段**（`in path` 要么全在、要么全不在）。
                    segments.push(param.head.clone());
                } else {
                    // 只有首段自己都放不下时才收敛它：路径中间省略、pattern 尾部省略。
                    let text = match param.head_shrink {
                        Shrink::Middle => text::ellipsize_middle(&param.head.text, available),
                        Shrink::Tail => truncate_with_ellipsis(&param.head.text, available),
                    };
                    if !text.is_empty() {
                        segments.push(Segment::new(text, param.head.style));
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
            // 主参数先于它自己的让位被保留（`drop_index` 会先丢光次要段），
            // 因此保留下来的次要段一定有参数可依附、` · ` 前缀不会孤立。
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
        // 让位顺序：次要字段（按优先级）→ 参数尾段 → 参数首段。
        if let Some(index) = drop_index(&extras, &kept) {
            kept[index] = false;
            continue;
        }
        if keep_tail {
            keep_tail = false;
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

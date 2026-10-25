//! 工具步骤渲染：基本形态是**两行**「`■ 动词` + `  └─ 详情`」（对齐消息底部通知的语法）。
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
//! - **第 2 行回答「对什么做了什么 / 出了什么事」**：参数（默认前景色）、
//!   次要计数（行范围 dim、命中数 dim、增删绿红、截断提示黄）。失败时**只放错误
//!   文案**（路径等信息 `failure_text` 已带），不再铺任何参数；中断同理；
//!   没有参数可展示的瞬间（`receiving input` / `pending` / `running` / `cancelled`）
//!   用 dim 的状态词兜底。**完成态没有兜底**：没有任何摘要时第 2 行只有 `└─`
//!   前缀（行数仍算 2）。
//!
//! **行数口径只有一个来源**：基本文件工具恒为 2（宽度 0 时为 0），第二行**单行
//! 绝不换行**；带差异正文的文件卡片、进程卡片与计划卡片各有自己的行数函数
//! （[`rows`] 分派），但 `history::layout`、`history::render`、置顶用户行与滚动条
//! 全都复用同一份口径。超宽时按「参数内的次要段（`search` 的 `in path`、长路径尾段）→ 次要计数按优先级
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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{AssistantStepView, CardSection, ToolCallView, ToolStatus};
use crate::ui::{style, text};

use super::super::Grid;
use super::{StepKind, StepMark, truncate_with_ellipsis, visible_row};

pub(super) use file_summary::{DETAIL_MAX_ROWS, DETAIL_PREVIEW_ROWS};

mod command;
mod file_diff;
mod file_summary;
mod plan;

#[cfg(test)]
use file_summary::{detail_prefix, fallback_parse_count, reset_fallback_parse_count};

/// 树形前缀列数（`  │  ` 与 `  └─ ` 等宽，都是 5 列）；命令/交互/计划卡片共用。
pub(super) const TREE_COLS: usize = 5;

/// 卡片折叠状态：主体（文件差异 / 命令参数）与命令输出**各自独立**。
///
/// 命令卡片两块可以分开折叠；其余卡片只有整体，`output` 无意义（恒为 false）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Folds {
    pub body: bool,
    pub output: bool,
}

impl Folds {
    /// 某一块的展开状态。
    pub(crate) fn of(self, section: CardSection) -> bool {
        match section {
            CardSection::Body => self.body,
            CardSection::Output => self.output,
        }
    }
}

/// 一个可交互区域的几何（相对步骤块首：标题行是偏移 0）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RowSpan {
    pub offset: usize,
    pub rows: usize,
}

/// 卡片内的一个交互区域：点击面与它折叠的是哪一块。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CardRegion {
    pub section: CardSection,
    pub card: RowSpan,
    /// 该区域折叠内容的行区间（不再驱动任何悬停提亮，保留为折叠几何）。
    pub fold: RowSpan,
    /// 点击是否真的会改变内容（false 时不登记点击面）。
    pub clickable: bool,
}

/// 工具步骤占用的渲染行数。
///
/// - 文件工具：标题 + 文件头 / 预览（折叠最多 3 个正文物理行，见 [`file_diff`]）；
/// - `exec_command` / `write_stdin`：标题 + 参数区 + 输出区（或运行中的一行引导）；
/// - `update_plan`：标题 + 步骤（长步骤换行）+ 解释行。
///
/// 命令卡片的参数区与输出区按 [`Folds`] 各自折叠；没有动作按钮、没有点击热区以外的
/// 控件——折叠/展开全靠「点该区块」，与键盘 `^o` / `^t` 共用同一份展开状态。
///
/// `history::layout` 与 `history::render` 必须共用本函数：工具行一旦换行，滚动条、
/// 置顶行与 thinking 命中矩形都会漂移。
pub(super) fn rows(call: &ToolCallView, width: u16, folds: Folds) -> usize {
    if width == 0 {
        return 0;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(_))
        | Some(crate::app::state::ToolDetailView::Interaction(_)) => {
            command::rows(call, width, folds)
        }
        Some(crate::app::state::ToolDetailView::Plan(_)) => plan::rows(call, width, folds.body),
        Some(crate::app::state::ToolDetailView::File(_)) if file_diff::is_card(call) => {
            file_diff::rows(&[call], width, &[folds.body])
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => {
            HEADER_ROWS + file_summary::detail_rows(call, width, folds.body)
        }
    }
}

/// 标题行行数。
const HEADER_ROWS: usize = 1;

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
    folds: Folds,
) {
    draw_animated(
        grid,
        rect,
        scroll,
        row_index,
        call,
        loading_frame,
        folds,
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
    folds: Folds,
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
                folds,
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
        Some(crate::app::state::ToolDetailView::File(_)) if file_diff::is_card(call) => {
            file_diff::draw(
                grid,
                rect,
                scroll,
                row_index,
                &[call],
                loading_frame,
                &[folds.body],
                animating,
            );
            return;
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => {}
    }
    if rect.width == 0 {
        return;
    }
    // 文件工具：第 1 行方块 + 动词，其后是 `  └─ 详情`（最多 2 行，超长可展开）。
    if let Some(y) = visible_row(rect, scroll, *row_index) {
        let segments = header(call, rect.width, loading_frame, animating);
        put_segments(grid, rect.x, y, rect.width, &segments);
    }
    *row_index += 1;
    // 普通工具的 `└─` 行**就是工具参数**，不铺展开底色（用户要求「不要覆盖到工具参数」）。
    for segments in file_summary::detail(call, rect.width, folds.body) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            put_segments(grid, rect.x, y, rect.width, &segments);
        }
        *row_index += 1;
    }
}

/// 标题行行数（子模块共用）。
pub(super) fn title_rows() -> usize {
    HEADER_ROWS
}

/// 从 `steps[start]` 起收集可合并的文件工具组（含起点）；不合并时返回空。
///
/// 合并条件是**同一工具名**（`edit` 并 `edit`、`write` 并 `write`）且两边都是有
/// 差异正文的文件卡片。`edit` 与 `write` 是两个不同的动词，不并——标题只能写一个
/// 动词，混并会把 `write` 说成 `Edit`。组内没有正文步骤可插：一旦被 part 打断就
/// 是两组。
///
/// 返回空的两种情况（不是文件卡片 / 已是单元素）由调用方按单调用原路径处理，
/// 因此这里不区分「不可合并」与「只有一个」。
pub(super) fn merge_group(steps: &[AssistantStepView], start: usize) -> Vec<&ToolCallView> {
    let Some(AssistantStepView::Tool { call, .. }) = steps.get(start) else {
        return Vec::new();
    };
    if !file_diff::is_card(call) {
        return Vec::new();
    }
    let mut group = vec![call];
    for step in &steps[start + 1..] {
        let AssistantStepView::Tool { call: next, .. } = step else {
            break;
        };
        if next.name != call.name || !file_diff::is_card(next) {
            break;
        }
        group.push(next);
    }
    group
}

/// 折叠区的几何：相对该调用块的内容首行（标题在块外）从哪一行开始、占几行。
///
/// 普通工具 / 命令卡片是整段折叠内容（含末尾的 `… N more lines` 提示行），
/// 文件差异卡片是整块折叠内容；计划卡片没有折叠内容。
pub(super) struct FoldRegion {
    /// 折叠区相对内容首行的起始偏移（0 = 内容第一行）。
    pub(super) offset: usize,
    /// 折叠区高度（0 = 该卡片没有折叠内容）。
    pub(super) rows: usize,
}

impl FoldRegion {
    const NONE: Self = Self { offset: 0, rows: 0 };
}

/// 折叠区几何（见 [`FoldRegion`]）。
pub(super) fn fold_region(call: &ToolCallView, width: u16) -> FoldRegion {
    if width == 0 {
        return FoldRegion::NONE;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(_))
        | Some(crate::app::state::ToolDetailView::Interaction(_)) => FoldRegion::NONE,
        Some(crate::app::state::ToolDetailView::Plan(_)) => FoldRegion::NONE,
        Some(crate::app::state::ToolDetailView::File(_)) if file_diff::is_card(call) => {
            FoldRegion {
                offset: 0,
                rows: file_diff::folded_rows(call, width),
            }
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => FoldRegion {
            offset: 0,
            rows: file_summary::foldable_rows(call, width),
        },
    }
}

/// 输出区的文字档位：折叠 = dim；展开 = 终端默认前景。
///
/// 只有**输出正文**随展开态变化；参数/路径恒为浅黄（[`style::chat_tool_param`]）。
pub(super) fn output_style(expanded: bool) -> Style {
    if expanded {
        style::text()
    } else {
        style::muted()
    }
}

/// 命令参数区折行后的一行（文本 + 着色档位）。
pub(super) struct ParamLine {
    pub(super) text: String,
    pub(super) style: Style,
}

/// 命令参数区折行：折叠 [DETAIL_PREVIEW_ROWS] 行、展开 [DETAIL_MAX_ROWS] 行，
/// 截断补 `…`，折叠态末尾单独一行写剩余行数。
///
/// agent 命令卡片与本地命令卡片共用：两个参数区必须同一口径（折行预算与截断），
/// 否则同样长度的一句话在两处显示成不同的样子。参数恒为浅黄，不随 hover/展开变色。
pub(super) fn param_lines(text: &str, width: u16, expanded: bool) -> Vec<ParamLine> {
    let style = style::chat_tool_param();
    let available = usize::from(width.saturating_sub(TREE_COLS as u16));
    if available < MIN_PARAM_COLS {
        return vec![ParamLine {
            text: text.to_string(),
            style,
        }];
    }
    let budget = if expanded {
        DETAIL_MAX_ROWS
    } else {
        DETAIL_PREVIEW_ROWS
    };
    // 组装按「可用列 × 行预算」放宽，让长命令真正折行而不是一路压成省略号。
    let safe = single_line(
        text,
        TEXT_SAMPLE_CHARS
            .saturating_mul(budget)
            .max(TEXT_SAMPLE_CHARS),
    );
    let chars: Vec<char> = safe.chars().collect();
    let mut out: Vec<ParamLine> = Vec::new();
    let mut start = 0usize;
    while start < chars.len() && out.len() < budget {
        let mut col = 0usize;
        let mut end = start;
        while end < chars.len() {
            let cell = chars[end].width().unwrap_or(1).max(1);
            if col + cell > available {
                break;
            }
            col += cell;
            end += 1;
        }
        if end == start {
            end = start + 1;
        }
        let mut row: String = chars[start..end].iter().collect();
        // 末行仍有内容没画完：补 `…`（截断标记不占新行）。
        if end < chars.len() && out.len() + 1 == budget {
            while row.width() + 1 > available && !row.is_empty() {
                row.pop();
            }
            row.push('…');
        }
        out.push(ParamLine {
            text: row.trim_end().to_string(),
            style,
        });
        start = end;
    }
    if out.is_empty() {
        out.push(ParamLine {
            text: String::new(),
            style,
        });
    }
    // 折叠态末尾单独一行写剩余行数（展开后消失）；不占折叠额度。
    // 提示行属于参数区，与参数同一档颜色（不是 meta 信息）。
    // 窄到放不下整句就不画，避免半截提示。
    if !expanded && start < chars.len() {
        let hint =
            text::agent::file_diff_more_lines(estimate_remaining_lines(&chars[start..], available));
        if hint.width() <= available {
            out.push(ParamLine { text: hint, style });
        }
    }
    out
}

/// 剩余内容还能折成几行（只用于提示文案，按整列硬切估算即可）。
fn estimate_remaining_lines(rest: &[char], available: usize) -> usize {
    let width = available.max(1);
    let cols: usize = rest.iter().map(|ch| ch.width().unwrap_or(1).max(1)).sum();
    cols.div_ceil(width).max(1)
}

/// 该调用的内容是否**能靠展开看到更多**（不能时不登记点击面）。
///
/// 判据必须是**内容行**的比较，而不是 `rows()`：折叠态末尾有一行「剩余行数」提示，
/// 直接比 `rows()` 会让「展开」反而比「折叠」少一行（真实 bug：卡片因此不可点）。
/// 各卡片按自己的语义提供内容行数：文件差异比 hunks、普通工具比详情、命令比折行命令。
pub(super) fn has_expandable_content(call: &ToolCallView, width: u16) -> bool {
    if width == 0 {
        return false;
    }
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(_)) => {
            command::params_expandable(call, width) || command::output_expandable(call, width)
        }
        Some(crate::app::state::ToolDetailView::Interaction(_)) => {
            command::params_expandable(call, width)
        }
        Some(crate::app::state::ToolDetailView::Plan(_)) => false,
        Some(crate::app::state::ToolDetailView::File(_)) if file_diff::is_card(call) => {
            file_diff::has_more_content(call, width)
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => {
            file_summary::detail_content_rows(call, width, true)
                > file_summary::detail_content_rows(call, width, false)
        }
    }
}

/// 卡片的可交互区域（几何相对步骤块首）。
///
/// 命令卡片把「命令参数」与「输出」拆成两块：各自的点击面与折叠状态互不影响，
/// 都不随鼠标悬停变色。文件工具是整块一个区域；运行中的命令卡片整卡点击打开
/// 后台任务面板。
pub(super) fn card_regions(
    call: &ToolCallView,
    width: u16,
    folds: Folds,
    step_rows: usize,
) -> Vec<CardRegion> {
    if width == 0 {
        return Vec::new();
    }
    let no_fold = RowSpan { offset: 0, rows: 0 };
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => {
            let blocks = command::blocks(call, width, folds);
            let params = RowSpan {
                offset: HEADER_ROWS,
                rows: blocks.params,
            };
            let guide = RowSpan {
                offset: HEADER_ROWS + blocks.params,
                rows: blocks.output,
            };
            if view.state.is_running() {
                // 长任务：参数整段与引导行都是「打开面板」的点击面。
                return vec![
                    CardRegion {
                        section: CardSection::Body,
                        card: RowSpan {
                            offset: 0,
                            rows: HEADER_ROWS + blocks.params,
                        },
                        fold: no_fold,
                        clickable: true,
                    },
                    CardRegion {
                        section: CardSection::Output,
                        card: guide,
                        fold: guide,
                        clickable: true,
                    },
                ];
            }
            let mut regions = vec![CardRegion {
                section: CardSection::Body,
                card: params,
                fold: no_fold,
                clickable: command::params_expandable(call, width),
            }];
            if blocks.output > 0 {
                // 只有**真被折叠**的输出（比预览多）才登记点击面：短输出点了也不会变。
                let expandable = command::output_expandable(call, width);
                regions.push(CardRegion {
                    section: CardSection::Output,
                    card: guide,
                    fold: if expandable { guide } else { no_fold },
                    clickable: expandable,
                });
            }
            regions
        }
        Some(crate::app::state::ToolDetailView::Interaction(_)) => {
            let params = RowSpan {
                offset: HEADER_ROWS,
                rows: step_rows.saturating_sub(HEADER_ROWS),
            };
            vec![CardRegion {
                section: CardSection::Body,
                card: params,
                fold: no_fold,
                clickable: command::params_expandable(call, width),
            }]
        }
        Some(crate::app::state::ToolDetailView::Plan(_)) => Vec::new(),
        Some(crate::app::state::ToolDetailView::File(_)) | None => {
            // 展开前后行数不变就不登记：留下一个点了没反应的点击面只会误导。
            if !has_expandable_content(call, width) {
                return Vec::new();
            }
            let region = fold_region(call, width);
            let fold_offset = HEADER_ROWS + region.offset;
            vec![CardRegion {
                section: CardSection::Body,
                card: RowSpan {
                    offset: 0,
                    rows: step_rows,
                },
                fold: RowSpan {
                    offset: fold_offset,
                    rows: region.rows.min(step_rows.saturating_sub(fold_offset)),
                },
                clickable: true,
            }]
        }
    }
}

/// 合并卡行数；`expanded` 与 `calls` 一一对应（逐调用独立）。
pub(super) fn group_rows(calls: &[&ToolCallView], width: u16, expanded: &[bool]) -> usize {
    file_diff::rows(calls, width, expanded)
}

/// 合并卡逐调用的块几何（行区间）。
pub(super) fn group_blocks(
    calls: &[&ToolCallView],
    width: u16,
    expanded: &[bool],
) -> Vec<file_diff::Block> {
    file_diff::blocks(calls, width, expanded)
}

/// 绘制合并卡；推进行数与 [`group_rows`] 严格相等。
#[allow(clippy::too_many_arguments)]
pub(super) fn group_draw_animated(
    grid: &mut Grid,
    rect: Rect,
    scroll: usize,
    row_index: &mut usize,
    calls: &[&ToolCallView],
    loading_frame: usize,
    expanded: &[bool],
    animating: bool,
) {
    file_diff::draw(
        grid,
        rect,
        scroll,
        row_index,
        calls,
        loading_frame,
        expanded,
        animating,
    );
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
        mark.style(StepKind::Tool, false, false),
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
        mark.style(StepKind::Tool, false, false),
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

/// 按显示列换行，同时保留分段样式：跨行处切开当前分段，不丢样式。
///
/// 差异正文（语法高亮）与工具参数（`└─` 详情行）共用这一份实现：两处都要
/// 「按列折行且不打断着色」，各写一遍必然一边宽字符算错一边样式丢失。
pub(super) fn wrap_spans(spans: &[Segment], width: usize) -> Vec<Vec<Segment>> {
    let width = width.max(1);
    let mut rows: Vec<Vec<Segment>> = Vec::new();
    let mut current: Vec<Segment> = Vec::new();
    let mut col = 0usize;
    for source in spans {
        let mut buf = String::new();
        for ch in source.text.chars() {
            let cell = ch.width().unwrap_or(1).max(1);
            if col + cell > width && col > 0 {
                if !buf.is_empty() {
                    current.push(Segment::new(std::mem::take(&mut buf), source.style));
                }
                rows.push(std::mem::take(&mut current));
                col = 0;
            }
            buf.push(ch);
            col += cell;
        }
        if !buf.is_empty() {
            current.push(Segment::new(buf, source.style));
        }
    }
    rows.push(current);
    rows
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

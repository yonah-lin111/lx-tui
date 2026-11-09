//! 工具步骤渲染：标题（方块 + 动词 + 右侧状态）与树体内容。
//!
//! ```text
//! ■ Run command · Exited 0 · 241ms
//!   ├─ $ cargo test
//!   └─ test one ... ok
//!      … 18 more lines
//! ```
//!
//! - **标题右侧**回答「这次调用什么状态」：进程退出码/耗时、`read` 的种类与目录条目数、
//!   `search` 的命中数、`plan` 的完成计数、委派卡片的调用条件；失败/中断只保留错误详情。
//! - **树体**回答「对什么做了什么」：一个节点是一段内容（命令、路径、命中、步骤块、
//!   问答对、diff 正文），首行画 `├─`/`└─`，同一节点的续行左对齐或画 `│`。
//! - 每个工具调用是独立卡片，块之间空一行；连续文件工具**不再合并**标题。
//! - 参数/输出各自可独立折叠（[`Folds`]），折叠提示只是续行，不单独收口。
//! - 参数一律取自 [`ToolCallView::summary`]（参数派生 + 结果体补全，见
//!   `ToolSummary::from_arguments` / `absorb_result_json`）；只有摘要缺字段时才回退解析
//!   `arguments_json`，渲染热路径不做常规 JSON 解析。
//!
//! 纯渲染：只读 [`ToolCallView`]，只写 lx 页网格；不执行 IO、不改状态、不 `unwrap`。

use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{CardSection, ToolCallView, ToolStatus};
use crate::ui::{style, text};

use super::super::Grid;
use super::{StepKind, StepMark, truncate_with_ellipsis, visible_row};

mod ask;
mod command;
mod file_diff;
mod file_summary;
mod mcp;
mod plan;
mod search;
mod subagent;
pub(crate) mod tree;
mod web_search;

#[cfg(test)]
use file_summary::{fallback_parse_count, reset_fallback_parse_count};

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
        Some(crate::app::state::ToolDetailView::Ask(_)) => ask::rows(call, width, folds.body),
        Some(crate::app::state::ToolDetailView::Subagent(_)) => {
            subagent::rows(call, width, folds.body)
        }
        Some(crate::app::state::ToolDetailView::Search(view)) => {
            search::rows(call, view, width, folds.body)
        }
        Some(crate::app::state::ToolDetailView::Mcp(view)) => mcp::rows(call, view, width, folds),
        Some(crate::app::state::ToolDetailView::WebSearch(view)) => {
            web_search::rows(call, view, width, folds)
        }
        Some(crate::app::state::ToolDetailView::File(_)) if file_diff::is_card(call) => {
            file_diff::rows(call, width, folds.body)
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => {
            HEADER_ROWS + file_summary::detail_rows(call, width, folds.body)
        }
    }
}

/// 标题行行数。
const HEADER_ROWS: usize = 1;

/// 回退解析参数 JSON 的**体积门槛**：参数还在流式接收（还没有摘要）时，只对这么大的
/// 参数体做兜底解析。多文件 `edit` 的 patch 最坏 1 MiB，每帧解析会白烧 CPU（P3-4）；
/// 而路径/patch 目标都在参数开头，小体积足够撑起「先看到一点」的体验。
pub(super) const MAX_FALLBACK_PARSE_BYTES: usize = 8 * 1024;
/// 参数 / 工具名 / 错误原因的字符取样上限（限住超长串的每帧成本）。
pub(super) const TEXT_SAMPLE_CHARS: usize = 512;
/// 单条输出行的字符上限：超长单行先取样再折行，避免每帧在渲染热路径展开巨量文本。
/// 截断处显式补 `…`，不静默吞掉内容。
pub(super) const MAX_OUTPUT_LINE_CHARS: usize = 8 * 1024;

/// 输出单行取样与控制字符净化（超过上限按字符截断并补 `…`；`\t` 及控制字符转单空格，
/// 避免 `Grid::put_clipped` 将 0 宽控制字符跳过导致词粘连或与折行列宽失配）。
pub(super) fn sample_output_line(line: &str) -> String {
    let mut text = String::with_capacity(line.len().min(MAX_OUTPUT_LINE_CHARS + 4));
    for (count, ch) in line.chars().enumerate() {
        if count == MAX_OUTPUT_LINE_CHARS {
            text.push('…');
            break;
        }
        text.push(if ch.is_control() { ' ' } else { ch });
    }
    text
}

/// 将多行原始输出按行做单行长度采样（上限 [`MAX_OUTPUT_LINE_CHARS`]）并剥离末尾空行，
/// 供 [`muted_markdown_lines`] 进行有界 Markdown 渲染。
pub(super) fn sample_markdown_source(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let mut out = Vec::new();
    for line in raw.split('\n') {
        let line = line.trim_end_matches('\r');
        out.push(sample_output_line(line));
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out.join("\n")
}

/// 将 markdown 源码按给定列宽排版为思考块同款的压暗多段物理行（`span_style.patch(style::muted())`）。
///
/// 展开内容中的水平线（`---`）保持字面量，不拉满整行 `─` 边框线；
/// 每条返回的 `Vec<Segment>` 对应一个已按 `width` 折好行的物理行（含 `MdLine::indent` 前导空格），
/// 显示总列宽 `<= width.max(1)`；送入 `tree::Node::block` 后经 `wrap_spans(logical, body)` 保持 1:1 物理行映射。
pub(super) fn muted_markdown_lines(source: &str, width: u16) -> Vec<Vec<Segment>> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    crate::ui::markdown_render::render_wrapped_literal_rules(source, width.max(1))
        .into_iter()
        .map(md_line_segments)
        .collect()
}

/// 单行 [`crate::ui::markdown_render::MdLine`] 转为思考块同款的压暗 [`Segment`] 序列：
/// - `line.indent > 0` 时在行首补等宽空格段（`style::muted()`）；
/// - 每个非空分段保留自身 markdown 语法前景色与修饰符，并叠加 `style::muted()`（`DIM`）。
pub(super) fn md_line_segments(line: crate::ui::markdown_render::MdLine) -> Vec<Segment> {
    let mut segments = Vec::with_capacity(line.spans.len() + usize::from(line.indent > 0));
    if line.indent > 0 {
        segments.push(Segment::new(
            " ".repeat(usize::from(line.indent)),
            style::muted(),
        ));
    }
    let mut pending_table = line.table;
    for (value, span_style) in line.spans {
        if !value.is_empty() {
            let mut seg = Segment::new(value, span_style.patch(style::muted()));
            if let Some(table_ref) = pending_table.take() {
                seg.table = Some(table_ref);
            }
            segments.push(seg);
        }
    }
    segments
}

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
        Some(crate::app::state::ToolDetailView::Ask(_)) => {
            ask::draw(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                animating,
                folds.body,
            );
            return;
        }
        Some(crate::app::state::ToolDetailView::Subagent(_)) => {
            subagent::draw(
                grid,
                rect,
                scroll,
                row_index,
                call,
                loading_frame,
                folds.body,
                animating,
            );
            return;
        }
        Some(crate::app::state::ToolDetailView::Search(view)) => {
            search::draw(
                grid,
                rect,
                scroll,
                row_index,
                call,
                view,
                loading_frame,
                folds.body,
                animating,
            );
            return;
        }
        Some(crate::app::state::ToolDetailView::Mcp(view)) => {
            mcp::draw(
                grid,
                rect,
                scroll,
                row_index,
                call,
                view,
                loading_frame,
                folds,
                animating,
            );
            return;
        }
        Some(crate::app::state::ToolDetailView::WebSearch(view)) => {
            web_search::draw(
                grid,
                rect,
                scroll,
                row_index,
                call,
                view,
                loading_frame,
                folds,
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
                call,
                loading_frame,
                folds.body,
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

/// 输出区与附属说明的基础文字档位：折叠与展开均保持 `style::muted()`（dim），
/// 不在展开后切换为默认亮白前景 `style::text()`，避免抢夺主回答正文的视觉焦点；
/// 参数/路径恒为浅黄（[`style::chat_tool_param`]）。
pub(super) fn output_style(_expanded: bool) -> Style {
    style::muted()
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
        Some(crate::app::state::ToolDetailView::Command(_))
        | Some(crate::app::state::ToolDetailView::Interaction(_)) => {
            command::params_expandable(call, width) || command::output_expandable(call, width)
        }
        Some(crate::app::state::ToolDetailView::Plan(_))
        | Some(crate::app::state::ToolDetailView::Ask(_)) => false,
        Some(crate::app::state::ToolDetailView::Subagent(_)) => subagent::has_more(call, width),
        Some(crate::app::state::ToolDetailView::Search(view)) => {
            search::has_more(call, view, width)
        }
        Some(crate::app::state::ToolDetailView::Mcp(view)) => mcp::has_more(call, view, width),
        Some(crate::app::state::ToolDetailView::WebSearch(view)) => {
            web_search::has_more(call, view, width)
        }
        Some(crate::app::state::ToolDetailView::File(_)) if file_diff::is_card(call) => {
            file_diff::has_more_content(call, width)
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => {
            file_summary::detail_content_rows(call, width, true)
                > file_summary::detail_content_rows(call, width, false)
        }
    }
}

/// 子 agent 卡片里的 agent 链接（相对步骤块首的行偏移）。
pub(super) fn agent_links(call: &ToolCallView, width: u16) -> Vec<(usize, String)> {
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Subagent(_)) => subagent::agent_links(call, width),
        _ => Vec::new(),
    }
}

/// MCP 卡片的点击面：参数块（Body）与结果块（Output）各自独立折叠。
///
/// 标题行跟随第一块，点击标题只切换参数块；结果块从参数块之后开始。
fn mcp_regions(
    call: &ToolCallView,
    view: &crate::app::state::McpDetailView,
    width: u16,
    folds: Folds,
) -> Vec<CardRegion> {
    let layout = mcp::layout(call, view, width, folds);
    split_regions(
        layout.arguments_rows,
        layout.result_rows,
        layout.arguments_expandable,
        layout.result_expandable,
    )
}

/// 联网搜索卡片的点击面：查询块（Body）与结果块（Output）各自独立折叠。
fn web_search_regions(
    call: &ToolCallView,
    view: &crate::app::state::WebSearchDetailView,
    width: u16,
    folds: Folds,
) -> Vec<CardRegion> {
    let layout = web_search::layout(call, view, width, folds);
    split_regions(
        layout.queries_rows,
        layout.result_rows,
        layout.queries_expandable,
        layout.result_expandable,
    )
}

/// 两块独立折叠卡片的点击面几何：第一块连同标题行，第二块紧随其后。
fn split_regions(
    first_rows: usize,
    second_rows: usize,
    first_expandable: bool,
    second_expandable: bool,
) -> Vec<CardRegion> {
    let mut regions = Vec::new();
    let mut offset = 0usize;
    if first_rows > 0 {
        let rows = HEADER_ROWS + first_rows;
        regions.push(CardRegion {
            section: CardSection::Body,
            card: RowSpan { offset: 0, rows },
            clickable: first_expandable,
        });
        offset = rows;
    }
    if second_rows > 0 {
        let rows = if offset == 0 {
            HEADER_ROWS + second_rows
        } else {
            second_rows
        };
        regions.push(CardRegion {
            section: CardSection::Output,
            card: RowSpan { offset, rows },
            clickable: second_expandable,
        });
    }
    regions
}

/// 命令/交互卡片的点击面：参数区与输入/输出区各自独立折叠。
fn command_regions(call: &ToolCallView, width: u16, blocks: command::Blocks) -> Vec<CardRegion> {
    let mut regions = vec![CardRegion {
        section: CardSection::Body,
        card: RowSpan {
            offset: HEADER_ROWS,
            rows: blocks.params,
        },
        clickable: command::params_expandable(call, width),
    }];
    if blocks.output > 0 {
        regions.push(CardRegion {
            section: CardSection::Output,
            card: RowSpan {
                offset: HEADER_ROWS + blocks.params,
                rows: blocks.output,
            },
            clickable: command::output_expandable(call, width),
        });
    }
    regions
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
    match call.detail.as_deref() {
        Some(crate::app::state::ToolDetailView::Command(view)) => {
            let blocks = command::blocks(call, width, folds);
            if view.state.is_running() {
                // 长任务：整卡都是「打开后台任务面板」的点击面。
                return vec![CardRegion {
                    section: CardSection::Body,
                    card: RowSpan {
                        offset: 0,
                        rows: HEADER_ROWS + blocks.params + blocks.output,
                    },
                    clickable: true,
                }];
            }
            command_regions(call, width, blocks)
        }
        Some(crate::app::state::ToolDetailView::Interaction(_)) => {
            command_regions(call, width, command::blocks(call, width, folds))
        }
        Some(crate::app::state::ToolDetailView::Plan(_))
        | Some(crate::app::state::ToolDetailView::Ask(_)) => Vec::new(),
        Some(crate::app::state::ToolDetailView::Subagent(_))
        | Some(crate::app::state::ToolDetailView::Search(_)) => {
            if !has_expandable_content(call, width) {
                return Vec::new();
            }
            vec![CardRegion {
                section: CardSection::Body,
                card: RowSpan {
                    offset: 0,
                    rows: step_rows,
                },
                clickable: true,
            }]
        }
        Some(crate::app::state::ToolDetailView::Mcp(view)) => mcp_regions(call, view, width, folds),
        Some(crate::app::state::ToolDetailView::WebSearch(view)) => {
            web_search_regions(call, view, width, folds)
        }
        Some(crate::app::state::ToolDetailView::File(_)) | None => {
            // 展开前后行数不变就不登记：留下一个点了没反应的点击面只会误导。
            if !has_expandable_content(call, width) {
                return Vec::new();
            }
            vec![CardRegion {
                section: CardSection::Body,
                card: RowSpan {
                    offset: 0,
                    rows: step_rows,
                },
                clickable: true,
            }]
        }
    }
}

/// 行内一段着色文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) table: Option<crate::ui::markdown_render::TableLineRef>,
}

impl Segment {
    pub(super) fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
            table: None,
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

/// 固定动词（卡片 title 主体）：运行中橙黄、已完成翡翠绿。
///
/// 用户要求「方块颜色和工具 title 统一为黄色」：动词必须与方块同色，
/// 否则一张卡片两种颜色。
fn verb_label(call: &ToolCallView) -> String {
    use text::agent as a;
    match Kind::of(&call.name) {
        Kind::Read => a::VERB_TOOL_READ.to_string(),
        Kind::Search => a::VERB_TOOL_SEARCH.to_string(),
        Kind::Edit => a::VERB_TOOL_EDIT.to_string(),
        Kind::Write => a::VERB_TOOL_WRITE.to_string(),
        Kind::Other if call.name == "curr_time" => a::VERB_TOOL_CURR_TIME.to_string(),
        Kind::Other => single_line(&call.name, TEXT_SAMPLE_CHARS),
    }
}

/// 组装第 1 行（`■ 动词 · 状态`）：状态紧跟动词，与进程/委派卡片同一读法。
///
/// 失败/中断整行红/黄；`read`/`search` 的种类、条目数与命中数由
/// [`file_summary::title_status`] 提供。
fn header(call: &ToolCallView, width: u16, loading_frame: usize, animating: bool) -> Vec<Segment> {
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
    // 终态整行同色（方块 + 动词）：失败红、中断黄。取消不是错误态，保持工具色
    // （方块与 title 统一；详情行的 `cancelled` 已经说明它没有产出结果）。
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
pub(super) fn put_segments(grid: &mut Grid, x: u16, y: u16, width: u16, segments: &[Segment]) {
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
        let mut pending_table = source.table.clone();
        for ch in source.text.chars() {
            let cell = ch.width().unwrap_or(1).max(1);
            if col + cell > width && col > 0 {
                if !buf.is_empty() {
                    let mut seg = Segment::new(std::mem::take(&mut buf), source.style);
                    seg.table = pending_table.take();
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

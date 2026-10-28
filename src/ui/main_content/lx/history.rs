//! 聊天历史渲染：消息、正文流增量、thinking 折叠块、本轮统计行、滚动条与滚动。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；命中几何（thinking 头部矩形）由
//! 同一布局函数提供，渲染与命中不漂移。
//!
//! - 用户消息：`> ` 前缀（占用整块底色，图标用输入框同款强调色）+ 整块浅底色
//!   （`style::chat_user_bg`），正文按显示列软换行。
//! - assistant 消息：每个步骤（思考 / 工具调用 / 正文块）首行以左侧状态标记起头
//!   （见 [`StepMark`]：文字数据用圆点 `●`/`○`、工具调用用方块 `■`/`□`；运行中两帧
//!   呼吸、终态静态并按语义着色），标记右侧
//!   的内容（含续行、展开的思考正文）整体缩进 [`STEP_GUTTER`] 列；正文用共享阅读
//!   渲染器（`ui::markdown_render`，与 UI 页 agent 回答同款；表格按宽度收缩自适应），
//!   不显示标题行；失败/中断在**消息底部**（统计行上方）插入一条通知（见 `notice.rs`），
//!   流中不插入（改用输入框上方 loading 行）。
//! - thinking：头部为圆点 + `Thinking`（生成中）/ `Thought · 6.3s`（完成后，无折叠
//!   箭头——圆点即状态标记）；点击头部或 Ctrl+T 折叠/展开；正文同样走 markdown
//!   渲染并整体压暗（对齐 opencode 的思考显示）。
//! - 本轮统计行（assistant 末尾）：模型名、输入/输出 token、缓存命中、tok/s 与耗时。
//! - 右侧预留一列滚动条（内容溢出时显示），宽度与输入框对齐。

use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::state::{CardSection, ChatMessageView, Lx, SHELL_SLOT};
use crate::ui::widgets::scrollbar;
use crate::ui::{style, text};

use super::Grid;
use super::notice;

mod cache;
mod cards;
mod compact;
#[cfg(test)]
mod layout;
mod paint;
mod render;
pub(crate) mod report;
mod rewind;
mod rows;
mod snapshot;
mod stats;
mod sticky;
mod timeline;
pub(crate) mod tool;
pub(crate) mod user_shell;

pub(crate) use cache::{PaneCache, RenderCache};
#[cfg(test)]
pub(crate) use layout::layout;
pub(crate) use render::{render, render_source};

pub use timeline::has_history;
use tool::Folds;

pub(crate) use report as report_block;
pub(crate) use rows::{content_rect, rows_to_u16, tool_rows, visible_row, wrap};
pub(crate) use stats::{format_duration, stats_line};

/// 用户消息前缀（`> `）。
const USER_PREFIX: &str = "> ";
/// 用户消息正文缩进列（与前缀等宽）。
const USER_INDENT: u16 = 2;
/// 每个 assistant 步骤左侧状态标记占用的列数（1 列字形 + 1 列空格）；标记右侧的
/// 内容（头部文案、续行、展开的思考正文、markdown 正文）一律缩进到该列之后，
/// 与用户消息的 `> ` 缩进同宽。
pub(super) const STEP_GUTTER: u16 = 2;
/// 头部文案与思考耗时之间的分隔（`Thought · 6.3s`）。
const THINKING_DURATION_SEP: &str = " · ";
/// 历史区右侧滚动条预留列数。
pub(super) const SCROLLBAR_COLUMNS: u16 = crate::layout::LX_HISTORY_SCROLLBAR;

/// assistant 步骤左侧状态标记的语义状态（思考 / 工具调用 / 正文块共用）。
///
/// 字形按 [`StepKind`] 分派（见 [`Self::glyph`]）：文字数据（正文 / 思考 / 通知）为
/// 圆点 `●`/`○`，工具调用为方块 `■`/`□`，子 agent 主动汇报为上三角 `▲`/`△`。
/// 运行中为实心/空心两帧呼吸（相位由 `LxChat::loading_frame` 决定，约 400ms 一相）；
/// 终态一律停为**实心**标记、仅以颜色区分：失败（红）、中断（黄）。取消不再特殊化：
/// 标记与动词同用角色色（用户要求「方块和 title 统一」，空心/白色方块都会显得这张卡
/// 是另一种状态）。
///
/// **颜色由「角色」决定，不只看状态**（用户要求区分四类步骤）：
/// - [`StepKind::Body`]（正文块）：**终端默认前景**——圆点跟正文同色，不额外抢眼；
/// - [`StepKind::Thinking`]（思考块）：折叠浅粉 `Indexed 182`、展开亮粉 `Indexed 218`
///   （鼠标悬停不改变颜色）；
/// - [`StepKind::Tool`]（工具调用）：翡翠绿 `Indexed 42`；
/// - [`StepKind::Report`]（子 agent 汇报）：沿用工具档颜色，仅字形改为上三角。
///
/// 失败 / 中断是**跨角色的语义状态**，一律覆盖角色色（红 / 黄），保证「出问题了」
/// 比「是哪类步骤」更先被看到；取消不是错误，沿用角色色，只由详情行的 `cancelled` 说明。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepMark {
    Running,
    /// 后台进程仍存活（`exec_command` / `write_stdin` 卡片专用）：橙色。
    ///
    /// 与 [`Self::Running`] 的区别只在**颜色**（橙 = 任务还在后台烧）；字形规则完全
    /// 相同——两者都靠"有动画时才闪"，见 [`Self::glyph`]。
    Alive,
    Done,
    Failed,
    Interrupted,
    Cancelled,
}

impl StepMark {
    /// 标记字形：**有动画时**运行中按帧呼吸；终态（含取消）恒实心。
    ///
    /// `animating` 是"这一帧的相位是否还在推进"（渲染层传 `lx.chat.status.streaming`）。
    /// **动画一停就必须画实心**：相位冻结在空心那一相会让一个活着的后台任务永久显示
    /// 成 `□`，看起来像失败/取消（用户带截图反馈的真实 bug）。把兜底写在这里而不是
    /// 让状态层在收尾时手动归零，是因为导致动画停下的路径很多（流结束、面板遮挡、
    /// 事件循环短暂卡住），漏掉任何一处都会复现。
    pub(super) fn glyph(self, kind: StepKind, loading_frame: usize, animating: bool) -> char {
        let filled = match self {
            Self::Running | Self::Alive => {
                !animating || (loading_frame / text::agent::STEP_BREATH_TICKS).is_multiple_of(2)
            }
            Self::Done | Self::Failed | Self::Interrupted | Self::Cancelled => true,
        };
        // 文字数据用圆点、工具调用用方块、子 agent 汇报用上三角。
        match (kind, filled) {
            (StepKind::Tool, true) => text::agent::STEP_MARK_FILLED,
            (StepKind::Tool, false) => text::agent::STEP_MARK_HOLLOW,
            (StepKind::Report, true) => text::agent::STEP_MARK_TRIANGLE_FILLED,
            (StepKind::Report, false) => text::agent::STEP_MARK_TRIANGLE_HOLLOW,
            (_, true) => text::agent::STEP_MARK_DOT_FILLED,
            (_, false) => text::agent::STEP_MARK_DOT_HOLLOW,
        }
    }

    /// 标记样式：先看状态（失败/中断/取消跨角色覆盖），否则按角色取色。
    ///
    /// `expanded` 只对思考块有意义（折叠浅粉 → 展开亮粉）；其余角色忽略它。
    /// 鼠标悬停不再影响颜色。
    pub(super) fn style(self, kind: StepKind, expanded: bool) -> Style {
        match self {
            Self::Failed => style::agent_fail(),
            Self::Interrupted => style::agent_warn(),
            // 取消不是错误态：方块与动词统一用角色色（工具行 = 工具绿）。
            Self::Cancelled => kind.style_done(expanded),
            Self::Running => kind.style_running(expanded),
            Self::Alive => style::chat_step_mark_alive(),
            Self::Done => kind.style_done(expanded),
        }
    }

    /// 标记段（字形 + 空格）；工具行与 thinking 头部共用同一份拼法。
    /// 颜色由调用方用 [`Self::style`] 单独上。
    pub(super) fn segment(self, kind: StepKind, loading_frame: usize, animating: bool) -> String {
        format!("{} ", self.glyph(kind, loading_frame, animating))
    }
}

/// 步骤的**类别**：决定运行中 / 已完成时的标记与文案用什么颜色与字形。
///
/// 四类的分工：
/// - `Body` 用终端默认前景 → 圆点与正文本就同色，视觉上不打断阅读；
/// - `Thinking` 用粉色（折叠 182 / 展开 218）→ 一眼看出「这段是模型的内心活动」；
/// - `Tool` 用翡翠绿 → 一眼看出「这段是真实动作」，且与 markdown 的 GitHub 色域不相交；
/// - `Report` 沿用工具档颜色，但字形用上三角 → 一眼看出「子 agent 主动汇报」，
///   与主 agent 自己发起的工具调用区分开。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepKind {
    /// 正文块（assistant 的可见回答）。
    Body,
    /// 思考块（reasoning / thinking）。
    Thinking,
    /// 工具调用行。
    Tool,
    /// 子 agent 主动汇报块。
    Report,
}

impl StepKind {
    /// 运行中（含呼吸）的样式。`expanded` 只影响思考块。
    fn style_running(self, expanded: bool) -> Style {
        match self {
            Self::Body => style::chat_step_mark_running_body(),
            Self::Thinking => style::chat_step_mark_running_thinking(expanded),
            // 运行中的工具统一橙黄（与 `Alive` 同档）：用户要求"所有正在运行中的
            // tool 的方块和 title 统一为黄色，完成才显示绿色"。
            Self::Tool | Self::Report => style::chat_step_mark_alive(),
        }
    }

    /// 已完成（静止）的样式。
    fn style_done(self, expanded: bool) -> Style {
        match self {
            Self::Body => style::chat_step_mark_done_body(),
            Self::Thinking => style::chat_step_mark_done_thinking(expanded),
            Self::Tool | Self::Report => style::chat_step_mark_done(),
        }
    }

    /// 该角色的头部文案样式（运行中 / 已完成）。`expanded` 只影响思考块。
    pub(super) fn label(self, running: bool, expanded: bool) -> Style {
        match (self, running) {
            (Self::Body, true) => style::chat_step_label_running_body(),
            (Self::Body, false) => style::chat_step_label_done_body(),
            (Self::Thinking, true) => style::chat_step_label_running_thinking(expanded),
            (Self::Thinking, false) => style::chat_step_label_done_thinking(expanded),
            (Self::Tool | Self::Report, true) => style::chat_step_label_running(),
            (Self::Tool | Self::Report, false) => style::chat_step_label_done(),
        }
    }
}

/// 一条消息在渲染行空间中的区间（`end` 不含；含它**前面**的空行，
/// 末尾那条含收尾呼吸行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRows {
    pub start: usize,
    pub end: usize,
    pub is_user: bool,
}

/// 历史区布局：内容矩形 + 每个 thinking 折叠块的头部矩形（仅测试使用）。
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLayout {
    pub area: Rect,
    /// （消息索引, 该消息内**步骤**下标, 头部矩形）。
    pub thinking: Vec<(usize, usize, Rect)>,
    /// 可折叠块的整块矩形与内容矩形（点击切换折叠；没有 hover 高亮）。
    pub cards: Vec<CardHit>,
    /// 每条消息的行区间（置顶行归属判定用）。
    pub messages: Vec<MessageRows>,
    /// 渲染出的内容总行数（滚动上限用）。
    pub total_rows: usize,
}

pub use sticky::StickyRow;
mod source;
pub(crate) use source::HistorySource;
use sticky::sticky_candidate;

pub(crate) use sticky::{render_sticky_prompt, sticky_rect, truncate_with_ellipsis};

pub use cards::{CardHit, CardKind};

/// 历史视图：渲染 / 滚轮钳制 / 滚动条 / 命中 / 置顶行的**唯一**几何口径。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryView {
    /// 历史整区（含置顶行与滚动条列）。
    pub area: Rect,
    /// 内容区：置顶行让位后比 `area` 少 1 行；不显示置顶行时等于 `area`。
    pub body: Rect,
    /// body 顶部行索引（0 = 最早）。
    pub scroll: usize,
    /// 渲染出的内容总行数（滚动上限用）。
    pub total_rows: usize,
    /// 每条消息的行区间。
    pub messages: Vec<MessageRows>,
    /// 要钉的用户消息索引（None = 不画置顶行）。
    pub sticky: Option<usize>,
    /// 本帧要写回状态的新记忆（[`crate::app::state::lx::chat::StickySuppression`]）。
    pub suppressed: Option<crate::app::state::lx::chat::StickySuppression>,
}

/// 合并卡里某一块是否登记成可点击面：只有**内容能展开**才登记。
///
/// 合并组只含文件差异卡片；展开前后行数不变时点了也不会有任何变化，
/// 登记只会制造「看得见点不动」的错觉。
fn should_register_card(call: &crate::app::state::ToolCallView, width: u16) -> bool {
    tool::has_expandable_content(call, width)
}

/// 本地命令卡片两块（参数 / 输出）的展开状态。
fn shell_folds(source: HistorySource<'_>, message_id: &str) -> Folds {
    Folds {
        body: source
            .chat
            .section_is_expanded(message_id, SHELL_SLOT, CardSection::Body),
        output: source
            .chat
            .section_is_expanded(message_id, SHELL_SLOT, CardSection::Output),
    }
}

/// 一张工具卡片两块（主体 / 输出）的展开状态。
fn folds_of(
    source: HistorySource<'_>,
    message_id: &str,
    call: &crate::app::state::ToolCallView,
) -> tool::Folds {
    tool::Folds {
        body: source.chat.detail_is_expanded(message_id, &call.call_id),
        output: source
            .chat
            .section_is_expanded(message_id, &call.call_id, CardSection::Output),
    }
}

/// 计算历史视图（纯函数，与渲染同源）：滚动偏移、body 矩形、置顶行结论与新记忆。
///
/// 置顶行判定（对齐 codex）：
/// 1. 先按**整块** body 判一次：顶行落在用户消息自己的行上 → 不钉（用户消息自带上下文），
///    否则向前找最近一条有正文的用户消息；
/// 2. 需要钉才让位一行（body 高度 -1）并在让位后的 scroll 上**重判**；重判不再需要
///    → 撤销让位、不画置顶行，并把"整块视口 + 顶行位置"记入记忆；
/// 3. 非跟随时，若记忆与当前位置一致 → 直接跳过重判（沿用抑制，防让位/撤销抖动）。
pub(super) fn view(area: Rect, lx: &Lx, cache: &mut PaneCache) -> HistoryView {
    view_source(area, HistorySource::from(lx), cache)
}

/// 数据源版本（子 agent 详情复用）。
pub(super) fn view_source(
    area: Rect,
    source: HistorySource<'_>,
    cache: &mut PaneCache,
) -> HistoryView {
    view_source_inner(area, source, cache, true)
}

/// 不计算置顶行的数据源视图：Agents 详情只画 transcript，没有置顶 prompt。
///
/// 若继续走 [`view_source`]，会为一条根本不会绘制的置顶行预留 1 行，造成首行空白、
/// 可见行数与滚动上限各差 1 行。
pub(super) fn view_source_plain(
    area: Rect,
    source: HistorySource<'_>,
    cache: &mut PaneCache,
) -> HistoryView {
    view_source_inner(area, source, cache, false)
}

fn view_source_inner(
    area: Rect,
    source: HistorySource<'_>,
    cache: &mut PaneCache,
    allow_sticky: bool,
) -> HistoryView {
    // 生产路径只要行数与消息区间：直接读快照，不构造每张卡片的 `CardHit`。
    let snapshot = cache.snapshot_source(content_rect(area).width, source);
    let total = snapshot.total_rows;
    let full_height = area.height;
    let scroll_for = |height: u16| -> usize {
        let max_top = total.saturating_sub(usize::from(height));
        if source.chat.follow_bottom {
            max_top
        } else {
            source.chat.history_scroll.min(max_top)
        }
    };
    let mut view = HistoryView {
        area,
        body: area,
        scroll: scroll_for(full_height),
        total_rows: total,
        messages: snapshot.messages.clone(),
        sticky: None,
        suppressed: None,
    };
    if !allow_sticky {
        return view;
    }
    if total == 0 || full_height < sticky::MIN_HEIGHT || area.width < sticky::MIN_WIDTH {
        return view;
    }
    let top = sticky::message_at(&view.messages, view.scroll);
    let position = top.map(
        |(message, row)| crate::app::state::lx::chat::StickySuppression {
            viewport: area,
            message,
            row,
        },
    );
    // 阅读态且位置未变：沿用抑制（本帧跳过重判）。
    if !source.chat.follow_bottom
        && source
            .chat
            .suppressed_prompt_header
            .is_some_and(|remembered| Some(remembered) == position)
    {
        view.suppressed = source.chat.suppressed_prompt_header;
        return view;
    }
    let Some(candidate) = sticky_candidate(source.messages, &view.messages, view.scroll) else {
        return view;
    };
    // 让位一行后重判（body 下移 1 行，置顶行占历史整区第 0 行）。
    let body = Rect {
        y: area.y.saturating_add(1),
        height: full_height - 1,
        ..area
    };
    let scroll_body = scroll_for(body.height);
    if sticky_candidate(source.messages, &view.messages, scroll_body).is_none() {
        view.suppressed = position;
        return view;
    }
    view.body = body;
    view.scroll = scroll_body;
    view.sticky = Some(candidate);
    view
}

/// 历史区滚动条几何（贴右缘一列；内容未溢出时为 None）。
/// 只量总行数（一次性缓存）：`Agents` 详情区的滚动上限用。
pub(crate) fn measure(width: u16, source: HistorySource<'_>) -> usize {
    if width == 0 {
        return 0;
    }
    let mut cache = PaneCache::default();
    cache.snapshot_source(width, source).total_rows
}

pub(super) fn scrollbar(
    area: Rect,
    total: usize,
    visible: usize,
    scroll: usize,
) -> Option<scrollbar::ScrollbarLayout> {
    scrollbar::layout(area, total, visible, scroll)
}

/// 消息底部通知：失败（错误 + 原因码）与中断（提示 + 固定原因）；完成态与流中不显示。
fn notice(message: &ChatMessageView) -> Option<notice::Notice> {
    if message.role == "user" {
        return None;
    }
    match message.status.as_str() {
        "failed" => Some(notice::Notice::new(
            notice::NoticeTitle::Failed,
            message
                .error_code
                .clone()
                .filter(|code| !code.is_empty())
                .unwrap_or_else(|| text::LX_CHAT_NOTICE_FAILED_REASON.to_string()),
        )),
        "interrupted" => Some(notice::Notice::new(
            notice::NoticeTitle::Interrupted,
            text::LX_CHAT_NOTICE_INTERRUPTED_REASON,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

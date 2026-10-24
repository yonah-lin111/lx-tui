//! 聊天历史渲染：消息、正文流增量、thinking 折叠块、本轮统计行、滚动条与滚动。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；命中几何（thinking 头部矩形）由
//! 同一布局函数提供，渲染与命中不漂移。
//!
//! - 用户消息：`> ` 前缀（占用整块底色，图标用输入框同款强调色）+ 整块浅底色
//!   （`style::chat_user_bg`），正文按显示列软换行。
//! - assistant 消息：每个步骤（思考 / 工具调用 / 正文块）首行以左侧方块标记起头
//!   （见 [`StepMark`]：运行中 `■`/`□` 两帧呼吸、终态静态并按语义着色），标记右侧
//!   的内容（含续行、展开的思考正文）整体缩进 [`STEP_GUTTER`] 列；正文用共享阅读
//!   渲染器（`ui::markdown_render`，与 UI 页 agent 回答同款；表格按宽度收缩自适应），
//!   不显示标题行；失败/中断在**消息底部**（统计行上方）插入一条通知（见 `notice.rs`），
//!   流中不插入（改用输入框上方 loading 行）。
//! - thinking：头部为方块 + `Thinking`（生成中）/ `Thought · 6.3s`（完成后，无折叠
//!   箭头——方块即状态标记）；点击头部或 Ctrl+T 折叠/展开；正文同样走 markdown
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
mod layout;
mod paint;
mod render;
mod rewind;
mod rows;
mod snapshot;
mod stats;
mod sticky;
mod timeline;
pub(crate) mod tool;
pub(crate) mod user_shell;

pub(crate) use cache::{PaneCache, RenderCache};
pub(crate) use layout::layout;
pub(crate) use render::render;

pub use timeline::has_history;
use tool::Folds;

pub(crate) use rows::{content_rect, rows_to_u16, tool_rows, visible_row, wrap};
pub(crate) use stats::{format_duration, stats_line};

/// 用户消息前缀（`> `）。
const USER_PREFIX: &str = "> ";
/// 用户消息正文缩进列（与前缀等宽）。
const USER_INDENT: u16 = 2;
/// 每个 assistant 步骤左侧方块标记占用的列数（1 列字形 + 1 列空格）；标记右侧的
/// 内容（头部文案、续行、展开的思考正文、markdown 正文）一律缩进到该列之后，
/// 与用户消息的 `> ` 缩进同宽。
pub(super) const STEP_GUTTER: u16 = 2;
/// 头部文案与思考耗时之间的分隔（`Thought · 6.3s`）。
const THINKING_DURATION_SEP: &str = " · ";
/// 历史区右侧滚动条预留列数。
pub(super) const SCROLLBAR_COLUMNS: u16 = crate::layout::LX_HISTORY_SCROLLBAR;

/// assistant 步骤左侧方块标记的语义状态（思考 / 工具调用 / 正文块共用）。
///
/// 运行中为 `■`/`□` 两帧呼吸（相位由 `LxChat::loading_frame` 决定，约 400ms 一相）；
/// 终态停为静态方块、仅以颜色区分：失败（红）、中断（黄）、取消（次要信息色 + 空心
/// `□`，表示未产出结果）。
///
/// **颜色由「角色」决定，不只看状态**（用户要求区分三类步骤）：
/// - [`StepKind::Body`]（正文块）：**终端默认前景**——方块跟正文同色，不额外抢眼；
/// - [`StepKind::Thinking`]（思考块）：折叠浅粉 `Indexed 182`、展开/悬停亮粉 `Indexed 218`；
/// - [`StepKind::Tool`]（工具调用）：翡翠绿 `Indexed 42`。
///
/// 失败 / 中断 / 取消是**跨角色的语义状态**，一律覆盖角色色（红 / 黄 / 次要灰），
/// 保证「出问题了」比「是哪类步骤」更先被看到。
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
    /// 方块字形：**有动画时**运行中按帧呼吸；取消恒空心，其余终态恒实心。
    ///
    /// `animating` 是"这一帧的相位是否还在推进"（渲染层传 `lx.chat.status.streaming`）。
    /// **动画一停就必须画实心**：相位冻结在空心那一相会让一个活着的后台任务永久显示
    /// 成 `□`，看起来像失败/取消（用户带截图反馈的真实 bug）。把兜底写在这里而不是
    /// 让状态层在收尾时手动归零，是因为导致动画停下的路径很多（流结束、面板遮挡、
    /// 事件循环短暂卡住），漏掉任何一处都会复现。
    pub(super) fn glyph(self, loading_frame: usize, animating: bool) -> char {
        let filled = match self {
            Self::Running | Self::Alive => {
                !animating || (loading_frame / text::agent::STEP_BREATH_TICKS).is_multiple_of(2)
            }
            Self::Done | Self::Failed | Self::Interrupted => true,
            Self::Cancelled => false,
        };
        if filled {
            text::agent::STEP_MARK_FILLED
        } else {
            text::agent::STEP_MARK_HOLLOW
        }
    }

    /// 方块样式：先看状态（失败/中断/取消跨角色覆盖），否则按角色取色。
    ///
    /// `expanded` / `hovered` 只对思考块有意义（折叠浅粉 → 展开/悬停亮粉）；
    /// 其余角色忽略它们。
    pub(super) fn style(self, kind: StepKind, expanded: bool, hovered: bool) -> Style {
        match self {
            Self::Failed => style::agent_fail(),
            Self::Interrupted => style::agent_warn(),
            Self::Cancelled => style::agent_step_detail(),
            Self::Running => kind.style_running(expanded, hovered),
            Self::Alive => style::chat_step_mark_alive(),
            Self::Done => kind.style_done(expanded, hovered),
        }
    }

    /// 标记段（字形 + 空格）；工具行与 thinking 头部共用同一份拼法。
    /// 颜色由调用方用 [`Self::style`] 单独上，所以这里不需要 [`StepKind`]。
    pub(super) fn segment(self, loading_frame: usize, animating: bool) -> String {
        format!("{} ", self.glyph(loading_frame, animating))
    }
}

/// 步骤的**类别**：决定运行中 / 已完成时的方块与文案用什么颜色。
///
/// 三类的分工（用户要求「正文方块用默认前景、thinking 用别的颜色」）：
/// - `Body` 用终端默认前景 → 方块与正文本就同色，视觉上不打断阅读；
/// - `Thinking` 用粉色（折叠 182 / 展开 218）→ 一眼看出「这段是模型的内心活动」；
/// - `Tool` 用翡翠绿 → 一眼看出「这段是真实动作」，且与 markdown 的 GitHub 色域不相交。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StepKind {
    /// 正文块（assistant 的可见回答）。
    Body,
    /// 思考块（reasoning / thinking）。
    Thinking,
    /// 工具调用行。
    Tool,
}

impl StepKind {
    /// 运行中（含呼吸）的样式。`expanded` / `hovered` 只影响思考块。
    fn style_running(self, expanded: bool, hovered: bool) -> Style {
        match self {
            Self::Body => style::chat_step_mark_running_body(),
            Self::Thinking => style::chat_step_mark_running_thinking(expanded, hovered),
            // 运行中的工具统一橙黄（与 `Alive` 同档）：用户要求"所有正在运行中的
            // tool 的方块和 title 统一为黄色，完成才显示绿色"。
            Self::Tool => style::chat_step_mark_alive(),
        }
    }

    /// 已完成（静止）的样式。
    fn style_done(self, expanded: bool, hovered: bool) -> Style {
        match self {
            Self::Body => style::chat_step_mark_done_body(),
            Self::Thinking => style::chat_step_mark_done_thinking(expanded, hovered),
            Self::Tool => style::chat_step_mark_done(),
        }
    }

    /// 该角色的头部文案样式（运行中 / 已完成）。`expanded` / `hovered` 只影响思考块。
    pub(super) fn label(self, running: bool, expanded: bool, hovered: bool) -> Style {
        match (self, running) {
            (Self::Body, true) => style::chat_step_label_running_body(),
            (Self::Body, false) => style::chat_step_label_done_body(),
            (Self::Thinking, true) => style::chat_step_label_running_thinking(expanded, hovered),
            (Self::Thinking, false) => style::chat_step_label_done_thinking(expanded, hovered),
            (Self::Tool, true) => style::chat_step_label_running(),
            (Self::Tool, false) => style::chat_step_label_done(),
        }
    }
}

/// 消息内**前一块与本块之间是否要空一行**。
///
/// 已定稿规则：**一律留空行**（含连续工具步骤之间、两个相邻正文块之间）。每个工具
/// 调用是独立的动作记录，两行式的 `■ 动词` / `  └─ 详情` 与下一个调用之间空一行才分得清
/// 彼此的边界——不留空行时会读成一棵树，误以为下面的 `└─` 属于上一个调用。
///
/// 该规则**当前不区分块类别**，因此本函数不需要任何参数，也不保留
/// 「`previous: Block, next: Block` 但恒返回 true」这种看起来支持区分、实则恒真的死抽象
/// （review P3-3）。`layout` 与 `render` 必须共用本函数，否则滚动条/置顶行/命中矩形会与
/// 实际绘制错位一行；将来若要按类别收敛留白，在这里加参数并同步两个调用点即可。
fn needs_spacer() -> bool {
    true
}

/// 一条消息在渲染行空间中的区间（`end` 不含；含它**前面**的空行，
/// 末尾那条含收尾呼吸行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRows {
    pub start: usize,
    pub end: usize,
    pub is_user: bool,
}

/// 历史区布局：内容矩形 + 每个 thinking 折叠块的头部矩形（命中用）。
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
fn shell_folds(lx: &Lx, message_id: &str) -> Folds {
    Folds {
        body: lx
            .chat
            .section_is_expanded(message_id, SHELL_SLOT, CardSection::Body),
        output: lx
            .chat
            .section_is_expanded(message_id, SHELL_SLOT, CardSection::Output),
    }
}

/// 一张工具卡片两块（主体 / 输出）的展开状态。
fn folds_of(lx: &Lx, message_id: &str, call: &crate::app::state::ToolCallView) -> tool::Folds {
    tool::Folds {
        body: lx.chat.detail_is_expanded(message_id, &call.call_id),
        output: lx
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
    let layout = layout(area, lx, cache);
    let total = layout.total_rows;
    let full_height = area.height;
    let scroll_for = |height: u16| -> usize {
        let max_top = total.saturating_sub(usize::from(height));
        if lx.chat.follow_bottom {
            max_top
        } else {
            lx.chat.history_scroll.min(max_top)
        }
    };
    let mut view = HistoryView {
        area,
        body: area,
        scroll: scroll_for(full_height),
        total_rows: total,
        messages: layout.messages,
        sticky: None,
        suppressed: None,
    };
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
    if !lx.chat.follow_bottom
        && lx
            .chat
            .suppressed_prompt_header
            .is_some_and(|remembered| Some(remembered) == position)
    {
        view.suppressed = lx.chat.suppressed_prompt_header;
        return view;
    }
    let Some(candidate) = sticky_candidate(&lx.chat.messages, &view.messages, view.scroll) else {
        return view;
    };
    // 让位一行后重判（body 下移 1 行，置顶行占历史整区第 0 行）。
    let body = Rect {
        y: area.y.saturating_add(1),
        height: full_height - 1,
        ..area
    };
    let scroll_body = scroll_for(body.height);
    if sticky_candidate(&lx.chat.messages, &view.messages, scroll_body).is_none() {
        view.suppressed = position;
        return view;
    }
    view.body = body;
    view.scroll = scroll_body;
    view.sticky = Some(candidate);
    view
}

/// 历史区滚动条几何（贴右缘一列；内容未溢出时为 None）。
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

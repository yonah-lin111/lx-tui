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

use crate::app::state::{AssistantStepView, CardSection, ChatMessageView, Lx, SHELL_SLOT};
use crate::ui::widgets::scrollbar;
use crate::ui::{style, text};

use super::Grid;
use super::notice;

mod body;
mod cards;
mod compact;
mod rewind;
mod rows;
mod stats;
mod sticky;
pub(crate) mod tool;
pub(crate) mod user_shell;

use tool::Folds;

pub(crate) use rows::{content_rect, rows_to_u16, text_rows, tool_rows, visible_row, wrap};
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

/// 历史时间线条目：真实消息、独立撤销提示与 compact 检查点按 DB sequence 合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimelineEntry {
    Message(usize),
    /// 连续撤销提示合并为一条；`last` 是最后一条（描述取它），`count` 用于 `xN`。
    Notices {
        last: usize,
        count: usize,
    },
    /// compact 检查点（独立条目；不合并，每条各自可折叠）。
    Compact(usize),
}

/// 是否有可绘制的历史（消息为空但提示/检查点非空时仍显示历史，不退回欢迎页）。
pub fn has_history(lx: &Lx) -> bool {
    !lx.chat.messages.is_empty() || !lx.chat.notices.is_empty() || !lx.compact.is_empty()
}

/// 按 sequence 合并消息、撤销提示与 compact 检查点。
///
/// 三者共用同一编号空间（DB 唯一分配），因此按 sequence 升序交叉合并即可；
/// 同序号不可能出现，保险起见消息在前。连续撤销提示仍合并成一条 `xN`，
/// 而 compact 检查点**不合并**：每条摘要都需要独立展开。
fn timeline(lx: &Lx) -> Vec<TimelineEntry> {
    let mut out = Vec::with_capacity(
        lx.chat.messages.len() + lx.chat.notices.len() + lx.compact.checkpoints.len(),
    );
    let mut message = 0usize;
    let mut notice = 0usize;
    let mut checkpoint = 0usize;
    while message < lx.chat.messages.len()
        || notice < lx.chat.notices.len()
        || checkpoint < lx.compact.checkpoints.len()
    {
        let next_message = lx.chat.messages.get(message).map(|item| item.sequence);
        let next_notice = lx.chat.notices.get(notice).map(|item| item.sequence);
        let next_checkpoint = lx
            .compact
            .checkpoints
            .get(checkpoint)
            .map(|item| item.sequence);
        // 取三者中最小 sequence 的条目（相等时消息优先，其次提示，最后检查点）。
        let take_message = match (next_message, next_notice, next_checkpoint) {
            (None, None, None) => break,
            (Some(sequence), notice, checkpoint) => {
                let mut smallest = sequence;
                for other in [notice, checkpoint].into_iter().flatten() {
                    smallest = smallest.min(other);
                }
                sequence == smallest
            }
            _ => false,
        };
        if take_message {
            out.push(TimelineEntry::Message(message));
            message += 1;
            continue;
        }
        let take_notice = match (next_notice, next_checkpoint) {
            (Some(sequence), Some(other)) => sequence <= other,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if take_notice {
            // 连续提示合并：吸收下一条条目前的所有提示（它们按 sequence 相邻）。
            notice += 1;
            let mut count = 1;
            while notice < lx.chat.notices.len() {
                let next = lx.chat.notices[notice].sequence;
                let blocked = next_message.is_some_and(|sequence| sequence <= next)
                    || next_checkpoint.is_some_and(|sequence| sequence <= next);
                if blocked {
                    break;
                }
                count += 1;
                notice += 1;
            }
            out.push(TimelineEntry::Notices {
                last: notice - 1,
                count,
            });
            continue;
        }
        out.push(TimelineEntry::Compact(checkpoint));
        checkpoint += 1;
    }
    out
}
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

/// 计算历史布局（按当前展开状态）。`area` 为整个历史区（含滚动条列）。
pub(crate) fn layout(area: Rect, lx: &Lx) -> HistoryLayout {
    let content = content_rect(area);
    let width = content.width;
    let mut rows = 0usize;
    let mut thinking = Vec::new();
    let mut cards: Vec<CardHit> = Vec::new();
    if width == 0 {
        return HistoryLayout {
            area,
            thinking,
            cards: Vec::new(),
            messages: Vec::new(),
            total_rows: 0,
        };
    }
    let mut messages_rows: Vec<MessageRows> = Vec::new();
    let timeline = timeline(lx);
    let mut entry_has_previous = false;
    for entry in &timeline {
        let message_index = match entry {
            TimelineEntry::Message(index) => *index,
            TimelineEntry::Notices { last, count } => {
                // 撤销提示是独立历史条目：与消息之间同样留一空行。
                if entry_has_previous {
                    rows += 1;
                }
                entry_has_previous = true;
                rows += rewind::rows(&lx.chat.notices[*last], *count, width);
                continue;
            }
            TimelineEntry::Compact(index) => {
                // compact 条目同样独立成条：与消息之间留一空行，行数与渲染同口径。
                if entry_has_previous {
                    rows += 1;
                }
                entry_has_previous = true;
                let start = rows;
                let notice = &lx.compact.checkpoints[*index];
                let expanded = compact::is_expanded(lx, &notice.id);
                let notice_rows = compact::rows(notice, 1, width, expanded);
                rows += notice_rows;
                // 整块都是点击面：折叠态只有头部两行，点哪里都应能展开。
                cards.push(CardHit {
                    kind: CardKind::Compact {
                        checkpoint_id: notice.id.clone(),
                        expanded,
                    },
                    card: Rect::new(
                        area.x,
                        area.y.saturating_add(rows_to_u16(start)),
                        width,
                        rows_to_u16(notice_rows),
                    ),
                    fold: Rect::new(
                        area.x,
                        area.y.saturating_add(rows_to_u16(start)),
                        width,
                        rows_to_u16(notice_rows),
                    ),
                    clickable: true,
                });
                continue;
            }
        };
        let message = &lx.chat.messages[message_index];
        // 条目间空行归**后**一条消息（codex 的 leading spacer 口径）：顶行落在轮边界
        // 空行上时算用户消息自己的行 → 用户消息自带上下文，不置顶。
        if entry_has_previous {
            rows += 1;
        }
        entry_has_previous = true;
        let start = rows;
        // 本地命令是独立卡片：不进入 assistant 步骤时间线，也不参与置顶用户行。
        if let Some(shell) = &message.shell {
            let folds = shell_folds(lx, &message.id);
            let shell_rows = user_shell::rows(
                shell,
                width,
                lx.chat.loading_frame,
                lx.chat.status.streaming || shell.is_running(),
                folds,
            );
            rows += shell_rows;
            let block_y =
                |offset: usize| -> u16 { area.y.saturating_add(rows_to_u16(start + offset)) };
            for region in user_shell::regions(shell, width, folds) {
                let expanded = folds.of(region.section);
                cards.push(CardHit {
                    kind: CardKind::Shell {
                        message_id: message.id.clone(),
                        section: region.section,
                        expanded,
                    },
                    card: Rect::new(
                        area.x,
                        block_y(region.offset),
                        width,
                        rows_to_u16(region.rows),
                    ),
                    // 参数区恒为浅黄、不提亮，只有输出区/引导行响应 hover。
                    fold: Rect::new(
                        area.x,
                        block_y(region.offset),
                        width,
                        if region.hoverable {
                            rows_to_u16(region.rows)
                        } else {
                            0
                        },
                    ),
                    clickable: region.clickable,
                });
            }
            messages_rows.push(MessageRows {
                start,
                end: rows,
                is_user: false,
            });
            continue;
        }
        // 消息内块之间一律空一行（见 [`needs_spacer`]）。
        let mut has_previous = false;
        let mut step_index = 0usize;
        while let Some(step) = message.steps.get(step_index) {
            // 同种文件工具的连续调用共用一行合并标题，但**每个调用各自独立**：
            // 自己的文件块、自己的展开状态与自己的展开入口（块间空一行分隔）。
            if let AssistantStepView::Tool { .. } = step {
                let group = tool::merge_group(&message.steps, step_index);
                if group.len() > 1 {
                    let expanded: Vec<bool> = group
                        .iter()
                        .map(|call| lx.chat.detail_is_expanded(&message.id, &call.call_id))
                        .collect();
                    let group_rows = tool::group_rows(&group, width, &expanded);
                    if group_rows > 0 {
                        if has_previous && needs_spacer() {
                            rows += 1;
                        }
                        has_previous = true;
                        let block_y = |offset: usize| -> u16 {
                            area.y
                                .saturating_add(u16::try_from(rows + offset).unwrap_or(u16::MAX))
                        };
                        let blocks = tool::group_blocks(&group, width, &expanded);
                        for (index, block) in blocks.iter().enumerate() {
                            let call = group[index];
                            // 每块各自一整块点击面：点它只切换这一块（展开互不影响）。
                            if should_register_card(group[index], width) {
                                cards.push(CardHit {
                                    kind: CardKind::Tool {
                                        message_id: message.id.clone(),
                                        call_id: call.call_id.clone(),
                                        section: CardSection::Body,
                                        expanded: expanded[index],
                                    },
                                    card: Rect::new(
                                        area.x,
                                        block_y(block.start),
                                        width,
                                        rows_to_u16(block.len),
                                    ),
                                    clickable: true,
                                    // 逐块自己的 hover 区：diff 卡只有提示行，其它卡片是
                                    // 整段折叠内容。展开后没有折叠内容，不再提亮。
                                    fold: if expanded[index] {
                                        Rect::new(area.x, block_y(block.start), width, 0)
                                    } else {
                                        let region = tool::fold_region(call, width);
                                        let start = block.start + region.offset;
                                        let rows = region
                                            .rows
                                            .min(block.len.saturating_sub(region.offset));
                                        Rect::new(area.x, block_y(start), width, rows_to_u16(rows))
                                    },
                                });
                            }
                        }
                        rows += group_rows;
                    }
                    step_index += group.len();
                    continue;
                }
            }
            let (is_reasoning, step_rows) = match step {
                AssistantStepView::Part { part, .. } => match part.kind {
                    crate::chat::events::PartKind::Text => {
                        (false, text_rows(message, &part.content, width))
                    }
                    crate::chat::events::PartKind::Reasoning => (
                        true,
                        1 + if lx.chat.reasoning_expanded(message_index, step_index) {
                            body::thinking_rows(&part.content, width)
                        } else {
                            0
                        },
                    ),
                },
                AssistantStepView::Tool { call, .. } => (
                    false,
                    tool_rows(call, width, folds_of(lx, &message.id, call)),
                ),
            };
            if step_rows == 0 {
                step_index += 1;
                continue; // 空正文块不占行，也不引入块间空行。
            }
            if has_previous && needs_spacer() {
                rows += 1;
            }
            has_previous = true;
            let block_y = |offset: usize| -> u16 {
                area.y
                    .saturating_add(u16::try_from(rows + offset).unwrap_or(u16::MAX))
            };
            if let AssistantStepView::Tool { call, .. } = step {
                let folds = folds_of(lx, &message.id, call);
                // 命令卡片登记两条：参数区与输出区各自点击/悬停，点其中一块只切那一块。
                // 文件工具仍是一条整卡点击面；展开前后行数不变的区域只响应 hover。
                for region in tool::card_regions(call, width, folds, step_rows) {
                    let card_rows = region
                        .card
                        .rows
                        .min(step_rows.saturating_sub(region.card.offset));
                    if card_rows == 0 {
                        continue;
                    }
                    let fold_rows = region
                        .fold
                        .rows
                        .min(step_rows.saturating_sub(region.fold.offset));
                    let expanded = match region.section {
                        CardSection::Body => folds.body,
                        CardSection::Output => folds.output,
                    };
                    cards.push(CardHit {
                        kind: CardKind::Tool {
                            message_id: message.id.clone(),
                            call_id: call.call_id.clone(),
                            section: region.section,
                            expanded,
                        },
                        card: Rect::new(
                            area.x,
                            block_y(region.card.offset),
                            width,
                            rows_to_u16(card_rows),
                        ),
                        fold: Rect::new(
                            area.x,
                            block_y(region.fold.offset),
                            width,
                            rows_to_u16(fold_rows),
                        ),
                        clickable: region.clickable,
                    });
                }
            }
            if is_reasoning {
                let expanded = lx.chat.reasoning_expanded(message_index, step_index);
                let header = Rect::new(area.x, block_y(0), width, 1);
                thinking.push((message_index, step_index, header));
                // 思考块整块可点（折叠态只有头部一行，也必须点得开）。
                cards.push(CardHit {
                    kind: CardKind::Thinking {
                        message_index,
                        step_index,
                        expanded,
                    },
                    card: Rect::new(area.x, block_y(0), width, rows_to_u16(step_rows)),
                    // 思考块恢复原来的 hover：悬停整块（含展开后的正文）都提亮。
                    fold: Rect::new(area.x, block_y(0), width, rows_to_u16(step_rows)),
                    clickable: true,
                });
            }
            rows += step_rows;
            step_index += 1;
        }
        // 消息底部通知（失败/中断）。
        if let Some(notice) = notice(message) {
            if has_previous && needs_spacer() {
                rows += 1;
            }
            has_previous = true;
            rows += notice::rows(&notice, width);
        }
        if let Some(line) = stats_line(message) {
            if has_previous && needs_spacer() {
                rows += 1;
            }
            rows += wrap(&line, width).len();
        }
        messages_rows.push(MessageRows {
            start,
            end: rows,
            is_user: message.role == "user" && message.shell.is_none(),
        });
    }
    // 末尾留一行呼吸位（滚动到底部时不贴死）；归最后一条消息（提示结尾时不并入消息区间）。
    if !timeline.is_empty() {
        rows += 1;
        if matches!(timeline.last(), Some(TimelineEntry::Message(_)))
            && let Some(last) = messages_rows.last_mut()
        {
            last.end = rows;
        }
    }
    HistoryLayout {
        area,
        thinking,
        cards,
        messages: messages_rows,
        total_rows: rows,
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
pub(super) fn view(area: Rect, lx: &Lx) -> HistoryView {
    let layout = layout(area, lx);
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

/// 渲染历史内容；`scroll` 为顶部行偏移（0 表示从最早开始）。`rect` 为内容区
/// （body，不含置顶行；含右侧滚动条列）。
///
/// 返回本次推进的内容行数：与 [`layout`] 的 `total_rows` **必须相等**
/// （两者共用同一份步骤行数口径；测试逐一比对，防几何漂移）。
pub(super) fn render(grid: &mut Grid, rect: Rect, lx: &Lx, scroll: usize) -> usize {
    if rect.width == 0 || rect.height == 0 {
        return 0;
    }
    let content = content_rect(rect);
    let mut row_index = 0usize;
    let selected_user = lx
        .rewind
        .selected
        .as_ref()
        .map(|target| target.user_message_id.as_str());
    for entry in &timeline(lx) {
        let message_index = match entry {
            TimelineEntry::Message(index) => *index,
            TimelineEntry::Notices { last, count } => {
                rewind::draw(
                    grid,
                    &lx.chat.notices[*last],
                    *count,
                    content,
                    scroll,
                    &mut row_index,
                );
                row_index += 1;
                continue;
            }
            TimelineEntry::Compact(index) => {
                let notice = &lx.compact.checkpoints[*index];
                let expanded = compact::is_expanded(lx, &notice.id);
                compact::draw(grid, notice, 1, expanded, content, scroll, &mut row_index);
                row_index += 1;
                continue;
            }
        };
        let message = &lx.chat.messages[message_index];
        let is_user = message.role == "user";
        // 本地命令卡片：与 layout 共用 `user_shell::rows` 的行数口径。
        if let Some(shell) = &message.shell {
            user_shell::draw(
                grid,
                content,
                scroll,
                &mut row_index,
                shell,
                lx.chat.loading_frame,
                lx.chat.status.streaming || shell.is_running(),
                shell_folds(lx, &message.id),
                lx.chat.shell_hovered_section(&message.id),
            );
            row_index += 1; // 消息间空行。
            continue;
        }
        // 消息内块之间一律空一行（与 `layout` 同口径）。
        let mut has_previous = false;
        let mut first_user_line = true;
        let mut step_index = 0usize;
        while let Some(step) = message.steps.get(step_index) {
            // 同种文件工具的连续调用共用一行合并标题（与 `layout` 同一份分组口径）。
            if let AssistantStepView::Tool { .. } = step {
                let group = tool::merge_group(&message.steps, step_index);
                if group.len() > 1 {
                    let expanded: Vec<bool> = group
                        .iter()
                        .map(|call| lx.chat.detail_is_expanded(&message.id, &call.call_id))
                        .collect();
                    let group_rows = tool::group_rows(&group, content.width, &expanded);
                    if group_rows > 0 {
                        if has_previous && needs_spacer() {
                            row_index += 1;
                        }
                        has_previous = true;
                        let hovered = group
                            .iter()
                            .position(|call| lx.chat.tool_is_hovered(&message.id, &call.call_id));
                        tool::group_draw_animated(
                            grid,
                            content,
                            scroll,
                            &mut row_index,
                            &group,
                            lx.chat.loading_frame,
                            &expanded,
                            lx.chat.status.streaming,
                            hovered,
                        );
                    }
                    step_index += group.len();
                    continue;
                }
            }
            let (block_rows, part_kind) = match step {
                AssistantStepView::Part { part, .. } => {
                    let expanded = lx.chat.reasoning_expanded(message_index, step_index);
                    let rows = match part.kind {
                        crate::chat::events::PartKind::Text => {
                            text_rows(message, &part.content, content.width)
                        }
                        crate::chat::events::PartKind::Reasoning => {
                            1 + if expanded {
                                body::thinking_rows(&part.content, content.width)
                            } else {
                                0
                            }
                        }
                    };
                    (rows, Some((part, expanded)))
                }
                AssistantStepView::Tool { call, .. } => (
                    tool_rows(call, content.width, folds_of(lx, &message.id, call)),
                    None,
                ),
            };
            if block_rows == 0 {
                step_index += 1;
                continue; // 空正文块不占行，也不引入块间空行（与 `layout` 同口径）。
            }
            if has_previous && needs_spacer() {
                row_index += 1;
            }
            has_previous = true;
            let Some((part, expanded)) = part_kind else {
                if let AssistantStepView::Tool { call, .. } = step {
                    tool::draw_animated(
                        grid,
                        content,
                        scroll,
                        &mut row_index,
                        call,
                        lx.chat.loading_frame,
                        folds_of(lx, &message.id, call),
                        lx.chat.status.streaming,
                        lx.chat.tool_hovered_section(&message.id, &call.call_id),
                    );
                }
                step_index += 1;
                continue;
            };
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    if is_user {
                        let selected = selected_user == Some(message.id.as_str());
                        for line in wrap(&part.content, content.width.saturating_sub(USER_INDENT)) {
                            if let Some(y) = visible_row(content, scroll, row_index) {
                                let bg = if selected {
                                    style::chat_user_target_bg()
                                } else {
                                    style::chat_user_bg()
                                };
                                // 整块底色覆盖整个内容列（含 `> ` 前缀列）。
                                grid.background_row(content.x, y, content.width, bg);
                                if first_user_line {
                                    // 图标用输入框同款强调色，并叠加同底色。
                                    grid.put(content.x, y, USER_PREFIX, style::accent().patch(bg));
                                    first_user_line = false;
                                }
                                grid.put_clipped(
                                    content.x.saturating_add(USER_INDENT),
                                    y,
                                    &line,
                                    bg,
                                    content.width.saturating_sub(USER_INDENT),
                                );
                            }
                            row_index += 1;
                        }
                    } else {
                        // 正文块也是步骤：消息仍在流式且它是最后一步 → 方块呼吸，
                        // 否则静态（已完成）。
                        let mark = if message.status == "streaming"
                            && step_index + 1 == message.steps.len()
                        {
                            StepMark::Running
                        } else {
                            StepMark::Done
                        };
                        body::draw_body(
                            grid,
                            content,
                            scroll,
                            &mut row_index,
                            &part.content,
                            body::BodyStyle {
                                mark,
                                kind: StepKind::Body,
                                loading_frame: lx.chat.loading_frame,
                                animating: lx.chat.status.streaming,
                            },
                        );
                    }
                }
                crate::chat::events::PartKind::Reasoning => {
                    let streaming = lx.chat.reasoning_streaming(message_index, step_index);
                    let hovered = lx.chat.thinking_is_hovered(message_index, step_index);
                    let mark = if streaming {
                        StepMark::Running
                    } else {
                        StepMark::Done
                    };
                    if let Some(y) = visible_row(content, scroll, row_index) {
                        // 生成中：`Thinking`；完成后：`Thought · 6.3s`。
                        // 折叠态浅粉、展开/悬停亮粉（见 `style::chat_step_*_thinking`）。
                        let header_style = StepKind::Thinking.label(streaming, expanded, hovered);
                        grid.put_char(
                            content.x,
                            y,
                            mark.glyph(lx.chat.loading_frame, lx.chat.status.streaming),
                            mark.style(StepKind::Thinking, expanded, hovered),
                        );
                        let x = content.x.saturating_add(STEP_GUTTER);
                        let limit = content.right();
                        if streaming {
                            grid.put_clipped(
                                x,
                                y,
                                text::LX_CHAT_THINKING_LABEL,
                                header_style,
                                limit.saturating_sub(x),
                            );
                        } else {
                            let mut label = String::from(text::LX_CHAT_THOUGHT_LABEL);
                            if let Some(duration) = body::thinking_duration(part) {
                                label.push_str(THINKING_DURATION_SEP);
                                label.push_str(&format_duration(duration));
                            }
                            grid.put_clipped(x, y, &label, header_style, limit.saturating_sub(x));
                        }
                    }
                    row_index += 1;
                    if expanded {
                        body::draw_thinking(grid, content, scroll, &mut row_index, &part.content);
                    }
                }
            }
            step_index += 1;
        }
        // 消息底部通知（失败/中断）。
        if let Some(notice) = notice(message) {
            if has_previous && needs_spacer() {
                row_index += 1;
            }
            has_previous = true;
            notice::draw(grid, &notice, content, scroll, &mut row_index);
        }
        if let Some(line) = stats_line(message) {
            if has_previous && needs_spacer() {
                row_index += 1;
            }
            for row in wrap(&line, content.width) {
                if let Some(y) = visible_row(content, scroll, row_index) {
                    grid.put_clipped(content.x, y, &row, style::muted(), content.width);
                }
                row_index += 1;
            }
        }
        row_index += 1; // 消息间空行。
    }
    row_index
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

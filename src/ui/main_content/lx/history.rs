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

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{AssistantStepView, ChatMessageView, Lx};
use crate::ui::markdown_render;
use crate::ui::widgets::scrollbar;
use crate::ui::{style, text};

use super::Grid;
use super::notice;

mod body;
mod code_copy;
pub(crate) mod tool;

/// 代码块复制按钮占用的列数（`[❐]` 三列）。
const CODE_COPY_WIDTH: u16 = 3;

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
/// 置顶行的最小可用视口（高度 / 宽度，对齐 codex）。
const STICKY_MIN_HEIGHT: u16 = 4;
const STICKY_MIN_WIDTH: u16 = 16;
/// 置顶行摘要的最大取样字符数（限住超长粘贴的每帧成本，对齐 codex 的 512）。
const STICKY_SAMPLE_CHARS: usize = 512;

/// assistant 步骤左侧方块标记的语义状态（思考 / 工具调用 / 正文块共用）。
///
/// 运行中为 `■`/`□` 两帧呼吸（相位由 `LxChat::loading_frame` 决定，约 400ms 一相）；
/// 终态停为静态方块、仅以颜色区分：失败（红）、中断（黄）、取消（次要信息色 + 空心
/// `□`，表示未产出结果）。
///
/// **颜色由「角色」决定，不只看状态**（用户要求区分三类步骤）：
/// - [`StepKind::Body`]（正文块）：**终端默认前景**——方块跟正文同色，不额外抢眼；
/// - [`StepKind::Thinking`]（思考块）：专用品红 `Indexed 218`；
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
    pub(super) fn style(self, kind: StepKind) -> Style {
        match self {
            Self::Failed => style::agent_fail(),
            Self::Interrupted => style::agent_warn(),
            Self::Cancelled => style::agent_step_detail(),
            Self::Running => kind.style_running(),
            Self::Alive => style::chat_step_mark_alive(),
            Self::Done => kind.style_done(),
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
/// - `Thinking` 用品红 → 一眼看出「这段是模型的内心活动」；
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
    /// 运行中（含呼吸）的样式。
    fn style_running(self) -> Style {
        match self {
            Self::Body => style::chat_step_mark_running_body(),
            Self::Thinking => style::chat_step_mark_running_thinking(),
            Self::Tool => style::chat_step_mark_running(),
        }
    }

    /// 已完成（静止）的样式。
    fn style_done(self) -> Style {
        match self {
            Self::Body => style::chat_step_mark_done_body(),
            Self::Thinking => style::chat_step_mark_done_thinking(),
            Self::Tool => style::chat_step_mark_done(),
        }
    }

    /// 该角色的头部文案样式（运行中 / 已完成）。
    pub(super) fn label(self, running: bool) -> Style {
        match (self, running) {
            (Self::Body, true) => style::chat_step_label_running_body(),
            (Self::Body, false) => style::chat_step_label_done_body(),
            (Self::Thinking, true) => style::chat_step_label_running_thinking(),
            (Self::Thinking, false) => style::chat_step_label_done_thinking(),
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
    /// 代码块复制按钮：消息索引、步骤下标、按钮矩形、该代码块正文（复制内容）。
    pub code_copy: Vec<CodeCopyButton>,
    /// 进程卡片 `[open]` 行的命中矩形：消息 id、call id、进程会话 id、可点区域。
    ///
    /// 每张进程卡片**只有一个**矩形（树体末行）；点标题或命令行都不触发任何动作。
    pub tools: Vec<ToolHit>,
    /// 每条消息的行区间（置顶行归属判定用）。
    pub messages: Vec<MessageRows>,
    /// 渲染出的内容总行数（滚动上限用）。
    pub total_rows: usize,
}

pub use code_copy::CodeCopyButton;

/// 进程卡片 `[open]` 行的命中区。
///
/// 只有进程卡片有命中区，且只有**一行**（树体末行的 `[open]`）：点它打开后台任务
/// 面板并把高亮定位到 `session_id` 对应的任务。标题行与命令行是纯展示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolHit {
    pub message_id: String,
    pub call_id: String,
    /// 该卡片关联的进程会话 ID（`[open]` 用它定位面板高亮）；未知时为 None。
    pub session_id: Option<u64>,
    pub rect: Rect,
}

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

/// 置顶行绘制目标：矩形 + 被钉的消息索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StickyRow {
    pub rect: Rect,
    pub message: usize,
}

/// 历史内容列（扣除右侧滚动条预留列）；宽度不足时退回整宽。
fn content_rect(area: Rect) -> Rect {
    if area.width > SCROLLBAR_COLUMNS {
        Rect {
            width: area.width - SCROLLBAR_COLUMNS,
            ..area
        }
    } else {
        area
    }
}

/// 计算历史布局（按当前展开状态）。`area` 为整个历史区（含滚动条列）。
pub(crate) fn layout(area: Rect, lx: &Lx) -> HistoryLayout {
    let content = content_rect(area);
    let width = content.width;
    let mut rows = 0usize;
    let mut thinking = Vec::new();
    let mut code_copy = Vec::new();
    let mut tools: Vec<ToolHit> = Vec::new();
    if width == 0 {
        return HistoryLayout {
            area,
            thinking,
            code_copy,
            tools,
            messages: Vec::new(),
            total_rows: 0,
        };
    }
    let mut messages_rows: Vec<MessageRows> = Vec::new();
    for (message_index, message) in lx.chat.messages.iter().enumerate() {
        // 消息间空行归**后**一条消息（codex 的 leading spacer 口径）：顶行落在轮边界
        // 空行上时算用户消息自己的行 → 用户消息自带上下文，不置顶。
        if message_index > 0 {
            rows += 1;
        }
        let start = rows;
        // 消息内块之间一律空一行（见 [`needs_spacer`]）。
        let mut has_previous = false;
        for (step_index, step) in message.steps.iter().enumerate() {
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
                    tool_rows(
                        call,
                        width,
                        lx.chat.detail_is_expanded(&message.id, &call.call_id),
                    ),
                ),
            };
            if step_rows == 0 {
                continue; // 空正文块不占行，也不引入块间空行。
            }
            if has_previous && needs_spacer() {
                rows += 1;
            }
            has_previous = true;
            if let AssistantStepView::Tool { call, .. } = step
                && let Some(offset) = tool::open_offset(call)
            {
                // 进程卡片只有 `[open]` 一行可点（打开后台任务面板）：标题与命令是
                // 纯展示，点它们什么都不发生——卡片已经没有可展开的内容。
                let open_y = area
                    .y
                    .saturating_add(u16::try_from(rows + offset).unwrap_or(u16::MAX));
                tools.push(ToolHit {
                    message_id: message.id.clone(),
                    call_id: call.call_id.clone(),
                    session_id: call.process_session_id(),
                    rect: Rect::new(area.x, open_y, width, 1),
                });
            }
            if is_reasoning {
                let header_y = area
                    .y
                    .saturating_add(u16::try_from(rows).unwrap_or(u16::MAX));
                thinking.push((
                    message_index,
                    step_index,
                    Rect::new(area.x, header_y, width, 1),
                ));
            } else if let AssistantStepView::Part { part, .. } = step {
                // 代码块头部的复制按钮：按渲染行序算出每块头部的屏幕矩形。
                code_copy::collect(
                    message_index,
                    step_index,
                    &part.content,
                    width.saturating_sub(STEP_GUTTER),
                    area,
                    rows,
                    &mut code_copy,
                );
            }
            rows += step_rows;
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
            is_user: message.role == "user",
        });
    }
    // 末尾留一行呼吸位（滚动到底部时不贴死）；归最后一条消息。
    if !messages_rows.is_empty() {
        rows += 1;
        if let Some(last) = messages_rows.last_mut() {
            last.end = rows;
        }
    }
    HistoryLayout {
        area,
        thinking,
        code_copy,
        tools,
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
    if total == 0 || full_height < STICKY_MIN_HEIGHT || area.width < STICKY_MIN_WIDTH {
        return view;
    }
    let top = message_at(&view.messages, view.scroll);
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

/// 顶行所属消息与消息内行偏移；空历史返回 None。
fn message_at(rows: &[MessageRows], scroll: usize) -> Option<(usize, usize)> {
    rows.iter()
        .enumerate()
        .find(|(_, range)| scroll >= range.start && scroll < range.end)
        .map(|(index, range)| (index, scroll - range.start))
}

/// 置顶候选：顶行落在用户消息自己的行上 → None（自带上下文）；
/// 否则取它前面最近一条**有正文**的用户消息。
fn sticky_candidate(
    messages: &[crate::app::state::ChatMessageView],
    rows: &[MessageRows],
    scroll: usize,
) -> Option<usize> {
    let (index, _) = message_at(rows, scroll)?;
    if rows.get(index)?.is_user {
        return None;
    }
    (0..index)
        .rev()
        .find(|candidate| rows[*candidate].is_user && sticky_text(&messages[*candidate]).is_some())
}

/// 置顶行摘要：该用户消息首个正文块的前 512 字符 → 控制字符转空格 → 空白折叠为单行；
/// 无正文或全空白返回 None（该条不钉）。
fn sticky_text(message: &crate::app::state::ChatMessageView) -> Option<String> {
    let part = message.text_parts().next()?;
    let sample: String = part.content.chars().take(STICKY_SAMPLE_CHARS).collect();
    let single: String = sample
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect();
    let collapsed = single.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then_some(collapsed)
}

/// 置顶行矩形（历史整区第 0 行整宽）；不显示时为 None。
pub(super) fn sticky_rect(view: &HistoryView) -> Option<StickyRow> {
    view.sticky.map(|message| StickyRow {
        rect: Rect::new(view.area.x, view.area.y, view.area.width, 1),
        message,
    })
}

/// 绘制置顶行（**表现层**：直接写 `Buffer`、不进网格 → 不参与复制与选区）；
/// 整宽 `style::chat_user_bg` 底 + `› ` 前缀（accent）+ 摘要（默认色，按列截断加 `…`）。
pub(super) fn render_sticky_prompt(
    buf: &mut Buffer,
    row: StickyRow,
    message: &crate::app::state::ChatMessageView,
) {
    let Some(text) = sticky_text(message) else {
        return;
    };
    let bg = style::chat_user_bg();
    let content = content_rect(row.rect);
    for x in content.x..content.right() {
        if let Some(cell) = buf.cell_mut((x, row.rect.y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(bg);
        }
    }
    let mut x = put_buffer_line(
        buf,
        content.x,
        row.rect.y,
        USER_PREFIX,
        style::accent().patch(bg),
        content.right(),
    );
    let remaining = usize::from(content.right().saturating_sub(x));
    if remaining > 0 {
        let text = truncate_with_ellipsis(&text, remaining);
        x = put_buffer_line(buf, x, row.rect.y, &text, bg, content.right());
    }
    // 尾部留白仍铺底（整宽一致）。
    for tail in x..content.right() {
        if let Some(cell) = buf.cell_mut((tail, row.rect.y)) {
            cell.reset();
            cell.set_char(' ');
            cell.set_style(bg);
        }
    }
}

/// 逐字符写 `Buffer`（宽字符占两列、不跨 `limit` 半写）；返回写入后的列。
fn put_buffer_line(
    buf: &mut Buffer,
    mut x: u16,
    y: u16,
    content: &str,
    style: Style,
    limit: u16,
) -> u16 {
    for ch in content.chars() {
        let width = ch.width().unwrap_or(0) as u16;
        if width == 0 {
            continue;
        }
        if x.saturating_add(width) > limit {
            break;
        }
        for offset in 0..width.max(1) {
            if let Some(cell) = buf.cell_mut((x.saturating_add(offset), y)) {
                cell.reset();
                cell.set_char(if offset == 0 { ch } else { ' ' });
                cell.set_style(style);
            }
        }
        x = x.saturating_add(width);
    }
    x
}

/// 按显示列截断，溢出以 `…` 收尾（宽字符按两列计）。
pub(super) fn truncate_with_ellipsis(value: &str, max: usize) -> String {
    if value.width() <= max {
        return value.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0;
    for ch in value.chars() {
        let cell = ch.width().unwrap_or(0);
        if width + cell > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        width += cell;
    }
    out.push('…');
    out
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
    for (message_index, message) in lx.chat.messages.iter().enumerate() {
        let is_user = message.role == "user";
        // 消息内块之间一律空一行（与 `layout` 同口径）。
        let mut has_previous = false;
        let mut first_user_line = true;
        for (step_index, step) in message.steps.iter().enumerate() {
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
                    tool_rows(
                        call,
                        content.width,
                        lx.chat.detail_is_expanded(&message.id, &call.call_id),
                    ),
                    None,
                ),
            };
            if block_rows == 0 {
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
                        lx.chat.detail_is_expanded(&message.id, &call.call_id),
                        lx.chat.status.streaming,
                    );
                }
                continue;
            };
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    if is_user {
                        for line in wrap(&part.content, content.width.saturating_sub(USER_INDENT)) {
                            if let Some(y) = visible_row(content, scroll, row_index) {
                                let bg = style::chat_user_bg();
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
                    let mark = if streaming {
                        StepMark::Running
                    } else {
                        StepMark::Done
                    };
                    if let Some(y) = visible_row(content, scroll, row_index) {
                        // 生成中：品红方块（加粗 + 呼吸）+ `Thinking`（同色不加粗）；
                        // 完成后：同款品红方块（去加粗、静止）+ `Thought · 6.3s`
                        // （无折叠箭头，折叠/展开由点击头部或 Ctrl+T 控制，方块即状态标记）。
                        // 思考用品红，与正文（默认前景）和工具（翡翠绿）都分得开。
                        let header_style = StepKind::Thinking.label(streaming);
                        grid.put_char(
                            content.x,
                            y,
                            mark.glyph(lx.chat.loading_frame, lx.chat.status.streaming),
                            mark.style(StepKind::Thinking),
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

/// 消息正文渲染行数：assistant 用 markdown 渲染（宽度自适应，扣步骤标记缩进），
/// 用户按纯文本换行（扣 `> ` 前缀）。
fn text_rows(message: &crate::app::state::ChatMessageView, content: &str, width: u16) -> usize {
    if message.role == "user" {
        return wrap(content, width.saturating_sub(USER_INDENT)).len();
    }
    markdown_rows(content, width.saturating_sub(STEP_GUTTER))
}

/// assistant markdown 正文渲染成若干行（已按 `width` 自适应：表格收缩、其余软换行）。
fn markdown_rows(content: &str, width: u16) -> usize {
    markdown_render::render_wrapped(content, width).len()
}

/// 工具步骤占用的渲染行数。
///
/// `layout` 与 `render` 共用本入口（内部即 `tool::rows`）：工具行一旦换行，滚动条、
/// 置顶用户行与 thinking 命中矩形都会漂移。`expanded` 只影响进程/计划卡片
/// （文件工具恒为 2 行，展开状态不改变它的几何）。
fn tool_rows(call: &crate::app::state::ToolCallView, width: u16, expanded: bool) -> usize {
    tool::rows(call, width, expanded)
}

/// 本轮统计行（assistant 消息末尾）：`模型 · 2.9k in · 307 out · 70% cached ·
/// 33.4 tok/s · 2.7s`；仍在流中或无任何可用数据时返回 None。
///
/// tok/s 对齐 opencode：输出 token ÷ 本轮耗时（秒）。
fn stats_line(message: &ChatMessageView) -> Option<String> {
    if message.role != "assistant" || message.status == "streaming" {
        return None;
    }
    let mut fields: Vec<String> = Vec::new();
    if let Some(input) = message.input_tokens.filter(|tokens| *tokens > 0) {
        fields.push(format!(
            "{}{}",
            text::token_count(u64::try_from(input).unwrap_or(0)),
            text::LX_CHAT_STATS_INPUT_SUFFIX
        ));
    }
    if let Some(output) = message.output_tokens.filter(|tokens| *tokens > 0) {
        fields.push(format!(
            "{}{}",
            text::token_count(u64::try_from(output).unwrap_or(0)),
            text::LX_CHAT_STATS_OUTPUT_SUFFIX
        ));
    }
    // 缓存命中率：cached / prompt（OpenAI 口径 prompt_tokens 已含 cached）。
    if let (Some(cache), Some(input)) = (
        message.cache_read_tokens.filter(|tokens| *tokens > 0),
        message.input_tokens.filter(|tokens| *tokens > 0),
    ) && let Some(percent) = cache.saturating_mul(100).checked_div(input)
    {
        fields.push(format!(
            "{}%{}",
            percent.clamp(0, 100),
            text::LX_CHAT_STATS_CACHE_SUFFIX
        ));
    }
    if let (Some(output), Some(duration)) = (
        message.output_tokens.filter(|tokens| *tokens > 0),
        message.duration_ms.filter(|duration| *duration > 0),
    ) {
        let speed = output as f64 / (duration as f64 / 1000.0);
        if speed.is_finite() && speed > 0.0 {
            fields.push(format!("{speed:.1}{}", text::LX_CHAT_STATS_SPEED_SUFFIX));
        }
    }
    if let Some(duration) = message.duration_ms.filter(|duration| *duration > 0) {
        fields.push(format_duration(duration));
    }
    // 仅「模型名」不算统计：没有任何用量/耗时数据时不出行（失败且无 usage 的轮次）。
    if fields.is_empty() {
        return None;
    }
    if let Some(model) = message
        .model_label
        .as_deref()
        .filter(|model| !model.is_empty())
    {
        fields.insert(0, model.to_string());
    }
    Some(fields.join(text::LX_CHAT_STATS_SEP))
}

/// token 计数短格式（对齐 lx-agent）：`999`、`1.2k`、`123k`、`1.2M`。
/// 耗时短格式（对齐 lx-agent）：`820ms`、`4.2s`、`38s`、`1m 12s`。
fn format_duration(ms: i64) -> String {
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1_000.0;
    if seconds < 60.0 {
        return if seconds < 10.0 {
            format!("{seconds:.1}s")
        } else {
            format!("{:.0}s", seconds)
        };
    }
    let minutes = (seconds / 60.0).floor() as i64;
    let rest = (seconds % 60.0).round() as i64;
    format!("{minutes}m {rest}s")
}

/// 行可见时返回其屏幕 Y。
pub(super) fn visible_row(rect: Rect, scroll: usize, row: usize) -> Option<u16> {
    if row < scroll {
        return None;
    }
    let offset = u16::try_from(row - scroll).unwrap_or(u16::MAX);
    let screen_y = rect.y.saturating_add(offset);
    (screen_y < rect.bottom()).then_some(screen_y)
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

/// 按内容宽度换行（保留原换行；宽字符按两列计）。
pub(super) fn wrap(content: &str, width: u16) -> Vec<String> {
    let width = usize::from(width.max(1));
    let mut out = Vec::new();
    for line in content.split('\n') {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut current = String::new();
        let mut col = 0usize;
        for ch in line.chars() {
            let cell = ch.width().unwrap_or(1).max(1);
            if col + cell > width && col > 0 {
                out.push(std::mem::take(&mut current));
                col = 0;
            }
            current.push(ch);
            col += cell;
        }
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests;

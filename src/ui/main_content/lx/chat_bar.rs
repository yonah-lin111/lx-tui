//! 输入框上方那一行：动态任务/阶段文案（含 token 计数）/ 「回到底部」控件。
//!
//! 纯渲染 + 纯几何：布局、绘制与命中共享同一矩形口径；只读 `Lx`、只写 lx 页网格。
//! 该行贴在输入框上沿、与输入框同宽；没有内容时整行不占位。
//!
//! **优先级：loading > 回到底部**（`↓ jump to bottom` 在被挤出时让位——用户此刻
//! 视线在输入框）。提示类反馈已全部改走 main content 的 toast（见
//! `app::update::chat::show_lx_hint`），这一行不再承载临时提示。
//!
//! **loading 文案**：由 [`activity_label`] 只读派生——会话计划有 `in_progress` 步骤时
//! 显示该步骤原文，否则按当前 assistant 的真实阶段显示 `Thinking...` /
//! `Running tools...` / `Working...`。文案带**从左到右的扫描高亮**：窗口外的字符用
//! 次要色（dim），3 字符宽的窗口内用当前帧色（按 Native 标签的 4 档渐变
//! `218 → 182 → 153 → 117` 循环，每 `LOADING_FRAME_INTERVAL` 换一档），走出末尾后
//! 从左侧重新进入。
//!
//! **统计优先**：括号里的统计先占位（`(3s · ↓320 tok)`，静态 dim、不参与扫描），
//! 文字只用剩余宽度并在尾部加 `…`；因此长 todo 永远不会挤掉耗时/token。统计自身
//! 都放不下时（W<S）文字整体隐藏、统计按列裁切可见前缀，绝不越界或换行。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{
    AssistantStepView, ChatMessageView, Lx, PlanStepView, RequestState, ToolStatus,
};
use crate::ui::{style, text};

use super::Grid;

/// 聊天栏高度（一行）。
pub(crate) const HEIGHT: u16 = 1;
/// 右侧控件与栏右缘之间的空档（列）。
const RIGHT_PAD: u16 = 1;
/// 箭头与说明文字之间的空档（列）。
const HINT_GAP: u16 = 1;
/// 扫描高亮窗口宽度（字符）。
const SCAN_WINDOW: usize = 3;
/// loading 文案与括号统计之间的空档（列）。
const STATS_GAP: u16 = 1;

/// 是否显示聊天栏：有临时提示、流中（loading）或上滚未跟随底部（回到底部控件）。
///
/// **后台任务面板打开时不显示**：这一行属于聊天历史区（loading / 回到底部都是
/// 针对消息列表的），而面板已经把历史区换成输出容器——此时漂一个
/// `↓ jump to bottom` 出来既是错位信息，点了也只会滚动看不见的历史。
pub(super) fn visible(lx: &Lx) -> bool {
    if lx.task.is_some() {
        return false;
    }
    if !super::history::has_history(lx) {
        return false;
    }
    lx.chat.status.streaming || !lx.chat.follow_bottom
}

/// 由输入框矩形推导聊天栏矩形（贴其上沿）；上方空间不足时 None。
pub(super) fn rect(area: Rect, input: Rect) -> Option<Rect> {
    (input.y >= area.y.saturating_add(HEIGHT))
        .then(|| Rect::new(input.x, input.y - HEIGHT, input.width, HEIGHT))
}

/// 回到底部控件矩形（未跟随底部、**无临时提示**且空间足够时）；供命中与测试。
pub(crate) fn jump_bottom_rect(area: Rect, lx: &Lx) -> Option<Rect> {
    // 面板打开时聊天栏整体不画（见 [`visible`]），命中区也必须一起消失，
    // 否则会出现"看不见却能点"的穿透。
    if lx.task.is_some() || lx.chat.follow_bottom || !super::history::has_history(lx) {
        return None;
    }
    let input = super::input_rect(area, lx)?;
    jump_rect(rect(area, input)?)
}

/// 栏内「回到底部」控件矩形（整体右对齐）。
///
/// 优先「箭头 + 空档 + 说明」；宽度只够箭头时退回纯箭头；再窄返回 None。
fn jump_rect(bar: Rect) -> Option<Rect> {
    let arrow = u16::try_from(text::LX_JUMP_BOTTOM.width()).unwrap_or(u16::MAX);
    let hint = u16::try_from(text::LX_JUMP_BOTTOM_HINT.width()).unwrap_or(u16::MAX);
    let width = arrow
        .saturating_add(HINT_GAP)
        .saturating_add(hint)
        .saturating_add(RIGHT_PAD);
    if bar.width >= width {
        let inner = arrow.saturating_add(HINT_GAP).saturating_add(hint);
        return Some(Rect::new(bar.right() - RIGHT_PAD - inner, bar.y, inner, 1));
    }
    (bar.width > arrow + RIGHT_PAD)
        .then(|| Rect::new(bar.right() - RIGHT_PAD - arrow, bar.y, arrow, 1))
}

/// 输入框上方那一行的展示文案（只读派生，不缓存）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivityLabel<'a> {
    /// 当前计划里第一个 `in_progress` 步骤的（已净化的）原文。
    Todo(&'a str),
    /// 当前 assistant 最后一步是开放思考块。
    Thinking,
    /// 当前 assistant 有 Pending/Running 工具。
    RunningTools,
    /// 阶段不明确或尚未开始。
    Working,
}

impl ActivityLabel<'_> {
    /// 实际显示的文字；英文短语来自 `ui::text::chat`，不在此硬编码。
    fn text(&self) -> &str {
        match self {
            Self::Todo(todo) => todo,
            Self::Thinking => text::chat::THINKING_LABEL,
            Self::RunningTools => text::chat::RUNNING_TOOLS_LABEL,
            Self::Working => text::chat::LOADING_LABEL,
        }
    }
}

/// 只读派生当前要显示的任务/阶段文案。
///
/// 优先级：会话计划里的 `in_progress` 步骤 > 当前 assistant 的 Pending/Running 工具 >
/// 当前 assistant 最后一步的开放思考块 > `Working...`。当前 assistant 严格按
/// [`RequestState::Streaming`] 的 `assistant_id` 定位——本地命令记录会追加在流式
/// assistant 之后，不能用 `messages.last()`。`Committing`/`Finishing` 没有当前消息，
/// 因此不会读出上一轮的思考或工具阶段。
fn activity_label<'a>(lx: &'a Lx, plan_text: Option<&'a str>) -> ActivityLabel<'a> {
    if let Some(todo) = plan_text {
        return ActivityLabel::Todo(todo);
    }
    let Some(message) = current_streaming_message(&lx.chat) else {
        return ActivityLabel::Working;
    };
    if message
        .tools()
        .any(|call| matches!(call.status, ToolStatus::Pending | ToolStatus::Running))
    {
        return ActivityLabel::RunningTools;
    }
    match message.steps.last() {
        Some(AssistantStepView::Part { part, .. }) if part.reasoning_open() => {
            ActivityLabel::Thinking
        }
        _ => ActivityLabel::Working,
    }
}

/// 当前仍在流转的 assistant 消息；仅在 `Streaming` 时按 `assistant_id` 定位。
fn current_streaming_message(chat: &crate::app::state::LxChat) -> Option<&ChatMessageView> {
    let RequestState::Streaming { assistant_id, .. } = &chat.request else {
        return None;
    };
    chat.messages
        .iter()
        .find(|message| message.id == *assistant_id && message.role == "assistant")
}

/// 计划里第一个 `in_progress` 步骤的原文（会话级快照，跨轮保留）。
fn plan_todo(plan: Option<&crate::app::state::PlanSnapshotView>) -> Option<&str> {
    plan?
        .steps
        .iter()
        .find(|(_, status)| *status == PlanStepView::InProgress)
        .map(|(step, _)| step.as_str())
}

/// 统计优先的宽度预算（设计 §6.2/§6.3/§6.4）：返回**文字可用列数**。
///
/// - 有统计时先预留完整统计宽度 S，文字只用 `W - S - G`；
/// - 文字预算为 0 时整段不画，也不留多余间隔（统计从起点开始）；
/// - 统计自身都放不下（W<S）时文字让位，统计按剩余列裁切可见前缀；
/// - 没有统计时全部可用宽度归文字。
/// - 唯一例外是 Esc 待命：最短停止提示与完整统计不能共存时停止反馈优先，
///   文字拿到全部可用宽度，统计在剩余列安全裁切（普通 todo 不走这条路径）。
fn title_budget(available: u16, stats_width: Option<u16>, stop_armed: bool) -> u16 {
    let Some(stats_w) = stats_width else {
        return available;
    };
    let budget = available.saturating_sub(stats_w).saturating_sub(STATS_GAP);
    // 唯一例外（设计 §6.4）：Esc 待命时最短停止提示与完整统计不能共存，停止反馈优先；
    // 统计由调用方按文字尾部的剩余列安全裁切。普通 todo 不走这条路径。
    if stop_armed && budget < stop_shortest_width() {
        return available;
    }
    if available < stats_w {
        // 极窄：连统计自身都放不下，文字先完全隐藏。
        return 0;
    }
    budget
}

/// 绘制聊天栏：loading（含 token 计数）> 回到底部。
pub(super) fn draw(grid: &mut Grid, bar: Option<Rect>, lx: &Lx) {
    let Some(bar) = bar else {
        return;
    };
    if bar.width == 0 {
        return;
    }
    // 回到底部控件（仅未跟随底部时）。
    if !lx.chat.follow_bottom
        && let Some(button) = jump_rect(bar)
    {
        let jump_style = style::chat_jump_bottom(lx.chat.jump_bottom_hover);
        grid.put_clipped(
            button.x,
            button.y,
            text::LX_JUMP_BOTTOM,
            jump_style,
            button.width,
        );
        // 说明文字在箭头右侧（宽度只够箭头时跳过）。
        let arrow = u16::try_from(text::LX_JUMP_BOTTOM.width()).unwrap_or(u16::MAX);
        let hint_x = button.x.saturating_add(arrow).saturating_add(HINT_GAP);
        if hint_x < button.right() {
            grid.put_clipped(
                hint_x,
                bar.y,
                text::LX_JUMP_BOTTOM_HINT,
                jump_style,
                button.right() - hint_x,
            );
        }
    }
    if !lx.chat.status.streaming {
        return;
    }
    // 计划原文先净化成单行副本（模型给的换行/Tab/控制字符不能进终端）。
    let todo = plan_todo(lx.chat.plan.as_ref()).map(text::chat::single_line_label);
    let label = activity_label(lx, todo.as_deref());
    // 统计静态 dim、不参与扫描：它回答「这轮跑了多久、大概用了多少 token」，
    // 与左侧的进行中动画是两种信息，不能一起闪。Esc 待命时同样保留（空间够时），
    // 极窄才让停止反馈优先。
    let stats =
        text::chat::stream_stats_label(lx.chat.turn_elapsed_ms, lx.chat.stream_output_tokens());
    draw_loading(
        grid,
        bar,
        lx.chat.loading_frame,
        lx.chat.stop_armed_at.is_some(),
        label.text(),
        stats.as_deref(),
    );
}

/// 画一行 loading：扫描高亮文字 + 静态统计，各自在自己的预算内绘制。
///
/// 顺序不可交换：先按统计优先算出文字预算并绘制文字，统计再紧贴**实际文字尾部**
/// 落位。这样长 todo 只会自己截断，不可能挤掉统计；也不会把短文案右对齐。
fn draw_loading(
    grid: &mut Grid,
    bar: Rect,
    loading_frame: usize,
    stop_armed: bool,
    activity: &str,
    stats: Option<&str>,
) {
    // 左右各留 1 列空档（既有口径：左侧起点、右侧不贴边）。
    let x = bar.x.saturating_add(1);
    let limit = bar.right().saturating_sub(1);
    if x >= limit {
        return;
    }
    let available = limit.saturating_sub(x);
    let stats_width = stats.map(|stats| u16::try_from(stats.width()).unwrap_or(u16::MAX));
    let budget = title_budget(available, stats_width, stop_armed);
    // Esc 待命：提示「再按一次停止」（按文字预算取第一条放得下的短语，对齐 opencode）；
    // 待命提示同样走扫描高亮（它也是「进行中」的提示，观感保持一致）。
    let text = if stop_armed {
        stop_phrase(budget)
    } else {
        activity
    };
    // 文字天然短于预算时不补空格：统计紧贴实际尾部（设计 §6.2 第 4 条）。
    let text_end = if budget > 0 {
        draw_scanned_label(
            grid,
            x,
            bar.y,
            x.saturating_add(budget),
            text,
            loading_frame,
        )
    } else {
        x
    };
    let Some(stats) = stats else {
        return;
    };
    // 文字被整体隐藏时统计从起点开始，不留无意义的前导间隔。
    let stats_x = if text_end > x {
        text_end.saturating_add(STATS_GAP)
    } else {
        x
    };
    if stats_x >= limit {
        return;
    }
    grid.put_clipped(
        stats_x,
        bar.y,
        stats,
        style::chat_loading_label(),
        limit.saturating_sub(stats_x),
    );
}

/// 按显示列宽把文案裁到 `budget` 列（尾部 `…`，`budget == 0` 返回空串）。
///
/// 复用共享的 [`crate::ui::text::truncate_to_width`]，不另造第二套截断器。
fn clip_label(label: &str, budget: u16) -> String {
    crate::ui::text::truncate_to_width(label, usize::from(budget))
}

/// 画扫描高亮文案，返回**文案结束后的列**（供紧随其后的统计段定位）。
///
/// 先按显示列宽裁切（中文不会被拆半），再用裁切后的可见文字计算扫描周期：
/// 长 todo 被截断后高亮不会因为按原文长度推进而长时间消失。
fn draw_scanned_label(
    grid: &mut Grid,
    x: u16,
    y: u16,
    limit: u16,
    label: &str,
    loading_frame: usize,
) -> u16 {
    let budget = limit.saturating_sub(x);
    if budget == 0 {
        return x;
    }
    let visible = clip_label(label, budget);
    // 扫描位置：每帧前进 1 个**字符**，走出末尾后从左侧重新进入。
    let chars: Vec<char> = visible.chars().collect();
    if chars.is_empty() {
        return x;
    }
    let span = chars.len() + SCAN_WINDOW;
    let head = loading_frame % span;
    let mut cell_x = x;
    for (index, ch) in chars.iter().enumerate() {
        // 零宽字符不占列：跳过（不写入、也不推进光标）。
        let Some(cell_width) = ch.width().filter(|width| *width > 0) else {
            continue;
        };
        let cell_width = u16::try_from(cell_width).unwrap_or(u16::MAX);
        // 落笔前测宽：宽字符不能半写、也不能侵入统计区。
        if cell_x.saturating_add(cell_width) > limit {
            break;
        }
        // 窗口内：`head - SCAN_WINDOW < index <= head`（head 为窗口右端）。
        let in_window = index + SCAN_WINDOW >= head && index < head;
        let cell_style = if in_window {
            style::chat_loading_scan(loading_frame)
        } else {
            style::chat_loading_label()
        };
        grid.put_char(cell_x, y, *ch, cell_style);
        cell_x = cell_x.saturating_add(cell_width);
    }
    cell_x
}

/// Esc 待命提示短语：从长到短取第一条放得下的；都放不下时用最短的（由调用方裁剪）。
fn stop_phrase(width: u16) -> &'static str {
    let width = usize::from(width);
    text::LX_CHAT_STOP_PHRASES
        .iter()
        .find(|phrase| phrase.width() <= width)
        .copied()
        .unwrap_or_else(|| text::LX_CHAT_STOP_PHRASES[text::LX_CHAT_STOP_PHRASES.len() - 1])
}

/// 最短停止提示的显示列宽（极窄时停止反馈优先的判定阈值）。
fn stop_shortest_width() -> u16 {
    text::LX_CHAT_STOP_PHRASES
        .iter()
        .map(|phrase| u16::try_from(phrase.width()).unwrap_or(u16::MAX))
        .min()
        .unwrap_or(0)
}

/// 命中「回到底部」控件的窗格；无控件或未命中返回 None。
pub fn jump_bottom_hit(
    rects: &[(crate::layout::PaneId, Rect)],
    state: &crate::app::state::AppState,
    column: u16,
    row: u16,
) -> Option<crate::layout::PaneId> {
    rects.iter().find_map(|(id, rect)| {
        let target = state
            .pane_anywhere(*id)
            .filter(|pane| pane.view == crate::app::state::PaneView::Lx)?;
        let inner = crate::layout::pane_inner_rect(*rect);
        jump_bottom_rect(inner, &target.lx)
            .filter(|area| area.contains((column, row).into()))
            .map(|_| *id)
    })
}

#[cfg(test)]
mod tests;

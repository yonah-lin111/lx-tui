//! 聊天历史渲染：消息、正文流增量、thinking 折叠块、本轮统计行、滚动条与滚动。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；命中几何（thinking 头部矩形）由
//! 同一布局函数提供，渲染与命中不漂移。
//!
//! - 用户消息：`> ` 前缀（占用整块底色，图标用输入框同款强调色）+ 整块浅底色
//!   （`style::chat_user_bg`），正文按显示列软换行。
//! - assistant 消息：正文用共享阅读渲染器（`ui::markdown_render`，与 UI 页 agent 回答
//!   同款；表格按宽度收缩自适应），不显示标题行；失败/中断在**消息底部**（统计行上方）
//!   插入一条通知（见 `notice.rs`），流中不插入（改用输入框上方 loading 行）。
//! - thinking：生成中的块自动展开（头部转轮 + 正文），完成后默认折叠；点击头部或
//!   Ctrl+T 切换；正文同样走 markdown 渲染并整体压暗（对齐 opencode 的思考显示）。
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

mod tool;

/// 用户消息前缀（`> `）。
const USER_PREFIX: &str = "> ";
/// 用户消息正文缩进列（与前缀等宽）。
const USER_INDENT: u16 = 2;
/// thinking 块左侧缩进列：与正文、统计行**左对齐**（0，不额外缩进）。
const THINKING_MARGIN: u16 = 0;
/// thinking 头部折叠/展开标记。
const THINKING_COLLAPSED_MARK: &str = "▸ ";
const THINKING_EXPANDED_MARK: &str = "▾ ";
/// 头部文案与思考耗时之间的分隔（`Thought · 6.3s`）。
const THINKING_DURATION_SEP: &str = " · ";
/// 历史区右侧滚动条预留列数。
pub(super) const SCROLLBAR_COLUMNS: u16 = crate::layout::LX_HISTORY_SCROLLBAR;
/// 置顶行的最小可用视口（高度 / 宽度，对齐 codex）。
const STICKY_MIN_HEIGHT: u16 = 4;
const STICKY_MIN_WIDTH: u16 = 16;
/// 置顶行摘要的最大取样字符数（限住超长粘贴的每帧成本，对齐 codex 的 512）。
const STICKY_SAMPLE_CHARS: usize = 512;

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
    /// 每条消息的行区间（置顶行归属判定用）。
    pub messages: Vec<MessageRows>,
    /// 渲染出的内容总行数（滚动上限用）。
    pub total_rows: usize,
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
pub(super) fn layout(area: Rect, lx: &Lx) -> HistoryLayout {
    let content = content_rect(area);
    let width = content.width;
    let mut rows = 0usize;
    let mut thinking = Vec::new();
    if width == 0 {
        return HistoryLayout {
            area,
            thinking,
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
        // 消息内块（各内容块 / 工具行 / 通知 / 统计行）之间空一行。
        let mut first_block = true;
        for (step_index, step) in message.steps.iter().enumerate() {
            let (is_reasoning, step_rows) = match step {
                AssistantStepView::Part { part, .. } => match part.kind {
                    crate::chat::events::PartKind::Text => {
                        (false, text_rows(message, &part.content, width))
                    }
                    crate::chat::events::PartKind::Reasoning => (
                        true,
                        1 + if lx.chat.reasoning_expanded(message_index, step_index) {
                            thinking_rows(&part.content, width)
                        } else {
                            0
                        },
                    ),
                },
                AssistantStepView::Tool { call, .. } => (false, tool_rows(call, width)),
            };
            if step_rows == 0 {
                continue; // 空正文块不占行，也不引入块间空行。
            }
            if !first_block {
                rows += 1;
            }
            first_block = false;
            if is_reasoning {
                let header_y = area
                    .y
                    .saturating_add(u16::try_from(rows).unwrap_or(u16::MAX));
                thinking.push((
                    message_index,
                    step_index,
                    Rect::new(area.x, header_y, width, 1),
                ));
            }
            rows += step_rows;
        }
        // 消息底部通知（失败/中断）。
        if let Some(notice) = notice(message) {
            if !first_block {
                rows += 1;
            }
            first_block = false;
            rows += notice::rows(&notice, width);
        }
        if let Some(line) = stats_line(message) {
            if !first_block {
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
fn truncate_with_ellipsis(value: &str, max: usize) -> String {
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
        // 消息内块（各内容块 / 通知 / 统计行）之间空一行（与 `layout` 同口径）。
        let mut first_block = true;
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
                                thinking_rows(&part.content, content.width)
                            } else {
                                0
                            }
                        }
                    };
                    (rows, Some((part, expanded)))
                }
                AssistantStepView::Tool { call, .. } => (tool_rows(call, content.width), None),
            };
            if block_rows == 0 {
                continue; // 空正文块不占行，也不引入块间空行（与 `layout` 同口径）。
            }
            if !first_block {
                row_index += 1;
            }
            first_block = false;
            let Some((part, expanded)) = part_kind else {
                if let AssistantStepView::Tool { call, .. } = step {
                    tool::draw(
                        grid,
                        content,
                        scroll,
                        &mut row_index,
                        call,
                        lx.chat.loading_frame,
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
                        draw_markdown(grid, content, scroll, &mut row_index, &part.content);
                    }
                }
                crate::chat::events::PartKind::Reasoning => {
                    let streaming = lx.chat.reasoning_streaming(message_index, step_index);
                    if let Some(y) = visible_row(content, scroll, row_index) {
                        // 生成中：转轮 + `Thinking`（蓝色，与 UI 页思考步骤同色；不带折叠标记）；
                        // 完成后：`▸ Thought · 6.3s` / `▾ Thought · 6.3s`（压暗一档的品牌蓝）。
                        let header_style = if streaming {
                            style::agent_verb()
                        } else {
                            style::agent_connector()
                        };
                        let x = content.x.saturating_add(THINKING_MARGIN);
                        let limit = content.right();
                        if streaming {
                            let frame = text::LX_LOADING_FRAMES
                                [lx.chat.loading_frame % text::LX_LOADING_FRAMES.len()];
                            grid.put_char(x, y, frame, header_style);
                            grid.put_clipped(
                                x.saturating_add(2),
                                y,
                                text::LX_CHAT_THINKING_LABEL,
                                header_style,
                                limit.saturating_sub(x.saturating_add(2)),
                            );
                        } else {
                            let mut label = String::from(if expanded {
                                THINKING_EXPANDED_MARK
                            } else {
                                THINKING_COLLAPSED_MARK
                            });
                            label.push_str(text::LX_CHAT_THOUGHT_LABEL);
                            if let Some(duration) = thinking_duration(part) {
                                label.push_str(THINKING_DURATION_SEP);
                                label.push_str(&format_duration(duration));
                            }
                            grid.put_clipped(x, y, &label, header_style, limit.saturating_sub(x));
                        }
                    }
                    row_index += 1;
                    if expanded {
                        draw_thinking(grid, content, scroll, &mut row_index, &part.content);
                    }
                }
            }
        }
        // 消息底部通知（失败/中断）。
        if let Some(notice) = notice(message) {
            if !first_block {
                row_index += 1;
            }
            first_block = false;
            notice::draw(grid, &notice, content, scroll, &mut row_index);
        }
        if let Some(line) = stats_line(message) {
            if !first_block {
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

/// 消息正文渲染行数：assistant 用 markdown 渲染（宽度自适应），用户按纯文本换行。
fn text_rows(message: &crate::app::state::ChatMessageView, content: &str, width: u16) -> usize {
    if message.role == "user" {
        return wrap(content, width.saturating_sub(USER_INDENT)).len();
    }
    markdown_rows(content, width)
}

/// assistant markdown 正文渲染成若干行（已按 `width` 自适应：表格收缩、其余软换行）。
fn markdown_rows(content: &str, width: u16) -> usize {
    markdown_render::render_wrapped(content, width).len()
}

/// 工具步骤占用的渲染行数（**恒为 1 行单行工具行**）。
///
/// `layout` 与 `render` 共用本入口（内部即 `tool::rows`）：工具行一旦换行，滚动条、
/// 置顶用户行与 thinking 命中矩形都会漂移。
fn tool_rows(call: &crate::app::state::ToolCallView, width: u16) -> usize {
    tool::rows(call, width)
}

/// 绘制一段 assistant markdown 正文（已按宽度自适应，无需再换行）。
fn draw_markdown(grid: &mut Grid, rect: Rect, scroll: usize, row_index: &mut usize, content: &str) {
    for line in markdown_render::render_wrapped(content, rect.width) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            let mut cx = rect.x.saturating_add(line.indent);
            for (value, span_style) in &line.spans {
                for ch in value.chars() {
                    let cell = ch.width().unwrap_or(1).max(1) as u16;
                    if cx.saturating_add(cell) > rect.right() {
                        break;
                    }
                    grid.put_char(cx, y, ch, *span_style);
                    cx = cx.saturating_add(cell);
                }
            }
        }
        *row_index += 1;
    }
}

/// thinking 块耗时（毫秒）：首个增量到最后一个增量的时间差；时间戳缺失返回 None。
fn thinking_duration(part: &crate::app::state::ChatPartView) -> Option<i64> {
    let started = part.started_at_ms?;
    let ended = part.ended_at_ms?;
    Some(ended.saturating_sub(started).max(0))
}

/// thinking 正文渲染行数（markdown；左缩进 `THINKING_MARGIN`）。
fn thinking_rows(content: &str, width: u16) -> usize {
    markdown_render::render_wrapped(content, width.saturating_sub(THINKING_MARGIN)).len()
}

/// 绘制展开的 thinking 正文：与正文同款 markdown 渲染，但整体压暗（`style::muted`）。
fn draw_thinking(grid: &mut Grid, rect: Rect, scroll: usize, row_index: &mut usize, content: &str) {
    let inner = rect.width.saturating_sub(THINKING_MARGIN);
    for line in markdown_render::render_wrapped(content, inner) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            let mut cx = rect
                .x
                .saturating_add(THINKING_MARGIN)
                .saturating_add(line.indent);
            for (value, span_style) in &line.spans {
                let cell_style = span_style.patch(style::muted());
                for ch in value.chars() {
                    let cell = ch.width().unwrap_or(1).max(1) as u16;
                    if cx.saturating_add(cell) > rect.right() {
                        break;
                    }
                    grid.put_char(cx, y, ch, cell_style);
                    cx = cx.saturating_add(cell);
                }
            }
        }
        *row_index += 1;
    }
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

//! 聊天历史渲染：消息、正文流增量、thinking 折叠块、本轮统计行、滚动条与滚动。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；命中几何（thinking 头部矩形）由
//! 同一布局函数提供，渲染与命中不漂移。
//!
//! - 用户消息：`> ` 前缀（占用整块底色，图标用输入框同款强调色）+ 整块浅底色
//!   （`style::chat_user_bg`），正文按显示列软换行。
//! - assistant 消息：正文用共享阅读渲染器（`ui::markdown_render`，与 UI 页 agent 回答
//!   同款；表格按宽度收缩自适应），不显示标题行；失败/中断/流中在此上方插入一行状态标记。
//! - thinking：生成中的块自动展开（头部转轮 + 正文），完成后默认折叠；点击头部或
//!   Ctrl+T 切换；正文同样走 markdown 渲染并整体压暗（对齐 opencode 的思考显示）。
//! - 本轮统计行（assistant 末尾）：模型名、输入/输出 token、缓存命中、tok/s 与耗时。
//! - 右侧预留一列滚动条（内容溢出时显示），宽度与输入框对齐。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::state::{ChatMessageView, Lx};
use crate::ui::markdown_render;
use crate::ui::widgets::scrollbar;
use crate::ui::{style, text};

use super::Grid;

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

/// 历史区布局：内容矩形 + 每个 thinking 折叠块的头部矩形（命中用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLayout {
    pub area: Rect,
    /// （消息索引, 该消息内 part 索引, 头部矩形）。
    pub thinking: Vec<(usize, usize, Rect)>,
    /// 渲染出的内容总行数（滚动上限用）。
    pub total_rows: usize,
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
            total_rows: 0,
        };
    }
    for (message_index, message) in lx.chat.messages.iter().enumerate() {
        // 消息内块（状态标记 / 各内容块 / 统计行）之间空一行。
        let mut first_block = true;
        if let Some(marker) = status_marker(message) {
            first_block = false;
            rows += wrap(&marker, width).len();
        }
        for (part_index, part) in message.parts.iter().enumerate() {
            let (is_reasoning, part_rows) = match part.kind {
                crate::chat::events::PartKind::Text => {
                    (false, text_rows(message, &part.content, width))
                }
                crate::chat::events::PartKind::Reasoning => (
                    true,
                    1 + if lx.chat.reasoning_expanded(message_index, part_index) {
                        thinking_rows(&part.content, width)
                    } else {
                        0
                    },
                ),
            };
            if part_rows == 0 {
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
                    part_index,
                    Rect::new(area.x, header_y, width, 1),
                ));
            }
            rows += part_rows;
        }
        if let Some(line) = stats_line(message) {
            if !first_block {
                rows += 1;
            }
            rows += wrap(&line, width).len();
        }
        rows += 1; // 消息间空行。
    }
    HistoryLayout {
        area,
        thinking,
        total_rows: rows,
    }
}

/// 渲染历史内容；`scroll` 为顶部行偏移（0 表示从最早开始）。`rect` 为整个历史区
/// （含右侧滚动条列）。
pub(super) fn render(grid: &mut Grid, rect: Rect, lx: &Lx, scroll: usize) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    let content = content_rect(rect);
    let mut row_index = 0usize;
    for (message_index, message) in lx.chat.messages.iter().enumerate() {
        let is_user = message.role == "user";
        // 消息内块（状态标记 / 各内容块 / 统计行）之间空一行（与 `layout` 同口径）。
        let mut first_block = true;
        // assistant 状态标记行（失败/中断/流中）；完成态无标记（移除 `assistant` 标签）。
        if !is_user && let Some(marker) = status_marker(message) {
            first_block = false;
            let marker_style = if message.status == "failed" {
                style::error()
            } else {
                style::muted()
            };
            for line in wrap(&marker, content.width) {
                if let Some(y) = visible_row(content, scroll, row_index) {
                    grid.put_clipped(content.x, y, &line, marker_style, content.width);
                }
                row_index += 1;
            }
        }
        let mut first_user_line = true;
        for (part_index, part) in message.parts.iter().enumerate() {
            let expanded = lx.chat.reasoning_expanded(message_index, part_index);
            let block_rows = match part.kind {
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
            if block_rows == 0 {
                continue; // 空正文块不占行，也不引入块间空行（与 `layout` 同口径）。
            }
            if !first_block {
                row_index += 1;
            }
            first_block = false;
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
                    let streaming = lx.chat.reasoning_streaming(message_index, part_index);
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
            token_count(input),
            text::LX_CHAT_STATS_INPUT_SUFFIX
        ));
    }
    if let Some(output) = message.output_tokens.filter(|tokens| *tokens > 0) {
        fields.push(format!(
            "{}{}",
            token_count(output),
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
fn token_count(tokens: i64) -> String {
    if tokens < 1_000 {
        return tokens.to_string();
    }
    if tokens < 10_000 {
        return format!("{:.1}k", tokens as f64 / 1_000.0);
    }
    if tokens < 1_000_000 {
        return format!("{}k", (tokens as f64 / 1_000.0).round() as i64);
    }
    format!("{:.1}M", tokens as f64 / 1_000_000.0)
}

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
fn visible_row(rect: Rect, scroll: usize, row: usize) -> Option<u16> {
    if row < scroll {
        return None;
    }
    let offset = u16::try_from(row - scroll).unwrap_or(u16::MAX);
    let screen_y = rect.y.saturating_add(offset);
    (screen_y < rect.bottom()).then_some(screen_y)
}

/// assistant 状态标记行：仅失败/中断显示；完成态与流中不显示（流中改用输入框上方
/// loading 行，移除消息列表里的 `assistant`/`streaming` 提示）。
fn status_marker(message: &crate::app::state::ChatMessageView) -> Option<String> {
    if message.role == "user" {
        return None;
    }
    match message.status.as_str() {
        "interrupted" => Some(format!("[{}]", text::LX_CHAT_INTERRUPTED_LABEL)),
        "failed" => Some(
            match message
                .error_code
                .as_deref()
                .filter(|code| !code.is_empty())
            {
                Some(code) => format!("[{}: {code}]", text::LX_CHAT_FAILED_LABEL),
                None => format!("[{}]", text::LX_CHAT_FAILED_LABEL),
            },
        ),
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

//! 聊天历史渲染：消息、正文流增量、thinking 折叠块、滚动条与滚动。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；命中几何（thinking 头部矩形）由
//! 同一布局函数提供，渲染与命中不漂移。
//!
//! - 用户消息：`> ` 前缀（占用整块底色，图标用输入框同款强调色）+ 整块浅底色
//!   （`style::chat_user_bg`），正文按显示列软换行。
//! - assistant 消息：正文用共享阅读渲染器（`ui::markdown_render`，与 UI 页 agent 回答
//!   同款；表格按宽度收缩自适应），不显示标题行；失败/中断/流中在此上方插入一行状态标记。
//! - 右侧预留一列滚动条（内容溢出时显示），宽度与输入框对齐。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::state::Lx;
use crate::ui::markdown_render;
use crate::ui::widgets::scrollbar;
use crate::ui::{style, text};

use super::Grid;

/// 用户消息前缀（`> `）。
const USER_PREFIX: &str = "> ";
/// 用户消息正文缩进列（与前缀等宽）。
const USER_INDENT: u16 = 2;
/// thinking 折叠块头部缩进列。
const THINKING_MARGIN: u16 = 2;
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
        if let Some(marker) = status_marker(message) {
            rows += wrap(&marker, width).len();
        }
        for (part_index, part) in message.parts.iter().enumerate() {
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    rows += text_rows(message, &part.content, width);
                }
                crate::chat::events::PartKind::Reasoning => {
                    let header_y = area
                        .y
                        .saturating_add(u16::try_from(rows).unwrap_or(u16::MAX));
                    thinking.push((
                        message_index,
                        part_index,
                        Rect::new(area.x, header_y, width, 1),
                    ));
                    rows += 1;
                    if lx.chat.reasoning_expanded(message_index, part_index) {
                        rows += wrap(&part.content, width.saturating_sub(THINKING_MARGIN)).len();
                    }
                }
            }
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
        // assistant 状态标记行（失败/中断/流中）；完成态无标记（移除 `assistant` 标签）。
        if !is_user && let Some(marker) = status_marker(message) {
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
                    let expanded = lx.chat.reasoning_expanded(message_index, part_index);
                    let label = if expanded {
                        text::LX_CHAT_THINKING_EXPANDED
                    } else {
                        text::LX_CHAT_THINKING_COLLAPSED
                    };
                    if let Some(y) = visible_row(content, scroll, row_index) {
                        grid.put_clipped(
                            content.x + THINKING_MARGIN,
                            y,
                            label,
                            style::muted(),
                            content.width.saturating_sub(THINKING_MARGIN),
                        );
                    }
                    row_index += 1;
                    if expanded {
                        let inner = content.width.saturating_sub(THINKING_MARGIN);
                        for line in wrap(&part.content, inner) {
                            if let Some(y) = visible_row(content, scroll, row_index) {
                                grid.put_clipped(
                                    content.x + THINKING_MARGIN,
                                    y,
                                    &line,
                                    style::muted(),
                                    inner,
                                );
                            }
                            row_index += 1;
                        }
                    }
                }
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

/// 行可见时返回其屏幕 Y。
fn visible_row(rect: Rect, scroll: usize, row: usize) -> Option<u16> {
    if row < scroll {
        return None;
    }
    let offset = u16::try_from(row - scroll).unwrap_or(u16::MAX);
    let screen_y = rect.y.saturating_add(offset);
    (screen_y < rect.bottom()).then_some(screen_y)
}

/// assistant 状态标记行：仅失败/中断/流中显示；完成态不显示（移除 `assistant` 标签）。
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
        "streaming" => Some(format!("[{}]", text::LX_CHAT_STREAMING_LABEL)),
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

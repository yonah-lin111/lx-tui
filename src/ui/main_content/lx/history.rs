//! 聊天历史渲染：消息、正文流增量、thinking 折叠块与滚动。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；命中几何（thinking 头部矩形）由
//! 同一布局函数提供，渲染与命中不漂移。
//!
//! - 用户消息：`> ` 前缀 + 整块浅底色（`style::chat_user_bg`），正文按显示列软换行。
//! - assistant 消息：正文用共享阅读渲染器（`ui::markdown_render`，与 UI 页 agent 回答
//!   同款），不显示标题行；失败/中断/流中在此上方插入一行状态标记（错误用 `style::error`）。

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthChar;

use crate::app::state::Lx;
use crate::ui::markdown_render::{self, MdLine};
use crate::ui::{style, text};

use super::Grid;

/// 用户消息前缀（`> `）。
const USER_PREFIX: &str = "> ";
/// 用户消息正文缩进列（与前缀等宽）。
const USER_INDENT: u16 = 2;
/// thinking 折叠块头部缩进列。
const THINKING_MARGIN: u16 = 2;

/// 历史区布局：内容矩形 + 每个 thinking 折叠块的头部矩形（命中用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryLayout {
    pub area: Rect,
    /// （消息索引, 该消息内 part 索引, 头部矩形）。
    pub thinking: Vec<(usize, usize, Rect)>,
    /// 渲染出的内容总行数（滚动上限用）。
    pub total_rows: usize,
}

/// 计算历史布局（按当前展开状态）。
pub(super) fn layout(area: Rect, lx: &Lx) -> HistoryLayout {
    let mut rows = 0usize;
    let mut thinking = Vec::new();
    if area.width == 0 {
        return HistoryLayout {
            area,
            thinking,
            total_rows: 0,
        };
    }
    for (message_index, message) in lx.chat.messages.iter().enumerate() {
        if let Some(marker) = status_marker(message) {
            rows += wrap(&marker, area.width).len();
        }
        for (part_index, part) in message.parts.iter().enumerate() {
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    rows += text_rows(message, &part.content, area.width);
                }
                crate::chat::events::PartKind::Reasoning => {
                    let header_y = area
                        .y
                        .saturating_add(u16::try_from(rows).unwrap_or(u16::MAX));
                    thinking.push((
                        message_index,
                        part_index,
                        Rect::new(area.x, header_y, area.width, 1),
                    ));
                    rows += 1;
                    if lx.chat.reasoning_expanded(message_index, part_index) {
                        rows +=
                            wrap(&part.content, area.width.saturating_sub(THINKING_MARGIN)).len();
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

/// 渲染历史内容；`scroll` 为顶部行偏移（0 表示从最早开始）。
pub(super) fn render(grid: &mut Grid, rect: Rect, lx: &Lx, scroll: usize) {
    if rect.width == 0 || rect.height == 0 {
        return;
    }
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
            for line in wrap(&marker, rect.width) {
                if let Some(y) = visible_row(rect, scroll, row_index) {
                    grid.put_clipped(rect.x, y, &line, marker_style, rect.width);
                }
                row_index += 1;
            }
        }
        let mut first_user_line = true;
        for (part_index, part) in message.parts.iter().enumerate() {
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    if is_user {
                        for line in wrap(&part.content, rect.width.saturating_sub(USER_INDENT)) {
                            if let Some(y) = visible_row(rect, scroll, row_index) {
                                let bg = style::chat_user_bg();
                                grid.background_row(rect.x, y, rect.width, bg);
                                if first_user_line {
                                    grid.put(rect.x, y, USER_PREFIX, style::muted());
                                    first_user_line = false;
                                }
                                grid.put_clipped(
                                    rect.x.saturating_add(USER_INDENT),
                                    y,
                                    &line,
                                    bg,
                                    rect.width.saturating_sub(USER_INDENT),
                                );
                            }
                            row_index += 1;
                        }
                    } else {
                        draw_markdown(grid, rect, scroll, &mut row_index, &part.content);
                    }
                }
                crate::chat::events::PartKind::Reasoning => {
                    let expanded = lx.chat.reasoning_expanded(message_index, part_index);
                    let label = if expanded {
                        text::LX_CHAT_THINKING_EXPANDED
                    } else {
                        text::LX_CHAT_THINKING_COLLAPSED
                    };
                    if let Some(y) = visible_row(rect, scroll, row_index) {
                        grid.put_clipped(
                            rect.x + THINKING_MARGIN,
                            y,
                            label,
                            style::muted(),
                            rect.width.saturating_sub(THINKING_MARGIN),
                        );
                    }
                    row_index += 1;
                    if expanded {
                        let inner = rect.width.saturating_sub(THINKING_MARGIN);
                        for line in wrap(&part.content, inner) {
                            if let Some(y) = visible_row(rect, scroll, row_index) {
                                grid.put_clipped(
                                    rect.x + THINKING_MARGIN,
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

/// 消息正文渲染行数：assistant 用 markdown 渲染（每行按宽度软换行），用户按纯文本换行。
fn text_rows(message: &crate::app::state::ChatMessageView, content: &str, width: u16) -> usize {
    if message.role == "user" {
        return wrap(content, width.saturating_sub(USER_INDENT)).len();
    }
    markdown_rows(content, width)
}

/// assistant markdown 正文渲染成若干行（软换行到 `width`）。
fn markdown_rows(content: &str, width: u16) -> usize {
    markdown_lines(content, width).len()
}

/// 把 markdown 渲染成按宽度软换行的行（每行携带缩进与分段）。
fn markdown_lines(content: &str, width: u16) -> Vec<(u16, Vec<(String, ratatui::style::Style)>)> {
    let mut out = Vec::new();
    for line in markdown_render::render(content) {
        wrap_spans(&line, width, &mut out);
    }
    out
}

/// 一条渲染行按可用宽度软换行（保留缩进），拆成多条单行分段。
fn wrap_spans(
    line: &MdLine,
    width: u16,
    out: &mut Vec<(u16, Vec<(String, ratatui::style::Style)>)>,
) {
    let avail = usize::from(width)
        .saturating_sub(usize::from(line.indent))
        .max(1);
    let mut current: Vec<(String, ratatui::style::Style)> = Vec::new();
    let mut col = 0usize;
    for (value, span_style) in &line.spans {
        let mut buffer = String::new();
        for ch in value.chars() {
            let cell = ch.width().unwrap_or(1).max(1);
            if col + cell > avail && col > 0 {
                if !buffer.is_empty() {
                    current.push((std::mem::take(&mut buffer), *span_style));
                }
                out.push((line.indent, std::mem::take(&mut current)));
                col = 0;
            }
            buffer.push(ch);
            col += cell;
        }
        if !buffer.is_empty() {
            current.push((buffer, *span_style));
        }
    }
    out.push((line.indent, current));
}

/// 绘制一段 assistant markdown 正文（每行按宽度软换行）。
fn draw_markdown(grid: &mut Grid, rect: Rect, scroll: usize, row_index: &mut usize, content: &str) {
    for (indent, spans) in markdown_lines(content, rect.width) {
        if let Some(y) = visible_row(rect, scroll, *row_index) {
            let mut cx = rect.x.saturating_add(indent);
            for (value, span_style) in &spans {
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

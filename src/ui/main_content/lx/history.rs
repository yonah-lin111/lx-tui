//! 聊天历史渲染：消息、正文流增量、thinking 折叠块与滚动。
//!
//! 纯渲染：只读 `Lx`，只写 lx 页网格；命中几何（thinking 头部矩形）由
//! 同一布局函数提供，渲染与命中不漂移。

use ratatui::layout::Rect;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::Lx;
use crate::ui::markdown::{self};
use crate::ui::{style, text};

use super::Grid;

/// 角色前缀（assistant 竖条 / user `>`）。
const ROLE_PREFIX: &str = "▌ ";
const USER_PREFIX: &str = "> ";
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
        rows += 1; // 角色标题行。
        for (part_index, part) in message.parts.iter().enumerate() {
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    rows += wrap(&part.content, area.width).len();
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
        let header = heading_for(message);
        let header_style = if is_user {
            style::muted()
        } else {
            style::accent()
        };
        if let Some(y) = visible_row(rect, scroll, row_index) {
            let prefix = if is_user { USER_PREFIX } else { ROLE_PREFIX };
            let prefix_w = u16::try_from(prefix.width()).unwrap_or(0);
            grid.put_clipped(rect.x, y, prefix, header_style, rect.width);
            grid.put_clipped(
                rect.x + prefix_w,
                y,
                &header,
                header_style,
                rect.width.saturating_sub(prefix_w),
            );
        }
        row_index += 1;
        for (part_index, part) in message.parts.iter().enumerate() {
            match part.kind {
                crate::chat::events::PartKind::Text => {
                    for line in wrap(&part.content, rect.width) {
                        if let Some(y) = visible_row(rect, scroll, row_index) {
                            draw_markdown_line(grid, rect.x, y, &line, rect.width);
                        }
                        row_index += 1;
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

/// 行可见时返回其屏幕 Y。
fn visible_row(rect: Rect, scroll: usize, row: usize) -> Option<u16> {
    if row < scroll {
        return None;
    }
    let offset = u16::try_from(row - scroll).unwrap_or(u16::MAX);
    let screen_y = rect.y.saturating_add(offset);
    (screen_y < rect.bottom()).then_some(screen_y)
}

/// 消息标题：assistant 显示模型标签或状态后缀。
fn heading_for(message: &crate::app::state::ChatMessageView) -> String {
    if message.role == "user" {
        return String::new();
    }
    let base = message
        .model_label
        .clone()
        .unwrap_or_else(|| text::LX_CHAT_ASSISTANT_LABEL.to_string());
    match message.status.as_str() {
        "interrupted" => format!("{base}  [{}]", text::LX_CHAT_INTERRUPTED_LABEL),
        "failed" => format!("{base}  [{}]", text::LX_CHAT_FAILED_LABEL),
        "streaming" => format!("{base}  [{}]", text::LX_CHAT_STREAMING_LABEL),
        _ => base,
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

/// 写入一行 markdown 高亮文本（按 token 逐字符着色）。
fn draw_markdown_line(grid: &mut Grid, x: u16, y: u16, line: &str, width: u16) {
    let tokens = markdown::scan(line);
    let line_tokens = tokens.first().map(Vec::as_slice).unwrap_or_default();
    let right = x.saturating_add(width);
    let mut cx = x;
    let mut token_index = 0;
    for (offset, ch) in line.char_indices() {
        let cell = ch.width().unwrap_or(0) as u16;
        if cell == 0 {
            continue;
        }
        while line_tokens
            .get(token_index)
            .is_some_and(|token| token.range.end <= offset)
        {
            token_index += 1;
        }
        let cell_style = line_tokens
            .get(token_index)
            .filter(|token| token.range.contains(&offset))
            .map_or_else(style::text, |token| {
                crate::ui::prompt::token_style(token.kind)
            });
        if cx.saturating_add(cell) > right {
            break;
        }
        grid.put_char(cx, y, ch, cell_style);
        cx = cx.saturating_add(cell);
    }
}

#[cfg(test)]
mod tests;

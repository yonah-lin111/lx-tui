//! 单行输入组件：按显示宽度计算视口与光标列；不依赖应用领域数据。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthChar;

use crate::ui::style;

/// 单行输入视口：可见文本、光标在视口内的显示列及视口首字符在全文中的字符索引。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputView<'a> {
    pub visible: &'a str,
    pub cursor_col: u16,
    pub start_char: usize,
}

/// 计算视口与光标列；`cursor` 为字符索引，`width` 为可用显示列数。
///
/// 视口按显示宽度滚动（宽字符按两列计），保证光标与 IME 预输入位置一致。
pub fn view(text: &str, cursor: usize, width: usize) -> InputView<'_> {
    if width == 0 {
        return InputView {
            visible: "",
            cursor_col: 0,
            start_char: 0,
        };
    }
    let cursor = cursor.min(text.chars().count());
    let cursor_width: usize = text.chars().take(cursor).map(char_cells).sum();
    let offset = cursor_width.saturating_sub(width - 1);
    let mut start = 0;
    let mut skipped = 0;
    for (byte, ch) in text.char_indices() {
        if skipped >= offset {
            break;
        }
        skipped += char_cells(ch);
        start = byte + ch.len_utf8();
    }
    let mut end = start;
    let mut used = 0;
    for (byte, ch) in text[start..].char_indices() {
        let cells = char_cells(ch);
        if used + cells > width {
            break;
        }
        used += cells;
        end = start + byte + ch.len_utf8();
    }
    let limit = u16::try_from(width.saturating_sub(1)).unwrap_or(u16::MAX);
    let cursor_col = u16::try_from(cursor_width.saturating_sub(skipped))
        .unwrap_or(u16::MAX)
        .min(limit);
    let start_char = text[..start].chars().count();
    InputView {
        visible: &text[start..end],
        cursor_col,
        start_char,
    }
}

/// 将视口内显示列换算为全文字符索引。
pub fn char_index_at_col(text: &str, cursor: usize, width: usize, col: u16) -> usize {
    let input = view(text, cursor, width);
    let mut current_col = 0;
    let target_col = usize::from(col);
    for (idx, ch) in input.visible.chars().enumerate() {
        let cells = char_cells(ch);
        if current_col + cells > target_col {
            return input.start_char + idx;
        }
        current_col += cells;
    }
    input.start_char + input.visible.chars().count()
}

/// 渲染单行输入文本；空文本且有占位时渲染暗色输入说明；
/// 返回硬件光标坐标（终端原生光标负责显示与闪烁）。
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    text: &str,
    cursor: usize,
    placeholder: Option<&str>,
) -> Option<(u16, u16)> {
    render_with_selection(frame, area, text, cursor, None, placeholder)
}

/// 渲染带选区高亮的单行输入文本；选区范围 `[start, end)` 字符索引内反色显示。
pub fn render_with_selection(
    frame: &mut Frame<'_>,
    area: Rect,
    text: &str,
    cursor: usize,
    selection: Option<(usize, usize)>,
    placeholder: Option<&str>,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let input = view(text, cursor, usize::from(area.width));
    if text.is_empty() {
        if let Some(placeholder) = placeholder {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(placeholder, style::muted()))),
                Rect { height: 1, ..area },
            );
        }
    } else {
        let spans = build_styled_spans(input.visible, input.start_char, selection);
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect { height: 1, ..area },
        );
    }
    Some((
        area.x
            .saturating_add(input.cursor_col)
            .min(area.right().saturating_sub(1)),
        area.y,
    ))
}

/// 根据字符选区构造带反色样式的分段。
pub fn build_styled_spans<'a>(
    visible: &'a str,
    start_char: usize,
    selection: Option<(usize, usize)>,
) -> Vec<Span<'a>> {
    let Some((sel_start, sel_end)) = selection else {
        return vec![Span::styled(visible, style::text())];
    };
    if sel_start >= sel_end {
        return vec![Span::styled(visible, style::text())];
    }
    let mut spans = Vec::new();
    let mut current_selected = false;
    let mut chunk_start = 0;

    for (char_idx, (byte_offset, _ch)) in visible.char_indices().enumerate() {
        let global_char = start_char + char_idx;
        let is_selected = global_char >= sel_start && global_char < sel_end;
        if char_idx == 0 {
            current_selected = is_selected;
            chunk_start = 0;
        } else if is_selected != current_selected {
            let chunk = &visible[chunk_start..byte_offset];
            let cell_style = if current_selected {
                style::text().add_modifier(Modifier::REVERSED)
            } else {
                style::text()
            };
            spans.push(Span::styled(chunk, cell_style));
            chunk_start = byte_offset;
            current_selected = is_selected;
        }
    }

    if chunk_start < visible.len() {
        let chunk = &visible[chunk_start..];
        let cell_style = if current_selected {
            style::text().add_modifier(Modifier::REVERSED)
        } else {
            style::text()
        };
        spans.push(Span::styled(chunk, cell_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(visible, style::text()));
    }
    spans
}

/// 字符显示列数；控制字符按 0 列。
fn char_cells(ch: char) -> usize {
    ch.width().unwrap_or(0)
}

#[cfg(test)]
mod tests;

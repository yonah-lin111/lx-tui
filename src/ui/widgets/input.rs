//! 单行输入组件：按显示宽度计算视口与光标列；不依赖应用领域数据。

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthChar;

use crate::ui::style;

/// 单行输入视口：可见文本与光标在视口内的显示列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputView<'a> {
    pub visible: &'a str,
    pub cursor_col: u16,
}

/// 计算视口与光标列；`cursor` 为字符索引，`width` 为可用显示列数。
///
/// 视口按显示宽度滚动（宽字符按两列计），保证光标与 IME 预输入位置一致。
pub fn view(text: &str, cursor: usize, width: usize) -> InputView<'_> {
    if width == 0 {
        return InputView {
            visible: "",
            cursor_col: 0,
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
    InputView {
        visible: &text[start..end],
        cursor_col,
    }
}

/// 渲染单行输入文本；空文本且有占位时渲染暗色输入说明；
/// 返回硬件光标坐标（显示由终端原生光标承担，闪烁相位由应用控制）。
pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    text: &str,
    cursor: usize,
    placeholder: Option<&str>,
) -> Option<(u16, u16)> {
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let input = view(text, cursor, usize::from(area.width));
    let (visible, text_style) = match placeholder {
        Some(placeholder) if text.is_empty() => (placeholder, style::muted()),
        _ => (input.visible, style::text()),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(visible, text_style))),
        Rect { height: 1, ..area },
    );
    Some((
        area.x
            .saturating_add(input.cursor_col)
            .min(area.right().saturating_sub(1)),
        area.y,
    ))
}

/// 字符显示列数；控制字符按 0 列。
fn char_cells(ch: char) -> usize {
    ch.width().unwrap_or(0)
}

#[cfg(test)]
mod tests;

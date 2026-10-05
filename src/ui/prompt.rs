//! prompt 编辑器渲染：文本、markdown 高亮、选区与光标，只读状态。

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthChar;

use crate::app::prompt::{Prompt, VisualRow};
use crate::app::selection::Selection;
use crate::ui::markdown::{self, Token, TokenKind};
use crate::ui::style;

/// 绘制 prompt 内容区；`focused` 为真时叠加光标反显。
pub fn render(
    area: Rect,
    buf: &mut Buffer,
    prompt: &Prompt,
    focused: bool,
    selection: Option<&Selection>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rows = prompt.visual_rows();
    let tokens = markdown::scan(prompt.text());
    let scroll = prompt.scroll().min(rows.len().saturating_sub(1));
    for (index, row) in rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(area.height))
    {
        let y = area.y + (index - scroll) as u16;
        let line_tokens = tokens.get(row.line).map(Vec::as_slice).unwrap_or_default();
        paint_row(buf, area, y, prompt.text(), row, line_tokens);
        if let Some(selection) = selection {
            paint_selection(buf, area, y, (index - scroll) as u16, selection);
        }
    }

    if focused && let Some((row, col)) = prompt.cursor_cell() {
        let col = col.min(area.width - 1);
        if let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) {
            cell.modifier |= Modifier::REVERSED;
        }
    }
}

/// 绘制一个视觉行：按 token 着色，宽字符占位单元格标记为跳过。
fn paint_row(buf: &mut Buffer, area: Rect, y: u16, text: &str, row: &VisualRow, tokens: &[Token]) {
    let mut x = area.x;
    let mut token_index = 0;
    for (offset, ch) in text[row.start..row.end].char_indices() {
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            continue;
        }
        while tokens
            .get(token_index)
            .is_some_and(|token| token.range.end <= offset)
        {
            token_index += 1;
        }
        let style = tokens
            .get(token_index)
            .filter(|token| token.range.contains(&offset))
            .map_or_else(style::text, |token| token_style(token.kind));
        if x.saturating_add(width as u16) > area.right() {
            break;
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.reset();
            cell.set_char(ch);
            cell.set_style(style);
            cell.set_diff_option(CellDiffOption::None);
        }
        for dx in 1..width as u16 {
            if let Some(cell) = buf.cell_mut((x + dx, y)) {
                cell.set_diff_option(CellDiffOption::Skip);
            }
        }
        x += width as u16;
    }
}

/// 选区按视口坐标反显；空白单元格同样覆盖，保证拖拽范围可见。
fn paint_selection(buf: &mut Buffer, area: Rect, y: u16, row: u16, selection: &Selection) {
    for col in 0..area.width {
        if selection.contains(row, col)
            && let Some(cell) = buf.cell_mut((area.x + col, y))
        {
            cell.modifier |= Modifier::REVERSED;
        }
    }
}

/// token 语义到样式的映射。
fn token_style(kind: TokenKind) -> Style {
    match kind {
        TokenKind::Marker => style::markdown_marker(),
        TokenKind::Heading => style::markdown_heading(),
        TokenKind::Strong => style::markdown_strong(),
        TokenKind::Emphasis => style::markdown_emphasis(),
        TokenKind::Strikethrough => style::markdown_strikethrough(),
        TokenKind::InlineCode => style::markdown_inline_code(),
        TokenKind::CodeBlock => style::markdown_code_block(),
        TokenKind::Quote => style::markdown_quote(),
        TokenKind::LinkText => style::markdown_link_text(),
        TokenKind::Url => style::markdown_url(),
    }
}

#[cfg(test)]
mod tests;

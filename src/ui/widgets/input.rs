//! 单行输入组件与多行软折行视口助手：按显示宽度计算视口、光标列与点击定位；
//! 不依赖应用领域数据。

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

/// 多行视口中的一个视觉行：文本切片与它在全文中的字符区间。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrappedLine<'a> {
    pub text: &'a str,
    pub start_char: usize,
    pub end_char: usize,
}

/// 多行输入视口：可见视觉行、窗口起始行、光标相对可见首行的偏移及光标列。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultilineInputView<'a> {
    /// 当前视口可见的 `1..=max_rows` 行。
    pub visible_lines: Vec<WrappedLine<'a>>,
    /// 当前可见窗口在全部折行中的起始行号。
    pub start_row: usize,
    /// 光标相对于可见首行的行偏移（`0..visible_lines.len()`）。
    pub cursor_row_offset: u16,
    /// 光标在当前视觉行内的显示列偏移。
    pub cursor_col: u16,
}

/// 按显示列宽折行并限制最多 `max_rows` 行；超出时在末行尾部用 `…` 截断。
///
/// 支持文本内换行符 `\n` 与按显示宽度的软折行；返回 `1..=max_rows` 行。
pub fn wrap_clamped_lines(text: &str, width: usize, max_rows: usize) -> Vec<String> {
    wrap_clamped_lines_indented(text, width, width, max_rows)
}

/// 按首行可用宽度 `first_width` 与续行可用宽度 `cont_width` 软折行，并限制最多 `max_rows` 行；
/// 超出时在末行尾部用 `…` 截断。
pub fn wrap_clamped_lines_indented(
    text: &str,
    first_width: usize,
    cont_width: usize,
    max_rows: usize,
) -> Vec<String> {
    let first_width = first_width.max(1);
    let cont_width = cont_width.max(1);
    let max_rows = max_rows.max(1);
    let mut lines = wrap_text(text, first_width, cont_width);
    if lines.len() > max_rows {
        lines.truncate(max_rows);
        let last_width = if max_rows <= 1 {
            first_width
        } else {
            cont_width
        };
        if let Some(last) = lines.last_mut() {
            *last = truncate_with_ellipsis(last, last_width);
        }
    }
    lines
}

/// 计算多行输入视口：软折行、光标位置与跟随光标的滑动窗口。
pub fn multiline_view(
    text: &str,
    cursor: usize,
    width: usize,
    max_rows: usize,
) -> MultilineInputView<'_> {
    multiline_view_scrolled(text, cursor, width, max_rows, 0)
}

/// 计算带既有滚动行偏移 `scroll_row` 的多行输入视口。
///
/// 仅当光标滚出 `[scroll_row, scroll_row + max_rows)` 时才上下推动窗口，
/// 避免在超 `max_rows` 行编辑中点击视口首行导致视口跳回第 0 行。
pub fn multiline_view_scrolled(
    text: &str,
    cursor: usize,
    width: usize,
    max_rows: usize,
    scroll_row: usize,
) -> MultilineInputView<'_> {
    multiline_view_scrolled_indented(text, cursor, width, width, max_rows, scroll_row)
}

/// 计算带首行宽度 `first_width`、续行宽度 `cont_width` 与滚动行偏移 `scroll_row` 的多行输入视口。
pub fn multiline_view_scrolled_indented(
    text: &str,
    cursor: usize,
    first_width: usize,
    cont_width: usize,
    max_rows: usize,
    scroll_row: usize,
) -> MultilineInputView<'_> {
    let first_width = first_width.max(1);
    let cont_width = cont_width.max(1);
    let max_rows = max_rows.max(1);
    let lines = wrapped_lines(text, first_width, cont_width);
    let (cursor_row, cursor_col) = cursor_position(&lines, cursor);
    let visible_count = lines.len().clamp(1, max_rows);
    let max_start = lines.len().saturating_sub(visible_count);
    let mut start_row = scroll_row.min(max_start);
    if cursor_row < start_row {
        start_row = cursor_row;
    } else if cursor_row >= start_row + visible_count {
        start_row = cursor_row + 1 - visible_count;
    }
    start_row = start_row.min(max_start);
    let end_row = start_row.saturating_add(visible_count).min(lines.len());
    MultilineInputView {
        visible_lines: lines[start_row..end_row].to_vec(),
        start_row,
        cursor_row_offset: u16::try_from(cursor_row.saturating_sub(start_row)).unwrap_or(u16::MAX),
        cursor_col: u16::try_from(cursor_col).unwrap_or(u16::MAX),
    }
}

/// 将点击的视觉行偏移与列偏移换算回全文字符索引。
pub fn multiline_char_index_at(
    text: &str,
    cursor: usize,
    width: usize,
    max_rows: usize,
    visual_row_offset: usize,
    rel_col: u16,
) -> usize {
    multiline_char_index_at_scrolled(text, cursor, width, max_rows, 0, visual_row_offset, rel_col)
}

/// 在给定 `scroll_row` 的多行视口下，将视觉行偏移与列偏移换算回全文字符索引。
pub fn multiline_char_index_at_scrolled(
    text: &str,
    cursor: usize,
    width: usize,
    max_rows: usize,
    scroll_row: usize,
    visual_row_offset: usize,
    rel_col: u16,
) -> usize {
    multiline_char_index_at_scrolled_indented(
        text,
        cursor,
        width,
        width,
        max_rows,
        scroll_row,
        visual_row_offset,
        rel_col,
    )
}

/// 在给定首行宽度 `first_width`、续行宽度 `cont_width` 与 `scroll_row` 的多行视口下，
/// 将视觉行偏移与列偏移换算回全文字符索引。
#[allow(clippy::too_many_arguments)]
pub fn multiline_char_index_at_scrolled_indented(
    text: &str,
    cursor: usize,
    first_width: usize,
    cont_width: usize,
    max_rows: usize,
    scroll_row: usize,
    visual_row_offset: usize,
    rel_col: u16,
) -> usize {
    let view = multiline_view_scrolled_indented(
        text,
        cursor,
        first_width,
        cont_width,
        max_rows,
        scroll_row,
    );
    let Some(line) = view.visible_lines.get(visual_row_offset) else {
        return view.visible_lines.last().map_or(0, |line| line.end_char);
    };
    let target_col = usize::from(rel_col);
    let line_width: usize = line.text.chars().map(char_cells).sum();
    if target_col >= line_width {
        return line.end_char;
    }
    let mut current_col = 0usize;
    for (index, ch) in line.text.chars().enumerate() {
        let cells = char_cells(ch);
        if current_col + cells > target_col {
            return line.start_char + index;
        }
        current_col += cells;
    }
    line.end_char
}

/// 按显示列宽完整折行（含 `\n`），不做行数限制；首行使用 `first_width`，后续行使用 `cont_width`。
fn wrap_text(text: &str, first_width: usize, cont_width: usize) -> Vec<String> {
    let line_limit = |idx: usize| if idx == 0 { first_width } else { cont_width };
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        let mut current = String::new();
        let mut col = 0usize;
        for ch in raw_line.chars() {
            let cells = char_cells(ch);
            let limit = line_limit(lines.len());
            if col + cells > limit && col > 0 {
                lines.push(std::mem::take(&mut current));
                col = 0;
            }
            current.push(ch);
            col += cells;
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// 保留不超过 `width` 的显示宽度，末列换成省略号 `…`。
fn truncate_with_ellipsis(line: &str, width: usize) -> String {
    const ELLIPSIS: char = '…';
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in line.chars() {
        let cells = char_cells(ch);
        if used + cells > width - 1 {
            break;
        }
        out.push(ch);
        used += cells;
    }
    out.push(ELLIPSIS);
    out
}

/// 完整视觉行分片：记录每行的文本切片与字符区间；首行使用 `first_width`，后续行使用 `cont_width`。
fn wrapped_lines(text: &str, first_width: usize, cont_width: usize) -> Vec<WrappedLine<'_>> {
    let line_limit = |idx: usize| if idx == 0 { first_width } else { cont_width };
    let mut lines = Vec::new();
    let mut start_byte = 0usize;
    let mut start_char = 0usize;
    let mut char_index = 0usize;
    let mut col = 0usize;
    for (byte, ch) in text.char_indices() {
        if ch == '\n' {
            let limit = line_limit(lines.len());
            lines.push(WrappedLine {
                text: &text[start_byte..byte],
                start_char,
                end_char: char_index,
            });
            if limit > 1 && col == limit {
                lines.push(WrappedLine {
                    text: "",
                    start_char: char_index,
                    end_char: char_index,
                });
            }
            start_byte = byte + ch.len_utf8();
            start_char = char_index + 1;
            char_index += 1;
            col = 0;
            continue;
        }
        let cells = char_cells(ch);
        let limit = line_limit(lines.len());
        if col + cells > limit && col > 0 {
            lines.push(WrappedLine {
                text: &text[start_byte..byte],
                start_char,
                end_char: char_index,
            });
            start_byte = byte;
            start_char = char_index;
            col = 0;
        }
        col += cells;
        char_index += 1;
    }
    let limit = line_limit(lines.len());
    lines.push(WrappedLine {
        text: &text[start_byte..],
        start_char,
        end_char: char_index,
    });
    if limit > 1 && col == limit {
        lines.push(WrappedLine {
            text: "",
            start_char: char_index,
            end_char: char_index,
        });
    }
    lines
}

/// 光标所在视觉行与列：软折行边界归下一行，硬换行前的行尾归当前行。
fn cursor_position(lines: &[WrappedLine<'_>], cursor: usize) -> (usize, usize) {
    let mut row = lines.len().saturating_sub(1);
    for (index, line) in lines.iter().enumerate() {
        let at_end = cursor == line.end_char;
        let hard_break = lines
            .get(index + 1)
            .is_none_or(|next| next.start_char > line.end_char);
        if cursor < line.end_char || (at_end && hard_break) {
            row = index;
            break;
        }
    }
    let line = &lines[row];
    let offset = cursor.min(line.end_char).saturating_sub(line.start_char);
    let col = line.text.chars().take(offset).map(char_cells).sum();
    (row, col)
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

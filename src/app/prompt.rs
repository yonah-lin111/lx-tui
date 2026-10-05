//! prompt 右栏编辑器：纯文本缓冲区、光标、软换行视口与选区取文，无 IO。

use unicode_width::UnicodeWidthChar;

use crate::layout::PaneId;

/// 粘贴文本中的制表符展开内容。
const TAB_STOP: &str = "    ";

/// prompt 右栏：全局面板标识与可编辑 markdown 文本状态。
#[derive(Debug)]
pub struct Prompt {
    id: PaneId,
    text: String,
    /// 光标字节偏移；始终位于字符边界。
    cursor: usize,
    /// 首个可见视觉行索引。
    scroll: usize,
    /// 内容区尺寸（列、行），随几何同步。
    width: u16,
    height: u16,
}

/// 一个视觉行：所属逻辑行与文本字节范围（不含行尾换行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisualRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

impl Prompt {
    /// 创建空编辑器。
    pub fn new(id: PaneId) -> Self {
        Self {
            id,
            text: String::new(),
            cursor: 0,
            scroll: 0,
            width: 1,
            height: 1,
        }
    }

    /// 面板标识；文本选择按此标识归属。
    pub fn id(&self) -> PaneId {
        self.id
    }

    /// 当前文本。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 首个可见视觉行索引。
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// 内容区尺寸（列、行）。
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// 同步内容区尺寸；光标保持在视口内。
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.width = cols.max(1);
        self.height = rows.max(1);
        self.keep_cursor_visible();
    }

    /// 插入一个字符；换行与制表符按编辑器语义处理，其余控制字符忽略。
    pub fn insert_char(&mut self, ch: char) {
        match ch {
            '\n' => self.insert_at("\n"),
            '\t' => self.insert_at(TAB_STOP),
            ch if ch.is_control() => {}
            ch => {
                let mut buffer = [0_u8; 4];
                self.insert_at(ch.encode_utf8(&mut buffer));
            }
        }
    }

    /// 插入一段文本；`\r\n`/`\r` 归一为 `\n`，制表符展开，其余控制字符丢弃。
    pub fn insert_str(&mut self, text: &str) {
        self.insert_at(&sanitize(text));
    }

    /// 在光标处换行。
    pub fn newline(&mut self) {
        self.insert_char('\n');
    }

    /// 删除光标前一个字符；行首为 no-op。
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let previous = prev_boundary(&self.text, self.cursor);
        self.text.remove(previous);
        self.cursor = previous;
        self.keep_cursor_visible();
    }

    /// 删除光标处字符；文末为 no-op。
    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        self.text.remove(self.cursor);
        self.keep_cursor_visible();
    }

    /// 光标左移一个字符。
    pub fn move_left(&mut self) {
        self.cursor = prev_boundary(&self.text, self.cursor);
        self.keep_cursor_visible();
    }

    /// 光标右移一个字符。
    pub fn move_right(&mut self) {
        self.cursor = next_boundary(&self.text, self.cursor);
        self.keep_cursor_visible();
    }

    /// 光标上移一个视觉行，保持显示列；首行时钳到行首。
    pub fn move_up(&mut self) {
        let rows = self.visual_rows();
        let (index, col) = self.cursor_visual_in(&rows);
        if index == 0 {
            self.cursor = rows[0].start;
        } else {
            let target = rows[index - 1];
            self.cursor = target.start + offset_at_col(&self.text[target.start..target.end], col);
        }
        self.keep_cursor_visible();
    }

    /// 光标下移一个视觉行，保持显示列；末行时钳到行尾。
    pub fn move_down(&mut self) {
        let rows = self.visual_rows();
        let (index, col) = self.cursor_visual_in(&rows);
        if index + 1 >= rows.len() {
            self.cursor = rows[rows.len() - 1].end;
        } else {
            let target = rows[index + 1];
            self.cursor = target.start + offset_at_col(&self.text[target.start..target.end], col);
        }
        self.keep_cursor_visible();
    }

    /// 光标移到当前视觉行行首。
    pub fn move_home(&mut self) {
        let rows = self.visual_rows();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = rows[index].start;
        self.keep_cursor_visible();
    }

    /// 光标移到当前视觉行行尾。
    pub fn move_end(&mut self) {
        let rows = self.visual_rows();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = rows[index].end;
        self.keep_cursor_visible();
    }

    /// 光标移到逻辑行行首。
    pub fn move_line_start(&mut self) {
        self.cursor = line_start(&self.text, self.cursor);
        self.keep_cursor_visible();
    }

    /// 光标移到逻辑行行尾（换行前）。
    pub fn move_line_end(&mut self) {
        self.cursor = line_end(&self.text, self.cursor);
        self.keep_cursor_visible();
    }

    /// 光标左移到前一个词的词首。
    pub fn move_word_backward(&mut self) {
        self.cursor = self.word_start_before(self.cursor);
        self.keep_cursor_visible();
    }

    /// 光标右移到下一个词的词尾（readline M-f 语义）。
    pub fn move_word_forward(&mut self) {
        self.cursor = self.word_end_after(self.cursor);
        self.keep_cursor_visible();
    }

    /// 删除光标前一个词及其后分隔符（readline ctrl+w 语义）。
    pub fn delete_word_backward(&mut self) {
        let start = self.word_start_before(self.cursor);
        if start < self.cursor {
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
            self.keep_cursor_visible();
        }
    }

    /// 删除光标到下一个词尾（readline M-d 语义）。
    pub fn delete_word_forward(&mut self) {
        let end = self.word_end_after(self.cursor);
        if end > self.cursor {
            self.text.replace_range(self.cursor..end, "");
            self.keep_cursor_visible();
        }
    }

    /// 删除光标到逻辑行首。
    pub fn delete_to_line_start(&mut self) {
        let start = line_start(&self.text, self.cursor);
        if start < self.cursor {
            self.text.replace_range(start..self.cursor, "");
            self.cursor = start;
            self.keep_cursor_visible();
        }
    }

    /// 删除光标到逻辑行尾（不删除换行）。
    pub fn delete_to_line_end(&mut self) {
        let end = line_end(&self.text, self.cursor);
        if end > self.cursor {
            self.text.replace_range(self.cursor..end, "");
            self.keep_cursor_visible();
        }
    }

    /// 按内容宽度软换行得到的视觉行。
    pub fn visual_rows(&self) -> Vec<VisualRow> {
        let width = usize::from(self.width.max(1));
        let mut rows = Vec::new();
        let mut line_start = 0;
        for (line, segment) in self.text.split('\n').enumerate() {
            let line_end = line_start + segment.len();
            let mut row_start = line_start;
            let mut col = 0;
            for (offset, ch) in segment.char_indices() {
                let absolute = line_start + offset;
                let cell = char_width(ch);
                if col > 0 && col + cell > width {
                    rows.push(VisualRow {
                        line,
                        start: row_start,
                        end: absolute,
                    });
                    row_start = absolute;
                    col = 0;
                }
                col += cell;
            }
            rows.push(VisualRow {
                line,
                start: row_start,
                end: line_end,
            });
            line_start = line_end + 1;
        }
        rows
    }

    /// 光标在视口内的坐标（行、列）；滚出视口时为 None。
    pub fn cursor_cell(&self) -> Option<(u16, u16)> {
        if self.height == 0 || self.width == 0 {
            return None;
        }
        let rows = self.visual_rows();
        let (row, col) = self.cursor_visual_in(&rows);
        let visible = row.checked_sub(self.scroll)?;
        if visible >= usize::from(self.height) {
            return None;
        }
        let col = col.min(usize::from(self.width) - 1);
        Some((visible as u16, col as u16))
    }

    /// 按视觉行滚动视口；光标不动，超界时钳到可滚动范围。
    pub fn scroll_by(&mut self, lines: isize) {
        let max = self
            .visual_rows()
            .len()
            .saturating_sub(usize::from(self.height.max(1)));
        let target = (self.scroll.min(max) as isize).saturating_add(lines);
        self.scroll = target.clamp(0, max as isize) as usize;
    }

    /// 将视口单元格坐标映射为光标位置；超出文本时钳到最近行行尾。
    pub fn set_cursor_from_cell(&mut self, row: u16, col: u16) {
        self.cursor = self.viewport_offset((row, col), false);
        self.keep_cursor_visible();
    }

    /// 取选区文本（视口坐标，端点包含）；空选区或选中内容为空时返回 None。
    pub fn selection_text(&self, start: (u16, u16), end: (u16, u16)) -> Option<String> {
        if start == end {
            return None;
        }
        let (first, last) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let from = self.viewport_offset(first, false);
        let to = self.viewport_offset(last, true);
        if from >= to {
            return None;
        }
        Some(self.text[from..to].to_string())
    }

    /// 在光标处插入已净化的文本并推进光标。
    fn insert_at(&mut self, insert: &str) {
        self.text.insert_str(self.cursor, insert);
        self.cursor += insert.len();
        self.keep_cursor_visible();
    }

    /// 光标前一个词（含尾随分隔符）的起点。
    fn word_start_before(&self, index: usize) -> usize {
        let mut boundary = 0;
        let mut in_word = false;
        let mut seen = false;
        for (offset, ch) in self.text[..index].char_indices().rev() {
            seen = true;
            if is_word_char(ch) {
                in_word = true;
                boundary = offset;
            } else if in_word {
                break;
            } else {
                boundary = offset;
            }
        }
        if seen { boundary } else { 0 }
    }

    /// 光标所在或下一个词的词尾（跳过前导分隔符）。
    fn word_end_after(&self, index: usize) -> usize {
        let mut end = index;
        let mut in_word = false;
        for (offset, ch) in self.text[index..].char_indices() {
            if is_word_char(ch) {
                in_word = true;
                end = index + offset + ch.len_utf8();
            } else if in_word {
                break;
            }
        }
        end
    }

    /// 光标在视觉行中的索引与显示列（跨折行边界时归入下一视觉行）。
    fn cursor_visual_in(&self, rows: &[VisualRow]) -> (usize, usize) {
        for (index, row) in rows.iter().enumerate() {
            if self.cursor < row.end {
                return (index, display_width(&self.text[row.start..self.cursor]));
            }
            if self.cursor == row.end {
                let continues = rows
                    .get(index + 1)
                    .is_some_and(|next| next.start == row.end && next.line == row.line);
                if !continues {
                    return (index, display_width(&self.text[row.start..row.end]));
                }
            }
        }
        let index = rows.len().saturating_sub(1);
        let row = rows[index];
        (index, display_width(&self.text[row.start..row.end]))
    }

    /// 视口坐标映射到文本字节偏移；`after` 为真时取该单元格字符之后。
    fn viewport_offset(&self, (row, col): (u16, u16), after: bool) -> usize {
        let rows = self.visual_rows();
        let index = (self.scroll + usize::from(row)).min(rows.len() - 1);
        let visual = rows[index];
        let slice = &self.text[visual.start..visual.end];
        let mut cell = 0;
        for (offset, ch) in slice.char_indices() {
            let width = char_width(ch);
            if usize::from(col) < cell + width {
                return visual.start
                    + if after {
                        offset + ch.len_utf8()
                    } else {
                        offset
                    };
            }
            cell += width;
        }
        visual.end
    }

    /// 调整滚动量，保证光标所在视觉行可见。
    fn keep_cursor_visible(&mut self) {
        let rows = self.visual_rows();
        let height = usize::from(self.height.max(1));
        let (row, _) = self.cursor_visual_in(&rows);
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + height {
            self.scroll = row + 1 - height;
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
    }
}

/// 词字符：Unicode 字母数字与下划线；CJK 视作词字符。
fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// 所在逻辑行的起始字节偏移（上一个换行之后）。
fn line_start(text: &str, index: usize) -> usize {
    text[..index].rfind('\n').map_or(0, |offset| offset + 1)
}

/// 所在逻辑行的结束字节偏移（下一个换行之前或文末）。
fn line_end(text: &str, index: usize) -> usize {
    text[index..]
        .find('\n')
        .map_or(text.len(), |offset| index + offset)
}

/// 字符显示宽度；零宽与不可打印字符按 0 处理。
fn char_width(ch: char) -> usize {
    ch.width().unwrap_or(0)
}

/// 文本显示宽度（列）。
fn display_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 前一个字符的字节边界。
fn prev_boundary(text: &str, index: usize) -> usize {
    text[..index]
        .char_indices()
        .next_back()
        .map(|(offset, _)| offset)
        .unwrap_or(0)
}

/// 后一个字符的字节边界。
fn next_boundary(text: &str, index: usize) -> usize {
    text[index..]
        .chars()
        .next()
        .map(|ch| index + ch.len_utf8())
        .unwrap_or(index)
}

/// 行内显示列对应的字节偏移；落在宽字符中间或超出时钳到对应边界。
fn offset_at_col(slice: &str, col: usize) -> usize {
    let mut cell = 0;
    for (offset, ch) in slice.char_indices() {
        let width = char_width(ch);
        if col < cell + width {
            return offset;
        }
        cell += width;
    }
    slice.len()
}

/// 粘贴文本净化：换行归一、制表符展开、其余控制字符丢弃。
fn sanitize(text: &str) -> String {
    let mut sanitized = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                sanitized.push('\n');
            }
            '\n' => sanitized.push('\n'),
            '\t' => sanitized.push_str(TAB_STOP),
            ch if ch.is_control() => {}
            ch => sanitized.push(ch),
        }
    }
    sanitized
}

#[cfg(test)]
mod tests;

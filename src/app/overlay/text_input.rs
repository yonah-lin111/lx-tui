//! 单行文本输入：字符缓冲、光标、选区与撤销/重做；供全部浮层与内联搜索框复用。

use unicode_width::UnicodeWidthChar;

use crate::app::actions::OverlayKey;
use crate::app::prompt::text::{offset_at_col, word_end_after, word_start_before};

/// 单行输入撤销历史上限（与 prompt 编辑器的 HISTORY_LIMIT 对齐，防止长会话无界增长）。
const MAX_UNDO_STEPS: usize = 200;

/// 单行文本输入：字符缓冲与字符索引光标（范围 `0..=字符数`），可选选区锚点与撤销/重做栈。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInput {
    text: String,
    cursor: usize,
    selection_anchor: Option<usize>,
    undo_stack: Vec<(String, usize)>,
    redo_stack: Vec<(String, usize)>,
}

impl TextInput {
    /// 构造输入并预填文本，光标落在末尾。
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let cursor = text.chars().count();
        Self {
            text,
            cursor,
            selection_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// 是否存在可撤销的历史步。
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// 清空撤销与重做历史（用于切换目录等重置输入框初始状态的场景）。
    pub fn reset_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    /// 登记当前文本和光标到撤销栈（文本发生实质变动前调用）。
    fn push_undo(&mut self) {
        if self.undo_stack.last().map(|(t, _)| t.as_str()) != Some(&self.text) {
            if self.undo_stack.len() >= MAX_UNDO_STEPS {
                self.undo_stack.remove(0);
            }
            self.undo_stack.push((self.text.clone(), self.cursor));
            self.redo_stack.clear();
        }
    }

    /// 撤销上一次文本修改；成功返回 true。
    pub fn undo(&mut self) -> bool {
        let Some((prev_text, prev_cursor)) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push((self.text.clone(), self.cursor));
        self.text = prev_text;
        self.cursor = prev_cursor.min(self.text.chars().count());
        self.selection_anchor = None;
        true
    }

    /// 重做上一次撤销的文本修改；成功返回 true。
    pub fn redo(&mut self) -> bool {
        let Some((next_text, next_cursor)) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push((self.text.clone(), self.cursor));
        self.text = next_text;
        self.cursor = next_cursor.min(self.text.chars().count());
        self.selection_anchor = None;
        true
    }

    /// 当前文本。
    pub fn text(&self) -> &str {
        &self.text
    }

    /// 光标字符索引。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// 当前选区字符范围 `[start, end)`；无选区或空选区返回 None。
    pub fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.selection_anchor?;
        (anchor != self.cursor).then_some((anchor.min(self.cursor), anchor.max(self.cursor)))
    }

    /// 是否存在有效选区。
    pub fn has_selection(&self) -> bool {
        self.selection().is_some()
    }

    /// 获取当前选中的文本；无有效选区返回 None。
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        let start_byte = self.byte_offset(start);
        let end_byte = self.byte_offset(end);
        Some(self.text[start_byte..end_byte].to_string())
    }

    /// 清除选区。
    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    /// 直接设置选区字符区间。
    pub fn set_selection(&mut self, start: usize, end: usize) {
        let len = self.text.chars().count();
        let start = start.min(len);
        let end = end.min(len);
        self.selection_anchor = Some(start);
        self.cursor = end;
    }

    /// 在指定字符索引处开始选区。
    pub fn begin_selection(&mut self, char_index: usize) {
        let char_index = char_index.min(self.text.chars().count());
        self.cursor = char_index;
        self.selection_anchor = Some(char_index);
    }

    /// 按点击次数在指定字符处开始选择（1=落光标、2=选词、3=全选）。
    pub fn begin_selection_with_clicks(&mut self, char_index: usize, clicks: u8) {
        match clicks {
            2 => self.select_word_at(char_index),
            clicks if clicks >= 3 => self.select_all(),
            _ => self.begin_selection(char_index),
        }
    }

    /// 拖拽延伸选区到指定字符索引。
    pub fn drag_selection(&mut self, char_index: usize) {
        let char_index = char_index.min(self.text.chars().count());
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor);
        }
        self.cursor = char_index;
    }

    /// 全选当前文本。
    pub fn select_all(&mut self) {
        self.selection_anchor = Some(0);
        self.cursor = self.text.chars().count();
    }

    /// 选中指定字符所在单词。
    pub fn select_word_at(&mut self, char_index: usize) {
        let total = self.text.chars().count();
        if total == 0 {
            self.clear_selection();
            self.cursor = 0;
            return;
        }
        let char_index = char_index.min(total.saturating_sub(1));
        let chars: Vec<char> = self.text.chars().collect();
        let is_word_char = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
        let target_word = is_word_char(chars[char_index]);
        let mut start = char_index;
        while start > 0
            && is_word_char(chars[start - 1]) == target_word
            && !chars[start - 1].is_whitespace()
        {
            start -= 1;
        }
        let mut end = char_index + 1;
        while end < total && is_word_char(chars[end]) == target_word && !chars[end].is_whitespace()
        {
            end += 1;
        }
        self.selection_anchor = Some(start);
        self.cursor = end;
    }

    /// 删除选中的文本区间，并把光标重置到选区起点。有选区删除返回 true。
    pub fn delete_selected(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            self.selection_anchor = None;
            return false;
        };
        let start_byte = self.byte_offset(start);
        let end_byte = self.byte_offset(end);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor = start;
        self.selection_anchor = None;
        true
    }

    /// 剪切当前选区：有选区时记一步撤销、删除选区并返回被剪切文本；无选区返回 None。
    pub fn cut_selected(&mut self) -> Option<String> {
        let selected = self.selected_text()?;
        self.push_undo();
        self.delete_selected();
        Some(selected)
    }

    /// 在光标处插入字符（有选区时先替换选区）。
    pub fn insert_char(&mut self, ch: char) {
        self.push_undo();
        if let Some((start, end)) = self.selection() {
            let start_byte = self.byte_offset(start);
            let end_byte = self.byte_offset(end);
            self.text.replace_range(start_byte..end_byte, "");
            self.cursor = start;
            self.selection_anchor = None;
        }
        let at = self.byte_offset(self.cursor);
        self.text.insert(at, ch);
        self.cursor += 1;
    }

    /// 在光标处插入一段文本（单行语义：换行与制表符替换为空格；有选区时先替换）；
    /// 返回是否实际插入。
    pub fn insert_str(&mut self, value: &str) -> bool {
        let cleaned: String = value
            .chars()
            .map(|ch| {
                if matches!(ch, '\n' | '\r' | '\t') {
                    ' '
                } else {
                    ch
                }
            })
            .filter(|ch| !ch.is_control())
            .collect();
        if cleaned.is_empty() {
            return false;
        }
        self.push_undo();
        if let Some((start, end)) = self.selection() {
            let start_byte = self.byte_offset(start);
            let end_byte = self.byte_offset(end);
            self.text.replace_range(start_byte..end_byte, "");
            self.cursor = start;
            self.selection_anchor = None;
        }
        let at = self.byte_offset(self.cursor);
        self.text.insert_str(at, &cleaned);
        self.cursor += cleaned.chars().count();
        true
    }

    /// 清空文本并把光标移到开头，清除选区。
    pub fn clear(&mut self) {
        if self.text.is_empty() {
            self.selection_anchor = None;
            return;
        }
        self.push_undo();
        self.text.clear();
        self.cursor = 0;
        self.selection_anchor = None;
    }

    /// 删除光标前一个字符（有选区时删除整段选区）。
    pub fn backspace(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selected();
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let start = self.byte_offset(self.cursor - 1);
        let end = self.byte_offset(self.cursor);
        self.text.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// 删除光标处字符（有选区时删除整段选区）。
    pub fn delete(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selected();
            return;
        }
        if self.cursor >= self.text.chars().count() {
            return;
        }
        self.push_undo();
        let start = self.byte_offset(self.cursor);
        let end = self.byte_offset(self.cursor + 1);
        self.text.replace_range(start..end, "");
    }

    /// 删除光标到行首的内容（Ctrl+U；有选区时删除选区）。
    pub fn delete_to_line_start(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selected();
            return;
        }
        if self.cursor == 0 {
            return;
        }
        self.push_undo();
        let end = self.byte_offset(self.cursor);
        self.text.replace_range(0..end, "");
        self.cursor = 0;
    }

    /// 删除光标到行尾的内容（Ctrl+K；有选区时删除选区）。
    pub fn delete_to_line_end(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selected();
            return;
        }
        let total = self.text.chars().count();
        if self.cursor >= total {
            return;
        }
        self.push_undo();
        let start = self.byte_offset(self.cursor);
        self.text.replace_range(start.., "");
    }

    /// 光标左移一个词（清除选区）。
    pub fn move_word_left(&mut self) {
        self.clear_selection();
        let byte = self.byte_offset(self.cursor);
        let target = word_start_before(&self.text, byte);
        self.cursor = self.text[..target].chars().count();
    }

    /// 光标右移一个词（清除选区）。
    pub fn move_word_right(&mut self) {
        self.clear_selection();
        let byte = self.byte_offset(self.cursor);
        let target = word_end_after(&self.text, byte);
        self.cursor = self.text[..target].chars().count();
    }

    /// 向前删除一个词（Ctrl+W / Alt+Backspace；有选区时删除选区）。
    pub fn delete_word_backward(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selected();
            return;
        }
        let byte = self.byte_offset(self.cursor);
        let start = word_start_before(&self.text, byte);
        if start < byte {
            self.push_undo();
            self.text.replace_range(start..byte, "");
            self.cursor = self.text[..start].chars().count();
        }
    }

    /// 向后删除一个词（Alt+D / Ctrl+Delete；有选区时删除选区，光标保持不动）。
    pub fn delete_word_forward(&mut self) {
        if self.has_selection() {
            self.push_undo();
            self.delete_selected();
            return;
        }
        let byte = self.byte_offset(self.cursor);
        let end = word_end_after(&self.text, byte);
        if byte < end {
            self.push_undo();
            self.text.replace_range(byte..end, "");
        }
    }

    /// 光标左移（有选区时折叠到选区开头）。
    pub fn move_left(&mut self) {
        if let Some((start, _)) = self.selection() {
            self.cursor = start;
            self.selection_anchor = None;
        } else {
            self.cursor = self.cursor.saturating_sub(1);
        }
    }

    /// 光标右移（有选区时折叠到选区结尾）。
    pub fn move_right(&mut self) {
        if let Some((_, end)) = self.selection() {
            self.cursor = end;
            self.selection_anchor = None;
        } else if self.cursor < self.text.chars().count() {
            self.cursor += 1;
        }
    }

    /// 光标移到开头（清除选区）。
    pub fn move_home(&mut self) {
        self.clear_selection();
        self.cursor = 0;
    }

    /// 光标移到末尾（清除选区）。
    pub fn move_end(&mut self) {
        self.clear_selection();
        self.cursor = self.text.chars().count();
    }

    /// 在给定折行宽度下将光标上移一个视觉行（首行再上移则落到行首）。
    pub fn move_up_wrapped(&mut self, width: usize) {
        self.move_up_wrapped_indented(width, width);
    }

    /// 在给定首行宽度 `first_width` 与续行宽度 `cont_width` 下将光标上移一个视觉行（首行再上移则落到行首）。
    pub fn move_up_wrapped_indented(&mut self, first_width: usize, cont_width: usize) {
        self.clear_selection();
        let lines = self.wrapped_ranges(first_width, cont_width);
        let (row, col) = self.cursor_wrapped_pos(&lines);
        if row == 0 {
            self.cursor = lines.first().map_or(0, |line| line.start_char);
            return;
        }
        let target = lines[row - 1];
        self.cursor = self.char_at_col(target, col);
    }

    /// 在给定折行宽度下将光标下移一个视觉行（末行再下移则落到行尾）。
    pub fn move_down_wrapped(&mut self, width: usize) {
        self.move_down_wrapped_indented(width, width);
    }

    /// 在给定首行宽度 `first_width` 与续行宽度 `cont_width` 下将光标下移一个视觉行（末行再下移则落到行尾）。
    pub fn move_down_wrapped_indented(&mut self, first_width: usize, cont_width: usize) {
        self.clear_selection();
        let lines = self.wrapped_ranges(first_width, cont_width);
        let (row, col) = self.cursor_wrapped_pos(&lines);
        let Some(target) = lines.get(row + 1).copied() else {
            self.cursor = lines
                .last()
                .map_or(self.text.chars().count(), |line| line.end_char);
            return;
        };
        self.cursor = self.char_at_col(target, col);
    }

    /// 统一处理编辑类浮层按键；返回是否消费了该按键。
    pub fn apply_edit_key(&mut self, key: OverlayKey) -> bool {
        match key {
            OverlayKey::Char(ch) => self.insert_char(ch),
            OverlayKey::Clear => self.clear(),
            OverlayKey::Cut => {
                self.cut_selected();
            }
            OverlayKey::Backspace => self.backspace(),
            OverlayKey::Delete => self.delete(),
            OverlayKey::Left => self.move_left(),
            OverlayKey::Right => self.move_right(),
            OverlayKey::Home | OverlayKey::LineStart => self.move_home(),
            OverlayKey::SelectAll => self.select_all(),
            OverlayKey::End | OverlayKey::LineEnd => self.move_end(),
            OverlayKey::WordLeft => self.move_word_left(),
            OverlayKey::WordRight => self.move_word_right(),
            OverlayKey::DeleteToLineStart => self.delete_to_line_start(),
            OverlayKey::DeleteToLineEnd => self.delete_to_line_end(),
            OverlayKey::DeleteWordBackward => self.delete_word_backward(),
            OverlayKey::DeleteWordForward => self.delete_word_forward(),
            OverlayKey::Undo => {
                self.undo();
            }
            OverlayKey::Redo => {
                self.redo();
            }
            _ => return false,
        }
        true
    }

    /// 字符索引到字节偏移；越界返回文本长度。
    fn byte_offset(&self, char_index: usize) -> usize {
        self.text
            .char_indices()
            .nth(char_index)
            .map(|(byte, _)| byte)
            .unwrap_or(self.text.len())
    }

    /// 按显示宽度折行得到的视觉行字符区间（与渲染层 `widgets::input` 同一算法）。
    fn wrapped_ranges(&self, first_width: usize, cont_width: usize) -> Vec<WrappedRange> {
        let first_width = first_width.max(1);
        let cont_width = cont_width.max(1);
        let line_limit = |idx: usize| if idx == 0 { first_width } else { cont_width };
        let mut lines = Vec::new();
        let mut start_char = 0usize;
        let mut char_index = 0usize;
        let mut col = 0usize;
        for ch in self.text.chars() {
            if ch == '\n' {
                let limit = line_limit(lines.len());
                lines.push(WrappedRange {
                    start_char,
                    end_char: char_index,
                });
                if limit > 1 && col == limit {
                    lines.push(WrappedRange {
                        start_char: char_index,
                        end_char: char_index,
                    });
                }
                start_char = char_index + 1;
                char_index += 1;
                col = 0;
                continue;
            }
            let cells = char_cells(ch);
            let limit = line_limit(lines.len());
            if col + cells > limit && col > 0 {
                lines.push(WrappedRange {
                    start_char,
                    end_char: char_index,
                });
                start_char = char_index;
                col = 0;
            }
            col += cells;
            char_index += 1;
        }
        let limit = line_limit(lines.len());
        lines.push(WrappedRange {
            start_char,
            end_char: char_index,
        });
        if limit > 1 && col == limit {
            lines.push(WrappedRange {
                start_char: char_index,
                end_char: char_index,
            });
        }
        lines
    }

    /// 光标所在视觉行与行内显示列；软折行边界归下一行。
    fn cursor_wrapped_pos(&self, lines: &[WrappedRange]) -> (usize, usize) {
        let mut row = lines.len().saturating_sub(1);
        for (index, line) in lines.iter().enumerate() {
            let at_end = self.cursor == line.end_char;
            let hard_break = lines
                .get(index + 1)
                .is_none_or(|next| next.start_char > line.end_char);
            if self.cursor < line.end_char || (at_end && hard_break) {
                row = index;
                break;
            }
        }
        let line = lines[row];
        let offset = self
            .cursor
            .min(line.end_char)
            .saturating_sub(line.start_char);
        let col = self
            .text
            .chars()
            .skip(line.start_char)
            .take(offset)
            .map(char_cells)
            .sum();
        (row, col)
    }

    /// 视觉行内显示列对应的全文字符索引。
    fn char_at_col(&self, line: WrappedRange, col: usize) -> usize {
        let start = self.byte_offset(line.start_char);
        let end = self.byte_offset(line.end_char);
        let slice = &self.text[start..end];
        let offset = offset_at_col(slice, col);
        line.start_char + slice[..offset].chars().count()
    }
}

/// 视觉行的字符区间。
#[derive(Debug, Clone, Copy)]
struct WrappedRange {
    start_char: usize,
    end_char: usize,
}

/// 字符显示列数；控制字符按 0 列。
fn char_cells(ch: char) -> usize {
    ch.width().unwrap_or(0)
}

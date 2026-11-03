//! `Lx` 输入框的文本编辑、光标移动与撤销/重做（含折叠粘贴 token 的原子边界处理）。

use crate::app::paste::{
    clamp_cursor, folded_tokens, mask_pasted_text, next_pasted_boundary, prev_pasted_boundary,
    snap_pasted_range, token_ending_at, token_starting_at,
};
use crate::app::prompt::{list, text};

use super::folded::{clamp_line_end, clamp_line_start};
use super::{LX_HISTORY_LIMIT, Lx, LxInputMode, LxSnapshot};

impl Lx {
    /// 记录一次编辑前的快照，并清空重做栈。
    pub(super) fn snapshot(&mut self) {
        self.text_selection = None;
        self.preview_selection = None;
        self.preview_drag = None;
        if self.undo.len() == LX_HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(LxSnapshot {
            text: self.input.clone(),
            cursor: self.cursor,
            mode: self.input_mode,
        });
        self.redo.clear();
        self.touch();
    }

    /// 在光标处插入一个字符（有选区时替换选区）。
    pub fn insert_char(&mut self, ch: char) {
        self.refer_suppressed = false;
        let selection = self
            .text_selection_range()
            .map(|(s, e)| snap_pasted_range(&self.input, s..e));
        self.snapshot();
        if let Some(range) = selection {
            self.input.replace_range(range.clone(), "");
            self.cursor = range.start;
        } else {
            self.cursor = clamp_cursor(&self.input, self.cursor);
        }
        self.input.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
        self.adopt_shell_prefix();
        self.refresh_panel();
    }

    /// 在光标处插入一段文本；净化规则同 prompt 编辑器（换行归一、Tab 展开、丢弃控制字符）。
    ///
    /// 整段粘贴作为一步撤销。粘贴不是“正在输入”：不刷新 md 块面板，避免粘贴文本
    /// 尾部的触发标记紧接着弹出面板、被下一次 Enter 误采纳（对齐 prompt 的粘贴语义）。
    pub fn insert_str(&mut self, text: &str) {
        let sanitized = text::sanitize(text);
        if sanitized.is_empty() {
            return;
        }
        let selection = self
            .text_selection_range()
            .map(|(s, e)| snap_pasted_range(&self.input, s..e));
        self.snapshot();
        if let Some(range) = selection {
            self.input.replace_range(range.clone(), "");
            self.cursor = range.start;
        } else {
            self.cursor = clamp_cursor(&self.input, self.cursor);
        }
        self.input.insert_str(self.cursor, &sanitized);
        self.cursor += sanitized.len();
        self.adopt_shell_prefix();
        if self.input_mode == LxInputMode::Chat {
            self.panel = None;
            self.mention.suppress();
        }
        self.scroll_cursor_into_view();
    }

    /// 在光标处换行；列表项自动续写标记（对齐 prompt 编辑器）。
    pub fn newline(&mut self) {
        // 有选区时先替换整段选区，再插入换行（一步撤销）。
        if let Some((start, end)) = self.text_selection_range() {
            let range = snap_pasted_range(&self.input, start..end);
            self.snapshot();
            self.input.replace_range(range.clone(), "\n");
            self.cursor = range.start + 1;
            if self.plain || self.input_mode == LxInputMode::Shell {
                self.scroll_cursor_into_view();
            } else {
                self.refresh_panel();
            }
            return;
        }
        self.cursor = clamp_cursor(&self.input, self.cursor);
        // 纯文本缓冲：换行就是换行，不续写列表标记、不刷新面板。
        if self.plain {
            self.snapshot();
            self.input.insert(self.cursor, '\n');
            self.cursor += 1;
            self.scroll_cursor_into_view();
            return;
        }
        if self.input_mode == LxInputMode::Shell {
            self.snapshot();
            self.input.insert(self.cursor, '\n');
            self.cursor += 1;
            self.scroll_cursor_into_view();
            return;
        }
        let masked = mask_pasted_text(&self.input);
        let cont = (!list::in_fence(&masked, self.cursor))
            .then(|| list::list_continuation(&masked, self.cursor))
            .flatten();
        drop(masked);
        if let Some(edit) = cont {
            self.snapshot();
            self.input.replace_range(edit.range, &edit.replacement);
            self.cursor = edit.cursor;
            self.refresh_panel();
            return;
        }
        self.snapshot();
        self.input.insert(self.cursor, '\n');
        self.cursor += 1;
        self.refresh_panel();
    }

    /// 删除光标前一个字符（遇到折叠 token 时整块原子删除）。
    pub fn backspace(&mut self) {
        self.refer_suppressed = false;
        if self.delete_selected() {
            return;
        }
        if self.input_mode == LxInputMode::Shell && self.cursor == 0 {
            self.snapshot();
            self.input_mode = LxInputMode::Chat;
            self.refresh_panel();
            return;
        }
        if self.cursor == 0 {
            return;
        }
        if let Some(tok) = token_ending_at(&self.input, self.cursor) {
            self.snapshot();
            self.cursor = tok.range.start;
            self.input.replace_range(tok.range, "");
            self.refresh_panel();
            return;
        }
        if !self.mention.is_open()
            && self.refer.is_none()
            && let Some(range) =
                crate::app::markdown::ref_content_deletion_range(&self.input, self.cursor)
                    .or_else(|| {
                        crate::app::markdown::mention_deletion_range(&self.input, self.cursor)
                    })
                    .or_else(|| {
                        crate::app::markdown::reference_deletion_range(&self.input, self.cursor)
                    })
                    .or_else(|| {
                        crate::app::markdown::short_ref_deletion_range(&self.input, self.cursor)
                    })
        {
            self.snapshot();
            self.cursor = range.start;
            self.input.replace_range(range, "");
            self.refresh_panel();
            return;
        }
        if let Some(prev) = self.input[..self.cursor].chars().next_back() {
            self.snapshot();
            self.cursor -= prev.len_utf8();
            self.input.remove(self.cursor);
            self.refresh_panel();
        }
    }

    /// 删除光标处字符（遇到折叠 token 时整块原子删除）。
    pub fn delete(&mut self) {
        self.refer_suppressed = false;
        if self.delete_selected() {
            return;
        }
        if self.cursor >= self.input.len() {
            return;
        }
        if let Some(tok) = token_starting_at(&self.input, self.cursor) {
            self.snapshot();
            self.cursor = tok.range.start;
            self.input.replace_range(tok.range, "");
            self.refresh_panel();
            return;
        }
        self.snapshot();
        self.input.remove(self.cursor);
        self.refresh_panel();
    }

    /// 光标左移一个字符（遇到折叠 token 时整块跨过）；有选区时折叠到选区开头。
    pub fn move_left(&mut self) {
        self.refer_suppressed = false;
        if let Some((start, _)) = self.text_selection_range() {
            self.cursor = clamp_cursor(&self.input, start);
            self.refresh_panel();
            return;
        }
        let next = prev_pasted_boundary(&self.input, self.cursor);
        if next != self.cursor {
            self.cursor = next;
            self.refresh_panel();
        }
    }

    /// 光标右移一个字符（遇到折叠 token 时整块跨过）；有选区时折叠到选区结尾。
    pub fn move_right(&mut self) {
        self.refer_suppressed = false;
        if let Some((_, end)) = self.text_selection_range() {
            self.cursor = clamp_cursor(&self.input, end);
            self.refresh_panel();
            return;
        }
        let next = next_pasted_boundary(&self.input, self.cursor);
        if next != self.cursor {
            self.cursor = next;
            self.refresh_panel();
        }
    }

    /// 光标移到当前视觉行行首。
    pub fn home(&mut self) {
        self.refer_suppressed = false;
        let rows = self.visual_rows();
        let proj = self.projection();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = proj.display_to_raw_directed(rows[index].start, false);
        self.refresh_panel();
    }

    /// 光标移到当前视觉行行尾。
    pub fn end(&mut self) {
        self.refer_suppressed = false;
        let rows = self.visual_rows();
        let proj = self.projection();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = proj.display_to_raw_directed(rows[index].end, true);
        self.refresh_panel();
    }

    /// 光标上移一个视觉行（保持显示列，越界钳到行尾；展开芯片内部行跳到芯片起点）。
    pub fn move_up(&mut self) {
        self.refer_suppressed = false;
        let rows = self.visual_rows();
        let proj = self.projection();
        let (index, col) = self.cursor_visual_in(&rows);
        if index == 0 {
            self.cursor = proj.display_to_raw_directed(rows[0].start, false);
        } else {
            let target = rows[index - 1];
            let disp =
                target.start + text::offset_at_col(&proj.text()[target.start..target.end], col);
            self.cursor = proj.display_to_raw_directed(disp, false);
        }
        self.refresh_panel();
    }

    /// 光标下移一个视觉行（保持显示列，越界钳到行尾；展开芯片内部行跳到芯片终点）。
    pub fn move_down(&mut self) {
        self.refer_suppressed = false;
        let rows = self.visual_rows();
        let proj = self.projection();
        let (index, col) = self.cursor_visual_in(&rows);
        if index + 1 >= rows.len() {
            self.cursor = proj.display_to_raw_directed(rows[rows.len() - 1].end, true);
        } else {
            let target = rows[index + 1];
            let disp =
                target.start + text::offset_at_col(&proj.text()[target.start..target.end], col);
            self.cursor = proj.display_to_raw_directed(disp, true);
        }
        self.refresh_panel();
    }

    /// 光标左移一个词；有选区时折叠到选区开头。
    pub fn word_left(&mut self) {
        self.refer_suppressed = false;
        if let Some((start, _)) = self.text_selection_range() {
            self.cursor = clamp_cursor(&self.input, start);
            self.refresh_panel();
            return;
        }
        let target = text::word_start_before(&self.input, self.cursor);
        self.cursor = clamp_cursor(&self.input, target);
        self.refresh_panel();
    }

    /// 光标右移一个词；有选区时折叠到选区结尾。
    pub fn word_right(&mut self) {
        self.refer_suppressed = false;
        if let Some((_, end)) = self.text_selection_range() {
            self.cursor = clamp_cursor(&self.input, end);
            self.refresh_panel();
            return;
        }
        let mut target = text::word_end_after(&self.input, self.cursor);
        for tok in folded_tokens(&self.input) {
            if tok.range.start < target && target < tok.range.end {
                target = tok.range.end;
                break;
            }
        }
        self.cursor = target;
        self.refresh_panel();
    }

    /// 删除光标到逻辑行首；光标已在行首时删除前一个换行（与上一行合并）。
    pub fn delete_to_line_start(&mut self) {
        self.refer_suppressed = false;
        if self.delete_selected() {
            return;
        }
        if self.cursor == 0 {
            return;
        }
        let start = clamp_line_start(&self.input, self.cursor);
        self.snapshot();
        if start < self.cursor {
            let range = snap_pasted_range(&self.input, start..self.cursor);
            self.input.replace_range(range.clone(), "");
            self.cursor = range.start;
        } else {
            let previous = prev_pasted_boundary(&self.input, self.cursor);
            let range = snap_pasted_range(&self.input, previous..self.cursor);
            self.input.replace_range(range.clone(), "");
            self.cursor = range.start;
        }
        self.refresh_panel();
    }

    /// 删除光标到逻辑行尾。
    pub fn delete_to_line_end(&mut self) {
        self.refer_suppressed = false;
        if self.delete_selected() {
            return;
        }
        let end = clamp_line_end(&self.input, self.cursor);
        if self.cursor < end {
            let range = snap_pasted_range(&self.input, self.cursor..end);
            self.snapshot();
            self.input.replace_range(range.clone(), "");
            self.cursor = range.start;
            self.refresh_panel();
        }
    }

    /// 向前删除一个词。
    pub fn delete_word_backward(&mut self) {
        self.refer_suppressed = false;
        if self.delete_selected() {
            return;
        }
        let start = text::word_start_before(&self.input, self.cursor);
        if start < self.cursor {
            let range = snap_pasted_range(&self.input, start..self.cursor);
            self.snapshot();
            self.input.replace_range(range.clone(), "");
            self.cursor = range.start;
            self.refresh_panel();
        }
    }

    /// 向后删除一个词（光标保持在删除起点，readline M-d 语义）。
    pub fn delete_word_forward(&mut self) {
        self.refer_suppressed = false;
        if self.delete_selected() {
            return;
        }
        let end = text::word_end_after(&self.input, self.cursor);
        if self.cursor < end {
            let range = snap_pasted_range(&self.input, self.cursor..end);
            self.snapshot();
            self.input.replace_range(range.clone(), "");
            self.cursor = range.start;
            self.refresh_panel();
        }
    }

    /// 清空输入框内容；空 Shell 模式也可借此退出。
    pub fn clear(&mut self) {
        self.refer_suppressed = false;
        self.text_paste = None;
        self.clear_expanded_pastes();
        if self.input.is_empty() && self.input_mode == LxInputMode::Chat {
            return;
        }
        self.snapshot();
        self.input.clear();
        self.cursor = 0;
        self.input_mode = LxInputMode::Chat;
        self.refresh_panel();
    }

    /// 撤销一步（恢复正文、光标与模式；修订号继续前进）。
    pub fn undo(&mut self) {
        if self.paste_panel().is_some() || self.text_paste_panel().is_some() {
            return;
        }
        self.refer_suppressed = false;
        if let Some(snapshot) = self.undo.pop() {
            let current = LxSnapshot {
                text: std::mem::replace(&mut self.input, snapshot.text),
                cursor: self.cursor,
                mode: self.input_mode,
            };
            self.redo.push(current);
            self.cursor = snapshot.cursor.min(self.input.len());
            self.input_mode = snapshot.mode;
            self.clear_expanded_pastes();
            self.touch();
            self.refresh_panel();
        }
    }

    /// 重做一步（恢复正文、光标与模式；修订号继续前进）。
    pub fn redo(&mut self) {
        if self.paste_panel().is_some() || self.text_paste_panel().is_some() {
            return;
        }
        self.refer_suppressed = false;
        if let Some(snapshot) = self.redo.pop() {
            let current = LxSnapshot {
                text: std::mem::replace(&mut self.input, snapshot.text),
                cursor: self.cursor,
                mode: self.input_mode,
            };
            self.undo.push(current);
            self.cursor = snapshot.cursor.min(self.input.len());
            self.input_mode = snapshot.mode;
            self.clear_expanded_pastes();
            self.touch();
            self.refresh_panel();
        }
    }
}

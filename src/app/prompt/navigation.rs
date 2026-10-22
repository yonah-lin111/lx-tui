//! prompt 光标移动与光标相对删除：上下左右、行内、按词与折叠区吸附。
//!
//! 纯状态更新；折叠补充块的隐藏行不进入视觉行序，字符/按词移动越过整块。

use super::text::*;
use super::{EditKind, Prompt};

impl Prompt {
    /// 光标左移一个字符。
    pub fn move_left(&mut self) {
        self.break_group();
        let at = prev_boundary(&self.text, self.cursor);
        self.cursor = self.snap_visible(at, false);
        self.settle();
    }

    /// 光标右移一个字符。
    pub fn move_right(&mut self) {
        self.break_group();
        let at = next_boundary(&self.text, self.cursor);
        self.cursor = self.snap_visible(at, true);
        self.settle();
    }

    /// 光标上移一个视觉行，保持显示列；首行时钳到行首。
    pub fn move_up(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        let (index, col) = self.cursor_visual_in(&rows);
        if index == 0 {
            self.cursor = rows[0].start;
        } else {
            let target = rows[index - 1];
            self.cursor = target.start + offset_at_col(&self.text[target.start..target.end], col);
        }
        self.settle();
    }

    /// 光标下移一个视觉行，保持显示列；末行时钳到行尾。
    pub fn move_down(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        let (index, col) = self.cursor_visual_in(&rows);
        if index + 1 >= rows.len() {
            self.cursor = rows[rows.len() - 1].end;
        } else {
            let target = rows[index + 1];
            self.cursor = target.start + offset_at_col(&self.text[target.start..target.end], col);
        }
        self.settle();
    }

    /// 光标移到当前视觉行行首。
    pub fn move_home(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = rows[index].start;
        self.settle();
    }

    /// 光标移到当前视觉行行尾。
    pub fn move_end(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = rows[index].end;
        self.settle();
    }

    /// 光标移到逻辑行行首。
    pub fn move_line_start(&mut self) {
        self.break_group();
        self.cursor = line_start(&self.text, self.cursor);
        self.settle();
    }

    /// 光标移到逻辑行行尾（换行前）。
    pub fn move_line_end(&mut self) {
        self.break_group();
        self.cursor = line_end(&self.text, self.cursor);
        self.settle();
    }

    /// 光标左移到前一个词的词首。
    pub fn move_word_backward(&mut self) {
        self.break_group();
        let at = self.word_start_before(self.cursor);
        self.cursor = self.snap_visible(at, false);
        self.settle();
    }

    /// 光标右移到下一个词的词尾（readline M-f 语义）。
    pub fn move_word_forward(&mut self) {
        self.break_group();
        let at = self.word_end_after(self.cursor);
        self.cursor = self.snap_visible(at, true);
        self.settle();
    }

    /// 删除光标前一个词及其后分隔符（readline ctrl+w 语义）。
    pub fn delete_word_backward(&mut self) {
        let start = self.word_start_before(self.cursor);
        if start < self.cursor && !self.protected_edit(start, self.cursor) {
            self.mark_dirty();
            self.splice_text(start, self.cursor, "");
            self.cursor = start;
            self.settle();
        }
    }

    /// 删除光标到下一个词尾（readline M-d 语义）。
    pub fn delete_word_forward(&mut self) {
        let end = self.word_end_after(self.cursor);
        if end > self.cursor && !self.protected_edit(self.cursor, end) {
            self.mark_dirty();
            self.splice_text(self.cursor, end, "");
            self.settle();
        }
    }

    /// 删除光标到逻辑行首；光标已在行首时删除前一个换行（对齐 opencode deleteToLineStart）。
    pub fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = line_start(&self.text, self.cursor);
        if start < self.cursor {
            if self.protected_edit(start, self.cursor) {
                return;
            }
            self.record(EditKind::Delete);
            self.splice_text(start, self.cursor, "");
            self.cursor = start;
            self.settle();
            return;
        }
        if self.protected_edit(self.cursor - 1, self.cursor) {
            return;
        }
        self.record(EditKind::Delete);
        self.splice_text(self.cursor - 1, self.cursor, "");
        self.cursor -= 1;
        self.settle();
    }

    /// 删除光标到逻辑行尾（不删除换行）。
    pub fn delete_to_line_end(&mut self) {
        let end = line_end(&self.text, self.cursor);
        if end > self.cursor && !self.protected_edit(self.cursor, end) {
            self.mark_dirty();
            self.splice_text(self.cursor, end, "");
            self.settle();
        }
    }
}

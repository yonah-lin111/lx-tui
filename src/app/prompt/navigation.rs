//! prompt 光标移动与光标相对删除：上下左右、行内与按词。

use crate::app::paste::{
    clamp_cursor, folded_tokens, next_pasted_boundary, prev_pasted_boundary, snap_pasted_range,
};

use super::folded::{clamp_line_end, clamp_line_start};
use super::text::*;
use super::{EditKind, INDENT_UNIT, Prompt, TAB_STOP};

impl Prompt {
    /// 光标左移一个字符（遇到折叠 token 时整块跨过）。
    pub fn move_left(&mut self) {
        self.break_group();
        self.cursor = prev_pasted_boundary(&self.text, self.cursor);
        self.settle();
    }

    /// 光标右移一个字符（遇到折叠 token 时整块跨过）。
    pub fn move_right(&mut self) {
        self.break_group();
        self.cursor = next_pasted_boundary(&self.text, self.cursor);
        self.settle();
    }

    /// 光标上移一个视觉行，保持显示列；首行时钳到行首。
    pub fn move_up(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        if rows.is_empty() {
            self.settle();
            return;
        }
        let proj = self.projection();
        let (index, col) = self.cursor_visual_in(&rows);
        if let Some(prev) = (0..index).rev().find(|&i| !rows[i].top_border) {
            let target = rows[prev];
            let disp = target.start + offset_at_col(&proj.text()[target.start..target.end], col);
            self.cursor = proj.display_to_raw_directed(disp, false);
        } else {
            self.cursor = proj.display_to_raw_directed(rows[index].start, false);
        }
        self.settle();
    }

    /// 光标下移一个视觉行，保持显示列；末行时钳到行尾。
    pub fn move_down(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        if rows.is_empty() {
            self.settle();
            return;
        }
        let proj = self.projection();
        let (index, col) = self.cursor_visual_in(&rows);
        if let Some(next) = ((index + 1)..rows.len()).find(|&i| !rows[i].top_border) {
            let target = rows[next];
            let disp = target.start + offset_at_col(&proj.text()[target.start..target.end], col);
            self.cursor = proj.display_to_raw_directed(disp, true);
        } else {
            self.cursor = proj.display_to_raw_directed(rows[index].end, true);
        }
        self.settle();
    }

    /// 光标移到当前视觉行行首。
    pub fn move_home(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        if rows.is_empty() {
            self.settle();
            return;
        }
        let proj = self.projection();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = proj.display_to_raw_directed(rows[index].start, false);
        self.settle();
    }

    /// 光标移到当前视觉行行尾。
    pub fn move_end(&mut self) {
        self.break_group();
        let rows = self.visual_rows();
        if rows.is_empty() {
            self.settle();
            return;
        }
        let proj = self.projection();
        let (index, _) = self.cursor_visual_in(&rows);
        self.cursor = proj.display_to_raw_directed(rows[index].end, true);
        self.settle();
    }

    /// 光标移到逻辑行行首。
    pub fn move_line_start(&mut self) {
        self.break_group();
        self.cursor = clamp_line_start(&self.text, self.cursor);
        self.settle();
    }

    /// 光标移到逻辑行行尾（换行前）。
    pub fn move_line_end(&mut self) {
        self.break_group();
        self.cursor = clamp_line_end(&self.text, self.cursor);
        self.settle();
    }

    /// 光标左移到前一个词的词首（遇到折叠 token 时跳到 token 起点）。
    pub fn move_word_backward(&mut self) {
        self.break_group();
        let target = self.word_start_before(self.cursor);
        self.cursor = clamp_cursor(&self.text, target);
        self.settle();
    }

    /// 光标右移到下一个词的词尾（遇到折叠 token 时跳到 token 终点）。
    pub fn move_word_forward(&mut self) {
        self.break_group();
        let mut target = self.word_end_after(self.cursor);
        for tok in folded_tokens(&self.text) {
            if tok.range.start < target && target < tok.range.end {
                target = tok.range.end;
                break;
            }
        }
        self.cursor = target;
        self.settle();
    }

    /// 删除光标前一个词及其后分隔符（readline ctrl+w 语义）。
    pub fn delete_word_backward(&mut self) {
        let raw_start = self.word_start_before(self.cursor);
        let range = snap_pasted_range(&self.text, raw_start..self.cursor);
        if range.start < range.end && !self.protected_edit(range.start, range.end) {
            self.mark_dirty();
            self.splice_text(range.start, range.end, "");
            self.cursor = range.start;
            self.settle();
        }
    }

    /// 删除光标到下一个词尾（readline M-d 语义）。
    pub fn delete_word_forward(&mut self) {
        let raw_end = self.word_end_after(self.cursor);
        let range = snap_pasted_range(&self.text, self.cursor..raw_end);
        if range.start < range.end && !self.protected_edit(range.start, range.end) {
            self.mark_dirty();
            self.splice_text(range.start, range.end, "");
            self.cursor = range.start;
            self.settle();
        }
    }

    /// 删除光标到逻辑行首；光标已在行首时删除前一个换行（对齐 opencode deleteToLineStart）。
    pub fn delete_to_line_start(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start = clamp_line_start(&self.text, self.cursor);
        if start < self.cursor {
            let range = snap_pasted_range(&self.text, start..self.cursor);
            if self.protected_edit(range.start, range.end) {
                return;
            }
            self.record(EditKind::Delete);
            self.splice_text(range.start, range.end, "");
            self.cursor = range.start;
            self.settle();
            return;
        }
        let range = snap_pasted_range(&self.text, (self.cursor - 1)..self.cursor);
        if self.protected_edit(range.start, range.end) {
            return;
        }
        self.record(EditKind::Delete);
        self.splice_text(range.start, range.end, "");
        self.cursor = range.start;
        self.settle();
    }

    /// 删除光标到逻辑行尾（不删除换行）。
    pub fn delete_to_line_end(&mut self) {
        let end = clamp_line_end(&self.text, self.cursor);
        if end > self.cursor {
            let range = snap_pasted_range(&self.text, self.cursor..end);
            if !self.protected_edit(range.start, range.end) {
                self.mark_dirty();
                self.splice_text(range.start, range.end, "");
                self.cursor = range.start;
                self.settle();
            }
        }
    }

    /// 当前逻辑行整体右移一个缩进单位；围栏代码块内改为光标处插入空格。
    pub fn indent(&mut self) {
        if self.in_fence() {
            self.insert_at(TAB_STOP);
            return;
        }
        let start = clamp_line_start(&self.text, self.cursor);
        if self.protected_edit(start, start) {
            return;
        }
        self.record(EditKind::Other);
        self.splice_text(start, start, TAB_STOP);
        self.cursor += INDENT_UNIT;
        self.settle();
    }

    /// 当前逻辑行整体左移一个缩进单位；行首无空格时 no-op。
    pub fn outdent(&mut self) {
        let start = clamp_line_start(&self.text, self.cursor);
        let spaces = self.text[start..]
            .bytes()
            .take_while(|byte| *byte == b' ')
            .count();
        let remove = spaces.min(INDENT_UNIT);
        if remove == 0 || self.protected_edit(start, start + remove) {
            return;
        }
        self.record(EditKind::Other);
        let offset = self.cursor - start;
        self.splice_text(start, start + remove, "");
        self.cursor = start + offset.saturating_sub(remove);
        self.settle();
    }
}

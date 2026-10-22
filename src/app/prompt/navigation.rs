//! prompt 导航与隐藏区跳过：显示投影上的上下移动、行首尾、按词移动与入口焦点钳制。

use super::presentation::PromptDisplayRow;
use super::text::{line_end, line_start, next_boundary, offset_at_col, prev_boundary};
use super::{Prompt, PromptFocus};

impl Prompt {
    /// 光标左移一个字符；正向/反向都跳过集中视图隐藏的附件区间。
    ///
    /// 附件入口焦点时回到入口之前的可见文本末尾（文首钳制）。
    pub fn move_left(&mut self) {
        self.break_group();
        if let PromptFocus::Supplements { parent_start } = self.focus {
            self.focus = PromptFocus::Text;
            self.cursor =
                self.snap_visible(self.hidden_before_offset(parent_start).unwrap_or(0), false);
            self.settle();
            return;
        }
        self.cursor = self.hidden_adjusted_boundary(prev_boundary(&self.text, self.cursor), false);
        self.settle();
    }

    /// 光标右移一个字符；正向跳过隐藏附件区间，直接落在其后首个可见字符边界。
    pub fn move_right(&mut self) {
        self.break_group();
        if self.focus.is_supplements() {
            self.focus = PromptFocus::Text;
            self.cursor = self.snap_visible(
                self.hidden_after_offset(self.focus_parent())
                    .unwrap_or(self.text.len()),
                true,
            );
            self.settle();
            return;
        }
        self.cursor = self.hidden_adjusted_boundary(next_boundary(&self.text, self.cursor), true);
        self.settle();
    }

    /// 光标上移一个显示行，保持显示列；遇合成附件入口进入入口焦点。
    pub fn move_up(&mut self) {
        self.break_group();
        let display = self.display_rows();
        let (index, col) = self.display_cursor_in(&display);
        if index == 0 {
            if let Some(PromptDisplayRow::Text { start, .. }) = display.first().copied() {
                self.focus = PromptFocus::Text;
                self.cursor = start;
            }
        } else {
            self.move_to_display_row(&display, index - 1, col);
        }
        self.settle();
    }

    /// 光标下移一个显示行，保持显示列；遇合成附件入口进入入口焦点。
    pub fn move_down(&mut self) {
        self.break_group();
        let display = self.display_rows();
        let (index, col) = self.display_cursor_in(&display);
        if index + 1 >= display.len() {
            if let Some(PromptDisplayRow::Text { end, .. }) = display.last().copied() {
                self.focus = PromptFocus::Text;
                self.cursor = end;
            }
        } else {
            self.move_to_display_row(&display, index + 1, col);
        }
        self.settle();
    }

    /// 光标移到当前显示行行首；附件入口焦点移到入口之前的可见文本末尾。
    pub fn move_home(&mut self) {
        self.break_group();
        if let PromptFocus::Supplements { parent_start } = self.focus {
            self.focus = PromptFocus::Text;
            self.cursor =
                self.snap_visible(self.hidden_before_offset(parent_start).unwrap_or(0), false);
        } else {
            let display = self.display_rows();
            let (index, _) = self.display_cursor_in(&display);
            if let Some(PromptDisplayRow::Text { start, .. }) = display.get(index).copied() {
                self.cursor = start;
            }
        }
        self.settle();
    }

    /// 光标移到当前显示行行尾；附件入口焦点移到入口之后的可见文本起点。
    pub fn move_end(&mut self) {
        self.break_group();
        if let PromptFocus::Supplements { parent_start } = self.focus {
            self.focus = PromptFocus::Text;
            self.cursor = self.snap_visible(
                self.hidden_after_offset(parent_start)
                    .unwrap_or(self.text.len()),
                true,
            );
        } else {
            let display = self.display_rows();
            let (index, _) = self.display_cursor_in(&display);
            if let Some(PromptDisplayRow::Text { end, .. }) = display.get(index).copied() {
                self.cursor = end;
            }
        }
        self.settle();
    }

    /// 落到指定显示行：文本行定位到该行起点加显示列，合成入口转入口焦点。
    fn move_to_display_row(&mut self, display: &[PromptDisplayRow], index: usize, col: usize) {
        match display.get(index).copied() {
            Some(PromptDisplayRow::Text {
                line: _,
                start,
                end,
            }) => {
                self.focus = PromptFocus::Text;
                self.cursor = start + offset_at_col(&self.text[start..end], col);
            }
            Some(PromptDisplayRow::Supplements { parent_start, .. }) => {
                self.focus = PromptFocus::Supplements { parent_start };
                if let Some((_, end)) = self.line_bounds(parent_start) {
                    self.cursor = end.min(self.text.len());
                }
            }
            None => {}
        }
    }

    /// 左侧最近可见文本边界：跳过紧邻光标之前的隐藏附件区间。
    fn hidden_before_offset(&self, parent_start: usize) -> Option<usize> {
        self.supplement_index()
            .entries_for(parent_start)
            .first()
            .map(|entry| entry.range.start)
    }

    /// 右侧最近可见文本边界：跳过该父任务的全部隐藏附件。
    fn hidden_after_offset(&self, parent_start: usize) -> Option<usize> {
        self.supplement_index()
            .entries_for(parent_start)
            .last()
            .map(|entry| entry.range.end)
    }

    /// 当前入口焦点所属的父任务起始行。
    fn focus_parent(&self) -> usize {
        match self.focus {
            PromptFocus::Supplements { parent_start } => parent_start,
            PromptFocus::Text => 0,
        }
    }

    /// 把光标钳到最近的可见文本边界：落在不可见的隐藏附件内或合成行时，
    /// 正向取其后首个可见文本行起点，反向取其前可见文本行末尾。
    fn snap_visible(&self, offset: usize, forward: bool) -> usize {
        let offset = offset.min(self.text.len());
        let spans: Vec<(usize, usize)> = self
            .display_rows()
            .iter()
            .filter_map(|row| match row {
                PromptDisplayRow::Text { start, end, .. } => Some((*start, *end)),
                PromptDisplayRow::Supplements { .. } => None,
            })
            .collect();
        if spans
            .iter()
            .any(|(start, end)| offset >= *start && offset <= *end)
        {
            return offset;
        }
        if forward {
            spans
                .iter()
                .find_map(|(start, _)| (*start >= offset).then_some(*start))
                .unwrap_or(self.text.len())
        } else {
            spans
                .iter()
                .rev()
                .find_map(|(_, end)| (*end <= offset).then_some(*end))
                .unwrap_or(0)
        }
    }

    /// 把字符移动候选边界修正到隐藏区之外：正向跳过，反向落到隐藏区之前。
    fn hidden_adjusted_boundary(&self, mut boundary: usize, forward: bool) -> usize {
        let index = self.supplement_index();
        loop {
            let Some(range) = index
                .hidden_ranges()
                .find(|range| boundary > range.start && boundary < range.end)
            else {
                return boundary;
            };
            boundary = if forward {
                next_boundary(&self.text, range.end)
            } else {
                range.start
            };
            if boundary >= self.text.len() && forward {
                return boundary;
            }
        }
    }

    /// 光标移到逻辑行行首；落点在隐藏附件内时归一到最近的可见边界。
    pub fn move_line_start(&mut self) {
        self.break_group();
        if self.focus.is_supplements() {
            let parent = self.focus_parent();
            self.focus = PromptFocus::Text;
            self.cursor = self.snap_visible(self.hidden_before_offset(parent).unwrap_or(0), false);
        } else {
            self.cursor = self.snap_visible(line_start(&self.text, self.cursor), false);
        }
        self.settle();
    }

    /// 光标移到逻辑行行尾（换行前）；落点在隐藏附件内时归一到最近的可见边界。
    pub fn move_line_end(&mut self) {
        self.break_group();
        if self.focus.is_supplements() {
            let parent = self.focus_parent();
            self.focus = PromptFocus::Text;
            self.cursor = self.snap_visible(
                self.hidden_after_offset(parent).unwrap_or(self.text.len()),
                true,
            );
        } else {
            self.cursor = self.snap_visible(line_end(&self.text, self.cursor), true);
        }
        self.settle();
    }

    /// 光标左移到前一个词的词首。
    pub fn move_word_backward(&mut self) {
        self.break_group();
        self.cursor = self.snap_visible(self.word_start_before(self.cursor), false);
        self.settle();
    }

    /// 光标右移到下一个词的词尾（readline M-f 语义）。
    pub fn move_word_forward(&mut self) {
        self.break_group();
        self.cursor = self.snap_visible(self.word_end_after(self.cursor), true);
        self.settle();
    }
}

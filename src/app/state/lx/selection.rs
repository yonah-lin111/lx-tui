//! `Lx` 输入框的键盘选区、选区删除与剪切：与鼠标选区共用同一份锚点模型。

use super::Lx;
use crate::app::paste::{
    clamp_cursor, folded_tokens, next_pasted_boundary, prev_pasted_boundary, snap_pasted_range,
};
use crate::app::prompt::text;
use crate::app::selection::SelectionUnit;

impl Lx {
    /// 删除输入框当前选中的文本区间并记录一步撤销；无选区返回 false。
    pub fn delete_selected(&mut self) -> bool {
        let Some((start, end)) = self.text_selection_range() else {
            self.text_selection = None;
            return false;
        };
        let range = snap_pasted_range(&self.input, start..end);
        self.refer_suppressed = false;
        self.snapshot();
        self.input.replace_range(range.clone(), "");
        self.cursor = range.start;
        self.adopt_shell_prefix();
        self.refresh_panel();
        true
    }

    /// 剪切输入框当前选中的文本：返回被剪切文本并将删除记入一步撤销；无选区返回 None。
    pub fn cut_selected(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.delete_selected();
        Some(text)
    }

    /// 首次键盘扩展选区时以当前光标为锚点；关闭块面板并压制提及。
    fn begin_keyboard_selection(&mut self) {
        if self.text_selection.is_none() {
            self.text_selection = Some((self.cursor, self.cursor, SelectionUnit::Character));
            self.panel = None;
            self.mention.suppress();
        }
    }

    /// 把选区光标扩展到 `target`；无现有选区且目标未移动时不建立空锚点。
    fn extend_selection_to(&mut self, target: usize) {
        let target = target.min(self.input.len());
        if self.text_selection.is_none() && target == self.cursor {
            return;
        }
        self.refer_suppressed = false;
        self.begin_keyboard_selection();
        self.cursor = target;
        self.scroll_cursor_into_view();
    }

    /// Shift+←：向左扩展一个字符（遇到折叠 token 时整块跨过）。
    pub fn select_left(&mut self) {
        let target = prev_pasted_boundary(&self.input, self.cursor);
        self.extend_selection_to(target);
    }

    /// Shift+→：向右扩展一个字符（遇到折叠 token 时整块跨过）。
    pub fn select_right(&mut self) {
        let target = next_pasted_boundary(&self.input, self.cursor);
        self.extend_selection_to(target);
    }

    /// Shift+Ctrl/Alt+←：向左扩展一个词。
    pub fn select_word_left(&mut self) {
        let target = clamp_cursor(
            &self.input,
            text::word_start_before(&self.input, self.cursor),
        );
        self.extend_selection_to(target);
    }

    /// Shift+Ctrl/Alt+→：向右扩展一个词。
    pub fn select_word_right(&mut self) {
        let mut target = text::word_end_after(&self.input, self.cursor);
        for tok in folded_tokens(&self.input) {
            if tok.range.start < target && target < tok.range.end {
                target = tok.range.end;
                break;
            }
        }
        self.extend_selection_to(target);
    }

    /// Shift+Home：扩展到当前视觉行行首。
    pub fn select_home(&mut self) {
        let rows = self.visual_rows();
        let proj = self.projection();
        let (index, _) = self.cursor_visual_in(&rows);
        let target = proj.display_to_raw_directed(rows[index].start, false);
        self.extend_selection_to(target);
    }

    /// Shift+End：扩展到当前视觉行行尾。
    pub fn select_end(&mut self) {
        let rows = self.visual_rows();
        let proj = self.projection();
        let (index, _) = self.cursor_visual_in(&rows);
        let target = proj.display_to_raw_directed(rows[index].end, true);
        self.extend_selection_to(target);
    }

    /// Shift+↑：向上扩展一个视觉行。
    pub fn select_up(&mut self) {
        let rows = self.visual_rows();
        let proj = self.projection();
        let (index, col) = self.cursor_visual_in(&rows);
        let target = if index == 0 {
            proj.display_to_raw_directed(rows[0].start, false)
        } else {
            let row = rows[index - 1];
            let disp = row.start + text::offset_at_col(&proj.text()[row.start..row.end], col);
            proj.display_to_raw_directed(disp, false)
        };
        self.extend_selection_to(target);
    }

    /// Shift+↓：向下扩展一个视觉行。
    pub fn select_down(&mut self) {
        let rows = self.visual_rows();
        let proj = self.projection();
        let (index, col) = self.cursor_visual_in(&rows);
        let target = if index + 1 >= rows.len() {
            proj.display_to_raw_directed(rows[rows.len() - 1].end, true)
        } else {
            let row = rows[index + 1];
            let disp = row.start + text::offset_at_col(&proj.text()[row.start..row.end], col);
            proj.display_to_raw_directed(disp, true)
        };
        self.extend_selection_to(target);
    }

    /// Ctrl/Cmd+Shift+A：全选输入框。
    pub fn select_all(&mut self) {
        if self.input.is_empty() {
            self.text_selection = None;
            self.cursor = 0;
            return;
        }
        self.text_selection = Some((0, self.input.len(), SelectionUnit::Character));
        self.cursor = self.input.len();
        self.panel = None;
        self.mention.suppress();
        self.scroll_cursor_into_view();
    }

    /// Ctrl+Home / Cmd+↑：移到输入框开头。
    pub fn doc_start(&mut self) {
        self.refer_suppressed = false;
        self.cursor = 0;
        self.refresh_panel();
    }

    /// Ctrl+End / Cmd+↓：移到输入框末尾。
    pub fn doc_end(&mut self) {
        self.refer_suppressed = false;
        self.cursor = self.input.len();
        self.refresh_panel();
    }

    /// Ctrl+Shift+Home / Cmd+Shift+↑：扩展到输入框开头。
    pub fn select_doc_start(&mut self) {
        self.extend_selection_to(0);
    }

    /// Ctrl+Shift+End / Cmd+Shift+↓：扩展到输入框末尾。
    pub fn select_doc_end(&mut self) {
        let target = self.input.len();
        self.extend_selection_to(target);
    }
}

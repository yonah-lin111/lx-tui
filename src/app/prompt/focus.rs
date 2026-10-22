//! prompt 展示方式与焦点：集中/原文切换、合成附件入口焦点与隐藏区归一。
//!
//! 切换不清空撤销历史、不置脏；落点修正统一走显示投影，绝不写入隐藏附件。

use super::presentation::PromptDisplayRow;
use super::{Prompt, PromptFocus, PromptPresentation};

impl Prompt {
    /// 当前展示方式。
    pub fn presentation(&self) -> PromptPresentation {
        self.presentation
    }

    /// 设置展示方式：不修改正文、不入历史、不置脏；切换后归一焦点。
    ///
    /// 集中模式下若光标落在被隐藏的附件内部，转为聚焦该父任务的附件入口。
    pub fn set_presentation(&mut self, presentation: PromptPresentation) {
        self.presentation = presentation;
        self.break_group();
        self.panel = None;
        self.slash = None;
        self.focus = PromptFocus::Text;
        self.normalize_hidden_focus();
        self.scroll_cursor_into_view();
    }

    /// 切换集中/原文展示；返回切换后的展示方式。
    pub fn toggle_presentation(&mut self) -> PromptPresentation {
        let next = match self.presentation {
            PromptPresentation::Compact => PromptPresentation::Source,
            PromptPresentation::Source => PromptPresentation::Compact,
        };
        self.set_presentation(next);
        next
    }

    /// 当前展示焦点。
    pub fn focus(&self) -> PromptFocus {
        self.focus
    }

    /// 设置展示焦点（文本/附件入口）；入口指向已不存在的父任务时保持原焦点。
    pub fn set_focus(&mut self, focus: PromptFocus) {
        match focus {
            PromptFocus::Text => {}
            PromptFocus::Supplements { parent_start } => {
                if !self.has_supplement_entry(parent_start) {
                    return;
                }
            }
        }
        self.focus = focus;
        self.break_group();
        self.scroll_cursor_into_view();
    }

    /// 光标落到附件入口后方对应父任务末尾的字节位置。
    pub fn focus_supplement_entry(&mut self, parent_start: usize) {
        if !self.has_supplement_entry(parent_start) {
            return;
        }
        if let Some((_, end)) = self.line_bounds(parent_start) {
            self.cursor = end.min(self.text.len());
        }
        self.focus = PromptFocus::Supplements { parent_start };
        self.break_group();
        self.scroll_cursor_into_view();
    }

    /// 该父任务在当前投影中是否有合成附件入口。
    fn has_supplement_entry(&self, parent_start: usize) -> bool {
        self.display_rows().iter().any(|row| {
            matches!(row, PromptDisplayRow::Supplements { parent_start: parent, .. } if *parent == parent_start)
        })
    }
}

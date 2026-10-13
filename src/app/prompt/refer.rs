//! prompt 上的字母快捷输入面板：候选重算与键鼠操作。
//!
//! 候选只来自当前作用域已出现的 `@` 提及与 `@[refer-*]` 引用；确认时用候选完整原文
//! 替换触发词，作为一次可撤销编辑。

use super::super::markdown::{self, ReferPanel};
use super::{EditKind, Prompt};

impl Prompt {
    /// 当前字母快捷面板；未打开时为 None。
    pub fn refer_panel(&self) -> Option<&ReferPanel> {
        self.refer.as_ref()
    }

    /// 重算快捷面板：触发词、作用域与候选;无候选时不显示。
    pub(super) fn refresh_refer_panel(&mut self) {
        self.refer = None;
        let Some(trigger) = markdown::refer_trigger(&self.text, self.cursor) else {
            return;
        };
        let Some(scope) = markdown::refer_scope(&self.text, self.cursor) else {
            return;
        };
        let candidates =
            markdown::refer_candidates(&self.text, &scope, &trigger.range, self.mention.entries());
        let filtered = markdown::filter_refer_candidates(candidates, &trigger.query);
        if filtered.is_empty() {
            return;
        }
        self.refer = Some(ReferPanel::new(trigger, filtered));
    }

    /// 面板打开时按偏移循环移动高亮；返回是否消费该按键。
    pub fn refer_move(&mut self, delta: isize) -> bool {
        let Some(panel) = self.refer.as_mut() else {
            return false;
        };
        panel.move_active(delta);
        true
    }

    /// 悬停设置高亮；面板未打开或索引未变返回 false。
    pub fn refer_set_active(&mut self, index: usize) -> bool {
        let Some(panel) = self.refer.as_mut() else {
            return false;
        };
        if panel.active() == index {
            return false;
        }
        panel.set_active(index);
        true
    }

    /// 确认高亮候选：完整原文替换触发词并压制重弹；返回是否消费该按键。
    pub fn refer_confirm(&mut self) -> bool {
        let Some(panel) = self.refer.take() else {
            return false;
        };
        let Some(candidate) = panel.active_candidate() else {
            return false;
        };
        let range = panel.trigger().range.clone();
        if range.start > range.end || range.end > self.text.len() {
            return false;
        }
        if self.protected_edit(range.start, range.end) {
            return false;
        }
        let insertion = markdown::refer_insertion(candidate);
        self.break_group();
        self.record(EditKind::Other);
        self.text.replace_range(range.clone(), &insertion);
        self.cursor = range.start + insertion.len();
        self.panel_suppressed = true;
        self.scroll_cursor_into_view();
        true
    }

    /// 关闭快捷面板且不改文本；返回是否消费该按键。
    pub fn refer_escape(&mut self) -> bool {
        if self.refer.take().is_none() {
            return false;
        }
        self.panel_suppressed = true;
        true
    }
}

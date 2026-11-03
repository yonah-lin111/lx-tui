//! lx 输入框的字母快捷输入面板：候选重算与键鼠交互。
//!
//! 候选融合：
//! 1. 当前输入框正文中已出现的 `@` 提及、`@[refer-*]` 与 `@[ref-content]/` 引用；
//! 2. 当前会话中用户历史消息发送过的提及路径与附件路径（按最近使用逆序去重）。
//!
//! 确认时统一插入 `@[ref-content]/<相对路径> `。

use crate::app::markdown::{
    ReferCandidate, ReferPanel, ReferTag, filter_refer_candidates, is_refer_duplicate,
    refer_candidates, refer_insertion, refer_trigger,
};

use super::Lx;

impl Lx {
    /// 当前字母快捷面板；未打开时为 None。
    pub fn refer_panel(&self) -> Option<&ReferPanel> {
        self.refer.as_ref()
    }

    /// 快捷面板是否可见。
    pub fn refer_active(&self) -> bool {
        self.refer.is_some()
    }

    /// 重算快捷面板：触发词、候选与过滤；无候选时不显示。
    pub(crate) fn refresh_refer_panel(&mut self) {
        if self.plain || self.refer_suppressed {
            self.refer = None;
            return;
        }
        let masked = crate::app::paste::mask_pasted_text(&self.input);
        let Some(trigger) = refer_trigger(&masked, self.cursor) else {
            self.refer = None;
            return;
        };

        // 1. 从当前输入框正文提取候选
        let scope = 0..masked.len();
        let mut candidates = refer_candidates(
            &masked,
            std::slice::from_ref(&scope),
            &trigger.range,
            self.mention.entries(),
        );

        // 2. 从会话历史提取用户发送过的提及与附件
        let history_paths = self.chat.user_history_reference_paths();
        for (path, is_dir) in history_paths {
            let name = path.rsplit('/').next().unwrap_or(&path).to_string();
            let tag = if is_dir {
                ReferTag::Folder
            } else {
                ReferTag::File
            };
            let text = format!("@{path}");
            let candidate = ReferCandidate {
                text,
                name,
                path,
                tag,
                order: candidates.len(),
            };
            // 查重：若已有指向同一目标的候选，升级为全相对路径；若无则追加
            if let Some(pos) = candidates
                .iter()
                .position(|c| is_refer_duplicate(c, &candidate))
            {
                if candidate.path.len() > candidates[pos].path.len() {
                    candidates[pos].path = candidate.path;
                    candidates[pos].text = candidate.text;
                }
            } else {
                candidates.push(candidate);
            }
        }

        let filtered = filter_refer_candidates(candidates, &trigger.query);
        if filtered.is_empty() {
            self.refer = None;
            return;
        }
        self.refer = Some(ReferPanel::new(trigger, filtered));
    }

    /// 滚轮滚动快捷面板视口；返回是否移动。
    pub fn refer_scroll(&mut self, delta: isize, base: usize) -> bool {
        let Some(panel) = self.refer.as_mut() else {
            return false;
        };
        panel.scroll_viewport(delta, base)
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

    /// 确认高亮候选：替换触发词并压制重弹；返回是否消费该按键。
    pub fn refer_confirm(&mut self) -> bool {
        let Some(panel) = self.refer.take() else {
            return false;
        };
        let Some(candidate) = panel.active_candidate().cloned() else {
            return false;
        };
        let range = panel.trigger().range.clone();
        if range.start > range.end || range.end > self.input.len() {
            return false;
        }
        let insertion = refer_insertion(&candidate);
        self.snapshot();
        self.input.replace_range(range.start..range.end, &insertion);
        self.cursor = range.start + insertion.len();
        self.refer_suppressed = true;
        self.scroll_cursor_into_view();
        self.refresh_panel();
        true
    }

    /// 点击确认指定项。
    pub fn refer_confirm_at(&mut self, index: usize) -> bool {
        if !self.refer_set_active(index) && self.refer.is_none() {
            return false;
        }
        self.refer_confirm()
    }

    /// 关闭快捷面板；返回是否消费该按键。
    pub fn refer_escape(&mut self) -> bool {
        if self.refer.take().is_none() {
            return false;
        }
        self.refer_suppressed = true;
        true
    }
}

#[cfg(test)]
mod tests;

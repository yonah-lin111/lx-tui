//! prompt 命令面板生命周期：块命令与斜杠命令面板的导航、确认、关闭与重算。

use super::super::markdown::{self, BlockPanel, SlashPanel};
use super::EditKind;
use super::Prompt;

impl Prompt {
    /// 当前块命令面板；未打开时为 None。
    pub fn panel(&self) -> Option<&BlockPanel> {
        self.panel.as_ref()
    }

    /// 面板打开时按偏移循环移动高亮；返回是否消费该按键。
    pub fn panel_move(&mut self, delta: isize) -> bool {
        match self.panel.as_mut() {
            Some(panel) => {
                panel.move_active(delta);
                true
            }
            None => false,
        }
    }

    /// 块命令面板悬停高亮：设置高亮索引、窗口锚点不动；越界或未变化返回 false。
    pub fn panel_set_active(&mut self, index: usize) -> bool {
        let Some(panel) = self.panel.as_mut() else {
            return false;
        };
        if index >= panel.items().len() || panel.active() == index {
            return false;
        }
        panel.set_active(index);
        true
    }

    /// 滚轮在块命令面板上滚动可见窗口：视口从 `base` 起偏移、越界钳制不循环；
    /// 高亮不动；未打开或视口未移动返回 false。
    pub fn panel_scroll(&mut self, delta: isize, base: usize) -> bool {
        let Some(panel) = self.panel.as_mut() else {
            return false;
        };
        panel.scroll_viewport(delta, base)
    }

    /// 块命令面板点选：设置高亮并确认插入；返回是否消费。
    pub fn panel_confirm_at(&mut self, index: usize) -> bool {
        self.panel_set_active(index);
        self.panel_confirm()
    }

    /// 面板打开时确认高亮命令：替换触发区间并压制重弹；返回是否消费该按键。
    pub fn panel_confirm(&mut self) -> bool {
        let Some(panel) = self.panel.as_ref() else {
            return false;
        };
        let trigger = panel.trigger();
        if trigger.from > trigger.to || trigger.to > self.text.len() {
            return false;
        }
        let Some(id) = panel.items().get(panel.active()).copied() else {
            return false;
        };
        let insertion = markdown::block_insertion(id);
        self.panel = None;
        self.break_group();
        self.record(EditKind::Other);
        self.text
            .replace_range(trigger.from..trigger.to, &insertion.text);
        self.cursor = trigger.from + insertion.cursor;
        self.panel_suppressed = true;
        self.scroll_cursor_into_view();
        true
    }

    /// 面板打开时关闭且不改文本；返回是否消费该按键。
    pub fn panel_escape(&mut self) -> bool {
        if self.panel.is_none() {
            return false;
        }
        self.panel = None;
        self.panel_suppressed = true;
        true
    }

    /// 当前斜杠命令面板；未打开时为 None。
    pub fn slash_panel(&self) -> Option<&SlashPanel> {
        self.slash.as_ref()
    }

    /// 斜杠命令面板打开时按偏移循环移动高亮；返回是否消费该按键。
    pub fn slash_move(&mut self, delta: isize) -> bool {
        match self.slash.as_mut() {
            Some(panel) => {
                panel.move_active(delta);
                true
            }
            None => false,
        }
    }

    /// 斜杠命令面板悬停高亮：设置高亮索引；未打开、越界或未变化返回 false。
    pub fn slash_set_active(&mut self, index: usize) -> bool {
        let Some(panel) = self.slash.as_mut() else {
            return false;
        };
        if index >= panel.items().len() || panel.active() == index {
            return false;
        }
        panel.set_active(index);
        true
    }

    /// 滚轮在斜杠命令面板上滚动可见窗口：视口从 `base` 起偏移、越界钳制不循环；
    /// 高亮不动；未打开或视口未移动返回 false。
    pub fn slash_scroll(&mut self, delta: isize, base: usize) -> bool {
        let Some(panel) = self.slash.as_mut() else {
            return false;
        };
        panel.scroll_viewport(delta, base)
    }

    /// 斜杠命令面板点选：设置高亮并确认插入；返回是否消费。
    pub fn slash_confirm_at(&mut self, index: usize) -> bool {
        self.slash_set_active(index);
        self.slash_confirm()
    }

    /// 斜杠命令面板确认：整行替换为标准模板块，光标落在标题占位符内；返回是否消费。
    pub fn slash_confirm(&mut self) -> bool {
        let Some(panel) = self.slash.as_ref() else {
            return false;
        };
        let trigger = panel.trigger().clone();
        let Some(id) = panel.items().get(panel.active()).copied() else {
            return false;
        };
        if trigger.line_start > trigger.line_end || trigger.line_end > self.text.len() {
            return false;
        }
        let (content, cursor) = markdown::slash_template_content(id);
        self.slash = None;
        self.break_group();
        self.record(EditKind::Other);
        self.text
            .replace_range(trigger.line_start..trigger.line_end, content);
        self.cursor = trigger.line_start + cursor;
        self.panel_suppressed = true;
        self.scroll_cursor_into_view();
        true
    }

    /// 斜杠命令面板打开时关闭且不改文本；返回是否消费该按键。
    pub fn slash_escape(&mut self) -> bool {
        if self.slash.is_none() {
            return false;
        }
        self.slash = None;
        self.panel_suppressed = true;
        true
    }

    /// 重算块命令面板：触发标记决定候选列表，同类同位置保留高亮。
    pub(super) fn refresh_panel(&mut self) {
        let Some(trigger) = markdown::block_trigger(&self.text, self.cursor) else {
            self.panel = None;
            return;
        };
        let previous = self.panel.as_ref().filter(|panel| {
            panel.trigger().kind == trigger.kind && panel.trigger().to == trigger.to
        });
        let active = previous.map_or(0, BlockPanel::active);
        let items = markdown::block_commands(trigger.kind);
        let active = active.min(items.len().saturating_sub(1));
        self.panel = Some(BlockPanel::new(trigger, items, active));
    }

    /// 重算斜杠命令面板：整行 `/` 触发决定候选，同位置保留高亮。
    pub(super) fn refresh_slash_panel(&mut self) {
        let Some(trigger) = markdown::slash_trigger(&self.text, self.cursor) else {
            self.slash = None;
            return;
        };
        let items = markdown::slash_filter(&trigger.query);
        if items.is_empty() {
            self.slash = None;
            return;
        }
        let previous = self
            .slash
            .as_ref()
            .filter(|panel| panel.trigger().line_start == trigger.line_start);
        let active = previous.map_or(0, SlashPanel::active).min(items.len() - 1);
        self.slash = Some(SlashPanel::new(trigger, items, active));
    }
}

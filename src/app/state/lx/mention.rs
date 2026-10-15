//! lx 输入框 `@` 文件提及面板状态：有效根、扫描缓存、异步代号、过滤面板与压制。
//!
//! 纯状态：不扫描文件、不发任务、不读进程；有效根由事件层从 `pane.cwd` 或工作区 cwd
//! 计算后传入。候选始终相对本次扫描根，根变化即作废旧候选与在途结果。

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::app::markdown::{
    self, MentionEntry, MentionPanel, filter_mentions, mention_insertion, mention_trigger,
};

/// 提及面板可用视图：无触发/无根、扫描中、无匹配或候选列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LxMentionView {
    /// 无有效触发或无根：不绘制面板。
    Hidden,
    /// 有效触发、根可用但扫描结果未到：固定尺寸状态行。
    Loading,
    /// 扫描完成但当前查询无匹配：固定尺寸状态行。
    Empty,
    /// 有候选：绘制候选列表。
    Items,
}

/// lx 提及面板状态：扫描根、缓存、在途代号、过滤面板与压制标记。
#[derive(Debug, Default)]
pub struct LxMention {
    /// 当前有效扫描根（`pane.cwd` 优先，回退工作区 cwd）；None 表示无可扫描根。
    root: Option<PathBuf>,
    /// 最近一次扫描结果；None 表示尚未取得。
    cache: Option<Vec<MentionEntry>>,
    /// 扫描代号：根变化或新一轮扫描递增，用于丢弃过期回执。
    generation: u64,
    /// 扫描是否在途。
    pending: bool,
    /// 过滤后的候选面板；无候选或未扫描时为 None。
    panel: Option<MentionPanel>,
    /// Esc/确认后压制自动重开，直到下一次编辑或根变化。
    suppressed: bool,
}

impl LxMention {
    /// 当前候选面板；未打开时为 None。
    pub fn panel(&self) -> Option<&MentionPanel> {
        self.panel.as_ref()
    }

    /// 候选面板是否可见（有可采纳条目）。
    pub fn is_open(&self) -> bool {
        self.panel.is_some()
    }

    /// 当前有效扫描根；无根时为 None。
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// 面板视图种类；`text`/`cursor` 为当前输入缓冲与光标。
    pub fn view(&self, text: &str, cursor: usize) -> LxMentionView {
        if self.panel.is_some() {
            return LxMentionView::Items;
        }
        if self.suppressed || self.root.is_none() {
            return LxMentionView::Hidden;
        }
        if mention_trigger(text, cursor).is_none() {
            return LxMentionView::Hidden;
        }
        if self.cache.is_some() {
            LxMentionView::Empty
        } else {
            LxMentionView::Loading
        }
    }

    /// 同步有效扫描根；根变化时清缓存、作废在途结果、解除压制并重算面板。
    ///
    /// 返回根是否发生变化（供调用方置脏重绘）。
    pub fn set_root(&mut self, root: Option<PathBuf>, text: &str, cursor: usize) -> bool {
        if self.root == root {
            return false;
        }
        self.root = root;
        self.cache = None;
        self.pending = false;
        self.suppressed = false;
        self.generation = self.generation.wrapping_add(1);
        self.refresh(text, cursor);
        true
    }

    /// 需要新扫描时返回（代号，扫描根）并标记在途；已有缓存或在途时返回 None。
    pub fn take_scan_request(&mut self, text: &str, cursor: usize) -> Option<(u64, PathBuf)> {
        if self.pending || self.cache.is_some() {
            return None;
        }
        let root = self.root.clone()?;
        mention_trigger(text, cursor)?;
        self.pending = true;
        Some((self.generation, root))
    }

    /// 写入扫描结果；代号或根不匹配（过期）时丢弃；返回是否采纳。
    pub fn apply_entries(
        &mut self,
        generation: u64,
        root: &Path,
        entries: Vec<MentionEntry>,
        text: &str,
        cursor: usize,
    ) -> bool {
        if generation != self.generation || self.root.as_deref() != Some(root) {
            return false;
        }
        self.pending = false;
        self.cache = Some(entries);
        self.refresh(text, cursor);
        true
    }

    /// 按当前文本、光标与缓存重算面板；压制时保持关闭。
    pub fn refresh(&mut self, text: &str, cursor: usize) {
        if self.suppressed {
            self.panel = None;
            return;
        }
        let Some(trigger) = mention_trigger(text, cursor) else {
            self.panel = None;
            return;
        };
        let Some(cache) = self.cache.as_ref() else {
            self.panel = None;
            return;
        };
        let items = filter_mentions(cache, &trigger.query);
        if items.is_empty() {
            self.panel = None;
            return;
        }
        // 同起点且同目录范围时保留高亮；进入/回退目录改变范围则回到首项。
        let previous = self.panel.as_ref().filter(|panel| {
            panel.trigger().from == trigger.from
                && markdown::mention_scope(&panel.trigger().query)
                    == markdown::mention_scope(&trigger.query)
        });
        let active = previous
            .map_or(0, MentionPanel::active)
            .min(items.len() - 1);
        self.panel = Some(MentionPanel::new(trigger, items, active));
    }

    /// 键盘或悬停移动高亮；面板未打开返回 false。
    pub fn move_active(&mut self, delta: isize) -> bool {
        match self.panel.as_mut() {
            Some(panel) => {
                panel.move_active(delta);
                true
            }
            None => false,
        }
    }

    /// 直接设置高亮索引；面板未打开、越界或未变化返回 false。
    pub fn set_active(&mut self, index: usize) -> bool {
        let Some(panel) = self.panel.as_mut() else {
            return false;
        };
        if index >= panel.items().len() || panel.active() == index {
            return false;
        }
        panel.set_active(index);
        true
    }

    /// 滚轮滚动可见窗口：`base` 为当前窗口起点，高亮不动；返回是否移动。
    pub fn scroll_viewport(&mut self, delta: isize, base: usize) -> bool {
        match self.panel.as_mut() {
            Some(panel) => panel.scroll_viewport(delta, base),
            None => false,
        }
    }

    /// 确认高亮条目：关闭面板、进入压制，返回替换区间与插入文本；无面板或越界返回 None。
    pub fn confirm(&mut self, text_len: usize) -> Option<(Range<usize>, String)> {
        let panel = self.panel.as_ref()?;
        let trigger = panel.trigger();
        if trigger.from > trigger.to || trigger.to > text_len {
            return None;
        }
        let entry = panel.items().get(panel.active())?.clone();
        let range = trigger.from..trigger.to;
        let insertion = mention_insertion(&entry);
        self.panel = None;
        self.suppressed = true;
        Some((range, insertion))
    }

    /// 进入高亮目录：返回触发区间与写回文本（`@目录/`，保持触发不闭合）。
    ///
    /// 高亮非目录、目录无缓存后代或区间越界时返回 None。
    pub fn enter_folder(&self, text_len: usize) -> Option<(Range<usize>, String)> {
        let panel = self.panel.as_ref()?;
        let entry = panel.items().get(panel.active())?;
        if !entry.is_directory || !self.has_children(&entry.path) {
            return None;
        }
        let trigger = panel.trigger();
        if trigger.from > trigger.to || trigger.to > text_len {
            return None;
        }
        Some((trigger.from..trigger.to, format!("@{}/", entry.path)))
    }

    /// 缓存中是否存在该目录的后代；无子项目录不可进入，避免面板因零候选消失。
    fn has_children(&self, path: &str) -> bool {
        let prefix = format!("{path}/");
        self.cache
            .as_ref()
            .is_some_and(|cache| cache.iter().any(|entry| entry.path.starts_with(&prefix)))
    }

    /// 关闭面板且不改文本、进入压制；面板未打开返回 false。
    pub fn escape(&mut self) -> bool {
        let open = self.panel.is_some();
        self.suppress();
        open
    }

    /// 关闭面板并压制自动重开（粘贴等场景）；保留缓存与在途标记以便后续复用。
    pub fn suppress(&mut self) {
        self.panel = None;
        self.suppressed = true;
    }

    /// 解除压制（编辑或移动光标后由调用方清除）。
    pub fn unsuppress(&mut self) {
        self.suppressed = false;
    }
}

#[cfg(test)]
mod tests;

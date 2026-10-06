//! prompt 文件提及面板：`@` 查询对应的候选、扫描缓存与异步结果代号。

use std::ops::Range;
use std::path::PathBuf;

use super::super::markdown::{self, MentionEntry, MentionPanel};

/// 文件提及面板状态：面板、扫描根、缓存、在途代号与高亮。
#[derive(Debug, Default)]
pub struct MentionState {
    panel: Option<MentionPanel>,
    root: Option<PathBuf>,
    cache: Option<Vec<MentionEntry>>,
    generation: u64,
    pending: bool,
}

impl MentionState {
    /// 当前面板；未打开时为 None。
    pub fn panel(&self) -> Option<&MentionPanel> {
        self.panel.as_ref()
    }

    /// 按偏移循环移动高亮；面板未打开返回 false。
    pub fn move_active(&mut self, delta: isize) -> bool {
        match self.panel.as_mut() {
            Some(panel) => {
                panel.move_active(delta);
                true
            }
            None => false,
        }
    }

    /// 设置高亮索引；面板未打开、索引越界或未变化返回 false。
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

    /// 确认高亮条目：关闭面板并返回替换区间与插入文本；未打开或区间越界返回 None。
    pub fn confirm(&mut self, text_len: usize) -> Option<(Range<usize>, String)> {
        let panel = self.panel.as_ref()?;
        let trigger = panel.trigger();
        if trigger.from > trigger.to || trigger.to > text_len {
            return None;
        }
        let entry = panel.items().get(panel.active())?.clone();
        let range = trigger.from..trigger.to;
        let insertion = markdown::mention_insertion(&entry);
        self.panel = None;
        Some((range, insertion))
    }

    /// 关闭面板且不改文本；返回是否消费该按键。
    pub fn escape(&mut self) -> bool {
        if self.panel.is_none() {
            return false;
        }
        self.panel = None;
        true
    }

    /// 失焦或折叠时清空面板并作废在途结果。
    pub fn clear(&mut self) {
        self.panel = None;
        self.pending = false;
        self.generation = self.generation.wrapping_add(1);
    }

    /// 同步扫描根；根变化时清除缓存、关闭面板并作废在途结果。
    pub fn set_root(&mut self, root: Option<PathBuf>, text: &str, cursor: usize) {
        if self.root == root {
            return;
        }
        self.root = root;
        self.cache = None;
        self.pending = false;
        self.generation = self.generation.wrapping_add(1);
        self.refresh(text, cursor);
    }

    /// 需要新扫描时返回（代号，根路径）并标记处理中；仅事件循环调用。
    pub fn take_scan_request(&mut self, text: &str, cursor: usize) -> Option<(u64, PathBuf)> {
        if self.pending || self.cache.is_some() {
            return None;
        }
        let root = self.root.clone()?;
        markdown::mention_trigger(text, cursor)?;
        self.pending = true;
        Some((self.generation, root))
    }

    /// 写入扫描结果；代号过期（根变化或失焦）时丢弃；返回是否采纳。
    pub fn apply_entries(
        &mut self,
        generation: u64,
        entries: Vec<MentionEntry>,
        text: &str,
        cursor: usize,
    ) -> bool {
        if generation != self.generation {
            return false;
        }
        self.pending = false;
        self.cache = Some(entries);
        self.refresh(text, cursor);
        true
    }

    /// 重算面板：触发或缓存缺失时隐藏，同起点保留高亮。
    pub(super) fn refresh(&mut self, text: &str, cursor: usize) {
        let Some(trigger) = markdown::mention_trigger(text, cursor) else {
            self.panel = None;
            return;
        };
        let Some(cache) = self.cache.as_ref() else {
            self.panel = None;
            return;
        };
        let items = markdown::filter_mentions(cache, &trigger.query);
        if items.is_empty() {
            self.panel = None;
            return;
        }
        let previous = self
            .panel
            .as_ref()
            .filter(|panel| panel.trigger().from == trigger.from);
        let active = previous
            .map_or(0, MentionPanel::active)
            .min(items.len() - 1);
        self.panel = Some(MentionPanel::new(trigger, items, active));
    }
}

//! lx 输入框 `@` 文件提及面板状态：有效根、扫描缓存、异步代号、过滤面板与压制。
//!
//! 纯状态：不扫描文件、不发任务、不读进程；有效根由事件层从 `pane.cwd` 或工作区 cwd
//! 计算后传入。候选始终相对本次扫描根，根变化即作废旧候选与在途结果。

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::app::markdown::{
    self, MentionEntry, MentionPanel, filter_mentions, mention_insertion, mention_trigger,
};
pub use crate::app::prompt::MentionTab;

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

/// lx 提及面板状态：扫描根、缓存、在途代号、过滤面板、Tab 与压制标记。
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
    /// 当前活跃 Tab：Files 或 refDir。
    tab: MentionTab,
    /// 引用文件夹列表快照（由上层同步注入）。
    referenced_folders: Vec<crate::app::referenced::ReferencedFolder>,
    /// refDir 列表的高亮索引。
    ref_active: usize,
    /// refDir 列表的窗口锚点索引。
    ref_anchor: usize,
    /// refDir 显式视口起点。
    ref_viewport: Option<usize>,
    /// 当前触发区间与查询词。
    trigger: Option<markdown::MentionTrigger>,
}

impl LxMention {
    /// 当前候选面板；未打开时为 None。
    pub fn panel(&self) -> Option<&MentionPanel> {
        self.panel.as_ref()
    }

    /// 当前活跃 Tab。
    pub fn tab(&self) -> MentionTab {
        self.tab
    }

    /// refDir 高亮索引。
    pub fn ref_active(&self) -> usize {
        self.ref_active
    }

    /// refDir 窗口锚点。
    pub fn ref_anchor(&self) -> usize {
        self.ref_anchor
    }

    /// refDir 显式视口。
    pub fn ref_viewport(&self) -> Option<usize> {
        self.ref_viewport
    }

    /// 同步引用文件夹快照；内容变化时重算面板并返回 true。
    pub fn sync_referenced_folders(
        &mut self,
        folders: &[crate::app::referenced::ReferencedFolder],
        text: &str,
        cursor: usize,
    ) -> bool {
        if self.referenced_folders == folders {
            return false;
        }
        self.referenced_folders = folders.to_vec();
        self.refresh(text, cursor);
        true
    }

    /// 获取基于当前触发词过滤后的引用文件夹列表。
    pub fn filtered_ref_folders(&self) -> Vec<&crate::app::referenced::ReferencedFolder> {
        let query = self
            .trigger
            .as_ref()
            .map(|t| t.query.as_str())
            .unwrap_or("");
        if query.is_empty() {
            return self.referenced_folders.iter().collect();
        }
        let query_lower = query.to_lowercase();
        self.referenced_folders
            .iter()
            .filter(|folder| {
                folder.display_name().to_lowercase().contains(&query_lower)
                    || folder
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query_lower)
            })
            .collect()
    }

    /// 切换 Tab：循环左右切换；未处于可见状态返回 false。
    pub fn switch_tab(&mut self, delta: isize, text: &str, cursor: usize) -> bool {
        if self.view(text, cursor) == LxMentionView::Hidden {
            return false;
        }
        let next = if delta > 0 {
            self.tab.next()
        } else {
            self.tab.prev()
        };
        if next == self.tab {
            return false;
        }
        self.tab = next;
        self.ref_active = 0;
        self.ref_anchor = 0;
        self.ref_viewport = None;
        true
    }

    /// 直接设置活跃 Tab。
    pub fn set_tab(&mut self, tab: MentionTab, text: &str, cursor: usize) -> bool {
        if self.view(text, cursor) == LxMentionView::Hidden || self.tab == tab {
            return false;
        }
        self.tab = tab;
        self.ref_active = 0;
        self.ref_anchor = 0;
        self.ref_viewport = None;
        true
    }

    /// 会话结束：状态复位。
    fn reset_session(&mut self) {
        self.panel = None;
        self.trigger = None;
        self.tab = MentionTab::Files;
        self.ref_active = 0;
        self.ref_anchor = 0;
        self.ref_viewport = None;
    }

    /// 候选面板是否可见（当前活跃 Tab 有可采纳条目）。
    pub fn is_open(&self) -> bool {
        match self.tab {
            MentionTab::Files => self.panel.is_some(),
            MentionTab::RefDir => {
                self.trigger.is_some()
                    && !self.suppressed
                    && !self.filtered_ref_folders().is_empty()
            }
        }
    }

    /// 扫描条目缓存（若已有）；无缓存返回空切片。
    pub fn entries(&self) -> &[MentionEntry] {
        self.cache.as_deref().unwrap_or_default()
    }

    /// 当前有效扫描根；无根时为 None。
    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// 面板视图种类；`text`/`cursor` 为当前输入缓冲与光标。
    pub fn view(&self, text: &str, cursor: usize) -> LxMentionView {
        if self.suppressed || mention_trigger(text, cursor).is_none() {
            return LxMentionView::Hidden;
        }
        if self.root.is_none()
            && self.referenced_folders.is_empty()
            && self.tab == MentionTab::Files
        {
            return LxMentionView::Hidden;
        }
        match self.tab {
            MentionTab::Files => {
                if self.panel.is_some() {
                    LxMentionView::Items
                } else if self.cache.is_some() || self.root.is_none() {
                    LxMentionView::Empty
                } else {
                    LxMentionView::Loading
                }
            }
            MentionTab::RefDir => {
                if self.filtered_ref_folders().is_empty() {
                    LxMentionView::Empty
                } else {
                    LxMentionView::Items
                }
            }
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
            self.reset_session();
            return;
        }
        let Some(trigger) = mention_trigger(text, cursor) else {
            self.reset_session();
            return;
        };
        if self.trigger.as_ref().map(|t| t.from) != Some(trigger.from) {
            self.tab = MentionTab::Files;
            self.ref_active = 0;
            self.ref_anchor = 0;
            self.ref_viewport = None;
        } else if self.trigger.as_ref().map(|t| t.query.as_str()) != Some(trigger.query.as_str()) {
            self.ref_viewport = None;
        }
        self.trigger = Some(trigger.clone());
        let filtered_count = self.filtered_ref_folders().len();
        if filtered_count == 0 {
            self.ref_active = 0;
            self.ref_anchor = 0;
            self.ref_viewport = None;
        } else if self.ref_active >= filtered_count {
            self.ref_active = filtered_count - 1;
            self.ref_anchor = self.ref_active;
        }
        if self.ref_viewport.is_some_and(|vp| vp >= filtered_count) {
            self.ref_viewport = None;
        }
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
        if self.tab == MentionTab::RefDir {
            let count = self.filtered_ref_folders().len();
            if count == 0 {
                return false;
            }
            let next = (self.ref_active as isize + delta).rem_euclid(count as isize) as usize;
            self.ref_active = next;
            self.ref_anchor = next;
            self.ref_viewport = None;
            return true;
        }
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
        if self.tab == MentionTab::RefDir {
            let count = self.filtered_ref_folders().len();
            if index >= count || self.ref_active == index {
                return false;
            }
            self.ref_active = index;
            return true;
        }
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
        if self.tab == MentionTab::RefDir {
            let count = self.filtered_ref_folders().len();
            if count == 0 {
                return false;
            }
            let current = self.ref_viewport.unwrap_or(base);
            let next = (current as isize + delta).max(0) as usize;
            let clamped = next.min(count.saturating_sub(1));
            if self.ref_viewport == Some(clamped) {
                return false;
            }
            self.ref_viewport = Some(clamped);
            return true;
        }
        match self.panel.as_mut() {
            Some(panel) => panel.scroll_viewport(delta, base),
            None => false,
        }
    }

    /// 确认高亮条目：关闭面板、进入压制，返回替换区间与插入文本；无面板或越界返回 None。
    pub fn confirm(&mut self, text_len: usize) -> Option<(Range<usize>, String)> {
        if self.tab == MentionTab::RefDir {
            let trigger = self.trigger.as_ref()?;
            if trigger.from > trigger.to || trigger.to > text_len {
                return None;
            }
            let filtered = self.filtered_ref_folders();
            let folder = filtered.get(self.ref_active)?;
            let range = trigger.from..trigger.to;
            let ref_text = crate::app::markdown::format_reference(
                crate::app::markdown::ReferKind::Folder,
                &folder.path,
            )
            .unwrap_or_else(|| format!("@[refer-folder]({})", folder.path.display()));
            let insertion = format!("{ref_text} ");
            self.reset_session();
            self.suppressed = true;
            return Some((range, insertion));
        }
        let panel = self.panel.as_ref()?;
        let trigger = panel.trigger();
        if trigger.from > trigger.to || trigger.to > text_len {
            return None;
        }
        let entry = panel.items().get(panel.active())?.clone();
        let range = trigger.from..trigger.to;
        let insertion = mention_insertion(&entry);
        self.reset_session();
        self.suppressed = true;
        Some((range, insertion))
    }

    /// 进入高亮目录：返回触发区间与写回文本（`@目录/`，保持触发不闭合）。
    ///
    /// 高亮非目录、目录无缓存后代或区间越界时返回 None。
    pub fn enter_folder(&self, text_len: usize) -> Option<(Range<usize>, String)> {
        if self.tab == MentionTab::RefDir {
            return None;
        }
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
        let open = self.panel.is_some()
            || (self.trigger.is_some()
                && !self.suppressed
                && (self.root.is_some()
                    || !self.referenced_folders.is_empty()
                    || self.tab == MentionTab::RefDir));
        self.suppress();
        open
    }

    /// 关闭面板并压制自动重开（粘贴等场景）；保留缓存与在途标记以便后续复用。
    pub fn suppress(&mut self) {
        self.reset_session();
        self.suppressed = true;
    }

    /// 解除压制（编辑或移动光标后由调用方清除）。
    pub fn unsuppress(&mut self) {
        self.suppressed = false;
    }
}

#[cfg(test)]
mod tests;

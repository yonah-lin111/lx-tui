//! prompt 文件提及面板：`@` 查询对应的候选、扫描缓存、异步结果代号与
//! 模板块 worktree 绑定解析。

use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// 未解析绑定的 worktree 列表最小重试间隔：避免持续停在块内时反复起 git。
const WORKTREE_RETRY_INTERVAL: Duration = Duration::from_secs(2);

use super::super::markdown::{
    self, MentionEntry, MentionPanel, WorktreeBinding, WorktreeCandidate, block_worktree_branch,
    resolve_binding,
};

/// 文件提及面板状态：面板、扫描根、缓存、在途代号、高亮与 worktree 绑定。
#[derive(Debug, Default)]
pub struct MentionState {
    panel: Option<MentionPanel>,
    /// 显式绑定根（prompt 顶栏 `ws:` 或工作区 cwd）；未绑定时 None。
    root: Option<PathBuf>,
    cache: Option<Vec<MentionEntry>>,
    generation: u64,
    pending: bool,
    /// 显示中工作区的 git 仓库根（主 checkout）。
    worktree_repo: Option<PathBuf>,
    /// 仓库的 worktree 候选（已排除 detached/bare）。
    worktree_candidates: Option<Vec<WorktreeCandidate>>,
    /// 候选查询在途标记。
    worktree_pending: bool,
    /// 最近一次发起 worktree 查询的时刻；用于未解析绑定的重试节流。
    worktree_queried_at: Option<Instant>,
    /// 面板压制：Esc/确认后保持关闭，直到绑定切换或编辑重置。
    suppressed: bool,
}

impl MentionState {
    /// 当前面板；未打开时为 None。
    pub fn panel(&self) -> Option<&MentionPanel> {
        self.panel.as_ref()
    }

    /// 当前仓库根（调用方比较变化用）。
    pub fn worktree_repo(&self) -> Option<&std::path::Path> {
        self.worktree_repo.as_deref()
    }

    /// 设置显示中工作区的 git 仓库根；变化时作废 worktree 候选与在途结果。
    pub fn set_worktree_repo(&mut self, repo: Option<PathBuf>, text: &str, cursor: usize) {
        if self.worktree_repo == repo {
            return;
        }
        self.worktree_repo = repo;
        self.worktree_candidates = None;
        self.worktree_pending = false;
        self.refresh(text, cursor);
    }

    /// 选择器打开时清空旧候选并允许新一轮加载（不直接发起，由事件层调度）。
    pub fn request_worktree_list(&mut self) {
        self.worktree_candidates = None;
        self.worktree_pending = false;
    }

    /// worktree 候选（已加载时）。
    pub fn worktree_candidates(&self) -> Option<&[WorktreeCandidate]> {
        self.worktree_candidates.as_deref()
    }

    /// worktree 列表是否在途（渲染加载占位）。
    pub fn worktree_loading(&self) -> bool {
        self.worktree_pending && self.worktree_candidates.is_none()
    }

    /// 光标所在块的 worktree 绑定；无块或无绑定返回 None。
    pub fn block_binding(&self, text: &str, cursor: usize) -> Option<WorktreeBinding> {
        let (_, branch) = block_worktree_branch(text, cursor)?;
        let Some(candidates) = self.worktree_candidates.as_ref() else {
            // 候选未加载：绑定存在但结果未知，按缺失处理（调用方触发加载）。
            return Some(WorktreeBinding::Missing(branch));
        };
        Some(resolve_binding(candidates, &branch))
    }

    /// 需要新扫描时返回（代号，根路径）并标记处理中；仅事件循环调用。
    pub fn take_scan_request(&mut self, text: &str, cursor: usize) -> Option<(u64, PathBuf)> {
        if self.pending {
            return None;
        }
        let root = self.effective_root(text, cursor);
        let root = root?;
        if self.cache.is_some() {
            return None;
        }
        markdown::mention_trigger(text, cursor)?;
        self.pending = true;
        Some((self.generation, root))
    }

    /// 需要 worktree 列表时返回仓库根；查询在途或仓库未知时 None。
    pub fn take_worktree_request(&mut self, text: &str, cursor: usize) -> Option<PathBuf> {
        let repo = self.worktree_repo.clone()?;
        if self.worktree_pending {
            return None;
        }
        let load = self.worktree_candidates.is_none();
        let unresolved = self.binding_unresolved(text, cursor);
        // 首次加载立即发起；块内绑定未解析时节流重试（外部可能新建 worktree）。
        let stale = unresolved
            && self
                .worktree_queried_at
                .is_none_or(|at| at.elapsed() >= WORKTREE_RETRY_INTERVAL);
        if !load && !stale {
            return None;
        }
        self.worktree_pending = true;
        self.worktree_queried_at = Some(Instant::now());
        Some(repo)
    }

    /// 选择器打开期间的强制刷新；结果到达前不重复发起。
    pub fn request_worktree_refresh(&mut self) -> Option<PathBuf> {
        let repo = self.worktree_repo.clone()?;
        if self.worktree_pending {
            return None;
        }
        self.worktree_pending = true;
        self.worktree_queried_at = Some(Instant::now());
        Some(repo)
    }

    /// 绑定存在但无法解析（分支缺失或多个 checkout 同分支）。
    pub fn binding_unresolved(&self, text: &str, cursor: usize) -> bool {
        matches!(
            self.block_binding(text, cursor),
            Some(WorktreeBinding::Missing(_) | WorktreeBinding::Ambiguous(_))
        )
    }

    /// 写入 worktree 列表结果；列表变化时清空旧扫描缓存并作废在途结果。
    pub fn apply_worktree_entries(
        &mut self,
        candidates: Vec<WorktreeCandidate>,
        text: &str,
        cursor: usize,
    ) {
        self.worktree_pending = false;
        if self.worktree_candidates.as_deref() != Some(candidates.as_slice()) {
            self.worktree_candidates = Some(candidates);
            self.cache = None;
            self.pending = false;
            self.generation = self.generation.wrapping_add(1);
        }
        self.refresh(text, cursor);
    }

    /// worktree 查询失败：清空候选并允许后续重试（节流由查询时刻控制）。
    pub fn worktree_failed(&mut self, text: &str, cursor: usize) {
        self.worktree_pending = false;
        self.worktree_candidates = None;
        self.refresh(text, cursor);
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

    /// 滚轮滚动可见窗口：显式视口从 `base` 起偏移、越界钳制不循环；高亮不动；
    /// 面板未打开或视口未移动返回 false。
    pub fn scroll_viewport(&mut self, delta: isize, base: usize) -> bool {
        match self.panel.as_mut() {
            Some(panel) => panel.scroll_viewport(delta, base),
            None => false,
        }
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

    /// 进入高亮目录：返回触发区间与写回文本（`@根相对路径/`，保持提及触发不闭合）。
    ///
    /// 高亮项不是目录、目录无缓存后代（进入后将无候选）、触发区间越界时返回 None。
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

    /// 扫描缓存中是否存在该目录的后代；无子项目录不可进入，避免面板因零候选消失。
    fn has_children(&self, path: &str) -> bool {
        let prefix = format!("{path}/");
        self.cache
            .as_ref()
            .is_some_and(|cache| cache.iter().any(|entry| entry.path.starts_with(&prefix)))
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

    /// 同步扫描根；根或 worktree 绑定变化时清除缓存、关闭面板并作废在途结果；
    /// 根未变化也重算面板（绑定可能随光标移动切换）。
    pub fn set_root(&mut self, root: Option<PathBuf>, text: &str, cursor: usize) {
        let previous = self.effective_root(text, cursor);
        self.root = root;
        let current = self.effective_root(text, cursor);
        if current != previous {
            self.cache = None;
            self.pending = false;
            self.generation = self.generation.wrapping_add(1);
            // 绑定切换是新上下文：解除 Esc/确认后的压制。
            self.suppressed = false;
        }
        self.refresh(text, cursor);
    }

    /// 面板压制标记：Esc 关闭或确认插入后不再重新弹出，直到绑定切换或显式清除。
    pub fn suppress(&mut self) {
        self.suppressed = true;
    }

    /// 显式解除压制（编辑/移动后由调用方清除）。
    pub fn unsuppress(&mut self) {
        self.suppressed = false;
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

    /// 当前扫描根：块内绑定优先，其次显式根/工作区 cwd；绑定不可解析时 None。
    fn effective_root(&self, text: &str, cursor: usize) -> Option<PathBuf> {
        match self.block_binding(text, cursor) {
            Some(WorktreeBinding::Bound(candidate)) => Some(candidate.path),
            Some(WorktreeBinding::Missing(_) | WorktreeBinding::Ambiguous(_)) => None,
            Some(WorktreeBinding::None) | None => self.root.clone(),
        }
    }

    /// 重算面板：触发或缓存缺失时隐藏，同起点且同目录范围保留高亮。
    ///
    /// 进入/回退目录会改变 query 的目录范围，此时高亮回到首项；同一层内继续过滤保持高亮。
    pub(super) fn refresh(&mut self, text: &str, cursor: usize) {
        if self.suppressed {
            self.panel = None;
            return;
        }
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
}

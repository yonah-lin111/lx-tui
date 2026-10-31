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

/// @ 面板取根选择：绑定 linked worktree 时先选主 checkout 或绑定 checkout。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MentionRootChoice {
    Main,
    Worktree,
}

/// @ 命令面板顶栏 Tab。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MentionTab {
    #[default]
    Files,
    RefDir,
    Skills,
    Mcp,
}

impl MentionTab {
    pub const ALL: [Self; 4] = [Self::Files, Self::RefDir, Self::Skills, Self::Mcp];

    /// 由顶栏 Tab 下标（0..4）解析。
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn next(self) -> Self {
        match self {
            Self::Files => Self::RefDir,
            Self::RefDir => Self::Skills,
            Self::Skills => Self::Mcp,
            Self::Mcp => Self::Files,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Files => Self::Mcp,
            Self::RefDir => Self::Files,
            Self::Skills => Self::RefDir,
            Self::Mcp => Self::Skills,
        }
    }
}

/// `@` 面板 `MCP` Tab 的候选条目（纯数据，由事件层按生效配置同步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpMentionEntry {
    pub name: String,
    pub detail: String,
}

impl McpMentionEntry {
    pub fn new(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            detail: detail.into(),
        }
    }
}

/// 文件提及面板状态：面板、扫描根、缓存、在途代号、高亮与 worktree 绑定。
#[derive(Debug, Default)]
pub struct MentionState {
    panel: Option<MentionPanel>,
    /// 显式绑定根（prompt 底栏 `ws:` 或工作区 cwd）；未绑定时 None。
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
    /// @ 面板取根选择（绑定 linked worktree 时先选 main / wt）。
    choice: Option<MentionRootChoice>,
    /// 取根选择器高亮索引（0 = main，1 = wt）；选择器未显示时为 None。
    chooser: Option<usize>,
    /// 选择取根时的块绑定签名；绑定变化时选择复位。
    choice_binding: Option<(usize, String)>,
    /// 当前活跃 Tab：Files / refDir / Skills / MCP。
    tab: MentionTab,
    /// 引用文件夹列表快照（由上层同步注入）。
    referenced_folders: Vec<crate::app::referenced::ReferencedFolder>,
    /// 当前可用 skill 列表快照（由上层同步注入）。
    skills: Vec<crate::skills::SkillEntry>,
    /// 当前可用 MCP server 列表快照（由上层同步注入）。
    mcp_servers: Vec<McpMentionEntry>,
    /// 非 Files Tab（refDir / Skills / MCP）列表的高亮索引。
    ref_active: usize,
    /// 非 Files Tab 列表的窗口锚点索引。
    ref_anchor: usize,
    /// 非 Files Tab 显式视口起点。
    ref_viewport: Option<usize>,
    /// 当前触发区间与查询词。
    trigger: Option<markdown::MentionTrigger>,
}

impl MentionRootChoice {
    /// 全部选项；面板按此顺序展示（0 = main，1 = wt）。
    pub const ALL: [Self; 2] = [Self::Main, Self::Worktree];

    /// 条目与面板标题后缀文案。
    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Worktree => "wt",
        }
    }
}

impl MentionState {
    /// 当前面板；未打开时为 None。
    pub fn panel(&self) -> Option<&MentionPanel> {
        self.panel.as_ref()
    }

    /// 取根选择器高亮索引；选择器未显示时为 None。
    pub fn chooser(&self) -> Option<usize> {
        self.chooser
    }

    /// 当前活跃 Tab。
    pub fn tab(&self) -> MentionTab {
        self.tab
    }

    /// 当前触发信息。
    pub fn trigger(&self) -> Option<&markdown::MentionTrigger> {
        self.trigger.as_ref()
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

    /// 同步引用文件夹快照。
    pub fn sync_referenced_folders(
        &mut self,
        folders: &[crate::app::referenced::ReferencedFolder],
    ) {
        self.referenced_folders = folders.to_vec();
    }

    /// 同步 skill 快照。
    pub fn set_skills(&mut self, skills: Vec<crate::skills::SkillEntry>) {
        self.skills = skills;
    }

    /// 同步 MCP server 快照。
    pub fn set_mcp_servers(&mut self, servers: Vec<McpMentionEntry>) {
        self.mcp_servers = servers;
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

    /// 获取基于当前触发词过滤后的已启用 skill 列表。
    pub fn filtered_skills(&self) -> Vec<&crate::skills::SkillEntry> {
        let query = self
            .trigger
            .as_ref()
            .map(|t| t.query.as_str())
            .unwrap_or("");
        let query_lower = query.to_lowercase();
        self.skills
            .iter()
            .filter(|skill| skill.enabled && skill.shadowed_by.is_none())
            .filter(|skill| {
                query_lower.is_empty()
                    || skill.name.to_lowercase().contains(&query_lower)
                    || skill.description.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    /// 获取基于当前触发词过滤后的 MCP server 列表。
    pub fn filtered_mcp_servers(&self) -> Vec<&McpMentionEntry> {
        let query = self
            .trigger
            .as_ref()
            .map(|t| t.query.as_str())
            .unwrap_or("");
        let query_lower = query.to_lowercase();
        self.mcp_servers
            .iter()
            .filter(|server| {
                query_lower.is_empty()
                    || server.name.to_lowercase().contains(&query_lower)
                    || server.detail.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    fn secondary_count(&self) -> usize {
        match self.tab {
            MentionTab::Files => 0,
            MentionTab::RefDir => self.filtered_ref_folders().len(),
            MentionTab::Skills => self.filtered_skills().len(),
            MentionTab::Mcp => self.filtered_mcp_servers().len(),
        }
    }

    /// 扫描是否在途且尚未取得缓存。
    pub fn is_loading(&self) -> bool {
        self.pending && self.cache.is_none()
    }

    /// 切换 Tab：循环左右切换；未处于可见状态返回 false。
    pub fn switch_tab(&mut self, delta: isize) -> bool {
        if !self.view_open() {
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
    pub fn set_tab(&mut self, tab: MentionTab) -> bool {
        if !self.view_open() || self.tab == tab {
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
        self.chooser = None;
        self.choice = None;
        self.choice_binding = None;
        self.trigger = None;
        self.tab = MentionTab::Files;
        self.ref_active = 0;
        self.ref_anchor = 0;
        self.ref_viewport = None;
    }

    /// 选择器或文件面板是否可见。
    pub fn view_open(&self) -> bool {
        self.trigger.is_some() && !self.suppressed
    }

    /// 面板标题后缀：绑定 linked worktree 且已选定取根时为 `main` / `wt`。
    pub fn root_label(&self, text: &str, cursor: usize) -> Option<&'static str> {
        self.root_choices(text, cursor)?;
        self.choice.map(MentionRootChoice::label)
    }

    /// 绑定块需要取根选择时返回（主 checkout, 绑定 checkout）；否则 None。
    fn root_choices(&self, text: &str, cursor: usize) -> Option<(PathBuf, PathBuf)> {
        let Some(WorktreeBinding::Bound(candidate)) = self.block_binding(text, cursor) else {
            return None;
        };
        let main = self.worktree_repo.clone()?;
        (main != candidate.path).then_some((main, candidate.path))
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

    /// 需要新扫描时返回（代号，扫描根列表）并标记处理中；仅事件循环调用。
    pub fn take_scan_request(&mut self, text: &str, cursor: usize) -> Option<(u64, PathBuf)> {
        if self.pending {
            return None;
        }
        let root = self.effective_root(text, cursor)?;
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
    /// 仅在候选未加载（新开选择器）时发起：结果到达后保持，避免持续重查把
    /// 用户在选择器里的移动反复重置。
    pub fn request_worktree_refresh(&mut self) -> Option<PathBuf> {
        let repo = self.worktree_repo.clone()?;
        if self.worktree_pending || self.worktree_candidates.is_some() {
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

    /// 取根选择器确认：记录选择、作废旧候选并转为文件面板；返回是否消费。
    pub fn choose_root(&mut self, text: &str, cursor: usize) -> bool {
        let Some(active) = self.chooser else {
            return false;
        };
        self.choice = Some(if active == 0 {
            MentionRootChoice::Main
        } else {
            MentionRootChoice::Worktree
        });
        self.choice_binding = markdown::block_worktree_branch(text, cursor);
        self.chooser = None;
        self.cache = None;
        self.pending = false;
        self.generation = self.generation.wrapping_add(1);
        self.refresh(text, cursor);
        true
    }

    /// 按偏移循环移动高亮；面板未打开返回 false。
    pub fn move_active(&mut self, delta: isize) -> bool {
        if self.tab != MentionTab::Files {
            let count = self.secondary_count();
            if count == 0 {
                return false;
            }
            let next = (self.ref_active as isize + delta).rem_euclid(count as isize) as usize;
            self.ref_active = next;
            self.ref_anchor = next;
            self.ref_viewport = None;
            return true;
        }
        if let Some(active) = self.chooser {
            let next = (active as isize + delta).rem_euclid(2) as usize;
            self.chooser = Some(next);
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

    /// 设置高亮索引；面板未打开、索引越界或未变化返回 false。
    pub fn set_active(&mut self, index: usize) -> bool {
        if self.tab != MentionTab::Files {
            let count = self.secondary_count();
            if index >= count || self.ref_active == index {
                return false;
            }
            self.ref_active = index;
            return true;
        }
        if let Some(active) = self.chooser {
            if index >= MentionRootChoice::ALL.len() || index == active {
                return false;
            }
            self.chooser = Some(index);
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

    /// 滚轮滚动可见窗口：显式视口从 `base` 起偏移、越界钳制不循环；高亮不动；
    /// 面板未打开或视口未移动返回 false。
    pub fn scroll_viewport(&mut self, delta: isize, base: usize) -> bool {
        if self.tab != MentionTab::Files {
            let count = self.secondary_count();
            if count == 0 {
                return false;
            }
            let max = count.saturating_sub(1) as isize;
            let clamped = (base.min(max as usize) as isize + delta).clamp(0, max) as usize;
            let changed = clamped != base;
            self.ref_viewport = Some(clamped);
            return changed;
        }
        match self.panel.as_mut() {
            Some(panel) => panel.scroll_viewport(delta, base),
            None => false,
        }
    }

    /// 确认高亮条目：关闭面板并返回替换区间与插入文本；未打开或区间越界返回 None。
    pub fn confirm(&mut self, text_len: usize) -> Option<(Range<usize>, String)> {
        if self.tab != MentionTab::Files {
            let trigger = self.trigger.as_ref()?;
            if trigger.from > trigger.to || trigger.to > text_len {
                return None;
            }
            let range = trigger.from..trigger.to;
            let ref_text = match self.tab {
                MentionTab::RefDir => {
                    let folder = *self.filtered_ref_folders().get(self.ref_active)?;
                    crate::app::markdown::format_reference(
                        crate::app::markdown::ReferKind::Folder,
                        &folder.path,
                    )
                    .unwrap_or_else(|| format!("@[refer-folder]({})", folder.path.display()))
                }
                MentionTab::Skills => {
                    let skill = *self.filtered_skills().get(self.ref_active)?;
                    crate::app::markdown::format_reference(
                        crate::app::markdown::ReferKind::Skill,
                        &skill.path,
                    )
                    .unwrap_or_else(|| format!("@[refer-skill]({})", skill.path.display()))
                }
                MentionTab::Mcp => {
                    let server = *self.filtered_mcp_servers().get(self.ref_active)?;
                    crate::app::markdown::format_reference(
                        crate::app::markdown::ReferKind::Mcp,
                        std::path::Path::new(&server.name),
                    )
                    .unwrap_or_else(|| format!("@[refer-mcp]({})", server.name))
                }
                MentionTab::Files => unreachable!(),
            };
            let insertion = format!("{ref_text} ");
            self.reset_session();
            return Some((range, insertion));
        }
        let panel = self.panel.as_ref()?;
        let trigger = panel.trigger();
        if trigger.from > trigger.to || trigger.to > text_len {
            return None;
        }
        let entry = panel.items().get(panel.active())?.clone();
        let range = trigger.from..trigger.to;
        let insertion = markdown::mention_insertion(&entry);
        self.reset_session();
        Some((range, insertion))
    }

    /// 进入高亮目录：返回触发区间与写回文本（`@根相对路径/`，保持提及触发不闭合）。
    ///
    /// 高亮项不是目录、目录无缓存后代（进入后将无候选）、触发区间越界时返回 None。
    pub fn enter_folder(&self, text_len: usize) -> Option<(Range<usize>, String)> {
        if self.tab != MentionTab::Files {
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

    /// 扫描缓存中是否存在该目录的后代；无子项目录不可进入，避免面板因零候选消失。
    fn has_children(&self, path: &str) -> bool {
        let prefix = format!("{path}/");
        self.cache
            .as_ref()
            .is_some_and(|cache| cache.iter().any(|entry| entry.path.starts_with(&prefix)))
    }

    /// 关闭面板且不改文本；返回是否消费该按键。
    pub fn escape(&mut self) -> bool {
        if !self.view_open() {
            return false;
        }
        self.reset_session();
        true
    }

    /// 当前扫描缓存；未扫描时为 None（快捷候选的目录判定用）。
    pub fn entries(&self) -> &[MentionEntry] {
        self.cache.as_deref().unwrap_or_default()
    }

    /// 失焦或折叠时清空面板并作废在途结果。
    pub fn clear(&mut self) {
        self.reset_session();
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

    /// 当前扫描根：绑定 linked worktree 且已选定取根时为该 checkout（main 为
    /// 仓库主 checkout，wt 为绑定 checkout）；绑定主 checkout 或未绑定时为单根；
    /// 选择器未决或绑定不可解析时 None。
    fn effective_root(&self, text: &str, cursor: usize) -> Option<PathBuf> {
        match self.block_binding(text, cursor) {
            Some(WorktreeBinding::Bound(candidate)) => {
                let Some((main, bound)) = self.root_choices(text, cursor) else {
                    return Some(candidate.path);
                };
                match self.choice? {
                    MentionRootChoice::Main => Some(main),
                    MentionRootChoice::Worktree => Some(bound),
                }
            }
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
            self.chooser = None;
            self.trigger = None;
            return;
        }
        let Some(trigger) = markdown::mention_trigger(text, cursor) else {
            // 触发消失即会话结束：取根选择复位。
            self.reset_session();
            return;
        };
        let from_changed = self.trigger.as_ref().map(|t| t.from) != Some(trigger.from);
        let query_changed =
            self.trigger.as_ref().map(|t| t.query.as_str()) != Some(trigger.query.as_str());
        if from_changed {
            self.tab = MentionTab::Files;
            self.ref_active = 0;
            self.ref_anchor = 0;
            self.ref_viewport = None;
        } else if query_changed {
            self.ref_active = 0;
            self.ref_anchor = 0;
            self.ref_viewport = None;
        }
        self.trigger = Some(trigger.clone());
        let filtered_count = self.secondary_count();
        if self.ref_active >= filtered_count {
            self.ref_active = 0;
            self.ref_anchor = 0;
            self.ref_viewport = None;
        }
        if self.ref_viewport.is_some_and(|vp| vp >= filtered_count) {
            self.ref_viewport = None;
        }
        // 绑定签名变化（换块、改分支、脱离块）才复位取根选择；签名随会话记录，
        // 同一会话内的重算不得复位：事件层每次按键都会先同步根并走到这里，
        // 否则选择器高亮每次都被打回 main（键盘选 wt 按回车写成 main）。
        let binding = markdown::block_worktree_branch(text, cursor);
        if self.choice_binding.as_ref() != binding.as_ref() {
            if self.choice.take().is_some() {
                self.cache = None;
                self.pending = false;
                self.generation = self.generation.wrapping_add(1);
            }
            self.chooser = None;
            self.choice_binding = binding;
        }
        let requiring_choice = self.root_choices(text, cursor).is_some() && self.choice.is_none();
        if requiring_choice {
            // 绑定 linked worktree：先选 main / wt，再列文件候选。
            self.chooser = Some(self.chooser.unwrap_or(0).min(1));
            self.panel = None;
            return;
        }
        self.chooser = None;
        let Some(cache) = self.cache.as_ref() else {
            self.panel = None;
            return;
        };
        let items = markdown::filter_mentions(cache, &trigger.query);
        if items.is_empty() {
            self.panel = None;
            return;
        }
        // 触发起点或过滤词变化时，对齐 codex 重置高亮至首项 (0)；未变时保留原高亮。
        let active = if from_changed || query_changed {
            0
        } else {
            let previous = self.panel.as_ref().filter(|panel| {
                panel.trigger().from == trigger.from
                    && markdown::mention_scope(&panel.trigger().query)
                        == markdown::mention_scope(&trigger.query)
            });
            previous
                .map_or(0, MentionPanel::active)
                .min(items.len() - 1)
        };
        self.panel = Some(MentionPanel::new(trigger, items, active));
    }
}

#[cfg(test)]
mod tests;

//! lx 输入框 `@` 提及面板状态：有效根、扫描缓存、异步代号、过滤面板、
//! 五 Tab（Files/refDir/Skills/MCP/Subagents）与压制。
//!
//! 纯状态：不扫描文件、不发任务、不读进程；有效根由事件层从 `pane.cwd` 或工作区 cwd
//! 计算后传入。候选始终相对本次扫描根，根变化即作废旧候选与在途结果。

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::app::markdown::{
    self, MentionEntry, MentionPanel, filter_mentions, mention_insertion, mention_trigger,
};
pub use crate::app::prompt::{McpMentionEntry, MentionTab};

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

/// `@` 面板 `Subagents` Tab 中的存活实例快照（由当前窗格 `agents.runs` 同步）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentRunMentionEntry {
    /// 稳定 agent_id。
    pub agent_id: String,
    /// 可选昵称（如 `"Turing"`）。
    pub nickname: Option<String>,
    /// 角色名（如 `"explorer"`）。
    pub role: String,
    /// 活动状态（`"running"` / `"idle"` / `"blocked"`）。
    pub status: String,
    /// 任务短标题。
    pub title: String,
}

impl SubagentRunMentionEntry {
    /// 引用标记中使用的目标标识：优先昵称，无昵称回退 `agent_id`。
    pub fn target_key(&self) -> &str {
        self.nickname
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(self.agent_id.as_str())
    }

    /// 首行展示标签：`<nickname_or_id> [<role>]`。
    pub fn display_label(&self) -> String {
        format!("{} [{}]", self.target_key(), self.role)
    }

    /// 确认后写入 `@[refer-subagent](...)` 括号内的目标串：`<nickname_or_id>:<role>`。
    pub fn reference_target(&self) -> String {
        format!("{}:{}", self.target_key(), self.role)
    }
}

/// `@` 面板 `Subagents` Tab 归一化后的可展示候选（存活实例 / 内置角色 / 自定义角色）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentMentionItem<'a> {
    /// 当前会话存活的 subagent 实例（`running` / `idle` / `blocked`）。
    Active(&'a SubagentRunMentionEntry),
    /// 内置或自定义角色规格。
    Role(&'a crate::config::subagents::ResolvedRoleSpec),
}

impl SubagentMentionItem<'_> {
    /// 确认选择时写入 `@[refer-subagent](<target>)` 的 `<target>`。
    pub fn reference_target(&self) -> String {
        match self {
            Self::Active(run) => run.reference_target(),
            Self::Role(role) => role.name.clone(),
        }
    }

    /// 查询词匹配（大小写不敏感）。
    pub fn matches_query(&self, query_lower: &str) -> bool {
        if query_lower.is_empty() {
            return true;
        }
        match self {
            Self::Active(run) => {
                run.target_key().to_lowercase().contains(query_lower)
                    || run.agent_id.to_lowercase().contains(query_lower)
                    || run.role.to_lowercase().contains(query_lower)
                    || run.status.to_lowercase().contains(query_lower)
                    || run.title.to_lowercase().contains(query_lower)
                    || "active".contains(query_lower)
            }
            Self::Role(role) => {
                let scope_tag = if role.is_builtin {
                    "built-in builtin"
                } else if role.is_project_scoped {
                    "custom project"
                } else {
                    "custom user"
                };
                role.name.to_lowercase().contains(query_lower)
                    || role.base_role.as_str().contains(query_lower)
                    || role.description.to_lowercase().contains(query_lower)
                    || scope_tag.contains(query_lower)
            }
        }
    }
}

/// lx 提及面板状态：扫描根、缓存、在途代号、过滤面板、Tab 与压制标记。
#[derive(Debug)]
pub struct LxMention {
    /// 当前有效扫描根（`pane.cwd` 优先，回退工作区 cwd）；None 表示无可扫描根。
    root: Option<PathBuf>,
    /// 最近一次扫描结果；None 表示尚未取得。
    cache: Option<Vec<MentionEntry>>,
    /// 当前可用 skill；由事件层在 skill 目录扫描完成后写入。
    skills: Vec<crate::skills::SkillEntry>,
    /// 当前可用 MCP server；由事件层在 MCP 配置初始化/刷新后写入。
    mcp_servers: Vec<McpMentionEntry>,
    /// 当前窗格手动禁用的 MCP server 名称集合（过滤 `@` MCP 候选）。
    disabled_mcp_servers: std::collections::BTreeSet<String>,
    /// 当前可用 subagent 角色列表（内置 + 自定义）；默认预填 3 个内置角色。
    subagent_roles: Vec<crate::config::subagents::ResolvedRoleSpec>,
    /// 当前会话存活的 subagent 运行实例列表（仅 `running` / `idle` / `blocked`）。
    subagent_runs: Vec<SubagentRunMentionEntry>,
    /// 扫描代号：根变化或新一轮扫描递增，用于丢弃过期回执。
    generation: u64,
    /// 扫描是否在途。
    pending: bool,
    /// 过滤后的候选面板；无候选或未扫描时为 None。
    panel: Option<MentionPanel>,
    /// Esc/确认后压制自动重开，直到下一次编辑或根变化。
    suppressed: bool,
    /// 当前活跃 Tab：Files / refDir / Skills / MCP / Subagents。
    tab: MentionTab,
    /// 引用文件夹列表快照（由上层同步注入）。
    referenced_folders: Vec<crate::app::referenced::ReferencedFolder>,
    /// 非 Files Tab（refDir / Skills / MCP / Subagents）列表的高亮索引。
    ref_active: usize,
    /// 非 Files Tab 列表的窗口锚点索引。
    ref_anchor: usize,
    /// 非 Files Tab 显式视口起点。
    ref_viewport: Option<usize>,
    /// 当前触发区间与查询词。
    trigger: Option<markdown::MentionTrigger>,
}

impl Default for LxMention {
    fn default() -> Self {
        Self {
            root: None,
            cache: None,
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            disabled_mcp_servers: std::collections::BTreeSet::new(),
            subagent_roles: crate::config::subagents::SubagentsSection::default().all_roles(),
            subagent_runs: Vec::new(),
            generation: 0,
            pending: false,
            panel: None,
            suppressed: false,
            tab: MentionTab::default(),
            referenced_folders: Vec::new(),
            ref_active: 0,
            ref_anchor: 0,
            ref_viewport: None,
            trigger: None,
        }
    }
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

    /// 非 Files Tab 的高亮索引。
    pub fn ref_active(&self) -> usize {
        self.ref_active
    }

    /// 非 Files Tab 的窗口锚点。
    pub fn ref_anchor(&self) -> usize {
        self.ref_anchor
    }

    /// 非 Files Tab 的显式视口。
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
            .filter(|server| !self.disabled_mcp_servers.contains(&server.name))
            .filter(|server| {
                query_lower.is_empty()
                    || server.name.to_lowercase().contains(&query_lower)
                    || server.detail.to_lowercase().contains(&query_lower)
            })
            .collect()
    }

    /// 获取基于当前触发词过滤后的 subagent 候选：存活实例 → 内置角色 →
    /// 项目级自定义角色 → 用户级自定义角色。
    pub fn filtered_subagents(&self) -> Vec<SubagentMentionItem<'_>> {
        let query_lower = self
            .trigger
            .as_ref()
            .map(|t| t.query.to_lowercase())
            .unwrap_or_default();
        let active_iter = self.subagent_runs.iter().map(SubagentMentionItem::Active);
        let builtin_iter = self
            .subagent_roles
            .iter()
            .filter(|role| role.is_builtin)
            .map(SubagentMentionItem::Role);
        let project_custom_iter = self
            .subagent_roles
            .iter()
            .filter(|role| !role.is_builtin && role.is_project_scoped)
            .map(SubagentMentionItem::Role);
        let user_custom_iter = self
            .subagent_roles
            .iter()
            .filter(|role| !role.is_builtin && !role.is_project_scoped)
            .map(SubagentMentionItem::Role);
        active_iter
            .chain(builtin_iter)
            .chain(project_custom_iter)
            .chain(user_custom_iter)
            .filter(|item| item.matches_query(&query_lower))
            .collect()
    }

    fn secondary_count(&self) -> usize {
        match self.tab {
            MentionTab::Files => 0,
            MentionTab::RefDir => self.filtered_ref_folders().len(),
            MentionTab::Skills => self.filtered_skills().len(),
            MentionTab::Mcp => self.filtered_mcp_servers().len(),
            MentionTab::Subagents => self.filtered_subagents().len(),
        }
    }

    /// 切换 Tab：在给定 Tab 集合内循环；未处于可见状态返回 false。
    pub fn switch_tab(
        &mut self,
        delta: isize,
        text: &str,
        cursor: usize,
        tabs: &[MentionTab],
    ) -> bool {
        if tabs.len() < 2 || self.view(text, cursor) == LxMentionView::Hidden {
            return false;
        }
        let index = tabs.iter().position(|tab| *tab == self.tab).unwrap_or(0) as isize;
        let len = tabs.len() as isize;
        let next = tabs[(index + delta).rem_euclid(len) as usize];
        if next == self.tab {
            return false;
        }
        self.tab = next;
        self.ref_active = 0;
        self.ref_anchor = 0;
        self.ref_viewport = None;
        true
    }

    /// 在给定 Tab 集合内直接设置活跃 Tab；不在集合内返回 false。
    pub fn set_tab(
        &mut self,
        tab: MentionTab,
        text: &str,
        cursor: usize,
        tabs: &[MentionTab],
    ) -> bool {
        if !tabs.contains(&tab)
            || self.view(text, cursor) == LxMentionView::Hidden
            || self.tab == tab
        {
            return false;
        }
        self.tab = tab;
        self.ref_active = 0;
        self.ref_anchor = 0;
        self.ref_viewport = None;
        true
    }

    /// 无视面板当前是否可见，强制重置活跃 Tab（用于切换至 AI Web 时归位到 `Files`）。
    pub fn force_tab(&mut self, tab: MentionTab) {
        self.tab = tab;
        self.ref_active = 0;
        self.ref_anchor = 0;
        self.ref_viewport = None;
    }

    /// 当前 `Files` 面板高亮条目是否为目录。
    pub fn active_is_directory(&self) -> bool {
        self.tab == MentionTab::Files
            && self
                .panel
                .as_ref()
                .and_then(|panel| panel.items().get(panel.active()))
                .is_some_and(|entry| entry.is_directory)
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
            MentionTab::RefDir | MentionTab::Skills | MentionTab::Mcp | MentionTab::Subagents => {
                self.trigger.is_some() && !self.suppressed && self.secondary_count() > 0
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
            && self.skills.is_empty()
            && self.mcp_servers.is_empty()
            && self.subagent_roles.is_empty()
            && self.subagent_runs.is_empty()
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
            MentionTab::RefDir | MentionTab::Skills | MentionTab::Mcp | MentionTab::Subagents => {
                if self.secondary_count() == 0 {
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

    /// 需要新扫描时返回（代号，扫描根）并标记在途；仅 `Files` Tab 生效，
    /// 已有缓存或在途时返回 None。
    pub fn take_scan_request(&mut self, text: &str, cursor: usize) -> Option<(u64, PathBuf)> {
        if self.tab != MentionTab::Files || self.pending || self.cache.is_some() {
            return None;
        }
        let root = self.root.clone()?;
        mention_trigger(text, cursor)?;
        self.pending = true;
        Some((self.generation, root))
    }

    /// 结果因归属切换被丢弃时清掉在途标记；不写缓存，原编辑器下次可重新扫描。
    pub fn abandon_scan(&mut self, generation: u64, root: &Path) {
        if self.pending && self.generation == generation && self.root.as_deref() == Some(root) {
            self.pending = false;
        }
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

    /// 键盘或悬停移动高亮；面板未打开返回 false。
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
        if self.tab != MentionTab::Files {
            let count = self.secondary_count();
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

    /// 确认高亮条目：关闭面板、进入压制，返回替换区间与插入文本；无面板或越界返回 None。
    ///
    /// `plain = true` 时只写入朴素文本（Agents 追问），不生成 `@[refer-*]` 标记；
    /// 标记语法在子 agent 侧没有消费方，写 marker 只会把丑文本转嫁给模型。
    pub fn confirm(&mut self, text_len: usize, plain: bool) -> Option<(Range<usize>, String)> {
        if self.tab != MentionTab::Files {
            let trigger = self.trigger.as_ref()?;
            if trigger.from > trigger.to || trigger.to > text_len {
                return None;
            }
            let range = trigger.from..trigger.to;
            let ref_text = match self.tab {
                MentionTab::RefDir => {
                    let folder = *self.filtered_ref_folders().get(self.ref_active)?;
                    if plain {
                        format!("@{}", folder.path.display())
                    } else {
                        crate::app::markdown::format_reference(
                            crate::app::markdown::ReferKind::Folder,
                            &folder.path,
                        )
                        .unwrap_or_else(|| format!("@[refer-folder]({})", folder.path.display()))
                    }
                }
                MentionTab::Skills => {
                    let skill = *self.filtered_skills().get(self.ref_active)?;
                    if plain {
                        format!("@{}", skill.name)
                    } else {
                        crate::app::markdown::format_reference(
                            crate::app::markdown::ReferKind::Skill,
                            &skill.path,
                        )
                        .unwrap_or_else(|| format!("@[refer-skill]({})", skill.path.display()))
                    }
                }
                MentionTab::Mcp => {
                    let server = *self.filtered_mcp_servers().get(self.ref_active)?;
                    if plain {
                        format!("@{}", server.name)
                    } else {
                        crate::app::markdown::format_reference(
                            crate::app::markdown::ReferKind::Mcp,
                            Path::new(&server.name),
                        )
                        .unwrap_or_else(|| format!("@[refer-mcp]({})", server.name))
                    }
                }
                MentionTab::Subagents => {
                    let item = *self.filtered_subagents().get(self.ref_active)?;
                    let target = item.reference_target();
                    if plain {
                        format!("@{target}")
                    } else {
                        crate::app::markdown::format_reference(
                            crate::app::markdown::ReferKind::Subagent,
                            Path::new(&target),
                        )
                        .unwrap_or_else(|| format!("@[refer-subagent]({target})"))
                    }
                }
                MentionTab::Files => unreachable!(),
            };
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
                    || !self.skills.is_empty()
                    || !self.mcp_servers.is_empty()
                    || self.tab != MentionTab::Files));
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

impl LxMention {
    /// 当前可用 skill 快照；分割窗格时复制给新窗格。
    pub fn skills(&self) -> &[crate::skills::SkillEntry] {
        &self.skills
    }

    /// 写入当前可用 skill；有打开的 `@` 面板时立即重算候选。
    pub fn set_skills(
        &mut self,
        skills: Vec<crate::skills::SkillEntry>,
        text: &str,
        cursor: usize,
    ) {
        if self.skills == skills {
            return;
        }
        self.skills = skills;
        self.refresh(text, cursor);
    }

    /// 当前可用 MCP server 快照；分割窗格时复制给新窗格。
    pub fn mcp_servers(&self) -> &[McpMentionEntry] {
        &self.mcp_servers
    }

    /// 当前已禁用的 MCP server 集合；Agents composer 同步上下文时复制。
    pub fn disabled_mcp_servers(&self) -> &std::collections::BTreeSet<String> {
        &self.disabled_mcp_servers
    }

    /// 写入当前可用 MCP server；有打开的 `@` 面板时立即重算候选。
    pub fn set_mcp_servers(&mut self, servers: Vec<McpMentionEntry>, text: &str, cursor: usize) {
        if self.mcp_servers == servers {
            return;
        }
        self.mcp_servers = servers;
        self.refresh(text, cursor);
    }

    /// 同步本窗格已禁用的 MCP server 集合；有打开的 `@` 面板时立即重算候选。
    pub fn set_disabled_mcp_servers(
        &mut self,
        disabled: std::collections::BTreeSet<String>,
        text: &str,
        cursor: usize,
    ) {
        if self.disabled_mcp_servers == disabled {
            return;
        }
        self.disabled_mcp_servers = disabled;
        self.refresh(text, cursor);
    }

    /// 当前引用文件夹快照；新建窗格/标签时复制给新窗格。
    pub fn referenced_folders(&self) -> &[crate::app::referenced::ReferencedFolder] {
        &self.referenced_folders
    }

    /// 当前可用 subagent 角色快照；分割窗格时复制给新窗格。
    pub fn subagent_roles(&self) -> &[crate::config::subagents::ResolvedRoleSpec] {
        &self.subagent_roles
    }

    /// 写入当前可用 subagent 角色；有打开的 `@` 面板时立即重算候选。
    pub fn set_subagent_roles(
        &mut self,
        roles: Vec<crate::config::subagents::ResolvedRoleSpec>,
        text: &str,
        cursor: usize,
    ) {
        self.subagent_roles = roles;
        self.refresh(text, cursor);
    }

    /// 当前会话存活 subagent 快照。
    pub fn subagent_runs(&self) -> &[SubagentRunMentionEntry] {
        &self.subagent_runs
    }

    /// 同步存活 subagent 快照；内容变化时重算候选并返回 true。
    pub fn sync_subagent_runs(
        &mut self,
        runs: Vec<SubagentRunMentionEntry>,
        text: &str,
        cursor: usize,
    ) -> bool {
        if self.subagent_runs == runs {
            return false;
        }
        self.subagent_runs = runs;
        self.refresh(text, cursor);
        true
    }
}

#[cfg(test)]
mod tests;

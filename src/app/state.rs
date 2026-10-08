//! 应用状态：唯一状态来源，纯数据，可在无终端环境下构造与测试。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::layout::{Direction, Rect};

use crate::layout::{BoundaryHit, PaneId, TileLayout};
use crate::terminal::Terminal;

use super::overlay::Overlay;
use super::prompt::Prompt;
use super::selection::Selection;
use super::toast::Toast;

/// 新建窗格的初始网格尺寸；首帧后由真实几何覆盖。
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// 全局 prompt 右栏的兜底初始宽度（列，含边框）；
/// 生产启动时按主区可用宽度的一半覆盖，之后可拖拽。
const DEFAULT_PROMPT_WIDTH: u16 = 30;

/// 侧栏展开宽度的兜底初值（列）；生产启动时由配置覆盖。
const DEFAULT_SIDEBAR_WIDTH: u16 = 24;

/// 选区拖拽边缘自动滚动方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoscrollDirection {
    Up,
    Down,
}

/// 选区拖拽边缘自动滚动计划。
///
/// 鼠标停在（或越出）窗格上下边缘时登记；`inner` 与 `mouse` 为登记时的几何快照，
/// 几何变化时调用方需停止滚动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionAutoscroll {
    pub pane: PaneId,
    pub direction: AutoscrollDirection,
    pub mouse: (u16, u16),
    pub inner: Rect,
    pub next_at: Instant,
}

/// 顶层层级：工作区包含标签，标签包含 BSP 窗格树与窗格终端；prompt 为全局右栏。
#[derive(Debug)]
pub struct AppState {
    pub should_quit: bool,
    pub sidebar_collapsed: bool,
    pub agents_collapsed: bool,
    /// 侧栏展开宽度（列）；启动时取配置值，可拖拽。
    pub sidebar_width: u16,
    /// 侧栏分割线是否正在拖拽。
    pub resizing_sidebar: bool,
    /// 侧栏分割线是否悬停。
    pub sidebar_hover: bool,
    /// 正在拖拽的主区窗格分割线（两侧窗格与方向）。
    pub resizing_pane: Option<BoundaryHit>,
    /// 悬停的主区窗格分割线。
    pub pane_hover: Option<BoundaryHit>,
    pub toast: Option<Toast>,
    /// prompt 等编辑器的视口选区。
    pub selection: Option<Selection>,
    /// 正在拖拽选择的终端窗格；选中内容存于仿真器内部（内容坐标）。
    pub terminal_selection: Option<PaneId>,
    /// 选区拖拽边缘自动滚动计划；None 表示未激活。
    pub selection_autoscroll: Option<SelectionAutoscroll>,
    pub resizing_prompt: bool,
    pub prompt_hover: bool,
    pub prompt_collapsed: bool,
    /// prompt 是否持有键盘焦点；为真时按键进入编辑器而非焦点窗格。
    pub prompt_focused: bool,
    pub prompt: Prompt,
    /// prompt 显式绑定的上下文根路径；None 时回退活动工作区 cwd。
    pub prompt_root: Option<PathBuf>,
    pub prompt_width: u16,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
    /// 标签栏滚动偏移（首个可见标签索引）；仅作用于当前工作区。
    pub tab_scroll: usize,
    /// 鼠标悬停的可见标签索引；None 表示未悬停任何标签。
    pub tab_hover: Option<usize>,
    /// 工作区列表滚动偏移（顶部项索引）。
    pub workspace_scroll: usize,
    /// 鼠标悬停的工作区索引（仅可见行）；None 表示未悬停任何行。
    pub workspace_hover: Option<usize>,
    /// 拖拽滚动条 thumb 时相对顶部的抓取偏移。
    pub workspace_scroll_drag: Option<u16>,
    /// 拖拽 prompt 滚动条 thumb 时相对顶部的抓取偏移。
    pub prompt_scroll_drag: Option<u16>,
    /// 拖拽终端窗格滚动条 thumb：窗格与相对顶部的抓取偏移。
    pub terminal_scroll_drag: Option<(PaneId, u16)>,
    /// lx 页动画相位（200ms 一帧）；lx 页不可见时冻结。
    pub lx_phase: u64,
    /// 上次 lx 动画推进时刻。
    pub lx_last_tick: Instant,
    /// 正在拖动排序的工作区当前索引；None 表示未拖拽。
    pub workspace_drag: Option<usize>,
    /// 拖拽是否已产生指针移动；按下未移动时不整块反显。
    pub workspace_dragging: bool,
    /// 同一时刻最多一个浮层：右键菜单、重命名、关闭确认或 worktree 对话框。
    pub overlay: Option<Overlay>,
    /// 待查询 git 元数据的工作区 cwd 队列；事件层取走并在后台执行。
    pub git_requests: Vec<PathBuf>,
    /// 已折叠的工作区分组（仓库根路径）；仅存内存，重启恢复展开。
    pub collapsed_groups: Vec<PathBuf>,
}

/// 工作区。
#[derive(Debug)]
pub struct Workspace {
    pub name: String,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    /// 用户是否手动改过名；为真时 cwd 变化不再自动改名。
    pub name_is_manual: bool,
    /// 驱动命名的窗格 cwd（自动命名跟踪用）。
    pub cwd: Option<PathBuf>,
    /// 是否为启动时创建的工作区；列表项显示不可移除的 `*` 标记。
    pub is_initial: bool,
    /// 所属 git checkout 的元数据；非仓库或尚未查询到时为 None。
    pub git: Option<WorkspaceGit>,
}

/// 工作区所属 git checkout 的只读元数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceGit {
    /// 仓库主 checkout 路径（同一仓库的分组键）。
    pub repo_root: PathBuf,
    /// 本工作区 cwd 所在 checkout 路径。
    pub checkout_path: PathBuf,
    /// 是否为 linked worktree（非主 checkout）。
    pub is_linked: bool,
    /// 当前分支短名；detached 或 bare 为 None。
    pub branch: Option<String>,
    /// 仓库主 checkout 的分支短名；linked worktree 的状态栏展示用。
    pub main_branch: Option<String>,
}

impl WorkspaceGit {
    /// 展示用分支名：去掉 `worktree/` 前缀；detached 或 bare 为 None。
    pub fn short_branch(&self) -> Option<&str> {
        self.branch
            .as_deref()
            .map(|branch| branch.strip_prefix("worktree/").unwrap_or(branch))
    }

    /// 状态栏展示分支：linked worktree 取主 checkout 分支（回退本 checkout 分支），
    /// 主 checkout 取自身分支；去掉 `worktree/` 前缀。
    pub fn status_branch(&self) -> Option<&str> {
        if self.is_linked
            && let Some(main) = self.main_branch.as_deref()
        {
            return Some(main.strip_prefix("worktree/").unwrap_or(main));
        }
        self.short_branch()
    }
}

/// 侧栏工作区列表的可见行；折叠的组只保留父项与当前激活子项。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceRow {
    /// 工作区索引。
    pub index: usize,
    /// 分组子项：缩进并显示分支短名。
    pub child: bool,
    /// 分组父项：显示折叠箭头。
    pub parent: bool,
    /// 父项所在组是否已折叠。
    pub collapsed: bool,
    /// 子项标签与组内更早子项重复时的去重序号（2、3…）；无重复为 None。
    pub child_index: Option<u32>,
}

impl Workspace {
    /// 单窗格终端工作区；新建工作区使用，初始为自动命名。
    pub(crate) fn single_terminal(name: String, cwd: Option<PathBuf>) -> Self {
        Self {
            name,
            tabs: vec![Tab::single_terminal()],
            active_tab: 0,
            name_is_manual: false,
            cwd,
            is_initial: false,
            git: None,
        }
    }

    /// 身份窗格：第一个标签的根窗格，工作区命名跟随它的 cwd。
    pub fn root_pane(&self) -> Option<PaneId> {
        self.tabs.first().and_then(|tab| tab.root_pane())
    }
}

/// 标签页：布局树与窗格载荷一一对应；未重命名时按位置显示自动标题。
#[derive(Debug)]
pub struct Tab {
    pub name: Option<String>,
    pub layout: TileLayout,
    panes: BTreeMap<PaneId, Pane>,
}

/// 窗格种类：终端运行 PTY，placeholder 为空占位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    Placeholder,
}

/// 窗格主内容视图：lx 欢迎页与终端网格互斥。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneView {
    /// lx 欢迎页（默认视图）。
    Lx,
    /// 终端网格。
    Terminal,
}

/// 窗格载荷：种类、主内容视图、终端仿真状态与退出标记。
#[derive(Debug)]
pub struct Pane {
    pub kind: PaneKind,
    pub view: PaneView,
    pub terminal: Terminal,
    pub exited: bool,
    /// 窗格 shell 进程 cwd 的显示标签；标题无 OSC 时回退展示。
    pub cwd_label: Option<String>,
    /// 窗格 shell 进程最近一次轮询到的 cwd；新建工作区取焦点窗格路径用。
    pub cwd: Option<PathBuf>,
}

impl Tab {
    fn with_layout(name: Option<String>, layout: TileLayout) -> Self {
        let panes = layout
            .pane_ids()
            .into_iter()
            .map(|id| (id, Pane::new()))
            .collect();
        Self {
            name,
            layout,
            panes,
        }
    }

    /// 单窗格终端标签；初始未重命名，按位置显示自动标题。
    pub(crate) fn single_terminal() -> Self {
        Self::with_layout(None, TileLayout::new())
    }

    /// 根窗格：树序第一个窗格，工作区身份跟随它。
    pub fn root_pane(&self) -> Option<PaneId> {
        self.layout.pane_ids().into_iter().next()
    }

    /// 在指定窗格处按方向分割：新窗格继承源窗格视图并获得焦点；源窗格不存在返回 None。
    pub fn split_pane(&mut self, source: PaneId, direction: Direction) -> Option<PaneId> {
        let view = self.pane(source)?.view;
        if !self.layout.focus_pane(source) {
            return None;
        }
        let id = self.layout.split_focused(direction, 0.5);
        let mut pane = Pane::new();
        pane.view = view;
        self.panes.insert(id, pane);
        Some(id)
    }

    /// 移除窗格：布局删叶提兄弟并同步移除载荷；仅剩一个窗格或窗格不存在返回 false。
    pub fn remove_pane(&mut self, id: PaneId) -> bool {
        if self.panes.len() <= 1 || !self.layout.remove_pane(id) {
            return false;
        }
        self.panes.remove(&id);
        true
    }

    /// 按标识取窗格。
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.get(&id)
    }

    /// 按标识取窗格（可变）。
    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.panes.get_mut(&id)
    }
}

impl Pane {
    fn new() -> Self {
        Self {
            kind: PaneKind::Terminal,
            view: PaneView::Lx,
            terminal: Terminal::new(DEFAULT_COLS, DEFAULT_ROWS),
            exited: false,
            cwd_label: None,
            cwd: None,
        }
    }
}

/// 自动工作区名：HOME 显示 `~`，否则取路径末段；根路径等无末段时显示完整路径。
///
/// 与 herdr 的 `fallback_label_from_cwd` 对齐（HOME 特判 + file_name 回退 display）。
pub fn workspace_label(cwd: &Path, home: Option<&Path>) -> String {
    if home.is_some_and(|home| cwd == home) {
        return "~".to_string();
    }
    cwd.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| cwd.display().to_string())
}

/// 工作区自动名：当前进程工作目录的标签；读不到 cwd 时回退 `workspace`。
pub fn workspace_name() -> String {
    current_workspace_identity().1
}

/// 当前进程 cwd 与其自动工作区名；读不到 cwd 时 cwd 为 None。
pub fn current_workspace_identity() -> (Option<PathBuf>, String) {
    match std::env::current_dir() {
        Ok(path) => {
            let name = workspace_label(&path, home_dir().as_deref());
            (Some(path), name)
        }
        // 读不到 cwd 属于极端环境问题，工作区名兜底即可，不阻断启动。
        Err(_) => (None, FALLBACK_WORKSPACE_NAME.to_string()),
    }
}

/// HOME 环境变量；缺失或为空时 None。
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// 当前路径无最后一段且显示为空时的兜底工作区名。
const FALLBACK_WORKSPACE_NAME: &str = "workspace";

/// 标签标题：重命名后显示自定义名，否则按位置显示 `tab N`。
pub fn tab_label(index: usize, name: Option<&str>) -> String {
    match name {
        Some(name) if !name.trim().is_empty() => name.to_string(),
        _ => format!("tab {}", index.saturating_add(1)),
    }
}

/// 去重命名：`base` 已被占用时追加最小未用序号（`base 2`、`base 3`…）。
///
/// `taken` 由调用方给出占用判定（排除自身）。
pub fn unique_workspace_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let mut suffix: u32 = 2;
    loop {
        let candidate = format!("{base} {suffix}");
        if !taken(&candidate) {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

impl AppState {
    /// 构造初始状态：单个工作区（以当前路径末段命名），单个自动命名终端标签，
    /// 加全局 prompt 右栏。
    pub fn demo() -> Self {
        let (cwd, name) = current_workspace_identity();
        Self {
            should_quit: false,
            sidebar_collapsed: false,
            agents_collapsed: false,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            resizing_sidebar: false,
            sidebar_hover: false,
            resizing_pane: None,
            pane_hover: None,
            toast: None,
            selection: None,
            terminal_selection: None,
            selection_autoscroll: None,
            resizing_prompt: false,
            prompt_hover: false,
            prompt_collapsed: false,
            prompt_focused: false,
            prompt: Prompt::new(PaneId::alloc()),
            prompt_root: None,
            prompt_width: DEFAULT_PROMPT_WIDTH,
            workspaces: vec![Workspace {
                name,
                tabs: vec![Tab::single_terminal()],
                active_tab: 0,
                name_is_manual: false,
                cwd,
                is_initial: true,
                git: None,
            }],
            active_workspace: 0,
            tab_scroll: 0,
            tab_hover: None,
            workspace_scroll: 0,
            workspace_hover: None,
            workspace_scroll_drag: None,
            prompt_scroll_drag: None,
            terminal_scroll_drag: None,
            lx_phase: 0,
            lx_last_tick: Instant::now(),
            workspace_drag: None,
            workspace_dragging: false,
            overlay: None,
            git_requests: Vec::new(),
            collapsed_groups: Vec::new(),
        }
    }

    /// 当前工作区。
    pub fn active_workspace(&self) -> &Workspace {
        &self.workspaces[self.active_workspace]
    }

    /// 当前工作区（可变）。
    pub fn active_workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active_workspace]
    }

    /// 当前标签。
    pub fn active_tab(&self) -> &Tab {
        let workspace = self.active_workspace();
        &workspace.tabs[workspace.active_tab]
    }

    /// 当前标签（可变）。
    pub fn active_tab_mut(&mut self) -> &mut Tab {
        let index = self.active_workspace().active_tab;
        &mut self.active_workspace_mut().tabs[index]
    }

    /// 当前焦点窗格。
    pub fn active_pane(&self) -> Option<&Pane> {
        let tab = self.active_tab();
        tab.pane(tab.layout.focus())
    }

    /// 任意工作区/标签中的窗格；PTY 装配与只读查询使用。
    pub fn pane_anywhere(&self, id: PaneId) -> Option<&Pane> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .find_map(|tab| tab.pane(id))
    }

    /// 窗格所属工作区的工作目录；PTY 启动目录与 git 查询依据。
    pub fn workspace_cwd_for_pane(&self, id: PaneId) -> Option<PathBuf> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.tabs.iter().any(|tab| tab.pane(id).is_some()))
            .and_then(|workspace| workspace.cwd.clone())
    }

    /// 指定窗格上的选区。
    pub fn selection_for(&self, pane: PaneId) -> Option<&Selection> {
        self.selection
            .as_ref()
            .filter(|selection| selection.pane() == pane)
    }

    /// 全选 prompt 文本：按视觉行数与文本区宽度构造覆盖全文的选区；
    /// 文本为空时构造空选区（无高亮、无脏状态）。
    pub fn select_all_prompt(&mut self) {
        let rows = self.prompt.visual_rows().len();
        let (width, _) = self.prompt.size();
        let max_row = rows.saturating_sub(1) as i32;
        let max_col = if self.prompt.text().is_empty() {
            0
        } else {
            width.saturating_sub(1)
        };
        self.selection = Some(Selection::full(self.prompt.id(), max_row, max_col));
    }

    /// 任意工作区/标签中的窗格（可变）；PTY 输出按窗格标识投递。
    pub fn pane_mut_anywhere(&mut self, id: PaneId) -> Option<&mut Pane> {
        for workspace in &mut self.workspaces {
            for tab in &mut workspace.tabs {
                if let Some(pane) = tab.pane_mut(id) {
                    return Some(pane);
                }
            }
        }
        None
    }

    /// 全部可运行 PTY 的窗格标识；prompt 编辑器不启动进程，不在其中。
    pub fn all_pane_ids(&self) -> Vec<PaneId> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .flat_map(|tab| tab.layout.pane_ids())
            .collect()
    }

    /// 当前标签是否存在 lx 视图窗格；动画推进与定时唤醒的依据。
    pub fn lx_visible(&self) -> bool {
        self.active_tab()
            .panes
            .values()
            .any(|pane| pane.view == PaneView::Lx)
    }
    /// 侧栏可见行：同仓库 ≥1 个 linked worktree 且含非 linked 主项时成组，
    /// 根之后（索引更大）的 linked 项为子项；重复主 checkout 与非组项为独立顶层行。
    /// 折叠的组只保留父项与当前激活子项。
    ///
    /// linked 子项标签与组内更早子项重复时给出显示去重序号（含被折叠隐藏的子项，序号稳定）。
    /// 列表保持工作区原始顺序，不做父项前置重排；渲染、命中与滚动共用此结果。
    pub fn workspace_rows(&self) -> Vec<WorkspaceRow> {
        let parents = self.group_parents();
        let collapsed = |key: &Path| {
            self.collapsed_groups
                .iter()
                .any(|group| group.as_path() == key)
        };
        let mut child_counts: HashMap<(&Path, &str), u32> = HashMap::new();
        let mut rows = Vec::with_capacity(self.workspaces.len());
        for (index, workspace) in self.workspaces.iter().enumerate() {
            let git = workspace.git.as_ref();
            let group = git.and_then(|git| {
                let parent = parents.get(git.repo_root.as_path())?;
                Some((git.repo_root.as_path(), *parent))
            });
            match group {
                Some((key, parent)) if parent == index => {
                    rows.push(WorkspaceRow {
                        index,
                        child: false,
                        parent: true,
                        collapsed: collapsed(key),
                        child_index: None,
                    });
                }
                Some((key, parent)) if index > parent && git.is_some_and(|git| git.is_linked) => {
                    let child_index = if workspace.name_is_manual {
                        None
                    } else {
                        match git.and_then(WorkspaceGit::short_branch) {
                            Some(label) => {
                                let count = child_counts.entry((key, label)).or_insert(0);
                                *count += 1;
                                (*count > 1).then_some(*count)
                            }
                            None => None,
                        }
                    };
                    if !collapsed(key) || self.active_workspace == index {
                        rows.push(WorkspaceRow {
                            index,
                            child: true,
                            parent: false,
                            collapsed: false,
                            child_index,
                        });
                    }
                }
                _ => rows.push(WorkspaceRow {
                    index,
                    child: false,
                    parent: false,
                    collapsed: false,
                    child_index: None,
                }),
            }
        }
        rows
    }

    /// 已成立分组的根索引：仓库根 → 创建最早的非 linked 主项（根窗格标识随创建递增，
    /// 拖拽换位不改变）；无主项或无 linked worktree 成员时不成组（重复主 checkout 平铺）。
    fn group_parents(&self) -> BTreeMap<&Path, usize> {
        let mut members: BTreeMap<&Path, Vec<usize>> = BTreeMap::new();
        for (index, workspace) in self.workspaces.iter().enumerate() {
            if let Some(git) = workspace.git.as_ref() {
                members
                    .entry(git.repo_root.as_path())
                    .or_default()
                    .push(index);
            }
        }
        members
            .into_iter()
            .filter_map(|(key, indices)| {
                if !indices
                    .iter()
                    .any(|index| self.workspace_git(*index).is_some_and(|git| git.is_linked))
                {
                    return None;
                }
                let parent = indices
                    .iter()
                    .copied()
                    .filter(|index| self.workspace_git(*index).is_some_and(|git| !git.is_linked))
                    .min_by_key(|index| self.workspace_created_key(*index))?;
                Some((key, parent))
            })
            .collect()
    }

    /// 工作区 git 元数据。
    fn workspace_git(&self, index: usize) -> Option<&WorkspaceGit> {
        self.workspaces
            .get(index)
            .and_then(|workspace| workspace.git.as_ref())
    }

    /// 创建序键：根窗格标识随创建单调递增；无窗格时排最后。
    fn workspace_created_key(&self, index: usize) -> u32 {
        self.workspaces
            .get(index)
            .and_then(Workspace::root_pane)
            .map(PaneId::raw)
            .unwrap_or(u32::MAX)
    }

    /// 拖拽块：树的根与 linked 子项整体移动；重复主 checkout 等独立项返回自身。
    ///
    /// 折叠隐藏的子项在块内；根按创建序固定，拖拽换位不会改变树的归属。
    pub fn workspace_block(&self, index: usize) -> Vec<usize> {
        let Some(git) = self.workspace_git(index) else {
            return vec![index];
        };
        let Some(parent) = self.group_parents().get(git.repo_root.as_path()).copied() else {
            return vec![index];
        };
        if index != parent && !git.is_linked {
            return vec![index];
        }
        let block: Vec<usize> = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(other, workspace)| {
                workspace.git.as_ref().is_some_and(|other_git| {
                    other_git.repo_root == git.repo_root
                        && (*other == parent || (*other > parent && other_git.is_linked))
                })
            })
            .map(|(other, _)| other)
            .collect();
        if block.contains(&index) {
            block
        } else {
            vec![index]
        }
    }

    /// 当前工作区焦点终端窗格的实时 cwd；非终端或尚未轮询到时 None。
    pub fn active_terminal_cwd(&self) -> Option<PathBuf> {
        self.active_pane()
            .filter(|pane| pane.kind == PaneKind::Terminal)
            .and_then(|pane| pane.cwd.clone())
    }
}

#[cfg(test)]
mod tests;

//! 应用状态：唯一状态来源，纯数据，可在无终端环境下构造与测试。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::layout::{Direction, Rect};

use crate::layout::{PaneId, TileLayout};
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
    pub prompt_width: u16,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
    /// 标签栏滚动偏移（首个可见标签索引）；仅作用于当前工作区。
    pub tab_scroll: usize,
    /// 工作区列表滚动偏移（顶部项索引）。
    pub workspace_scroll: usize,
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
    /// 同一时刻最多一个浮层：右键菜单、重命名或关闭确认。
    pub overlay: Option<Overlay>,
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
            toast: None,
            selection: None,
            terminal_selection: None,
            selection_autoscroll: None,
            resizing_prompt: false,
            prompt_hover: false,
            prompt_collapsed: false,
            prompt_focused: false,
            prompt: Prompt::new(PaneId::alloc()),
            prompt_width: DEFAULT_PROMPT_WIDTH,
            workspaces: vec![Workspace {
                name,
                tabs: vec![Tab::single_terminal()],
                active_tab: 0,
                name_is_manual: false,
                cwd,
                is_initial: true,
            }],
            active_workspace: 0,
            tab_scroll: 0,
            workspace_scroll: 0,
            workspace_scroll_drag: None,
            prompt_scroll_drag: None,
            terminal_scroll_drag: None,
            lx_phase: 0,
            lx_last_tick: Instant::now(),
            workspace_drag: None,
            overlay: None,
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

    /// 指定窗格上的选区。
    pub fn selection_for(&self, pane: PaneId) -> Option<&Selection> {
        self.selection
            .as_ref()
            .filter(|selection| selection.pane() == pane)
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
}

#[cfg(test)]
mod tests;

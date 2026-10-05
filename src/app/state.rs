//! 应用状态：唯一状态来源，纯数据，可在无终端环境下构造与测试。

use std::collections::BTreeMap;

use crate::layout::{PaneId, TileLayout};
use crate::terminal::Terminal;

use super::selection::Selection;
use super::toast::Toast;

/// 新建窗格的初始网格尺寸；首帧后由真实几何覆盖。
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// 全局 prompt 右栏的兜底初始宽度（列，含边框）；
/// 生产启动时按主区可用宽度的一半覆盖，之后可拖拽。
const DEFAULT_PROMPT_WIDTH: u16 = 30;

/// prompt 占位窗格的演示文本（承载可选择的静态内容，行宽适配窄窗格）。
const DEMO_PROMPT: &str = concat!(
    "Drag to select, release to copy.\r\n",
    "拖拽选中这段文字，松开即复制。\r\n",
    "Line 3: mixed ASCII 与宽字符。"
);

/// 顶层层级：工作区包含标签，标签包含 BSP 窗格树与窗格终端；prompt 为全局右栏。
#[derive(Debug)]
pub struct AppState {
    pub should_quit: bool,
    pub sidebar_collapsed: bool,
    pub agents_collapsed: bool,
    pub toast: Option<Toast>,
    pub selection: Option<Selection>,
    pub resizing_prompt: bool,
    pub prompt_hover: bool,
    pub prompt_collapsed: bool,
    pub prompt: Prompt,
    pub prompt_width: u16,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
}

/// 全局 prompt 面板：右侧固定区域，所有标签与工作区共享。
#[derive(Debug)]
pub struct Prompt {
    id: PaneId,
    pane: Pane,
}

impl Prompt {
    fn new(id: PaneId) -> Self {
        let mut pane = Pane::new();
        pane.kind = PaneKind::Prompt;
        let _ = pane.terminal.feed(DEMO_PROMPT.as_bytes());
        Self { id, pane }
    }

    /// 面板标识；文本选择按此标识归属。
    pub fn id(&self) -> PaneId {
        self.id
    }

    /// 面板载荷。
    pub fn pane(&self) -> &Pane {
        &self.pane
    }

    /// 面板载荷（可变）。
    pub fn pane_mut(&mut self) -> &mut Pane {
        &mut self.pane
    }
}

/// 工作区。
#[derive(Debug)]
pub struct Workspace {
    pub name: String,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

/// 标签页：布局树与窗格载荷一一对应。
#[derive(Debug)]
pub struct Tab {
    pub title: String,
    pub layout: TileLayout,
    panes: BTreeMap<PaneId, Pane>,
}

/// 窗格种类：终端运行 PTY，placeholder 为空占位，prompt 为占位面板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    Placeholder,
    Prompt,
}

/// 窗格载荷：种类、终端仿真状态与退出标记。
#[derive(Debug)]
pub struct Pane {
    pub kind: PaneKind,
    pub terminal: Terminal,
    pub exited: bool,
}

impl Tab {
    fn with_layout(title: &str, layout: TileLayout) -> Self {
        let panes = layout
            .pane_ids()
            .into_iter()
            .map(|id| (id, Pane::new()))
            .collect();
        Self {
            title: title.to_string(),
            layout,
            panes,
        }
    }

    /// 单窗格终端标签。
    fn single_terminal(title: &str) -> Self {
        Self::with_layout(title, TileLayout::new())
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
            terminal: Terminal::new(DEFAULT_COLS, DEFAULT_ROWS),
            exited: false,
        }
    }
}

/// 当前路径无最后一段（根路径）或不可用时的兜底工作区名。
const FALLBACK_WORKSPACE_NAME: &str = "workspace";

/// 工作区名：当前进程工作路径的最后一段；解析失败回退常量。
fn workspace_name() -> String {
    match std::env::current_dir() {
        Ok(path) => workspace_name_from(&path),
        // 读不到 cwd 属于极端环境问题，工作区名兜底即可，不阻断启动。
        Err(_) => FALLBACK_WORKSPACE_NAME.to_string(),
    }
}

/// 路径的最后一段；根路径、空段或非 UTF-8 名称回退常量。
fn workspace_name_from(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(FALLBACK_WORKSPACE_NAME)
        .to_string()
}

impl AppState {
    /// 构造初始状态：单个工作区（以当前路径末段命名），shell 与 logs 两个终端标签，
    /// 加全局 prompt 右栏。
    pub fn demo() -> Self {
        Self {
            should_quit: false,
            sidebar_collapsed: false,
            agents_collapsed: false,
            toast: None,
            selection: None,
            resizing_prompt: false,
            prompt_hover: false,
            prompt_collapsed: false,
            prompt: Prompt::new(PaneId::alloc()),
            prompt_width: DEFAULT_PROMPT_WIDTH,
            workspaces: vec![Workspace {
                name: workspace_name(),
                tabs: vec![Tab::single_terminal("shell"), Tab::single_terminal("logs")],
                active_tab: 0,
            }],
            active_workspace: 0,
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

    /// 任意工作区/标签或全局 prompt 右栏中的窗格；PTY 装配与只读查询使用。
    pub fn pane_anywhere(&self, id: PaneId) -> Option<&Pane> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .find_map(|tab| tab.pane(id))
            .or_else(|| (id == self.prompt.id()).then(|| self.prompt.pane()))
    }

    /// 指定窗格上的选区范围（左上 -> 右下），供渲染高亮使用。
    pub fn selection_range(&self, pane: PaneId) -> Option<((u16, u16), (u16, u16))> {
        self.selection_for(pane).and_then(Selection::range)
    }

    /// 指定窗格上的选区。
    pub fn selection_for(&self, pane: PaneId) -> Option<&Selection> {
        self.selection
            .as_ref()
            .filter(|selection| selection.pane() == pane)
    }

    /// 任意工作区/标签或全局 prompt 右栏中的窗格（可变）；PTY 输出按窗格标识投递。
    pub fn pane_mut_anywhere(&mut self, id: PaneId) -> Option<&mut Pane> {
        for workspace in &mut self.workspaces {
            for tab in &mut workspace.tabs {
                if let Some(pane) = tab.pane_mut(id) {
                    return Some(pane);
                }
            }
        }
        if id == self.prompt.id() {
            return Some(self.prompt.pane_mut());
        }
        None
    }

    /// 全部窗格标识（含全局 prompt），用于启动时装配 PTY。
    pub fn all_pane_ids(&self) -> Vec<PaneId> {
        let ids = self
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .flat_map(|tab| tab.layout.pane_ids());
        ids.chain(std::iter::once(self.prompt.id())).collect()
    }
}

#[cfg(test)]
mod tests;

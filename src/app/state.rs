//! 应用状态：唯一状态来源，纯数据，可在无终端环境下构造与测试。

use std::collections::BTreeMap;

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
    /// prompt 是否持有键盘焦点；为真时按键进入编辑器而非焦点窗格。
    pub prompt_focused: bool,
    pub prompt: Prompt,
    pub prompt_width: u16,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
    /// 同一时刻最多一个浮层：右键菜单、重命名或关闭确认。
    pub overlay: Option<Overlay>,
    /// 新建工作区的下一个编号；单调递增不回收。
    pub next_workspace_number: u32,
}

/// 工作区。
#[derive(Debug)]
pub struct Workspace {
    pub name: String,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

impl Workspace {
    /// 单窗格终端工作区；新建工作区使用。
    pub(crate) fn single_terminal(name: String) -> Self {
        Self {
            name,
            tabs: vec![Tab::single_terminal("shell")],
            active_tab: 0,
        }
    }
}

/// 标签页：布局树与窗格载荷一一对应。
#[derive(Debug)]
pub struct Tab {
    pub title: String,
    pub layout: TileLayout,
    panes: BTreeMap<PaneId, Pane>,
}

/// 窗格种类：终端运行 PTY，placeholder 为空占位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    Placeholder,
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
    pub(crate) fn single_terminal(title: &str) -> Self {
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
            prompt_focused: false,
            prompt: Prompt::new(PaneId::alloc()),
            prompt_width: DEFAULT_PROMPT_WIDTH,
            workspaces: vec![Workspace {
                name: workspace_name(),
                tabs: vec![Tab::single_terminal("shell"), Tab::single_terminal("logs")],
                active_tab: 0,
            }],
            active_workspace: 0,
            overlay: None,
            next_workspace_number: 2,
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
}

#[cfg(test)]
mod tests;

//! 应用状态：唯一状态来源，纯数据，可在无终端环境下构造与测试。

use std::collections::BTreeMap;

use ratatui::layout::Direction;

use crate::layout::{PaneId, TileLayout};
use crate::terminal::Terminal;

/// 新建窗格的初始网格尺寸；首帧后由真实几何覆盖。
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// 应用界面模式；同一时刻只处于一个模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Help,
}

/// 顶层层级：工作区包含标签，标签包含 BSP 窗格树与窗格终端。
#[derive(Debug)]
pub struct AppState {
    pub should_quit: bool,
    pub mode: Mode,
    pub sidebar_collapsed: bool,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
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

/// 窗格载荷：终端仿真状态与退出标记。
#[derive(Debug)]
pub struct Pane {
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

    /// 演示用：左侧一窗格，右侧上下两窗格。
    fn demo_split(title: &str) -> Self {
        let mut layout = TileLayout::new();
        layout.split_focused(Direction::Horizontal, 0.5);
        layout.split_focused(Direction::Vertical, 0.6);
        Self::with_layout(title, layout)
    }

    /// 演示用：单窗格标签。
    fn demo_single(title: &str) -> Self {
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
            terminal: Terminal::new(DEFAULT_COLS, DEFAULT_ROWS),
            exited: false,
        }
    }
}

impl AppState {
    /// 构造演示状态：两个工作区，含多窗格标签；窗格终端为空。
    pub fn demo() -> Self {
        Self {
            should_quit: false,
            mode: Mode::Normal,
            sidebar_collapsed: false,
            workspaces: vec![
                Workspace {
                    name: "main".to_string(),
                    tabs: vec![Tab::demo_split("shell"), Tab::demo_single("logs")],
                    active_tab: 0,
                },
                Workspace {
                    name: "notes".to_string(),
                    tabs: vec![Tab::demo_single("notes")],
                    active_tab: 0,
                },
            ],
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

    /// 任意工作区/标签中的窗格；PTY 装配与只读查询使用。
    pub fn pane_anywhere(&self, id: PaneId) -> Option<&Pane> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .find_map(|tab| tab.pane(id))
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

    /// 全部窗格标识，用于启动时装配 PTY。
    pub fn all_pane_ids(&self) -> Vec<PaneId> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .flat_map(|tab| tab.layout.pane_ids())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_state_has_expected_shape() {
        let state = AppState::demo();
        assert_eq!(state.workspaces.len(), 2);
        assert_eq!(state.active_workspace().name, "main");
        assert_eq!(state.active_tab().title, "shell");
        assert_eq!(state.active_tab().layout.pane_ids().len(), 3);
    }

    #[test]
    fn single_pane_tab_has_one_pane() {
        let state = AppState::demo();
        assert_eq!(state.workspaces[0].tabs[1].layout.pane_ids().len(), 1);
        assert_eq!(state.workspaces[1].tabs[0].layout.pane_ids().len(), 1);
    }

    #[test]
    fn every_layout_pane_has_payload() {
        let state = AppState::demo();
        assert!(state.active_pane().is_some());
        let layout_ids: usize = state
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .map(|tab| tab.layout.pane_ids().len())
            .sum();
        assert_eq!(state.all_pane_ids().len(), layout_ids);
    }
}

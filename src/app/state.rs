//! 应用状态：唯一状态来源，纯数据，可在无终端环境下构造与测试。

use std::collections::BTreeMap;

use ratatui::layout::Direction;

use crate::layout::{PaneId, TileLayout};
use crate::terminal::Terminal;

use super::selection::Selection;

/// 新建窗格的初始网格尺寸；首帧后由真实几何覆盖。
const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;

/// prompt 占位窗格的演示文本（承载可选择的静态内容，行宽适配窄窗格）。
const DEMO_PROMPT: &str = concat!(
    "Drag to select, release to copy.\r\n",
    "拖拽选中这段文字，松开即复制。\r\n",
    "Line 3: mixed ASCII 与宽字符。"
);

/// 顶层层级：工作区包含标签，标签包含 BSP 窗格树与窗格终端。
#[derive(Debug)]
pub struct AppState {
    pub should_quit: bool,
    pub sidebar_collapsed: bool,
    pub selection: Option<Selection>,
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

    /// 演示用：左侧空占位，右侧 prompt 占位；焦点默认在空占位。
    fn demo_split(title: &str) -> Self {
        let mut layout = TileLayout::new();
        let placeholder = layout.focus();
        let prompt = layout.split_focused(Direction::Horizontal, 0.5);
        layout.focus_pane(placeholder);
        let mut tab = Self::with_layout(title, layout);
        if let Some(pane) = tab.pane_mut(placeholder) {
            pane.kind = PaneKind::Placeholder;
        }
        if let Some(pane) = tab.pane_mut(prompt) {
            pane.kind = PaneKind::Prompt;
            let _ = pane.terminal.feed(DEMO_PROMPT.as_bytes());
        }
        tab
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

    /// 当前标签中的 prompt 占位窗格。
    pub fn prompt_pane(&self) -> Option<PaneId> {
        self.panes
            .iter()
            .find(|(_, pane)| pane.kind == PaneKind::Prompt)
            .map(|(id, _)| *id)
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

impl AppState {
    /// 构造演示状态：两个工作区，含多窗格标签；窗格终端为空。
    pub fn demo() -> Self {
        Self {
            should_quit: false,
            sidebar_collapsed: false,
            selection: None,
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
        assert_eq!(state.active_tab().layout.pane_ids().len(), 2);
    }

    #[test]
    fn demo_split_has_placeholder_prompt_and_focus() {
        let state = AppState::demo();
        let tab = state.active_tab();
        let focus = tab.layout.focus();
        assert!(
            tab.pane(focus)
                .is_some_and(|pane| pane.kind == PaneKind::Placeholder)
        );
        let prompt = tab.prompt_pane();
        assert!(prompt.is_some());
        assert_ne!(Some(focus), prompt);
    }

    #[test]
    fn demo_prompt_pane_has_selectable_content() {
        let mut state = AppState::demo();
        let prompt = state.active_tab().prompt_pane();
        assert!(prompt.is_some());
        let Some(prompt) = prompt else { return };
        let Some(pane) = state.active_tab_mut().pane_mut(prompt) else {
            return;
        };
        let text = pane
            .terminal
            .text_in_range((0, 0), (0, 3))
            .unwrap_or_default();
        assert_eq!(text, "Drag");
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

//! 窗格与标签页模型：BSP 布局树与窗格载荷映射。

use std::collections::BTreeMap;
use std::path::PathBuf;

pub use ratatui::layout::Direction;

use crate::detect::PaneAgentSnapshot;
use crate::layout::{PaneId, TileLayout};
use crate::terminal::Terminal;

use super::lx::Lx;

/// 新建窗格的初始网格尺寸；首帧后由真实几何覆盖。
pub const DEFAULT_COLS: u16 = 80;
pub const DEFAULT_ROWS: u16 = 24;

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
    /// lx 视图的交互状态（选中入口与输入缓冲）。
    pub lx: Lx,
    pub exited: bool,
    /// 窗格 shell 进程 cwd 的显示标签；标题无 OSC 时回退展示。
    pub cwd_label: Option<String>,
    /// 窗格 shell 进程最近一次轮询到的 cwd；新建工作区取焦点窗格路径用。
    pub cwd: Option<PathBuf>,
    /// 窗格前台进程探测到的 Agent 快照；None 表示纯 Shell 或未识别程序。
    pub agent: Option<PaneAgentSnapshot>,
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
    pub(crate) fn new() -> Self {
        Self {
            kind: PaneKind::Terminal,
            view: PaneView::Lx,
            terminal: Terminal::new(DEFAULT_COLS, DEFAULT_ROWS),
            lx: Lx::new(),
            exited: false,
            cwd_label: None,
            cwd: None,
            agent: None,
        }
    }
}

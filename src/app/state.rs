//! 应用状态：唯一状态来源，纯数据，可在无终端环境下构造与测试。

use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::layout::Rect;

use crate::layout::{BoundaryHit, PaneId};

use super::effects::Effect;
use super::overlay::Overlay;
use super::prompt::{Prompt, PromptPanel};
use super::referenced::ReferencedFolders;
use super::selection::Selection;
use super::toast::Toast;
use super::tooltip::Tooltip;
use super::trust::TrustResolution;

pub mod lx;
pub mod pane;
pub mod prompt_history;
pub mod workspace;

pub use lx::LxInputMode;
pub use lx::{
    AssistantStepView, CardSection, CardTarget, ChatMessageView, ChatPartView, CommandView,
    CompactNoticeView, DiffHunkView, DiffLineView, FileChangeView, FileDiffDetailView,
    FileDiffView, FrozenModel, HistoryEntry, HistoryPanel, InputHintLevel, InteractionAction, Lx,
    LxChat, LxCompact, LxEntry, ModelCandidate, ModelPicker, ModelPickerStage, ModelsSnapshot,
    PanelWindow, PendingShell, PlanSnapshotView, PlanStepView, ProcessEntry,
    ProcessInteractionView, ProcessStateView, QueuedSubmission, RequestKey, RequestState,
    RewindAction, RewindExecution, RewindNoticeView, RewindPanel, RewindTarget, RewindTargetKind,
    SHELL_SLOT, SlashCommand, SlashPanel, SnapshotState, TaskPage, TaskTab, ToolCallView,
    ToolDetailView, ToolStatus, ToolSummary, UserShellView,
};
pub use pane::{Direction, Pane, PaneKind, PaneView, Tab};
pub use prompt_history::{PROMPT_HISTORY_LIMIT, PROMPT_HISTORY_MAX_BYTES, PromptHistory};
pub use workspace::{
    PromptDraft, Workspace, WorkspaceGit, WorkspaceIdentity, WorkspaceRow,
    current_workspace_identity, home_dir, path_tail_two, tab_label, unique_workspace_name,
    workspace_label, workspace_name,
};

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

/// 顶层层级：工作区包含标签与挂起的 prompt 草稿，标签包含 BSP 窗格树与窗格终端；
/// prompt 面板为全局右栏，显示激活（或钉住）工作区的草稿。
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
    /// 鼠标悬停的按钮提示；到显示时刻前只计时。
    pub tooltip: Option<Tooltip>,
    /// prompt 等编辑器的视口选区。
    pub selection: Option<Selection>,
    /// 正在拖拽选择的终端窗格；选中内容存于仿真器内部（内容坐标）。
    pub terminal_selection: Option<PaneId>,
    /// lx 页文本选区（内容坐标）；松开鼠标时复制并清除。
    pub lx_selection: Option<Selection>,
    /// 正在拖拽选择的 lx 输入框所在窗格；选区锚点存于该窗格的 `Lx` 内。
    pub lx_input_drag: Option<PaneId>,
    /// 按下时落在**已展开卡片正文**上的目标：松开若未拖动（无选区范围）即折叠该块。
    ///
    /// 与 `lx_selection` 并行存在——按下正文既开始选区（允许长按拖拽复制），
    /// 又记录"这可能是一次点击"。是否折叠由松开时的选区范围决定（拖动过就不折叠）。
    pub lx_card_press: Option<(PaneId, crate::app::state::CardTarget)>,
    /// 选区拖拽边缘自动滚动计划；None 表示未激活。
    pub selection_autoscroll: Option<SelectionAutoscroll>,
    pub resizing_prompt: bool,
    pub prompt_hover: bool,
    pub prompt_collapsed: bool,
    /// 主区是否折叠为右缘窄条；仅 prompt 在左（右列为主区）时有效，交换即复位。
    pub panes_collapsed: bool,
    /// prompt 是否持有键盘焦点；为真时按键进入编辑器而非焦点窗格。
    pub prompt_focused: bool,
    /// 面板当前显示的 prompt 编辑器；其余工作区的草稿挂起在 `Workspace.prompt`。
    pub prompt: Prompt,
    /// 显示中 prompt 的显式绑定上下文根路径；None 时回退显示中工作区 cwd。
    pub prompt_root: Option<PathBuf>,
    /// 钉住的显示来源工作区索引；None 时跟随激活工作区。
    pub prompt_pinned: Option<usize>,
    pub prompt_width: u16,
    /// prompt 是否与主区窗格互换、显示在主区左侧；仅存内存，启动复位。
    pub prompt_left: bool,
    pub workspaces: Vec<Workspace>,
    pub active_workspace: usize,
    /// 标签栏滚动偏移（首个可见标签索引）；仅作用于当前工作区。
    pub tab_scroll: usize,
    /// 鼠标悬停的可见标签索引；None 表示未悬停任何标签。
    pub tab_hover: Option<usize>,
    /// 鼠标悬停的提示词列表行索引（树视图可见行）；None 表示未悬停。
    pub prompt_tree_hover: Option<usize>,
    /// 鼠标是否停留在当前右键菜单/页码菜单容器内；离开容器清除选中高亮反馈。
    pub menu_hover: bool,
    /// 工作区列表滚动偏移（顶部项索引）。
    pub workspace_scroll: usize,
    /// 鼠标悬停的工作区索引（仅可见行）；None 表示未悬停任何行。
    pub workspace_hover: Option<usize>,
    /// 拖拽滚动条 thumb 时相对顶部的抓取偏移。
    pub workspace_scroll_drag: Option<u16>,
    /// 拖拽 prompt 滚动条 thumb 时相对顶部的抓取偏移。
    pub prompt_scroll_drag: Option<u16>,
    /// 拖拽后台任务输出容器滚动条 thumb 时相对顶部的抓取偏移。
    pub task_scroll_drag: Option<u16>,
    /// prompt 树行最近一次左键点击（可见行索引、时刻）；双击判定用。
    pub prompt_row_click: Option<(usize, Instant)>,
    /// 引用文件夹目录弹窗最近一次左键点击（行下标、时刻）；双击判定用。
    pub referenced_menu_click: Option<(usize, Instant)>,
    /// 拖拽引用文件夹搜索弹窗滚动条 thumb 时相对顶部的抓取偏移。
    pub referenced_search_scroll_drag: Option<u16>,
    /// 拖拽引用文件夹目录弹窗滚动条 thumb 时相对顶部的抓取偏移。
    pub referenced_menu_scroll_drag: Option<u16>,
    /// 拖拽终端窗格滚动条 thumb：窗格与相对顶部的抓取偏移。
    pub terminal_scroll_drag: Option<(PaneId, u16)>,
    /// 拖拽 lx 输入框滚动条 thumb：窗格与相对顶部的抓取偏移。
    pub lx_scroll_drag: Option<(PaneId, u16)>,
    /// 拖拽 lx 聊天历史滚动条 thumb：窗格与相对顶部的抓取偏移。
    pub lx_history_scroll_drag: Option<(PaneId, u16)>,
    /// 鼠标悬停的侧栏 Agents 条目槽位索引（含滚动偏移）；None 表示未悬停。
    pub agent_hover: Option<usize>,
    /// 侧栏 Agents 列表滚动偏移（顶部条目索引）。
    pub agent_scroll: usize,
    /// 拖拽 Agents 滚动条 thumb 时相对顶部的抓取偏移。
    pub agent_scroll_drag: Option<u16>,
    /// 正在拖动排序的工作区当前索引；None 表示未拖拽。
    pub workspace_drag: Option<usize>,
    /// 拖拽是否已产生指针移动；按下未移动时不整块反显。
    pub workspace_dragging: bool,
    /// 同一时刻最多一个浮层：右键菜单、重命名、关闭确认或 worktree 对话框。
    pub overlay: Option<Overlay>,
    /// 待执行的效应出箱（DB 读写与剪贴板）；由持久化服务取走执行。
    pub effects: Vec<Effect>,
    /// 待查询 git 元数据的工作区 cwd 队列；事件层取走并在后台执行。
    pub git_requests: Vec<PathBuf>,
    /// 待解析仓库根的信任检查请求；事件层取走并在后台解析后回投。
    pub trust_resolutions: Vec<TrustResolution>,
    /// 显示中项目的引用文件夹快照（按 prompt 键归属；钉住时取被钉住工作区）。
    pub referenced: ReferencedFolders,
    /// 待执行的后台文件系统请求（扫描与新增路径校验）；事件层取走执行。
    pub referenced_requests: Vec<super::referenced::ReferencedRequest>,
    /// 引用文件夹请求代号生成器；每次打开/换目录/校验递增。
    pub referenced_generation: u64,
    /// 已折叠的工作区分组（仓库根路径）；仅存内存，重启恢复展开。
    pub collapsed_groups: Vec<PathBuf>,
    /// 本地设置 Web 服务是否可用；决定底边框设置按钮的可用态。
    pub settings_available: bool,
    /// 设置按钮点击请求：事件层取走后打开浏览器。
    pub settings_open_requested: bool,
    /// 模型配置快照：设置页保存后由事件层更新；发送时冻结成 `FrozenModel`。
    pub models: ModelsSnapshot,
    /// 全局已发送 prompt 历史（跨窗格/工作区/会话共享；末尾为最近使用）。
    pub prompt_history: PromptHistory,
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
            tooltip: None,
            selection: None,
            terminal_selection: None,
            lx_selection: None,
            lx_input_drag: None,
            lx_card_press: None,
            selection_autoscroll: None,
            resizing_prompt: false,
            prompt_hover: false,
            prompt_collapsed: false,
            panes_collapsed: false,
            prompt_focused: false,
            prompt: Prompt::new(PaneId::alloc()),
            prompt_root: None,
            prompt_pinned: None,
            prompt_width: DEFAULT_PROMPT_WIDTH,
            prompt_left: false,
            workspaces: vec![Workspace {
                name,
                tabs: vec![Tab::single_terminal()],
                active_tab: 0,
                name_is_manual: false,
                cwd,
                is_initial: true,
                git: None,
                prompt: None,
                prompt_panel: PromptPanel::new(),
            }],
            active_workspace: 0,
            tab_scroll: 0,
            tab_hover: None,
            prompt_tree_hover: None,
            menu_hover: false,
            workspace_scroll: 0,
            workspace_hover: None,
            workspace_scroll_drag: None,
            prompt_scroll_drag: None,
            task_scroll_drag: None,
            prompt_row_click: None,
            referenced_menu_click: None,
            referenced_search_scroll_drag: None,
            referenced_menu_scroll_drag: None,
            terminal_scroll_drag: None,
            lx_scroll_drag: None,
            lx_history_scroll_drag: None,
            agent_hover: None,
            agent_scroll: 0,
            agent_scroll_drag: None,
            workspace_drag: None,
            workspace_dragging: false,
            overlay: None,
            effects: Vec::new(),
            git_requests: Vec::new(),
            trust_resolutions: Vec::new(),
            referenced: ReferencedFolders::default(),
            referenced_requests: Vec::new(),
            referenced_generation: 0,
            collapsed_groups: Vec::new(),
            settings_available: false,
            settings_open_requested: false,
            models: ModelsSnapshot::default(),
            prompt_history: PromptHistory::new(),
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

    /// 显示中 prompt 的工作区索引：钉住时为其目标，否则为激活工作区。
    pub fn prompt_workspace(&self) -> usize {
        self.prompt_pinned.unwrap_or(self.active_workspace)
    }

    /// 显示中 prompt 所属工作区；索引由构造与切换路径维持有效。
    pub fn prompt_owner(&self) -> &Workspace {
        &self.workspaces[self.prompt_workspace()]
    }

    /// 引用文件夹的归属键：显示中 prompt 所属工作区的 prompt 键；
    /// 钉住时取被钉住工作区，切换侧栏活动工作区不改变归属。
    pub fn referenced_key(&self) -> Option<PathBuf> {
        self.workspaces
            .get(self.prompt_workspace())
            .and_then(|workspace| workspace.repo_key())
            .map(PathBuf::from)
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

    /// 当前焦点窗格（可变）；lx 输入按此写入。
    pub fn active_pane_mut(&mut self) -> Option<&mut Pane> {
        let focus = self.active_tab().layout.focus();
        self.active_tab_mut().pane_mut(focus)
    }

    /// 任意工作区/标签中的窗格；PTY 装配与只读查询使用。
    pub fn pane_anywhere(&self, id: PaneId) -> Option<&Pane> {
        self.workspaces
            .iter()
            .flat_map(|workspace| workspace.tabs.iter())
            .find_map(|tab| tab.pane(id))
    }

    /// 窗格所属工作区（任意标签内）；渲染身份标签与工作区级查询使用。
    pub fn workspace_for_pane(&self, id: PaneId) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.tabs.iter().any(|tab| tab.pane(id).is_some()))
    }

    /// 窗格所属工作区的工作目录；PTY 启动目录、git 查询与 agent 工具根依据。
    pub fn workspace_cwd_for_pane(&self, id: PaneId) -> Option<PathBuf> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.tabs.iter().any(|tab| tab.pane(id).is_some()))
            .and_then(|workspace| workspace.cwd.clone())
    }

    /// 窗格的 chat 会话存储键：git 工作区取仓库主 checkout，使同一仓库的主
    /// checkout 与 linked worktree 共用一份 `/resume` 列表；其余取工作区 cwd。
    ///
    /// **只用于 DB 键**：agent 执行 cwd 仍是 [`Self::workspace_cwd_for_pane`]，
    /// 否则在 worktree 里发的消息会把文件改到主 checkout。
    pub fn chat_session_key_for_pane(&self, id: PaneId) -> Option<PathBuf> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.tabs.iter().any(|tab| tab.pane(id).is_some()))
            .and_then(|workspace| workspace.repo_key())
            .map(Path::to_path_buf)
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

    /// 是否存在正在生成（流中）的 lx 窗格；驱动 loading 转轮。
    pub fn has_streaming_lx(&self) -> bool {
        self.all_pane_ids().into_iter().any(|pane| {
            self.pane_anywhere(pane).is_some_and(|target| {
                target.lx.chat.status.streaming || target.lx.chat.has_running_shell()
            })
        })
    }

    /// 当前激活工作区内全部窗格标识（按标签与树序）；Agent 探测扫描用。
    pub fn active_pane_ids(&self) -> Vec<PaneId> {
        self.active_workspace()
            .tabs
            .iter()
            .flat_map(|tab| tab.layout.pane_ids())
            .collect()
    }

    /// 当前工作区内探测到 Agent 的窗格（标签索引，窗格标识），按标签与树序排列；
    /// Agents 分区渲染、命中与滚动计数共用。
    pub fn agent_panes(&self) -> Vec<(usize, PaneId)> {
        let workspace = self.active_workspace();
        let mut panes = Vec::new();
        for (tab_index, tab) in workspace.tabs.iter().enumerate() {
            for id in tab.layout.pane_ids() {
                if tab.pane(id).is_some_and(|pane| pane.agent.is_some()) {
                    panes.push((tab_index, id));
                }
            }
        }
        panes
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

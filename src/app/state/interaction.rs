//! 界面交互瞬态类型：拖拽排序、选区边缘自动滚动与右栏页签状态。

use std::path::PathBuf;
use std::time::Instant;

use ratatui::layout::Rect;

use crate::tiling::PaneId;

/// 选区与列表拖拽边缘自动滚动方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoscrollDirection {
    Up,
    Down,
}

/// 选区拖拽边缘自动滚动计划（prompt 与终端窗格）。
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

/// lx 主内容区（主消息列表或后台任务页）选区拖拽的边缘自动滚动计划。
///
/// 由事件层在每次步进时实时解析可见行数、内容几何与多击粒度选区，避免使用过期快照。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LxSelectionAutoscroll {
    pub pane: PaneId,
    pub surface: LxScrollSurface,
    pub direction: AutoscrollDirection,
    /// 指针距最近边缘的行数；越靠外每 tick 滚得越多。
    pub distance: u16,
    pub mouse: (u16, u16),
    pub next_at: Instant,
}

/// lx 主内容区当前可由拖选自动滚动的视口。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LxScrollSurface {
    /// 主 agent 消息历史。
    History,
    /// 后台任务页 Processes 输出。
    TaskOutput,
    /// 后台任务页 Agents 详情。
    TaskAgents,
    /// 后台任务页 BTW 对话。
    TaskBtw,
}

/// 拖拽排序列表的边缘自动滚动计划。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragAutoscroll {
    pub surface: DragScrollSurface,
    pub direction: AutoscrollDirection,
    /// 指针距最近边缘的行数；越靠外每 tick 滚得越多。
    pub distance: u16,
    pub mouse: (u16, u16),
    pub next_at: Instant,
}

/// 支持边缘自动滚动的拖拽排序列表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragScrollSurface {
    WorkspaceActive,
    WorkspaceHistory,
    Todo,
    PromptTree,
}

/// 右侧统一容器的当前面板。
///
/// 纯视图偏好：不落库、不写配置，启动恒为 `Prompt`（与 `prompt_left` 同级）。
/// 切换工作区不改变它。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RightTab {
    #[default]
    Prompt,
    Todo,
}

/// prompt 列表拖拽起点：被拖对象的**持久身份**。
///
/// `Temp` 草稿只存内存、没有 DB id，因此不进入本枚举——它可点击但不可拖。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDragSource {
    Folder(i64),
    Item(i64),
}

/// 拖动中的落点反馈：已通过容器校验，渲染照着铺 hover 底色即可。
///
/// 只描述"会落到哪"，不含业务规则；`None` 覆盖树外、非法目标与不会产生移动的落点
/// （拖到自己、已是同一容器），避免画出一个按下去什么都不会发生的高亮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptDropMark {
    /// 无有效落点。
    None,
    /// 落到该可见行对应的容器。
    Row(usize),
    /// 落到末行以下空白，即回根层。
    Root,
}

/// 一次 prompt 列表拖拽的现场；只在 Down→Up 之间存活。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptDrag {
    /// 按下时显示中 prompt 所属工作区；Up 时与之不符即整体丢弃。
    pub workspace: usize,
    /// 被拖对象；None 表示该行可点击但不可拖（`Temp` 行）。
    pub source: Option<PromptDragSource>,
    /// 按下时的可见行索引（也是未拖动时点击的目标行）。
    pub from_row: usize,
    /// 按下的时刻：双击窗口按 Down→Down 判定，不能用松开时刻顶替。
    pub pressed_at: Instant,
    /// 指针是否已跨行（行级阈值；只横向抖动不算拖动）。
    pub dragging: bool,
    /// 当前落点反馈；未置臂或落点无效时为 `None`。
    pub drop_mark: PromptDropMark,
    /// 本次是否允许拖动（`Temp` 行、搜索过滤态为 false）。
    pub enabled: bool,
}

/// Todo 列表行拖拽合并现场。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoDrag {
    /// 按下时的源条目 ID。
    pub source_id: String,
    /// 按下时的显示下标。
    pub from_display: usize,
    /// 按下时刻。
    pub pressed_at: Instant,
    /// 是否已跨行进入拖拽态。
    pub dragging: bool,
    /// 当前悬停的可合并目标列表下标（与源同条或同组时为 None）。
    pub target_index: Option<usize>,
}

/// 持有键盘焦点的容器：同一时刻**只有一个**。
///
/// 高亮、硬件光标归属与按键去向全部由它决定，杜绝「主区与右侧容器同时描边强调色」
/// 与「光标画在别的容器输入框里」这类分裂状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusedContainer {
    /// 左侧栏（工作区 / Agents 列表）。
    Sidebar,
    /// 主区窗格（终端或 lx 页）。
    #[default]
    Panes,
    /// 右侧容器的 Prompt 面板。
    Prompt,
    /// 右侧容器的 Todo 面板。
    Todo,
}

/// 在途的 Prompt 模板块 `/title` 生成请求（保存在内存中，生成完成前不将占位符持久化入库）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPromptTitle {
    pub request_id: u64,
    pub workspace_cwd: Option<PathBuf>,
    pub workspace_index: usize,
    pub editing: crate::app::prompt::PromptEditing,
    pub page_id: crate::app::prompt::PromptPageId,
    pub block_start_line: usize,
    pub original_title: Option<String>,
}

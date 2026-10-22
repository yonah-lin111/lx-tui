//! 最新轮次撤销的纯状态：待命、最新目标、四项动作面板与执行中。
//!
//! 只保存选择与已加载数据；所有 IO 经 `Effect` 交给事件层。`ui/` 只读本状态。

use std::path::PathBuf;
use std::time::Instant;

use crate::db::chat::{FileAvailability, NoticeRecord};
use crate::layout::PaneId;

use super::slash::PanelWindow;

/// 双 Esc 待命窗口（与流中停止的 5 秒一致）。
pub const REWIND_ARM_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// 四项动作（面板顺序即 `ALL` 下标）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindAction {
    RevertAndDelete,
    DeleteOnly,
    RevertOnly,
    Cancel,
}

impl RewindAction {
    /// 面板顺序：默认第一项。
    pub const ALL: [RewindAction; 4] = [
        RewindAction::RevertAndDelete,
        RewindAction::DeleteOnly,
        RewindAction::RevertOnly,
        RewindAction::Cancel,
    ];

    /// 稳定动作值（DB / 测试共用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RevertAndDelete => "revert_and_delete",
            Self::DeleteOnly => "delete_only",
            Self::RevertOnly => "revert_only",
            Self::Cancel => "cancel",
        }
    }

    /// 是否恢复文件。
    pub fn reverts_files(self) -> bool {
        matches!(self, Self::RevertAndDelete | Self::RevertOnly)
    }

    /// 是否删除最新轮次对话。
    pub fn deletes_conversation(self) -> bool {
        matches!(self, Self::RevertAndDelete | Self::DeleteOnly)
    }

    /// 成功后是否回填用户原文（只有删对话才回填）。
    pub fn fills_draft(self) -> bool {
        self.deletes_conversation()
    }
}

/// 冻结的撤销目标：稳定 user message ID + 会话 revision + 发起窗格上下文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindTarget {
    pub session_id: String,
    pub user_message_id: String,
    pub expected_revision: u64,
    pub pane: PaneId,
    pub cwd: PathBuf,
    /// 发起时输入框内容：成功后只在与该值仍相同时回填。
    pub input_at_open: String,
    /// 用户原文（来自持久化 user message）。
    pub user_text: String,
}

/// 文件快照可用性（面板据此禁用含文件动作）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SnapshotState {
    /// 尚未查询完成。
    #[default]
    Loading,
    Managed {
        files: usize,
    },
    KnownEmpty,
    AlreadyRestored,
    Unavailable,
    Incomplete,
}

impl SnapshotState {
    /// 从 DB 可用性映射。
    pub fn from_db(availability: &FileAvailability) -> Self {
        match availability {
            FileAvailability::Ready { paths: 0 } => Self::KnownEmpty,
            FileAvailability::Ready { paths } => Self::Managed { files: *paths },
            FileAvailability::KnownEmpty => Self::KnownEmpty,
            FileAvailability::AlreadyRestored => Self::AlreadyRestored,
            FileAvailability::Unavailable => Self::Unavailable,
            FileAvailability::Incomplete => Self::Incomplete,
        }
    }

    /// 含文件动作是否可用。
    pub fn files_available(self) -> bool {
        matches!(self, Self::Managed { .. } | Self::KnownEmpty)
    }

    /// 面板文件选项禁用时的说明键。
    pub fn unavailable_reason(self) -> Option<&'static str> {
        match self {
            Self::Unavailable => Some("File snapshot unavailable"),
            Self::Incomplete => Some("File snapshot incomplete"),
            Self::Loading => Some("Checking file snapshot..."),
            _ => None,
        }
    }
}

/// 四项动作面板状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindPanel {
    pub window: PanelWindow,
    pub snapshot: SnapshotState,
}

impl RewindPanel {
    /// 以默认第一项打开。
    pub fn new(snapshot: SnapshotState) -> Self {
        Self {
            window: PanelWindow::new(0),
            snapshot,
        }
    }

    /// 高亮动作。
    pub fn active_action(&self) -> RewindAction {
        RewindAction::ALL
            .get(self.window.active)
            .copied()
            .unwrap_or(RewindAction::Cancel)
    }

    /// 该动作当前是否可选（含文件动作要求快照可用）。
    pub fn enabled(&self, action: RewindAction) -> bool {
        !action.reverts_files() || self.snapshot.files_available()
    }
}

/// 执行中的操作（唯一 operation id 防重复提交）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindExecution {
    pub operation_id: String,
    pub action: RewindAction,
    pub target: RewindTarget,
}

/// 撤销提示的展示视图（独立于消息，不进入模型上下文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewindNoticeView {
    pub id: String,
    pub sequence: i64,
    pub kind: String,
    pub restored_file_count: usize,
    pub files_already_restored: bool,
}

impl RewindNoticeView {
    /// 从持久化记录构造。
    pub fn from_record(record: &NoticeRecord) -> Self {
        Self {
            id: record.id.clone(),
            sequence: record.sequence,
            kind: record.kind.clone(),
            restored_file_count: usize::try_from(record.restored_file_count).unwrap_or(0),
            files_already_restored: record.files_already_restored,
        }
    }
}

/// 每窗格的撤销状态。
#[derive(Debug, Clone, Default)]
pub struct LxRewind {
    /// 空闲双 Esc 的待命时刻；编辑/焦点/会话变化清除。
    pub armed_at: Option<Instant>,
    /// 已选中的最新轮次目标；`Some` 即 SelectedLatest。
    pub selected: Option<RewindTarget>,
    /// 下一次取得几何时要把该用户消息滚到视口居中（只消费一次）。
    pub center_target: Option<String>,
    /// 最近一次预检得到的快照可用性（打开面板时沿用）。
    pub snapshot: SnapshotState,
    /// 四项动作面板；与 `selected` 同时存在时为 ActionPanel。
    pub panel: Option<RewindPanel>,
    /// 执行中；吞掉重复确认。
    pub executing: Option<RewindExecution>,
}

impl LxRewind {
    /// 清除待命与选择（面板由调用方按语义处理）。
    pub fn clear(&mut self) {
        self.armed_at = None;
        self.selected = None;
        self.center_target = None;
        self.panel = None;
        self.executing = None;
    }

    /// 是否正在执行（禁止发送、重复确认与草稿修改）。
    pub fn is_executing(&self) -> bool {
        self.executing.is_some()
    }
}

//! 工作区信任模型：触发场景与浮层载荷；信任判定本身由持久层服务执行。

use std::path::PathBuf;

use crate::app::overlay::WorktreeOpenEntry;
use crate::layout::PaneId;

/// 信任检查触发场景；决定“信任”后的续作与“拒绝”后的回退。
///
/// 启动进入的信任门在系统控制台完成（进入 TUI 前，见 `db::service`），不使用此浮层；
/// 本枚举只覆盖应用内部触发的信任检查。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustTrigger {
    /// 新建工作区：信任后创建，拒绝则取消创建并提示。
    CreateWorkspace { cwd: PathBuf },
    /// 终端右键同步工作区路径：信任后进入同步确认，拒绝则取消并提示。
    SyncWorkspaceCwd { workspace: usize, pane: PaneId },
    /// 打开 worktree 条目：信任后创建其工作区（必要时自动补开主 checkout），拒绝则取消并提示。
    OpenWorktree {
        entry: WorktreeOpenEntry,
        /// 条目所属仓库的主 checkout 路径。
        repo_root: PathBuf,
        /// 主 checkout 分支；linked worktree 状态栏展示用。
        main_branch: Option<String>,
    },
}

/// 信任浮层载荷：目标路径与触发场景。
///
/// 目标路径为仓库主 checkout（linked worktree 的信任归属），非 git 路径为目标自身。
#[derive(Debug)]
pub struct TrustWorkspace {
    pub path: PathBuf,
    pub trigger: TrustTrigger,
}

/// 仓库根解析请求：未信任的显式路径需先由事件层后台解析所属仓库主 checkout。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustResolution {
    pub path: PathBuf,
    pub trigger: TrustTrigger,
}

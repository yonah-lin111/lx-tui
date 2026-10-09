//! 工作区信任模型：触发场景与浮层载荷；信任判定本身由持久层服务执行。

use std::path::PathBuf;

use crate::layout::PaneId;

/// 信任检查触发场景；决定“信任”后的续作与“拒绝”后的回退。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustTrigger {
    /// 启动进入应用：拒绝则恢复终端并退出。
    Startup,
    /// 新建工作区：信任后创建，拒绝则取消创建并提示。
    CreateWorkspace { cwd: PathBuf },
    /// 终端右键同步工作区路径：信任后进入同步确认，拒绝则取消并提示。
    SyncWorkspaceCwd { workspace: usize, pane: PaneId },
}

/// 信任浮层载荷：目标路径与触发场景。
#[derive(Debug)]
pub struct TrustWorkspace {
    pub path: PathBuf,
    pub trigger: TrustTrigger,
}

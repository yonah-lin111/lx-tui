//! 连接状态与管理器事件。

use std::path::PathBuf;

/// 连接状态（设置页与启动进度用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerStatus {
    Disabled,
    Starting,
    Ready { tools: usize },
    Failed { error: String },
    NeedsLogin,
}

/// 管理器事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpEvent {
    Status {
        project: PathBuf,
        server: String,
        status: ServerStatus,
    },
    /// 首轮启动进度：已结算（Ready / Failed / NeedsLogin）/ 已启用总数。
    StartupProgress {
        project: PathBuf,
        done: usize,
        total: usize,
    },
}

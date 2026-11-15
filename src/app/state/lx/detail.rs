//! 详情会话（Agents / BTW）的共享指向与撤销计划。
//!
//! 只保存纯数据：目标解析与 IO 在事件层完成，渲染层只读。

/// 详情会话的稳定指向：Agent 用 agent_id，BTW 用 thread_id。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailTarget {
    Agent(String),
    Btw(String),
}

/// 撤销确认面板只区分文案，不携带执行数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailUndoKind {
    Agent,
    Btw,
}

/// 预检成功后冻结的撤销计划；确认后原样转成 `RewindRequest`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailUndoPlan {
    pub target: DetailTarget,
    pub workspace: std::path::PathBuf,
    pub child_session: String,
    pub user_message_id: String,
    pub expected_revision: u64,
    /// 确认后若草稿仍等于 input_at_open，则回填这段原文。
    pub input_at_open: String,
    pub user_text: String,
    pub action: crate::app::state::RewindAction,
    pub snapshot: crate::app::state::SnapshotState,
}

/// 执行中的幂等键；`TaskPage` 持有它用于迟到回执过滤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailUndoExecution {
    pub operation_id: String,
    pub target: DetailTarget,
}

impl DetailTarget {
    /// 目标是否仍是当前详情：Agents 看编辑中的 agent，BTW 看当前 QA 线程。
    pub fn is_active(&self, lx: &crate::app::state::Lx) -> bool {
        match self {
            Self::Agent(agent_id) => lx.agent_edit_id().is_some_and(|active| active == agent_id),
            Self::Btw(thread_id) => {
                lx.btw
                    .active_thread()
                    .and_then(|thread| thread.thread_id.as_deref())
                    == Some(thread_id.as_str())
            }
        }
    }
}

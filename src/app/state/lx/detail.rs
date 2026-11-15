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
pub enum DetailRewindKind {
    Agent,
    Btw,
}

/// 预检成功后冻结的撤销计划；确认后原样转成 `RewindRequest`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRewindPlan {
    pub target: DetailTarget,
    pub workspace: std::path::PathBuf,
    pub child_session: String,
    /// 普通轮次为用户消息 ID；compact 为检查点 ID。
    pub user_message_id: String,
    pub expected_revision: u64,
    /// 确认后若草稿仍等于 input_at_open，则回填这段原文。
    pub input_at_open: String,
    pub user_text: String,
    /// 用户确认的动作（打开面板时先填默认项）。
    pub action: crate::app::state::RewindAction,
    pub snapshot: crate::app::state::SnapshotState,
    /// 目标种类：普通轮次或 compact 检查点（决定动作集与执行路径）。
    pub kind: crate::app::state::RewindTargetKind,
}

/// 执行中的幂等键；`TaskPage` 持有它用于迟到回执过滤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailRewindExecution {
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

impl crate::app::state::Lx {
    /// 是否有详情会话（子 agent / BTW 线程）正在压缩。
    pub fn any_detail_compacting(&self) -> bool {
        self.agents.runs.iter().any(|run| run.compact.is_running()) || self.btw.any_compacting()
    }

    /// 刷新详情压缩耗时并推进其呼吸帧（摘要请求没有流式输出，只能靠计时与动画
    /// 证明还在跑；与主聊天 `/compact` 的 loading 行同一套反馈）。
    pub fn tick_detail_compacting(&mut self, now_ms: i64) {
        for run in &mut self.agents.runs {
            if run.compact.is_running() {
                run.compact.elapsed_ms = run
                    .compact
                    .started_at_ms
                    .map(|started| now_ms.saturating_sub(started).max(0));
                // 历史里的运行中检查点行用 `run.chat.loading_frame` 呼吸；
                // 主 loop 只给 running run 同步帧，手动压缩时 run 是空闲的。
                run.chat.loading_frame = run.chat.loading_frame.wrapping_add(1);
            }
        }
        self.btw.tick_compacting(now_ms);
    }
}

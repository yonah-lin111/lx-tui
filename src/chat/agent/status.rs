//! 子 agent 生命周期状态与投影规则（TUI 与无头 exec 共用唯一真相）。

use serde::{Deserialize, Serialize};

/// 子 agent 对外可见的统一状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// 正在执行某轮采样或工具调用。
    Running,
    /// 处于活动待命态（已显式恢复或被 `interrupt_agent` 暂停，可随时接收输入）。
    Idle,
    /// 队首输入校验失败（如图片缺失），等待取消该条输入。
    Blocked,
    /// 最新一轮正常完成且未处于挂起待命态。
    Completed,
    /// 最新一轮执行失败（协议错误、模型报错或未恢复异常）。
    Failed,
    /// 被 `close_agent` 主动关闭（不产生报告）。
    Cancelled,
    /// 被 `interrupt_agent`、预算熔断或崩溃恢复中断。
    Interrupted,
}

impl AgentStatus {
    /// 稳定字符串值（工具结果 / DB / UI 共用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Idle => "idle",
            Self::Blocked => "blocked",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }

    /// 解析稳定字符串值；兼容旧别名 `pending` / `complete`。
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "running" | "pending" => Some(Self::Running),
            "idle" => Some(Self::Idle),
            "blocked" => Some(Self::Blocked),
            "completed" | "complete" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    /// 是否仍在运行中。
    pub const fn is_running(self) -> bool {
        matches!(self, Self::Running)
    }

    /// 是否已到达终态（可供 `wait_agent` 结算）。
    ///
    /// 注意：`Running` 不是终态；`Idle` 若是由 `interrupt_agent` 或 `budget_exceeded`
    /// 刚停下的轮次，对等待该轮结果的 `wait_agent` 而言已停止运行，由调用方结合
    /// [`Self::is_running`] 判定。
    pub const fn is_settled(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// 统一投影函数：由运行时在途标记、恢复集合、队首阻塞状态与 DB 持久状态计算出对外 `AgentStatus`。
pub fn projected_agent_status(
    is_running_in_memory: bool,
    is_resumed_or_paused: bool,
    has_blocked_head: bool,
    db_status: &str,
) -> AgentStatus {
    if is_running_in_memory {
        return AgentStatus::Running;
    }
    if has_blocked_head {
        return AgentStatus::Blocked;
    }
    if is_resumed_or_paused {
        return AgentStatus::Idle;
    }
    AgentStatus::parse(db_status).unwrap_or(AgentStatus::Completed)
}

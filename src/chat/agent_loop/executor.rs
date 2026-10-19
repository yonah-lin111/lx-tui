//! 生产工具执行器：文件工具走 `spawn_blocking`，进程/计划工具走会话持有的运行实例。
//!
//! 分派规则（对应 design §5 / §8.1）：
//!
//! ```text
//! exec_command -> ProcessManager::exec（跨轮复用同一实例）
//! write_stdin  -> ProcessManager::interact / stop
//! update_plan  -> 纯校验 + 修订号分配（提交由 FinishTool 后的落库完成）
//! 其它         -> 既有两个文件工具执行器（保持 `spawn_blocking`）
//! ```
//!
//! BeginTool 门槛由 agent loop 原调用链先取得：执行器入口不会被绕过来 spawn 进程。
//! 执行器不修改 `AppState`、不写数据库、不碰 UI：结果只经 `ToolExecutionResult` 返回。

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::chat::plan::PlanSnapshot;
use crate::chat::process::{ProcessManager, ProcessResultKind};
use crate::chat::tools::{
    ToolExecutionResult, build_plan_result, parse_exec_args, parse_stdin_args, process_failure,
};

use super::{FileTools, ReadyToolCall, ToolExecutor};

/// 一轮执行上下文：本轮新建的进程集合与当前正在等待/交互的进程。
///
/// 供 D9 的停止语义使用：收到停止请求时清理「本轮新建集合 ∪ 当前活动交互」，
/// 本轮曾经读过、但此刻不再等待的旧服务不因这次停止被清理。
#[derive(Debug, Default)]
pub struct TurnExecution {
    /// 本轮新建的进程会话 ID。
    pub created: std::collections::BTreeSet<u64>,
    /// 当前正在等待/交互的进程（工具调用存续期间）。
    pub active_interaction: Option<u64>,
}

impl TurnExecution {
    /// 停止目标 = 本轮新建 ∪ 当前活动交互。
    pub fn stop_targets(&self) -> std::collections::BTreeSet<u64> {
        let mut targets = self.created.clone();
        if let Some(active) = self.active_interaction {
            targets.insert(active);
        }
        targets
    }

    /// 记录本次调用涉及的进程（新建或交互）。
    fn observe(&mut self, name: &str, result: &ToolExecutionResult) {
        let ToolExecutionResult::Process(result) = result else {
            return;
        };
        if result.kind == ProcessResultKind::ExecCommand {
            self.created.insert(result.session_id);
        }
        // 仍在运行的进程是本轮“当前活动交互”；已退出则清除，避免误杀历史服务。
        if result.state.is_terminal() {
            if self.active_interaction == Some(result.session_id) {
                self.active_interaction = None;
            }
        } else {
            self.active_interaction = Some(result.session_id);
        }
        let _ = name;
    }
}

/// 生产执行器：持有会话进程管理器、计划修订计数与本轮执行上下文。
///
/// `Clone` 只复制共享句柄（进程管理器、修订计数、本轮上下文共用同一份），
/// 因此「同一轮的多个执行器」不会各自持有互相矛盾的进程状态。
#[derive(Clone)]
pub struct AgentTools {
    processes: Arc<ProcessManager>,
    turn: Arc<Mutex<TurnExecution>>,
    /// 计划修订计数（每次成功 update_plan 单调递增）。
    plan_revision: Arc<std::sync::atomic::AtomicU64>,
}

impl std::fmt::Debug for AgentTools {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentTools")
            .field("owner", self.processes.owner())
            .finish_non_exhaustive()
    }
}

impl AgentTools {
    /// 以会话进程管理器构造执行器（`turn` 由事件层在创建轮次时传入同一实例）。
    pub fn new(processes: Arc<ProcessManager>, turn: Arc<Mutex<TurnExecution>>) -> Self {
        Self {
            processes,
            turn,
            plan_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// 会话进程管理器（事件层清理与查询用）。
    pub fn processes(&self) -> &Arc<ProcessManager> {
        &self.processes
    }

    /// 本轮执行上下文（Esc 停止时由事件层取停止目标）。
    pub fn turn(&self) -> &Arc<Mutex<TurnExecution>> {
        &self.turn
    }

    /// 执行进程工具：`exec_command` / `write_stdin`。
    async fn execute_process(
        &self,
        cwd: &Path,
        name: &str,
        arguments_json: &str,
        call_id: &str,
    ) -> ToolExecutionResult {
        if name == "exec_command" {
            let args = match parse_exec_args(arguments_json) {
                Ok(args) => args,
                Err(failure) => return failure.into_result(Vec::new()),
            };
            return match self.processes.exec(args, cwd, call_id).await {
                Ok(result) => ToolExecutionResult::Process(result),
                Err(error) => process_failure(error),
            };
        }
        let args = match parse_stdin_args(arguments_json) {
            Ok(args) => args,
            Err(failure) => return failure.into_result(Vec::new()),
        };
        let session_id = args.session_id;
        let result = match self.processes.interact(args).await {
            Ok(result) => ToolExecutionResult::Process(result),
            Err(error) => process_failure(error),
        };
        // 交互失败（进程不存在/已过期）不改变本轮活动集合，但也清除指向它的活动项。
        if result.is_failure() && self.processes.snapshot(session_id).is_none() {
            let mut turn = self.turn.lock().await;
            if turn.active_interaction == Some(session_id) {
                turn.active_interaction = None;
            }
        }
        result
    }
}

impl ToolExecutor for AgentTools {
    async fn execute(&self, cwd: &Path, call: &ReadyToolCall) -> ToolExecutionResult {
        let result = match call.name.as_str() {
            "exec_command" | "write_stdin" => {
                self.execute_process(cwd, &call.name, &call.arguments_json, &call.call_id)
                    .await
            }
            "update_plan" => {
                // 修订号先取号后校验：校验失败不消耗可观察的提交（顶层不落库）。
                let next = self
                    .plan_revision
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    .saturating_add(1);
                match build_plan_result(&call.arguments_json, next) {
                    Ok(snapshot) => ToolExecutionResult::Plan(snapshot),
                    Err(failure) => failure.into_result(Vec::new()),
                }
            }
            _ => FileTools.execute(cwd, call).await,
        };
        if !call.name.is_empty() {
            let mut turn = self.turn.lock().await;
            turn.observe(&call.name, &result);
        }
        result
    }
}

/// 计划快照的修订号（测试与事件层断言用）。
pub fn plan_snapshot_of(result: &ToolExecutionResult) -> Option<&PlanSnapshot> {
    match result {
        ToolExecutionResult::Plan(snapshot) => Some(snapshot),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

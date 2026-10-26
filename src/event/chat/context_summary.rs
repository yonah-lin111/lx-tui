//! 本轮运行时摘要：存活进程、当前计划、在册子 agent 与未投递报告。
//!
//! 它是**本地运行状态**，不是工具结果：不新增 call/result 对、不触发新的模型请求，
//! 只在首次采样时并入 user 消息末尾，避免历史裁剪后模型忘记 `session_id` 或计划。

use super::*;

/// 受管后台任务摘要（环境快照用）：只含 session id、状态、cwd 与命令首行。
///
/// 不包含完整命令行、输出或环境变量：环境快照写入模型上下文，必须最小化。
pub(super) fn background_tasks(
    session_id: Option<&str>,
    streams: &ChatStreams,
) -> Vec<crate::chat::prompt::BackgroundTask> {
    let Some(session_id) = session_id else {
        return Vec::new();
    };
    let Some(runtime) = streams.runtimes.get(session_id) else {
        return Vec::new();
    };
    runtime
        .manager()
        .snapshots()
        .into_iter()
        .filter(|snapshot| !snapshot.state.is_terminal() || snapshot.background_alive)
        .map(|snapshot| crate::chat::prompt::BackgroundTask {
            session_id: snapshot.session_id,
            state: snapshot.state.as_str().to_string(),
            cwd: snapshot.cwd,
            command: snapshot
                .command
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(120)
                .collect(),
        })
        .collect()
}

/// 本轮运行上下文摘要：存活进程（有界）与当前计划（有界）。
///
/// 它是**本地运行状态**，不是工具结果：不新增 call/result 对，也不触发新的模型请求。
pub(super) fn runtime_context_summary(
    manager: &crate::chat::process::ProcessManager,
    plan: &Option<crate::app::state::PlanSnapshotView>,
) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let running: Vec<crate::chat::process::ProcessSnapshot> = manager
        .snapshots()
        .into_iter()
        .filter(|snapshot| !snapshot.state.is_terminal())
        .take(MAX_CONTEXT_PROCESSES)
        .collect();
    if !running.is_empty() {
        lines.push(
            "[local runtime] running processes (use write_stdin with session_id):".to_string(),
        );
        for snapshot in running {
            let command: String = snapshot.command.chars().take(120).collect();
            lines.push(format!(
                "- session_id={} cwd={} · {}",
                snapshot.session_id, snapshot.cwd, command
            ));
        }
    }
    if let Some(plan) = plan
        && !plan.is_empty()
    {
        lines.push("[local runtime] current plan:".to_string());
        for (step, status) in plan.steps.iter().take(MAX_CONTEXT_PLAN_STEPS) {
            let marker = text::agent::plan_marker(*status);
            lines.push(format!("- {marker} {step}"));
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

/// 摘要里最多列出的运行进程数。
const MAX_CONTEXT_PROCESSES: usize = 8;
/// 摘要里最多列出的计划步骤数。
const MAX_CONTEXT_PLAN_STEPS: usize = 16;

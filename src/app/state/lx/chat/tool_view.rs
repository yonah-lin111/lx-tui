//! 进程 / 计划工具的**类型化展示视图**（纯数据，渲染层只读）。
//!
//! 为什么不复用 [`super::steps::ToolSummary`]：文件摘要只描述“哪个路径、多少行、几个
//! 命中”，进程工具真正要表达的是「命令、cwd、运行状态、退出码、尾部输出」，计划工具
//! 要表达的是「步骤三态与完成计数」。把进程 ID 塞进 `path`、把步骤数塞进 `matches`
//! 会让渲染层被迫按工具名分支解析，且无法区分「模型列出的待办」与「已经执行的动作」。
//!
//! 这些类型由事件层在**结算与重载**两条路径上从同一条规范化结果 JSON 生成，
//! 渲染层不做 JSON 解析、不读数据库、不读进程。

use serde_json::Value;

use crate::chat::plan::{PlanSnapshot, PlanStepStatus};
use crate::chat::process::{ProcessResult, ProcessResultKind, ProcessState};

/// 最多展示的输出行数（紧凑卡片）。
pub const COMMAND_PREVIEW_LINES: usize = 6;

/// 进程状态（UI 语义；未知值按 `Interrupted` 保守处理，不伪装成功）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStateView {
    Starting,
    Running,
    Exited { code: Option<i32> },
    Stopped,
    TimedOut,
    Interrupted,
    Failed,
}

impl From<ProcessState> for ProcessStateView {
    fn from(state: ProcessState) -> Self {
        match state {
            ProcessState::Starting => Self::Starting,
            ProcessState::Running => Self::Running,
            ProcessState::Exited { code } => Self::Exited { code },
            ProcessState::Stopped => Self::Stopped,
            ProcessState::TimedOut => Self::TimedOut,
            ProcessState::Interrupted => Self::Interrupted,
            ProcessState::Failed { .. } => Self::Failed,
        }
    }
}

impl ProcessStateView {
    /// 是否仍在运行（决定卡片是否呼吸、以及“Running”优先于任何结果文案）。
    pub fn is_running(self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }

    /// 是否失败（非零退出也算）。
    pub fn is_failure(self) -> bool {
        match self {
            Self::TimedOut | Self::Interrupted | Self::Failed => true,
            Self::Exited { code } => code.is_some_and(|code| code != 0),
            Self::Starting | Self::Running | Self::Stopped => false,
        }
    }

    /// 退出码（只有 `Exited`）。
    pub fn exit_code(self) -> Option<i32> {
        match self {
            Self::Exited { code } => code,
            _ => None,
        }
    }

    /// 稳定状态字符串（与结果 JSON 的 `process_state` 一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Exited { .. } => "exited",
            Self::Stopped => "stopped",
            Self::TimedOut => "timed_out",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
        }
    }
}

/// `exec_command` 的展示视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandView {
    /// 模型见到的进程会话 ID（技术 ID 只在详情里出现，不做卡片标题）。
    pub process_session_id: Option<u64>,
    pub command: String,
    pub cwd: String,
    pub tty: bool,
    pub state: ProcessStateView,
    /// 尾部输出预览（最近 [`COMMAND_PREVIEW_LINES`] 行）。
    pub output_preview: Vec<String>,
    /// 是否还有更多保留输出（展开可见）。
    pub truncated: bool,
    /// 被淘汰的字节数（> 0 时明确提示 Earlier output discarded）。
    pub dropped_bytes: u64,
    /// 保留的完整尾部输出（详情展开用）。
    pub output_tail: String,
    /// 起止时刻（毫秒；时长由事件 tick 提供 now_ms 计算，渲染层不取时钟）。
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
}

/// `write_stdin` 本次调用的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InteractionAction {
    /// 空 `chars`：只读取后续输出。
    ReadOutput,
    /// 非空 `chars`：写入 stdin。
    SendInput,
    /// `terminate: true`：显式停止进程。
    StopCommand,
}

impl InteractionAction {
    /// 稳定动作值（结果 JSON / 视觉判断共用）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOutput => "read",
            Self::SendInput => "send",
            Self::StopCommand => "stop",
        }
    }
}

/// `write_stdin` 的展示视图：**该次调用**的快照，不随最新进程缓冲变化。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInteractionView {
    pub action: InteractionAction,
    pub process_session_id: u64,
    /// 关联的原命令（来自本次结果）。
    pub command: String,
    /// 输入预览（已限长、换行已转义）；空写为 None。
    pub input_preview: Option<String>,
    /// 本次返回的输出（原始文本，行切分在渲染层按宽度做）。
    pub output: String,
    pub state: ProcessStateView,
    pub exit_code: Option<i32>,
    pub truncated: bool,
    pub dropped_bytes: u64,
}

/// 计划步骤的展示状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanStepView {
    Pending,
    InProgress,
    Completed,
}

impl From<PlanStepStatus> for PlanStepView {
    fn from(status: PlanStepStatus) -> Self {
        match status {
            PlanStepStatus::Pending => Self::Pending,
            PlanStepStatus::InProgress => Self::InProgress,
            PlanStepStatus::Completed => Self::Completed,
        }
    }
}

/// `update_plan` 的展示视图：该次提交的快照（历史卡片不倒改成最新计划）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanSnapshotView {
    /// 本次提交的修订号。
    pub revision: u64,
    pub explanation: Option<String>,
    pub steps: Vec<(String, PlanStepView)>,
}

impl PlanSnapshotView {
    /// 已完成步骤数（只认 completed）。
    pub fn completed(&self) -> usize {
        self.steps
            .iter()
            .filter(|(_, status)| *status == PlanStepView::Completed)
            .count()
    }

    /// 是否为空计划（清空语义）。
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// 由规范化快照构造（结算与重载共用）。
    pub fn from_snapshot(snapshot: &PlanSnapshot) -> Self {
        Self {
            revision: snapshot.revision,
            explanation: snapshot.explanation.clone(),
            steps: snapshot
                .steps
                .iter()
                .map(|step| (step.step.clone(), PlanStepView::from(step.status)))
                .collect(),
        }
    }
}

/// 三种新增工具的详情视图（文件工具沿用既有摘要，不在这里重复一套）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDetailView {
    Command(CommandView),
    Interaction(ProcessInteractionView),
    Plan(PlanSnapshotView),
}

impl ToolDetailView {
    /// 由进程结果构造（`exec_command` 与 `write_stdin` 共用同一份形状解析）。
    pub fn from_process(result: &ProcessResult) -> Self {
        let state = ProcessStateView::from(result.state.clone());
        match result.kind {
            ProcessResultKind::ExecCommand => Self::Command(CommandView {
                process_session_id: Some(result.session_id),
                command: result.command.clone(),
                cwd: result.cwd.clone(),
                tty: result.tty,
                state,
                output_preview: preview_lines(&result.output),
                truncated: result.truncated,
                dropped_bytes: result.dropped_bytes,
                output_tail: result.output.clone(),
                started_at_ms: None,
                ended_at_ms: None,
            }),
            ProcessResultKind::WriteStdin => {
                let action = if result.terminate {
                    InteractionAction::StopCommand
                } else if result.wrote_input {
                    InteractionAction::SendInput
                } else {
                    InteractionAction::ReadOutput
                };
                Self::Interaction(ProcessInteractionView {
                    action,
                    process_session_id: result.session_id,
                    command: result.command.clone(),
                    input_preview: result.input.clone(),
                    output: result.output.clone(),
                    state,
                    exit_code: result.exit_code,
                    truncated: result.truncated,
                    dropped_bytes: result.dropped_bytes,
                })
            }
        }
    }

    /// 由落库结果 JSON 还原（历史重载路径；形状不符返回 None）。
    pub fn from_result_json(result_json: Option<&str>) -> Option<Self> {
        let value: Value = serde_json::from_str(result_json?).ok()?;
        match value.get("tool").and_then(Value::as_str)? {
            "exec_command" => Some(Self::Command(CommandView {
                process_session_id: value.get("session_id").and_then(Value::as_u64),
                command: value.get("command")?.as_str()?.to_string(),
                cwd: value
                    .get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                tty: value.get("tty").and_then(Value::as_bool).unwrap_or(false),
                state: state_view_of(&value),
                output_preview: preview_lines(
                    value.get("output").and_then(Value::as_str).unwrap_or(""),
                ),
                truncated: value
                    .get("truncated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                dropped_bytes: value
                    .get("dropped_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                output_tail: value
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                started_at_ms: None,
                ended_at_ms: None,
            })),
            "write_stdin" => {
                let terminate = value
                    .get("terminate")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let input = value
                    .get("input")
                    .and_then(Value::as_str)
                    .filter(|text| !text.is_empty())
                    .map(str::to_string);
                let action = if terminate {
                    InteractionAction::StopCommand
                } else if input.is_some() {
                    InteractionAction::SendInput
                } else {
                    InteractionAction::ReadOutput
                };
                Some(Self::Interaction(ProcessInteractionView {
                    action,
                    process_session_id: value.get("session_id").and_then(Value::as_u64)?,
                    command: value
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    input_preview: input,
                    output: value
                        .get("output")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    state: state_view_of(&value),
                    exit_code: value
                        .get("exit_code")
                        .and_then(Value::as_i64)
                        .and_then(|code| i32::try_from(code).ok()),
                    truncated: value
                        .get("truncated")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    dropped_bytes: value
                        .get("dropped_bytes")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                }))
            }
            "update_plan" => {
                let snapshot = PlanSnapshot::from_json(&value)?;
                Some(Self::Plan(PlanSnapshotView::from_snapshot(&snapshot)))
            }
            _ => None,
        }
    }
}

/// 结果 JSON 的 `process_state` → 视图状态（未知按 Interrupted）。
fn state_view_of(value: &Value) -> ProcessStateView {
    match value.get("process_state").and_then(Value::as_str) {
        Some("starting") => ProcessStateView::Starting,
        Some("running") => ProcessStateView::Running,
        Some("exited") => ProcessStateView::Exited {
            code: value
                .get("exit_code")
                .and_then(Value::as_i64)
                .and_then(|code| i32::try_from(code).ok()),
        },
        Some("stopped") => ProcessStateView::Stopped,
        Some("timed_out") => ProcessStateView::TimedOut,
        Some("failed") => ProcessStateView::Failed,
        _ => ProcessStateView::Interrupted,
    }
}

/// 尾部输出预览：最近 [`COMMAND_PREVIEW_LINES`] 个非末尾空行。
pub fn preview_lines(output: &str) -> Vec<String> {
    let mut lines: Vec<String> = output
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    // 末尾换行会产生一个空行：去掉它，避免预览少显示一行真实内容。
    if lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let start = lines.len().saturating_sub(COMMAND_PREVIEW_LINES);
    lines.split_off(start)
}

#[cfg(test)]
mod tests;

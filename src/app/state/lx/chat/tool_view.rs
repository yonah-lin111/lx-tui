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

/// 视图快照保留的输出尾部窗口行数（变化检测用）。
///
/// 渲染实际用的是 `output_tail`（按卡片宽度现场切行）：运行中的卡片根本不铺输出，
/// 终态输出区按首 3 + 尾 2 行折叠。这个窗口只服务于「快照是否变化」的比较。
pub const COMMAND_PREVIEW_LINES: usize = 3;

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

    /// 是否已到终态（进程不会再产生新输出，标题上的时长必须停住）。
    ///
    /// 卡片只有在**非终态**时才把 `started_at_ms` 换算成「已经跑了多久」；
    /// 一旦终态就只显示结果（`Exited 0` / `exit 1` / `Stopped`），这正是
    /// 「关掉之后还在涨时间」那个 bug 的判据。
    pub fn is_terminal(self) -> bool {
        !self.is_running()
    }

    /// 进程是否仍在后台存活（终态为 false）。
    pub fn is_alive(self) -> bool {
        self.is_running()
    }

    /// 是否为**进程级**失败：超时 / 被中断 / 启动失败。
    ///
    /// **不含"命令以非零码退出"**。非零退出经常是正常结果（`grep` 无匹配 = 1、
    /// `diff` 有差异 = 1、`test` 条件不成立 = 1），把它当失败会让界面把一次成功的
    /// 查询标成红叉。退出码本身仍会在标题行上显示（`exit 1`），信息不丢失。
    pub fn is_process_failure(self) -> bool {
        matches!(self, Self::TimedOut | Self::Interrupted | Self::Failed)
    }

    /// 命令是否以非零码退出（跑完了，但退出码不是 0）。
    ///
    /// 与 [`Self::is_process_failure`] 分开：它驱动的只是"用中性色显示退出码"，
    /// 不驱动红色失败标记。
    pub fn is_nonzero_exit(self) -> bool {
        matches!(self, Self::Exited { code } if code.is_some_and(|code| code != 0))
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

    /// 由 `agent_process.state` 的稳定字符串还原（历史重载用）。
    ///
    /// 未知值按 [`Self::Interrupted`] 保守处理：宁可显示「结果未知」，
    /// 也不把一条来历不明的记录伪装成正常跑完。
    pub fn from_stored(state: &str, exit_code: Option<i32>) -> Self {
        match state {
            "starting" => Self::Starting,
            "running" => Self::Running,
            "exited" => Self::Exited { code: exit_code },
            "stopped" => Self::Stopped,
            "timed_out" => Self::TimedOut,
            "failed" => Self::Failed,
            "interrupted" => Self::Interrupted,
            _ => Self::Interrupted,
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
    /// 尾部输出窗口（最近 [`COMMAND_PREVIEW_LINES`] 行；变化检测用，不直接渲染）。
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

/// 一道题的作答展示数据（`ask` 卡片；不含选项与 preview）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskAnsweredView {
    /// 题目标签（短标题）。
    pub header: String,
    /// 完整题干。
    pub question: String,
    /// 勾选项在前、手写文本在最后（已去重）。
    pub answers: Vec<String>,
}

/// `ask` 的展示视图：该次提问的题面与答案（自包含，卡片不需要再读参数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskDetailView {
    /// 按提问顺序的作答记录。
    pub questions: Vec<AskAnsweredView>,
}

impl AskDetailView {
    /// 由工具的作答结果构造（实时结算路径）。
    pub fn from_answered(questions: &[crate::chat::tools::AnsweredQuestion]) -> Self {
        Self {
            questions: questions
                .iter()
                .map(|question| AskAnsweredView {
                    header: question.header.clone(),
                    question: question.question.clone(),
                    answers: question.answers.clone(),
                })
                .collect(),
        }
    }

    /// 由落库结果 JSON 还原；非 `ask` 结果或字段缺失返回 None（沿用旧记录兜底策略）。
    pub fn from_result_json(result_json: Option<&str>) -> Option<Self> {
        let value: Value = serde_json::from_str(result_json?).ok()?;
        Self::from_value(&value)
    }

    /// 由已解析的结果 JSON 构造（`ToolDetailView::from_result_json` 共用）。
    fn from_value(value: &Value) -> Option<Self> {
        if value.get("tool").and_then(Value::as_str)? != "ask" {
            return None;
        }
        let entries = value.get("questions")?.as_array()?;
        let mut questions = Vec::with_capacity(entries.len());
        for entry in entries {
            let answers = entry
                .get("answers")?
                .as_array()?
                .iter()
                .map(|answer| answer.as_str().map(str::to_string))
                .collect::<Option<Vec<_>>>()?;
            questions.push(AskAnsweredView {
                header: entry.get("header")?.as_str()?.to_string(),
                question: entry.get("question")?.as_str()?.to_string(),
                answers,
            });
        }
        Some(Self { questions })
    }
}

/// 工具详情视图：进程/计划/提问走各自快照，文件工具走**已解码的差异**。
///
/// 文件分支与摘要并存：摘要回答"哪个文件、多少行"，详情回答"哪几行变成什么"。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolDetailView {
    Command(CommandView),
    Interaction(ProcessInteractionView),
    Plan(PlanSnapshotView),
    File(super::file_diff::FileDiffDetailView),
    Ask(AskDetailView),
    Subagent(SubagentDetailView),
}

/// `spawn_agent` / `wait_agent` / `close_agent` 共用的详情（按 `name` 取标题）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentDetailView {
    /// spawn 的角色。
    pub role: Option<crate::chat::agent::AgentRole>,
    /// spawn 的完整任务文本（展开显示）。
    pub task: Option<String>,
    /// spawn/close 的 agent id。
    pub agent_id: Option<String>,
    /// wait 的目标 id。
    pub targets: Vec<String>,
    /// wait 是否超时。
    pub timed_out: bool,
    /// wait 结算到的 agent 状态。
    pub agents: Vec<crate::chat::agent::AgentStatusView>,
    /// close 前的状态。
    pub closed_status: Option<String>,
}

impl SubagentDetailView {
    /// 由工具名 + 参数 + 落库结果 JSON 构造（实时结算与历史重载共用）。
    pub fn from_tool(name: &str, arguments_json: &str, result_json: Option<&str>) -> Option<Self> {
        let value: Value = serde_json::from_str(result_json?).ok()?;
        Self::from_outcome_with_arguments(&value, Some((name, arguments_json)))
    }

    /// 由结果 JSON +（可选）调用参数构造。
    pub fn from_outcome_with_arguments(
        value: &Value,
        arguments: Option<(&str, &str)>,
    ) -> Option<Self> {
        let mut view = Self::from_outcome_value(value)?;
        let (name, arguments_json) =
            arguments.or_else(|| Some((value.get("tool")?.as_str()?, "{}")))?;
        if let Some(Ok(op)) = crate::chat::agent::parse_control(name, arguments_json) {
            match op {
                crate::chat::agent::SubagentOp::Spawn { role, task } => {
                    view.role = Some(role);
                    view.task = Some(task);
                }
                crate::chat::agent::SubagentOp::Wait { targets, .. } => {
                    view.targets = targets;
                }
                crate::chat::agent::SubagentOp::Close { target } => {
                    view.agent_id = Some(target);
                }
            }
        }
        Some(view)
    }

    /// 由落库结果 JSON 的形状构造（无参数侧信息）。
    pub fn from_outcome_value(value: &Value) -> Option<Self> {
        let mut view = Self {
            role: None,
            task: None,
            agent_id: None,
            targets: Vec::new(),
            timed_out: false,
            agents: Vec::new(),
            closed_status: None,
        };
        match value.get("tool").and_then(Value::as_str)? {
            "spawn_agent" => {
                view.role = value
                    .get("role")
                    .and_then(Value::as_str)
                    .and_then(crate::chat::agent::AgentRole::parse);
                view.agent_id = value
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            "wait_agent" => {
                view.timed_out = value
                    .get("timed_out")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Some(agents) = value.get("agents").and_then(Value::as_array) {
                    for agent in agents {
                        let Some(agent_id) = agent.get("agent_id").and_then(Value::as_str) else {
                            continue;
                        };
                        let Some(role) = agent
                            .get("role")
                            .and_then(Value::as_str)
                            .and_then(crate::chat::agent::AgentRole::parse)
                        else {
                            continue;
                        };
                        view.agents.push(crate::chat::agent::AgentStatusView {
                            agent_id: agent_id.to_string(),
                            role,
                            status: agent
                                .get("status")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            report: agent
                                .get("report")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                        });
                    }
                }
            }
            "close_agent" => {
                view.agent_id = value
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                view.closed_status = value
                    .get("previous_status")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            _ => return None,
        }
        Some(view)
    }
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
            "ask" => Some(Self::Ask(AskDetailView::from_value(&value)?)),
            "spawn_agent" | "wait_agent" | "close_agent" => Some(Self::Subagent(
                SubagentDetailView::from_outcome_value(&value)?,
            )),
            _ => None,
        }
    }

    /// 文件工具的差异详情（结算与重载共用同一解码入口）。
    ///
    /// 坏文件详情在解码内部局部降级为 `Unavailable`，只有**完全**不是文件结果时才返回
    /// None（此时调用方沿用原两行摘要）。
    pub fn file_detail(result_json: Option<&str>) -> Option<Self> {
        let value: Value = serde_json::from_str(result_json?).ok()?;
        if !super::file_diff::has_file_detail(&value) {
            return None;
        }
        super::file_diff::FileDiffDetailView::from_result_json(Some(result_json?)).map(Self::File)
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

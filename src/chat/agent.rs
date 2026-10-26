//! 子 agent 领域层：角色、工具面、控制工具参数、结果视图与报告信封。
//!
//! 纯数据，无 IO、无 `app/`/`ui/` 依赖。工具名、角色名与错误码是稳定契约
//! （工具参数 / DB `agent_role` / 提示 / 日志），不得改名；改名等于换 agent，
//! 必须同时递增 [`crate::chat::prompt::PROMPT_VERSION`]。

use serde_json::{Value, json};

use crate::chat::tools::{ToolErrorCode, ToolExecutionResult, ToolFailure};

#[cfg(test)]
mod tests;

/// 内置子 agent 角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    /// 只读调查：`read` / `search`。
    Explorer,
    /// 有边界的实现：文件与进程工具，无 `ask`、无计划槽、无递归。
    Executor,
}

impl AgentRole {
    /// 全部角色（固定顺序）。
    pub const ALL: [AgentRole; 2] = [Self::Explorer, Self::Executor];

    /// 稳定存储/协议值：`explorer` / `executor`。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Executor => "executor",
        }
    }

    /// 解析稳定值；未知值返回 None。
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "explorer" => Some(Self::Explorer),
            "executor" => Some(Self::Executor),
            _ => None,
        }
    }

    /// 角色系统提示（英文，见 `docs/feature/lx-subagent/design.md` §7）。
    pub const fn system_prompt(self) -> &'static str {
        match self {
            Self::Explorer => EXPLORER_PROMPT,
            Self::Executor => EXECUTOR_PROMPT,
        }
    }

    /// 该角色可用工具名白名单。
    pub const fn tools(self) -> &'static [&'static str] {
        ToolSurface::from_role(self).names()
    }
}

/// 工具面：决定一次请求声明哪些工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSurface {
    /// 主 agent：现有 8 项 + 3 个控制工具。
    Main,
    /// 只读调查。
    Explorer,
    /// 有边界的实现。
    Executor,
}

/// 主 agent 工具名（顺序即呈现顺序）。
const MAIN_TOOLS: [&str; 11] = [
    "read",
    "search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
    "update_plan",
    "ask",
    "spawn_agent",
    "wait_agent",
    "close_agent",
];

/// explorer 工具名。
const EXPLORER_TOOLS: [&str; 2] = ["read", "search"];

/// executor 工具名。
const EXECUTOR_TOOLS: [&str; 6] = [
    "read",
    "search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
];

impl ToolSurface {
    /// 该工具面的工具名（逐字稳定，测试锁死）。
    pub const fn names(self) -> &'static [&'static str] {
        match self {
            Self::Main => &MAIN_TOOLS,
            Self::Explorer => &EXPLORER_TOOLS,
            Self::Executor => &EXECUTOR_TOOLS,
        }
    }

    /// 角色对应的工具面。
    pub const fn from_role(role: AgentRole) -> Self {
        match role {
            AgentRole::Explorer => Self::Explorer,
            AgentRole::Executor => Self::Executor,
        }
    }
}

/// 每会话并发上限（硬拒绝，不排队）。
pub const MAX_CONCURRENT_SUBAGENTS: usize = 3;
/// 单次 `wait_agent` 的目标数上限。
pub const MAX_WAIT_TARGETS: usize = 8;
/// `wait_agent` 默认超时（毫秒）。
pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 60_000;
/// `wait_agent` 超时上限（毫秒）。
pub const MAX_WAIT_TIMEOUT_MS: u64 = 300_000;
/// 报告正文上限（字节）。
pub const MAX_REPORT_BYTES: usize = 8 * 1024;
/// `spawn_agent` 的 task 文本上限（字符）。
pub const MAX_TASK_CHARS: usize = 8_000;

/// 并发超限的稳定错误码。
pub const LIMIT_REACHED_CODE: &str = "subagent concurrency limit reached";
/// 目标不存在的稳定错误码。
pub const NOT_FOUND_CODE: &str = "subagent not found";
/// 目标已结束的稳定错误码。
pub const FINISHED_CODE: &str = "subagent already finished";
/// 子 agent 内禁止控制工具的稳定错误码。
pub const UNAVAILABLE_CODE: &str = "subagent tools unavailable";

/// 三个控制工具之一；由 loop 转投事件层执行（与 `ask` 同一分支位置）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOp {
    /// 派发一个子 agent。
    Spawn { role: AgentRole, task: String },
    /// 等待若干子 agent 结束（或超时）。
    Wait { targets: Vec<String>, timeout_ms: u64 },
    /// 停止一个子 agent（不产生报告）。
    Close { target: String },
}

/// 操作结果：工具结果 JSON 与 UI 详情共用的唯一来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOutcome {
    /// 已派发（不等待）。
    Spawned {
        agent_id: String,
        role: AgentRole,
    },
    /// 等待结算；`report` 只在本次领取到报告时非空。
    Waited {
        timed_out: bool,
        agents: Vec<AgentStatusView>,
    },
    /// 已停止；不产生报告。
    Closed {
        agent_id: String,
        previous_status: String,
    },
}

/// 一个子 agent 的结算视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusView {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 角色。
    pub role: AgentRole,
    /// `running` / `completed` / `failed` / `cancelled`。
    pub status: String,
    /// 本次等待结算到的报告正文；未结束或已投递过为 None。
    pub report: Option<String>,
}

impl SubagentOutcome {
    /// 结果序列化（UI 详情、历史恢复、模型回执同源）。
    pub fn to_json(&self) -> Value {
        match self {
            Self::Spawned { agent_id, role } => json!({
                "tool": "spawn_agent",
                "agent_id": agent_id,
                "role": role.as_str(),
            }),
            Self::Waited { timed_out, agents } => json!({
                "tool": "wait_agent",
                "timed_out": timed_out,
                "agents": agents
                    .iter()
                    .map(|agent| json!({
                        "agent_id": agent.agent_id,
                        "role": agent.role.as_str(),
                        "status": agent.status,
                        "report": agent.report,
                    }))
                    .collect::<Vec<_>>(),
            }),
            Self::Closed {
                agent_id,
                previous_status,
            } => json!({
                "tool": "close_agent",
                "agent_id": agent_id,
                "previous_status": previous_status,
            }),
        }
    }

    /// 从结果 JSON 还原（历史重载用）；形状不符返回 None。
    pub fn from_json(value: &Value) -> Option<Self> {
        match value.get("tool").and_then(Value::as_str)? {
            "spawn_agent" => Some(Self::Spawned {
                agent_id: value.get("agent_id")?.as_str()?.to_string(),
                role: AgentRole::parse(value.get("role")?.as_str()?)?,
            }),
            "wait_agent" => {
                let agents = value.get("agents")?.as_array()?;
                let mut parsed = Vec::with_capacity(agents.len());
                for agent in agents {
                    parsed.push(AgentStatusView {
                        agent_id: agent.get("agent_id")?.as_str()?.to_string(),
                        role: AgentRole::parse(agent.get("role")?.as_str()?)?,
                        status: agent.get("status")?.as_str()?.to_string(),
                        report: agent
                            .get("report")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
                Some(Self::Waited {
                    timed_out: value.get("timed_out")?.as_bool()?,
                    agents: parsed,
                })
            }
            "close_agent" => Some(Self::Closed {
                agent_id: value.get("agent_id")?.as_str()?.to_string(),
                previous_status: value.get("previous_status")?.as_str()?.to_string(),
            }),
            _ => None,
        }
    }

    /// 转成通用工具结果（loop 的 `finish_tool` 与 UI 详情共用）。
    pub fn into_result(self) -> ToolExecutionResult {
        ToolExecutionResult::Subagent(Box::new(self))
    }
}

/// 汇报块数据源（父消息 `subagent_report_json` 反序列化）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentReportView {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 角色。
    pub role: AgentRole,
    /// `completed` / `failed` / `cancelled`。
    pub status: String,
    /// 去重后的改动文件数。
    pub files_changed: usize,
    /// 子 turn 耗时（毫秒）。
    pub duration_ms: Option<i64>,
    /// 报告正文（已按上限截断）。
    pub body: String,
    /// 报告生成时刻（UTC 毫秒）。
    pub delivered_at_ms: i64,
}

impl SubagentReportView {
    /// 落库 JSON（`agent_subagent.report_json` 与消息 `subagent_report_json` 共用）。
    pub fn to_json(&self) -> String {
        json!({
            "agent_id": self.agent_id,
            "role": self.role.as_str(),
            "status": self.status,
            "files_changed": self.files_changed,
            "duration_ms": self.duration_ms,
            "body": self.body,
            "delivered_at_ms": self.delivered_at_ms,
        })
        .to_string()
    }

    /// 从落库 JSON 还原；损坏返回 None。
    pub fn from_json(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        Some(Self {
            agent_id: value.get("agent_id")?.as_str()?.to_string(),
            role: AgentRole::parse(value.get("role")?.as_str()?)?,
            status: value.get("status")?.as_str()?.to_string(),
            files_changed: value
                .get("files_changed")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            duration_ms: value.get("duration_ms").and_then(Value::as_i64),
            body: value.get("body")?.as_str()?.to_string(),
            delivered_at_ms: value
                .get("delivered_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
        })
    }
}

/// 注入父会话的报告文本：纯文本 + XML 包裹 + 元字符转义。
pub fn format_report_envelope(report: &SubagentReportView) -> String {
    let body = truncate_report_body(&sanitize_control(&report.body));
    let duration = report
        .duration_ms
        .map(|value| format!("{value}"))
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "<subagent_report agent_id=\"{}\" role=\"{}\" status=\"{}\" files_changed=\"{}\" duration_ms=\"{}\">\n{}\n</subagent_report>",
        escape_attribute(&report.agent_id),
        report.role.as_str(),
        escape_attribute(&report.status),
        report.files_changed,
        duration,
        escape_text(&body),
    )
}

/// 三个控制工具之一；非控制工具返回 None，参数非法返回 Err。
///
/// 只接受白名单字段（多余字段直接拒绝）：schema 是给模型看的，执行层不信任它。
pub fn parse_control(name: &str, arguments_json: &str) -> Option<Result<SubagentOp, ToolFailure>> {
    match name {
        "spawn_agent" => Some(parse_spawn(arguments_json)),
        "wait_agent" => Some(parse_wait(arguments_json)),
        "close_agent" => Some(parse_close(arguments_json)),
        _ => None,
    }
}

/// `spawn_agent` 的参数解析。
fn parse_spawn(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["role", "task"])?;
    let role_text = object
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid("`role` must be \"explorer\" or \"executor\""))?;
    let role = AgentRole::parse(role_text).ok_or_else(|| {
        ToolFailure::invalid("`role` must be \"explorer\" or \"executor\"")
    })?;
    let task = object
        .get("task")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid("`task` must be a string"))?
        .to_string();
    if task.trim().is_empty() {
        return Err(ToolFailure::invalid("`task` must not be empty"));
    }
    if task.chars().count() > MAX_TASK_CHARS {
        return Err(ToolFailure::new(
            ToolErrorCode::TooLarge,
            format!("`task` must be at most {MAX_TASK_CHARS} characters"),
        ));
    }
    Ok(SubagentOp::Spawn { role, task })
}

/// `wait_agent` 的参数解析。
fn parse_wait(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["targets", "timeout_ms"])?;
    let targets = object
        .get("targets")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolFailure::invalid("`targets` must be an array of agent ids"))?;
    if targets.is_empty() || targets.len() > MAX_WAIT_TARGETS {
        return Err(ToolFailure::invalid(format!(
            "`targets` must contain 1 to {MAX_WAIT_TARGETS} agent ids"
        )));
    }
    let mut parsed = Vec::with_capacity(targets.len());
    for target in targets {
        let target = target
            .as_str()
            .ok_or_else(|| ToolFailure::invalid("`targets` must contain only strings"))?;
        if target.trim().is_empty() {
            return Err(ToolFailure::invalid("`targets` must not contain empty ids"));
        }
        parsed.push(target.to_string());
    }
    let timeout_ms = match object.get("timeout_ms") {
        None | Some(Value::Null) => DEFAULT_WAIT_TIMEOUT_MS,
        Some(value) => {
            let raw = value
                .as_i64()
                .ok_or_else(|| ToolFailure::invalid("`timeout_ms` must be an integer"))?;
            raw.clamp(1, MAX_WAIT_TIMEOUT_MS as i64) as u64
        }
    };
    Ok(SubagentOp::Wait {
        targets: parsed,
        timeout_ms: timeout_ms.clamp(1, MAX_WAIT_TIMEOUT_MS),
    })
}

/// `close_agent` 的参数解析。
fn parse_close(arguments_json: &str) -> Result<SubagentOp, ToolFailure> {
    let object = object_of(arguments_json)?;
    reject_unknown(&object, &["target"])?;
    let target = object
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolFailure::invalid("`target` must be a string"))?
        .to_string();
    if target.trim().is_empty() {
        return Err(ToolFailure::invalid("`target` must not be empty"));
    }
    Ok(SubagentOp::Close { target })
}

/// 参数体必须是 JSON 对象。
fn object_of(arguments_json: &str) -> Result<serde_json::Map<String, Value>, ToolFailure> {
    serde_json::from_str::<Value>(arguments_json)
        .ok()
        .and_then(|value| value.as_object().cloned())
        .ok_or_else(|| ToolFailure::invalid("arguments must be a JSON object"))
}

/// 拒绝白名单外的字段（只报字段名，不回显参数全文）。
fn reject_unknown(
    object: &serde_json::Map<String, Value>,
    allowed: &[&str],
) -> Result<(), ToolFailure> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ToolFailure::invalid(format!("unknown field: `{key}`")));
        }
    }
    Ok(())
}

/// 报告正文截断（按字节上限，且不切断 UTF-8 字符）。
pub fn truncate_report_body(body: &str) -> String {
    if body.len() <= MAX_REPORT_BYTES {
        return body.to_string();
    }
    let mut end = MAX_REPORT_BYTES;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[truncated]", &body[..end])
}

/// 剥离 ANSI/控制字符（保留换行与制表符）。
fn sanitize_control(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else {
                let _ = chars.next();
            }
            continue;
        }
        if ch == '\n' || ch == '\t' || !ch.is_control() {
            out.push(ch);
        }
    }
    out
}

/// 正文 XML 元字符转义（防止伪造信封边界）。
fn escape_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 属性值转义（含引号）。
fn escape_attribute(text: &str) -> String {
    escape_text(text).replace('"', "&quot;")
}

/// 子 agent 系统提示：角色提示 + 环境快照。
///
/// **不含**父会话的后台任务摘要与 plan：子 agent 只看到自己的任务。
pub fn compose_subagent_prompt(role: AgentRole, environment: Option<&str>) -> String {
    format!("{}\n\n{}\n", role.system_prompt(), environment.unwrap_or_default())
}

/// explorer 角色提示。
pub const EXPLORER_PROMPT: &str = r#"You are an explorer: a read-only investigator working inside the user's workspace.

You answer one specific question about the codebase. You have `read` and `search` and nothing
else — you cannot modify files, run commands, or spawn agents.

# How you work

- Start from the question, not from the repository. Work out which paths answer it, then read them.
- Follow the code, not the docs: a comment can be stale, the call graph cannot.
- Cite evidence as `path:line`. A claim without a location is a guess; say so if you cannot find one.
- Stop when the question is answered. Do not audit unrelated code, do not suggest refactors, and
  do not write patches — the caller decides what to do with your findings.
- If the question cannot be answered with the tools you have (it needs a command, a test run, or a
  file change), say exactly that and what would be needed. Do not pretend you checked.

# Final message

Your final message is the report and the only thing the caller reads. Lead with the answer, then
the evidence, then anything the caller must know to act on it. Keep it under roughly 40 lines.
State explicitly what you could not verify."#;

/// executor 角色提示。
pub const EXECUTOR_PROMPT: &str = r#"You are an executor: a coding agent working inside the user's workspace on one assigned task.

# Your boundary

You were given a goal, a boundary, and a deliverable. Work inside that boundary. If the task turns
out to need changes outside it, stop and report why instead of widening the scope yourself.

You are not alone in this workspace. Other agents may be editing other files at the same time.
Never revert, reformat, or overwrite changes you did not make; if a file changed under you, re-read
it and work with the current content.

# How you work

- Read before you change: `edit` and `write` reject a file you have not read in this session or
  that changed on disk after the read.
- Prefer the smallest exact `edit` over rewriting a file. Do not refactor, rename, or reformat
  anything the task did not ask for.
- Fix the root cause instead of the symptom.
- Verify with the narrowest check that covers what you changed, using the project's own commands.
  Report exactly what you ran and what it returned.
- Do not commit, amend, create branches, or run destructive git commands.
- You cannot ask the user anything. If a decision genuinely blocks you, do the safe thing, finish
  what is unambiguous, and report the open question.

# Final message

Your final message is the report and the only thing the caller reads. State what changed (with
paths), what you verified (with commands and results), and what remains open. Never claim a check
passed that you did not run."#;

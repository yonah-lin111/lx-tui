//! 子 agent 领域层：角色、工具面、控制工具参数、结果视图与报告信封。
//!
//! 纯数据，无 IO、无 `app/`/`ui/` 依赖。工具名、角色名与错误码是稳定契约
//! （工具参数 / DB `agent_role` / 提示 / 日志），不得改名；改名等于换 agent，
//! 必须同时递增 [`crate::chat::prompt::PROMPT_VERSION`]。

use serde_json::{Value, json};

use crate::chat::tools::ToolExecutionResult;

mod parse;
pub use parse::parse_control;

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
const MAIN_TOOLS: [&str; 16] = [
    "read",
    "search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
    "update_plan",
    "ask",
    "curr_time",
    "spawn_agent",
    "send_input",
    "resume_agent",
    "wait_agent",
    "close_agent",
    "list_agents",
    "cancel_agent_input",
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
///
/// 子 agent 的完整调查通常要几分钟，60s 会让父轮次在目标仍在运行时提前收尾；
/// 与 codex 的“prefer longer waits (minutes)”一致，默认给 5 分钟。
pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 300_000;
/// `wait_agent` 超时上限（毫秒），对齐 codex 的 1 小时硬上限。
pub const MAX_WAIT_TIMEOUT_MS: u64 = 3_600_000;
/// 报告正文上限（字节）。
pub const MAX_REPORT_BYTES: usize = 8 * 1024;
/// `spawn_agent` 的 task 文本上限（字符）。
pub const MAX_TASK_CHARS: usize = 8_000;
/// `spawn_agent` 的短标题上限（字符）。标题给人和父消息卡片看，不是任务提示词。
pub const MAX_TITLE_CHARS: usize = 80;
/// 单次 `spawn_agent` / `send_input` 的结构化 `items` 条目上限。
pub const MAX_INPUT_ITEMS: usize = 32;

/// 并发超限的稳定错误码。
pub const LIMIT_REACHED_CODE: &str = "subagent concurrency limit reached";
/// 目标不存在的稳定错误码。
pub const NOT_FOUND_CODE: &str = "subagent not found";
/// 目标已结束的稳定错误码。
pub const FINISHED_CODE: &str = "subagent already finished";
/// 目标未恢复（终态或重启后需先 `resume_agent`）的稳定错误码。
pub const NOT_ACTIVE_CODE: &str = "subagent is not active; call resume_agent first";
/// 排队输入不存在的稳定错误码。
pub const INPUT_NOT_FOUND_CODE: &str = "subagent input not found";
/// 归档图片丢失或校验失败的稳定错误码。
pub const IMAGE_MISSING_CODE: &str = "image_missing";
/// 子 agent 内禁止控制工具的稳定错误码。
pub const UNAVAILABLE_CODE: &str = "subagent tools unavailable";

/// `wait_agent` 的完成条件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitMode {
    /// 等全部目标结束（默认）。
    All,
    /// 任意一个目标结束就返回。
    Any,
}

impl WaitMode {
    /// 稳定 JSON 值。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Any => "any",
        }
    }

    /// 由稳定 JSON 值还原；未知值回退 `All`（兼容旧参数）。
    pub fn parse(value: &str) -> Self {
        match value {
            "any" => Self::Any,
            _ => Self::All,
        }
    }
}

/// 子 agent 结构化输入项（保持原始出现顺序，文本与图片可交错）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentInputItem {
    /// 纯文本段落：`{"type":"text","text":"..."}`。
    Text { text: String },
    /// 本地图片路径：`{"type":"local_image","path":"..."}`（相对路径以父轮冻结 cwd 解析）。
    LocalImage { path: String },
    /// 内联 data URL 图片：`{"type":"image","image_url":"data:image/...;base64,..."}`。
    Image { image_url: String },
}

/// 将结构化输入项中的文本合并为可读任务摘要；纯图片输入回退为 `"[image]"`。
pub fn joined_input_text(items: &[SubagentInputItem]) -> String {
    let mut parts = Vec::new();
    let mut image_count = 0usize;
    for item in items {
        match item {
            SubagentInputItem::Text { text } => {
                if !text.is_empty() {
                    parts.push(text.as_str());
                }
            }
            SubagentInputItem::LocalImage { .. } | SubagentInputItem::Image { .. } => {
                image_count = image_count.saturating_add(1);
            }
        }
    }
    if !parts.is_empty() {
        parts.join("\n")
    } else if image_count > 0 {
        format!("[{image_count} image(s)]")
    } else {
        String::new()
    }
}

/// `list_agents` 的单条子 agent 概览（不含完整消息流或报告正文）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentListItemView {
    pub agent_id: String,
    pub role: AgentRole,
    pub title: String,
    pub status: String,
    pub queued_count: usize,
}

/// 子 agent 控制工具操作；由 loop 转投事件层执行（与 `ask` 同一分支位置）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOp {
    /// 派发一个子 agent。
    Spawn {
        role: AgentRole,
        /// 面向用户的短标题（不是 task 原文）。
        title: String,
        task: String,
        /// 有序结构化输入项（旧 `task` 参数走单条 `Text`）。
        items: Vec<SubagentInputItem>,
    },
    /// 向已恢复或运行中的子 agent 发送输入（持久排队或插队中断当前轮）。
    SendInput {
        target: String,
        items: Vec<SubagentInputItem>,
        interrupt: bool,
    },
    /// 显式恢复终态或重启后的子 agent。
    Resume { id: String },
    /// 等待子 agent 结束（或超时）。`targets` 为空表示等待本会话全部
    /// 子 agent；`mode` 决定等全部还是任意一个。
    Wait {
        targets: Vec<String>,
        mode: WaitMode,
        timeout_ms: u64,
    },
    /// 停止一个子 agent（不产生报告）。
    Close { target: String },
    /// 列出当前父会话下的全部子 agent 状态与排队计数。
    List,
    /// 取消一条尚未开始的排队输入。
    CancelInput { submission_id: String },
}

/// 操作结果：工具结果 JSON 与 UI 详情共用的唯一来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOutcome {
    /// 已派发（不等待）。
    Spawned {
        agent_id: String,
        role: AgentRole,
        title: String,
    },
    /// 输入已持久接收并入队（或立即启动）。
    InputAccepted {
        submission_id: String,
        agent_id: String,
        status: String,
        queued_count: usize,
        interrupt: bool,
    },
    /// 子 agent 已显式恢复。
    Resumed {
        agent_id: String,
        status: String,
        queued_count: usize,
    },
    /// 等待结算；`report` 只在本次领取到报告时非空。
    Waited {
        timed_out: bool,
        agents: Vec<AgentStatusView>,
        /// 本次是否以「全部子 agent」为目标（targets 为空）。
        all: bool,
        /// 全部等待或任意一个完成。
        mode: WaitMode,
    },
    /// 已停止；不产生报告。
    Closed {
        agent_id: String,
        previous_status: String,
    },
    /// 当前父会话下的子 agent 列表。
    Listed { agents: Vec<AgentListItemView> },
    /// 排队输入取消结果（仅 queued 转为 cancelled；running/done 返回现状）。
    InputCancelled {
        submission_id: String,
        agent_id: String,
        status: String,
        queued_count: usize,
    },
}

/// 一个子 agent 的结算视图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusView {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 面向用户的短标题；旧记录可能为空。
    pub title: String,
    /// 角色。
    pub role: AgentRole,
    /// `running` / `completed` / `failed` / `cancelled`。
    pub status: String,
    /// 本次等待结算到的报告正文；未结束或已投递过为 None。
    pub report: Option<String>,
}

impl SubagentOutcome {
    /// 结果对应的稳定工具名。
    pub const fn tool_name(&self) -> &'static str {
        match self {
            Self::Spawned { .. } => "spawn_agent",
            Self::InputAccepted { .. } => "send_input",
            Self::Resumed { .. } => "resume_agent",
            Self::Waited { .. } => "wait_agent",
            Self::Closed { .. } => "close_agent",
            Self::Listed { .. } => "list_agents",
            Self::InputCancelled { .. } => "cancel_agent_input",
        }
    }

    /// 结果序列化（UI 详情、历史恢复、模型回执同源）。
    pub fn to_json(&self) -> Value {
        match self {
            Self::Spawned {
                agent_id,
                role,
                title,
            } => json!({
                "tool": "spawn_agent",
                "agent_id": agent_id,
                "role": role.as_str(),
                "title": title,
            }),
            Self::InputAccepted {
                submission_id,
                agent_id,
                status,
                queued_count,
                interrupt,
            } => json!({
                "tool": "send_input",
                "submission_id": submission_id,
                "agent_id": agent_id,
                "status": status,
                "queued_count": queued_count,
                "interrupt": interrupt,
            }),
            Self::Resumed {
                agent_id,
                status,
                queued_count,
            } => json!({
                "tool": "resume_agent",
                "agent_id": agent_id,
                "status": status,
                "queued_count": queued_count,
            }),
            Self::Waited {
                timed_out,
                agents,
                all,
                mode,
            } => json!({
                "tool": "wait_agent",
                "timed_out": timed_out,
                "all": all,
                "wait_for": mode.as_str(),
                "agents": agents
                    .iter()
                    .map(|agent| json!({
                        "agent_id": agent.agent_id,
                        "title": agent.title,
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
            Self::Listed { agents } => json!({
                "tool": "list_agents",
                "agents": agents
                    .iter()
                    .map(|agent| json!({
                        "agent_id": agent.agent_id,
                        "role": agent.role.as_str(),
                        "title": agent.title,
                        "status": agent.status,
                        "queued_count": agent.queued_count,
                    }))
                    .collect::<Vec<_>>(),
            }),
            Self::InputCancelled {
                submission_id,
                agent_id,
                status,
                queued_count,
            } => json!({
                "tool": "cancel_agent_input",
                "submission_id": submission_id,
                "agent_id": agent_id,
                "status": status,
                "queued_count": queued_count,
            }),
        }
    }

    /// 从结果 JSON 还原（历史重载用）；形状不符返回 None。
    pub fn from_json(value: &Value) -> Option<Self> {
        match value.get("tool").and_then(Value::as_str)? {
            "spawn_agent" => Some(Self::Spawned {
                agent_id: value.get("agent_id")?.as_str()?.to_string(),
                role: AgentRole::parse(value.get("role")?.as_str()?)?,
                title: value
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            }),
            "send_input" => Some(Self::InputAccepted {
                submission_id: value.get("submission_id")?.as_str()?.to_string(),
                agent_id: value.get("agent_id")?.as_str()?.to_string(),
                status: value.get("status")?.as_str()?.to_string(),
                queued_count: usize::try_from(
                    value
                        .get("queued_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
                .unwrap_or(0),
                interrupt: value
                    .get("interrupt")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }),
            "resume_agent" => Some(Self::Resumed {
                agent_id: value.get("agent_id")?.as_str()?.to_string(),
                status: value.get("status")?.as_str()?.to_string(),
                queued_count: usize::try_from(
                    value
                        .get("queued_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
                .unwrap_or(0),
            }),
            "wait_agent" => {
                let agents = value.get("agents")?.as_array()?;
                let mut parsed = Vec::with_capacity(agents.len());
                for agent in agents {
                    parsed.push(AgentStatusView {
                        agent_id: agent.get("agent_id")?.as_str()?.to_string(),
                        title: agent
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
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
                    all: value.get("all").and_then(Value::as_bool).unwrap_or(false),
                    mode: WaitMode::parse(
                        value
                            .get("wait_for")
                            .and_then(Value::as_str)
                            .unwrap_or("all"),
                    ),
                })
            }
            "close_agent" => Some(Self::Closed {
                agent_id: value.get("agent_id")?.as_str()?.to_string(),
                previous_status: value.get("previous_status")?.as_str()?.to_string(),
            }),
            "list_agents" => {
                let entries = value.get("agents")?.as_array()?;
                let mut agents = Vec::with_capacity(entries.len());
                for entry in entries {
                    agents.push(AgentListItemView {
                        agent_id: entry.get("agent_id")?.as_str()?.to_string(),
                        role: AgentRole::parse(entry.get("role")?.as_str()?)?,
                        title: entry
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        status: entry.get("status")?.as_str()?.to_string(),
                        queued_count: usize::try_from(
                            entry
                                .get("queued_count")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                        )
                        .unwrap_or(0),
                    });
                }
                Some(Self::Listed { agents })
            }
            "cancel_agent_input" => Some(Self::InputCancelled {
                submission_id: value.get("submission_id")?.as_str()?.to_string(),
                agent_id: value
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                status: value.get("status")?.as_str()?.to_string(),
                queued_count: usize::try_from(
                    value
                        .get("queued_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
                .unwrap_or(0),
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
    /// 面向用户的短标题；旧记录为空。
    pub title: String,
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
    /// 落库 JSON（`agent_subagent.report_json` 共用；消息侧批量存数组）。
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }

    fn to_value(&self) -> Value {
        json!({
            "agent_id": self.agent_id,
            "title": self.title,
            "role": self.role.as_str(),
            "status": self.status,
            "files_changed": self.files_changed,
            "duration_ms": self.duration_ms,
            "body": self.body,
            "delivered_at_ms": self.delivered_at_ms,
        })
    }

    /// 从落库 JSON 还原单条报告；损坏返回 None。
    pub fn from_json(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        Self::from_value(&value)
    }

    fn from_value(value: &Value) -> Option<Self> {
        Some(Self {
            agent_id: value.get("agent_id")?.as_str()?.to_string(),
            title: value
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
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

/// 消息 `subagent_report_json` 反序列化：新格式为数组，旧格式为单对象。
pub fn reports_from_json(text: &str) -> Vec<SubagentReportView> {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(SubagentReportView::from_value)
            .collect(),
        Value::Object(_) => SubagentReportView::from_value(&value).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// 批量报告落库 JSON：统一存数组，单条也走数组。
pub fn reports_to_json(reports: &[SubagentReportView]) -> String {
    Value::Array(reports.iter().map(SubagentReportView::to_value).collect()).to_string()
}

/// 注入父会话的报告文本：纯文本 + XML 包裹 + 元字符转义。
pub fn format_report_envelope(report: &SubagentReportView) -> String {
    let body = truncate_report_body(&sanitize_control(&report.body));
    let duration = report
        .duration_ms
        .map(|value| format!("{value}"))
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "<subagent_report agent_id=\"{}\" title=\"{}\" role=\"{}\" status=\"{}\" files_changed=\"{}\" duration_ms=\"{}\">\n{}\n</subagent_report>",
        escape_attribute(&report.agent_id),
        escape_attribute(&report.title),
        report.role.as_str(),
        escape_attribute(&report.status),
        report.files_changed,
        duration,
        escape_text(&body),
    )
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
    format!(
        "{}\n\n{}\n",
        role.system_prompt(),
        environment.unwrap_or_default()
    )
}

/// explorer 角色提示。
pub const EXPLORER_PROMPT: &str = r#"You are an explorer: a read-only investigator working inside the user's workspace.

You answer one specific question about the codebase or inspect images attached to your task.
You have `read` and `search` and nothing else — you cannot modify files, run commands, or spawn
agents. Any image passed in your user message is already attached directly to this conversation:
inspect and describe it directly from the message without calling `read`.

# How you work

- Start from the question, not from the repository. If images are attached in the user message,
  inspect them directly; for codebase questions, work out which paths answer them and read them.
- Follow the code, not the docs: a comment can be stale, the call graph cannot.
- For codebase claims, cite evidence as `path:line` (a codebase claim without a location is a
  guess; say so if you cannot find one). Visual observations of attached images do not require
  a file path citation.
- Stop when the question is answered. Do not audit unrelated code, do not suggest refactors, and
  do not write patches — the caller decides what to do with your findings.
- If the question cannot be answered with the tools and attached inputs you have (it needs a
  command, a test run, or a file change), say exactly that and what would be needed. Do not
  pretend you checked.

# Final message

Your final message is the report and the only thing the caller reads. Lead with the answer, then
the evidence, then anything the caller must know to act on it. Keep it under roughly 40 lines.
State explicitly what you could not verify."#;

/// executor 角色提示。
pub const EXECUTOR_PROMPT: &str = r#"You are an executor: a coding agent working inside the user's workspace on one assigned task.

# Your boundary

You were given a goal, a boundary, and a deliverable (plus any images attached directly to your
user message, which you can inspect visually without calling `read`). Work inside that boundary.
If the task turns out to need changes outside it, stop and report why instead of widening the
scope yourself.

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

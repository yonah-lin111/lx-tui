//! 子 agent 领域层：角色、工具面、控制工具参数、结果视图与报告信封。
//!
//! 纯数据，无 IO、无 `app/`/`ui/` 依赖。工具名、角色名与错误码是稳定契约
//! （工具参数 / DB `agent_role` / 提示 / 日志），不得改名；改名等于换 agent，
//! 必须同时递增 [`crate::chat::prompt::PROMPT_VERSION`]。

use serde_json::{Value, json};

use crate::chat::tools::ToolExecutionResult;

mod budget;
pub mod exec;
mod instructions;
mod model_switch;
mod nickname;
mod parse;
mod path;
mod report;
mod role;
mod status;

pub use budget::*;
pub use instructions::{
    AGENTS_MD_SEPARATOR, CANDIDATE_AGENT_FILENAMES, DEFAULT_AGENTS_MD_FILENAME,
    LOCAL_AGENTS_MD_FILENAME, LoadedInstructions, MAX_GLOBAL_DOC_BYTES, MAX_PROJECT_DOC_BYTES,
    ensure_global_agents_md, load_instructions,
};
pub use model_switch::{
    ModelIdentity, SOURCE_CORE_AGENTS_MD, SOURCE_CORE_MODEL, SOURCE_CORE_REASONING_EFFORT,
    compute_updated_sources, render_model_switch_update, wrap_system_update,
};
pub use nickname::{DEFAULT_NICKNAME_CANDIDATES, allocate_nickname, format_agent_identity};
pub use parse::parse_control;
pub use path::*;
pub use report::{
    MAX_REPORT_CHANGED_PATHS, SubagentReportView, format_report_envelope, reports_from_json,
    reports_to_json, synthesize_partial_report_body, truncate_report_body,
};
pub use role::{AgentRole, BaseRole, MAX_ROLE_NAME_LEN, is_valid_role_name};
pub use status::*;

#[cfg(test)]
mod tests;

/// 工具面：决定一次请求声明哪些工具。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSurface {
    /// 主 agent：现有内置工具 + 8 个控制工具。
    Main,
    /// 只读调查。
    Explorer,
    /// 有边界的实现（叶节点或已达 `max_depth`）。
    Executor,
    /// 未达 `max_depth` 的实现者：7 项实现工具 + 8 项子 agent 控制工具（共 15 项）。
    ExecutorCoordinator,
    /// CLI exec 无头主 agent：剔除交互式 ask 后的全部主工具（17 项）。
    ExecMain,
}

/// 主 agent 工具名（顺序即呈现顺序）。
const MAIN_TOOLS: [&str; 18] = [
    "read",
    "search",
    "web_search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
    "update_plan",
    "ask",
    "curr_time",
    "spawn_agent",
    "send_input",
    "interrupt_agent",
    "resume_agent",
    "wait_agent",
    "close_agent",
    "list_agents",
    "cancel_agent_input",
];

/// exec 无头主 agent 工具名：`MAIN_TOOLS` 去掉交互式 `ask`，顺序保持一致。
const EXEC_MAIN_TOOLS: [&str; 17] = [
    "read",
    "search",
    "web_search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
    "update_plan",
    "curr_time",
    "spawn_agent",
    "send_input",
    "interrupt_agent",
    "resume_agent",
    "wait_agent",
    "close_agent",
    "list_agents",
    "cancel_agent_input",
];

/// explorer 工具名。
const EXPLORER_TOOLS: [&str; 3] = ["read", "search", "web_search"];

/// executor 工具名。
const EXECUTOR_TOOLS: [&str; 7] = [
    "read",
    "search",
    "web_search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
];

/// 协调型 executor 工具名：`EXECUTOR_TOOLS` + 8 个控制工具。
const EXECUTOR_COORDINATOR_TOOLS: [&str; 15] = [
    "read",
    "search",
    "web_search",
    "edit",
    "write",
    "exec_command",
    "write_stdin",
    "spawn_agent",
    "send_input",
    "interrupt_agent",
    "resume_agent",
    "wait_agent",
    "close_agent",
    "list_agents",
    "cancel_agent_input",
];

impl ToolSurface {
    /// 该工具面的工具名（逐字稳定，测试锁死）。
    pub const fn names(self) -> &'static [&'static str] {
        match self {
            Self::Main => &MAIN_TOOLS,
            Self::Explorer => &EXPLORER_TOOLS,
            Self::Executor => &EXECUTOR_TOOLS,
            Self::ExecutorCoordinator => &EXECUTOR_COORDINATOR_TOOLS,
            Self::ExecMain => &EXEC_MAIN_TOOLS,
        }
    }

    /// 角色对应的工具面（由底层 `BaseRole` 决定）。
    pub const fn from_role(role: AgentRole) -> Self {
        match role.base() {
            BaseRole::Explorer => Self::Explorer,
            BaseRole::Executor => Self::Executor,
        }
    }

    /// 角色在指定深度下的工具面：`Executor` 仅在 `depth < max_depth` 时获得控制工具。
    pub const fn from_role_at_depth(role: AgentRole, depth: usize, max_depth: usize) -> Self {
        match role.base() {
            BaseRole::Explorer => Self::Explorer,
            BaseRole::Executor => {
                if depth < max_depth {
                    Self::ExecutorCoordinator
                } else {
                    Self::Executor
                }
            }
        }
    }
}

/// 每会话并发上限（硬拒绝，不排队）。
pub const MAX_CONCURRENT_SUBAGENTS: usize = 4;
/// 单次 `spawn_agent` 允许指定的最大分叉轮次数。
pub const MAX_FORK_TURNS: u32 = 64;
/// Token 预算耗尽时的稳定停止原因码。
pub const BUDGET_EXCEEDED_CODE: &str = "budget_exceeded";
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
/// exec 无头模式不支持交互式 `ask` 的稳定错误码。
pub const EXEC_ASK_UNSUPPORTED_CODE: &str =
    "ask is not supported in exec mode; proceed with best-effort assumptions";

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

/// 子 agent 历史分叉策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkSpec {
    /// 不继承父会话历史（默认）。
    None,
    /// 继承父会话当前流式轮次之前的全部已完成轮次。
    AllCompleted,
    /// 仅继承父会话当前流式轮次之前的最近 `N` 个已完成轮次（`1..=64`）。
    RecentTurns(u32),
}

impl ForkSpec {
    /// 是否启用历史分叉。
    pub const fn is_enabled(self) -> bool {
        !matches!(self, Self::None)
    }

    /// 按轮次截取时的轮次数（`AllCompleted` / `None` 返回 None）。
    pub const fn fork_turns(self) -> Option<u32> {
        match self {
            Self::RecentTurns(turns) => Some(turns),
            Self::None | Self::AllCompleted => None,
        }
    }

    /// 由 `fork_context` 与可选 `fork_turns` 归一化：`Some(0)` 等价于不分叉，
    /// `Some(n > 0)` 隐含启用分叉。
    pub fn from_params(fork_context: bool, fork_turns: Option<u32>) -> Self {
        match fork_turns {
            Some(0) => Self::None,
            Some(turns) => Self::RecentTurns(turns.min(MAX_FORK_TURNS)),
            None if fork_context => Self::AllCompleted,
            None => Self::None,
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
    pub nickname: Option<String>,
    /// 规范路径（如 `/root/turing`）；旧记录为 None。
    pub agent_path: Option<String>,
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
        /// 可选路径段（`[a-z0-9][a-z0-9_]{0,31}`）；未传时由昵称派生。
        task_name: Option<String>,
        task: String,
        /// 有序结构化输入项（旧 `task` 参数走单条 `Text`）。
        items: Vec<SubagentInputItem>,
        /// 是否克隆父会话当前流式轮次之前的已完成历史。
        fork_context: bool,
        /// 可选分叉轮次数（`0..=64`；`> 0` 时隐含 `fork_context = true`）。
        fork_turns: Option<u32>,
        /// 可选单轮 Token 预算覆写（至少 1000）。
        max_tokens: Option<u64>,
        /// 可选模型覆写（`"provider/model"` 或 `"model"`）。
        model: Option<String>,
        /// 可选思考等级覆写（对应模型 `variants` key）。
        reasoning_effort: Option<String>,
    },
    /// 向已恢复或运行中的子 agent 发送输入（持久排队或插队中断当前轮）。
    SendInput {
        target: String,
        items: Vec<SubagentInputItem>,
        interrupt: bool,
        /// 是否在子 agent 空闲时立即唤醒启动新轮次（默认 true；false 仅入队）。
        trigger_turn: bool,
    },
    /// 中断正在运行的子 agent 当前轮次，保留其 `idle` 活动态并暂停消费后续排队输入。
    Interrupt { target: String },
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
    /// 列出当前父会话（或指定路径前缀下）的子 agent 状态与排队计数。
    List { path_prefix: Option<String> },
    /// 取消一条尚未开始的排队输入。
    CancelInput { submission_id: String },
}

/// 操作结果：工具结果 JSON 与 UI 详情共用的唯一来源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentOutcome {
    /// 已派发（不等待）。
    Spawned {
        agent_id: String,
        nickname: Option<String>,
        /// 规范路径（如 `/root/turing`）；旧记录为 None。
        agent_path: Option<String>,
        role: AgentRole,
        title: String,
        model: Option<String>,
        reasoning_effort: Option<String>,
        fork_context: bool,
        /// 按轮次分叉时的轮次数；未指定为 None。
        fork_turns: Option<u32>,
        /// 本次派发生效的 Token 预算上限；未指定为 None。
        max_tokens: Option<u64>,
    },
    /// 输入已持久接收并入队（或立即启动）。
    InputAccepted {
        submission_id: String,
        agent_id: String,
        status: String,
        queued_count: usize,
        interrupt: bool,
        /// 是否立即唤醒空闲子 agent（默认 true）。
        trigger_turn: bool,
    },
    /// 已中断当前正在运行的轮次（保留半截输出、保持 `idle` 且暂停后续队列自动消费）。
    Interrupted {
        agent_id: String,
        nickname: Option<String>,
        previous_status: String,
        queued_count: usize,
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
    /// 可选唯一昵称。
    pub nickname: Option<String>,
    /// 规范路径（如 `/root/turing`）；旧记录为 None。
    pub agent_path: Option<String>,
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
            Self::Interrupted { .. } => "interrupt_agent",
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
                nickname,
                agent_path,
                role,
                title,
                model,
                reasoning_effort,
                fork_context,
                fork_turns,
                max_tokens,
            } => {
                let mut obj = json!({
                    "tool": "spawn_agent",
                    "agent_id": agent_id,
                    "role": role.as_str(),
                    "title": title,
                });
                if let Some(map) = obj.as_object_mut() {
                    if let Some(nick) = nickname.as_deref().filter(|s| !s.is_empty()) {
                        map.insert("nickname".into(), Value::String(nick.to_string()));
                    }
                    if let Some(path) = agent_path.as_deref().filter(|s| !s.is_empty()) {
                        map.insert("agent_path".into(), Value::String(path.to_string()));
                    }
                    if let Some(m) = model.as_deref().filter(|s| !s.is_empty()) {
                        map.insert("model".into(), Value::String(m.to_string()));
                    }
                    if let Some(effort) = reasoning_effort.as_deref().filter(|s| !s.is_empty()) {
                        map.insert("reasoning_effort".into(), Value::String(effort.to_string()));
                    }
                    if *fork_context {
                        map.insert("fork_context".into(), Value::Bool(true));
                    }
                    if let Some(turns) = fork_turns {
                        map.insert("fork_turns".into(), Value::from(*turns));
                    }
                    if let Some(tokens) = max_tokens {
                        map.insert("max_tokens".into(), Value::from(*tokens));
                    }
                }
                obj
            }
            Self::InputAccepted {
                submission_id,
                agent_id,
                status,
                queued_count,
                interrupt,
                trigger_turn,
            } => json!({
                "tool": "send_input",
                "submission_id": submission_id,
                "agent_id": agent_id,
                "status": status,
                "queued_count": queued_count,
                "interrupt": interrupt,
                "trigger_turn": trigger_turn,
            }),
            Self::Interrupted {
                agent_id,
                nickname,
                previous_status,
                queued_count,
            } => {
                let mut obj = json!({
                    "tool": "interrupt_agent",
                    "agent_id": agent_id,
                    "previous_status": previous_status,
                    "queued_count": queued_count,
                });
                if let Some(nick) = nickname.as_deref().filter(|s| !s.is_empty())
                    && let Some(map) = obj.as_object_mut()
                {
                    map.insert("nickname".into(), Value::String(nick.to_string()));
                }
                obj
            }
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
                    .map(|agent| {
                        let mut item = json!({
                            "agent_id": agent.agent_id,
                            "title": agent.title,
                            "role": agent.role.as_str(),
                            "status": agent.status,
                            "report": agent.report,
                        });
                        if let Some(map) = item.as_object_mut() {
                            if let Some(nick) = agent.nickname.as_deref().filter(|s| !s.is_empty()) {
                                map.insert("nickname".into(), Value::String(nick.to_string()));
                            }
                            if let Some(path) = agent.agent_path.as_deref().filter(|s| !s.is_empty())
                            {
                                map.insert("agent_path".into(), Value::String(path.to_string()));
                            }
                        }
                        item
                    })
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
                    .map(|agent| {
                        let mut item = json!({
                            "agent_id": agent.agent_id,
                            "role": agent.role.as_str(),
                            "title": agent.title,
                            "status": agent.status,
                            "queued_count": agent.queued_count,
                        });
                        if let Some(map) = item.as_object_mut() {
                            if let Some(nick) = agent.nickname.as_deref().filter(|s| !s.is_empty()) {
                                map.insert("nickname".into(), Value::String(nick.to_string()));
                            }
                            if let Some(path) = agent.agent_path.as_deref().filter(|s| !s.is_empty())
                            {
                                map.insert("agent_path".into(), Value::String(path.to_string()));
                            }
                        }
                        item
                    })
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
                nickname: value
                    .get("nickname")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                agent_path: value
                    .get("agent_path")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                role: AgentRole::parse(value.get("role")?.as_str()?)?,
                title: value
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                model: value
                    .get("model")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                reasoning_effort: value
                    .get("reasoning_effort")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                fork_context: value
                    .get("fork_context")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                fork_turns: value
                    .get("fork_turns")
                    .and_then(Value::as_u64)
                    .and_then(|turns| u32::try_from(turns).ok()),
                max_tokens: value.get("max_tokens").and_then(Value::as_u64),
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
                trigger_turn: value
                    .get("trigger_turn")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            }),
            "interrupt_agent" => Some(Self::Interrupted {
                agent_id: value.get("agent_id")?.as_str()?.to_string(),
                nickname: value
                    .get("nickname")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                previous_status: value
                    .get("previous_status")
                    .and_then(Value::as_str)
                    .unwrap_or("running")
                    .to_string(),
                queued_count: usize::try_from(
                    value
                        .get("queued_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                )
                .unwrap_or(0),
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
                        nickname: agent
                            .get("nickname")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                        agent_path: agent
                            .get("agent_path")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
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
                        nickname: entry
                            .get("nickname")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
                        agent_path: entry
                            .get("agent_path")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string),
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

/// 子 agent 系统提示：角色提示 + 环境快照。
///
/// **不含**父会话的后台任务摘要与 plan：子 agent 只看到自己的任务。
pub fn compose_subagent_prompt(
    role: AgentRole,
    parts: crate::chat::prompt::PromptParts<'_>,
) -> String {
    compose_subagent_prompt_with_override(role, None, parts)
}

/// 子 agent 系统提示（支持传入设置页覆写或自定义角色的系统提示词）。
pub fn compose_subagent_prompt_with_override(
    role: AgentRole,
    custom_prompt: Option<&str>,
    parts: crate::chat::prompt::PromptParts<'_>,
) -> String {
    let head = custom_prompt
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| role.system_prompt());
    format!("{head}\n\n{}\n", parts.render_tail_with(false))
}

/// explorer 角色提示。
pub const EXPLORER_PROMPT: &str = r#"You are an explorer: a read-only investigator working inside the user's workspace.

You answer one specific question about the codebase or inspect images attached to your task.
You have `read`, `search`, `web_search`, and any MCP tools listed for this turn — you cannot modify
files, run commands, or spawn agents. `web_search` reaches the public internet; when your answer
relies on an external source, cite it inline as `[title](url)`. MCP tools may have side effects: as
an explorer, only call MCP tools that read data; never create, update or delete anything through
them. Any image passed in your user message is already attached directly to this conversation:
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

You have only the tools listed in this turn's tool declarations. Unless subagent control tools
are explicitly provided in your tool declarations, do not attempt to delegate further.

You are not alone in this workspace. Other agents may be editing other files at the same time.
Never revert, reformat, or overwrite changes you did not make; if a file changed under you, re-read
it and work with the current content.

# How you work

- Read before you change: `edit` and `write` reject a file you have not read in this session or
  that changed on disk after the read.
- When an external library or API contract decides the fix, call `web_search` to verify it instead
  of guessing, and cite the source inline as `[title](url)`.
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

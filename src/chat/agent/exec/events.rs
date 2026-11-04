//! `exec` 输出事件模型：与设计 §4.3.1 对齐的强类型 JSONL 事件，以及
//! 「工具调用 → 事件项」的纯映射与人类可读单行格式化。
//!
//! 本模块只有纯数据与纯函数：不写 IO、不读时钟、不依赖 `app`/`ui`/`event`。
//! 输出由 [`super::processor`] 负责。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::agent::SubagentOutcome;
use crate::chat::agent_loop::{ReadyToolCall, ToolOutcome};
use crate::chat::process::ProcessResultKind;
use crate::chat::tools::{FileChange, FileChangeKind, ReadKind, ToolExecutionResult, to_summary};
use crate::db::chat::Usage;

/// 人类可读标题的最大字符数（超出截断，避免一行撑满终端）。
const MAX_HEADER_CHARS: usize = 120;
/// 单行摘要的最大字符数。
const MAX_SUMMARY_CHARS: usize = 200;

/// `--json` 模式下每行输出一条的顶层事件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ThreadEvent {
    #[serde(rename = "thread.started")]
    ThreadStarted { thread_id: String },
    #[serde(rename = "turn.started")]
    TurnStarted,
    #[serde(rename = "item.started")]
    ItemStarted { item: ThreadItem },
    #[serde(rename = "item.updated")]
    ItemUpdated { item: ThreadItem },
    #[serde(rename = "item.completed")]
    ItemCompleted { item: ThreadItem },
    #[serde(rename = "turn.completed")]
    TurnCompleted { usage: ExecUsage },
    #[serde(rename = "turn.failed")]
    TurnFailed { error: ThreadError },
    #[serde(rename = "error")]
    Error { message: String },
}

/// 一条带稳定 id 的事件项。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadItem {
    pub id: String,
    #[serde(flatten)]
    pub details: ThreadItemDetails,
}

/// 事件项的具体形状（按 `type` 内部标记）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ThreadItemDetails {
    AgentMessage {
        text: String,
    },
    Reasoning {
        text: String,
    },
    CommandExecution {
        command: String,
        aggregated_output: String,
        exit_code: Option<i32>,
        status: ItemStatus,
    },
    FileChange {
        changes: Vec<FileChangeEntry>,
        status: ItemStatus,
    },
    ToolCall {
        name: String,
        arguments: Value,
        summary: Option<String>,
        status: ItemStatus,
    },
    McpToolCall {
        server: String,
        tool: String,
        arguments: Value,
        status: ItemStatus,
        error: Option<String>,
    },
    CollabToolCall {
        tool: String,
        agent_id: Option<String>,
        role: Option<String>,
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        nickname: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fork_context: Option<bool>,
        status: ItemStatus,
    },
    WebSearch {
        query: String,
        status: ItemStatus,
    },
    TodoList {
        items: Vec<TodoEntry>,
    },
}

/// 事件项的终态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

/// 一次文件变更的展示条目。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileChangeEntry {
    pub path: String,
    /// `added` / `updated` / `deleted` / `moved`（稳定英文值）。
    pub kind: String,
}

/// 计划中的一步。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoEntry {
    pub step: String,
    pub status: String,
}

/// 一轮的 token 用量（未知值按 0 落盘，形状与 codex 对齐）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecUsage {
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_input_tokens: i64,
    pub output_tokens: i64,
}

impl From<Usage> for ExecUsage {
    fn from(usage: Usage) -> Self {
        Self {
            input_tokens: usage.input.unwrap_or_default(),
            cached_input_tokens: usage.cache_read.unwrap_or_default(),
            cache_write_input_tokens: usage.cache_write.unwrap_or_default(),
            output_tokens: usage.output.unwrap_or_default(),
        }
    }
}

/// 失败事件的错误体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadError {
    pub message: String,
}

/// 工具开始执行时的事件项。
pub fn item_details_for_start(call: &ReadyToolCall) -> ThreadItemDetails {
    let arguments = parse_arguments(call);
    match call.name.as_str() {
        "exec_command" | "write_stdin" => ThreadItemDetails::CommandExecution {
            command: command_from_arguments(&call.name, &arguments),
            aggregated_output: String::new(),
            exit_code: None,
            status: ItemStatus::InProgress,
        },
        "edit" | "write" => ThreadItemDetails::FileChange {
            changes: file_changes_from_arguments(&call.name, &arguments),
            status: ItemStatus::InProgress,
        },
        "web_search" => ThreadItemDetails::WebSearch {
            query: web_queries(&arguments).join(", "),
            status: ItemStatus::InProgress,
        },
        "update_plan" => ThreadItemDetails::TodoList {
            items: plan_items(&arguments),
        },
        name if is_collab_tool(name) => {
            let identity = collab_start_identity(name, &arguments);
            ThreadItemDetails::CollabToolCall {
                tool: name.to_string(),
                agent_id: identity.agent_id,
                role: identity.role,
                title: identity.title,
                nickname: identity.nickname,
                model: identity.model,
                fork_context: identity.fork_context,
                status: ItemStatus::InProgress,
            }
        }
        name if crate::mcp::is_mcp_tool(name) => {
            let (server, tool) = mcp_parts(name, None);
            ThreadItemDetails::McpToolCall {
                server,
                tool,
                arguments,
                status: ItemStatus::InProgress,
                error: None,
            }
        }
        name => ThreadItemDetails::ToolCall {
            name: name.to_string(),
            arguments,
            summary: None,
            status: ItemStatus::InProgress,
        },
    }
}

/// 工具结算时的事件项。
pub fn item_details_for_finish(call: &ReadyToolCall, outcome: &ToolOutcome) -> ThreadItemDetails {
    let arguments = parse_arguments(call);
    let status = status_of(outcome);
    let result = match outcome {
        ToolOutcome::Executed(result) => Some(result),
        ToolOutcome::Cancelled => None,
    };
    match call.name.as_str() {
        "exec_command" | "write_stdin" => {
            let process = result.and_then(|result| match result {
                ToolExecutionResult::Process(process) => Some(process.as_ref()),
                _ => None,
            });
            ThreadItemDetails::CommandExecution {
                command: process
                    .map(|process| process.command.clone())
                    .unwrap_or_else(|| command_from_arguments(&call.name, &arguments)),
                aggregated_output: process
                    .map(|process| process.output.clone())
                    .unwrap_or_default(),
                exit_code: process.and_then(|process| process.exit_code),
                status,
            }
        }
        "edit" | "write" => {
            let changes = result
                .filter(|result| {
                    matches!(
                        result,
                        ToolExecutionResult::Edit { .. }
                            | ToolExecutionResult::Write { .. }
                            | ToolExecutionResult::Failed { .. }
                    )
                })
                .map(file_changes_of)
                .unwrap_or_else(|| file_changes_from_arguments(&call.name, &arguments));
            ThreadItemDetails::FileChange { changes, status }
        }
        "web_search" => {
            let mut queries = web_queries(&arguments);
            if queries.is_empty()
                && let Some(ToolExecutionResult::WebSearch(search)) = result
            {
                queries = search.queries.clone();
            }
            ThreadItemDetails::WebSearch {
                query: queries.join(", "),
                status,
            }
        }
        "update_plan" => ThreadItemDetails::TodoList {
            items: result
                .and_then(|result| match result {
                    ToolExecutionResult::Plan(snapshot) => Some(
                        snapshot
                            .steps
                            .iter()
                            .map(|step| TodoEntry {
                                step: step.step.clone(),
                                status: step.status.as_str().to_string(),
                            })
                            .collect(),
                    ),
                    _ => None,
                })
                .unwrap_or_else(|| plan_items(&arguments)),
        },
        name if is_collab_tool(name) => {
            let identity = result
                .and_then(|result| match result {
                    ToolExecutionResult::Subagent(outcome) => Some(collab_identity(outcome)),
                    _ => None,
                })
                .unwrap_or_else(|| collab_start_identity(name, &arguments));
            let status = if collab_wait_failed(result) {
                ItemStatus::Failed
            } else {
                status
            };
            ThreadItemDetails::CollabToolCall {
                tool: name.to_string(),
                agent_id: identity.agent_id,
                role: identity.role,
                title: identity.title,
                nickname: identity.nickname,
                model: identity.model,
                fork_context: identity.fork_context,
                status,
            }
        }
        name if crate::mcp::is_mcp_tool(name) => {
            let details = result.and_then(|result| match result {
                ToolExecutionResult::Mcp(details) => Some(details.as_ref()),
                _ => None,
            });
            let (server, tool) = mcp_parts(name, details.map(|details| details.server.as_str()));
            ThreadItemDetails::McpToolCall {
                server,
                tool,
                arguments,
                status,
                error: result.and_then(failure_message),
            }
        }
        name => ThreadItemDetails::ToolCall {
            name: name.to_string(),
            arguments,
            summary: format_tool_summary(call, outcome),
            status,
        },
    }
}

/// 人类可读的调用标题（不含 `■`）。
pub fn format_tool_header(call: &ReadyToolCall) -> String {
    let arguments = parse_arguments(call);
    let header = match call.name.as_str() {
        "read" => format!("read {}", arg_str(&arguments, "path").unwrap_or_default()),
        "search" => {
            let pattern = arg_str(&arguments, "pattern").unwrap_or_default();
            match arg_str(&arguments, "path") {
                Some(path) => format!("search `{pattern}` in {path}"),
                None => format!("search `{pattern}`"),
            }
        }
        "write" => format!("write {}", arg_str(&arguments, "path").unwrap_or_default()),
        "edit" => {
            let paths: Vec<String> = file_changes_from_arguments("edit", &arguments)
                .into_iter()
                .map(|change| change.path)
                .collect();
            if paths.is_empty() {
                "edit".to_string()
            } else {
                format!("edit {}", paths.join(", "))
            }
        }
        "exec_command" => {
            let command = arg_str(&arguments, "cmd").unwrap_or_default();
            match arg_str(&arguments, "workdir") {
                Some(workdir) => format!("exec_command `{command}` in {workdir}"),
                None => format!("exec_command `{command}`"),
            }
        }
        "write_stdin" => match arguments.get("session_id").and_then(Value::as_i64) {
            Some(session) => format!("write_stdin session {session}"),
            None => "write_stdin".to_string(),
        },
        "web_search" => format!("web_search \"{}\"", web_queries(&arguments).join(", ")),
        "update_plan" => format!("update_plan ({} steps)", plan_items(&arguments).len()),
        name if is_collab_tool(name) => {
            let identity = collab_start_identity(name, &arguments);
            if identity.role.is_some() || identity.title.is_some() {
                let mut header = name.to_string();
                if let Some(role) = identity.role {
                    header.push(' ');
                    header.push_str(&role);
                }
                if let Some(title) = identity.title {
                    header.push_str(" · ");
                    header.push_str(&title);
                }
                header
            } else if let Some(target) = identity.agent_id {
                format!("{name} {target}")
            } else if name == "wait_agent" {
                "wait_agent (all)".to_string()
            } else {
                name.to_string()
            }
        }
        name if crate::mcp::is_mcp_tool(name) => {
            let (server, tool) = mcp_parts(name, None);
            format!("mcp {server}/{tool}")
        }
        name => match arg_str(&arguments, "path") {
            Some(path) => format!("{name} {path}"),
            None => name.to_string(),
        },
    };
    truncate(&header, MAX_HEADER_CHARS)
}

/// 人类可读的结算行（不含 `└─` 前缀）与是否按错误着色。
pub fn format_tool_outcome_line(
    call: &ReadyToolCall,
    outcome: &ToolOutcome,
    duration_ms: i64,
) -> (String, bool) {
    match outcome {
        ToolOutcome::Cancelled => ("cancelled".to_string(), false),
        ToolOutcome::Executed(result) => {
            let is_error = result.is_failure() || collab_wait_failed(Some(result));
            let line = match result {
                // `write_stdin` 只读回后台进程的一段输出：耗时属于本次交互，
                // 而不是进程自启动以来的总生存时间。
                ToolExecutionResult::Process(process)
                    if matches!(process.kind, ProcessResultKind::WriteStdin)
                        || call.name == "write_stdin" =>
                {
                    process_summary_line(process, duration_ms.max(0))
                }
                ToolExecutionResult::Process(process) => {
                    let elapsed = i64::try_from(process.wall_time_ms).unwrap_or(i64::MAX);
                    process_summary_line(process, elapsed)
                }
                _ => format!("{} ({}ms)", summarize_result(result), duration_ms.max(0)),
            };
            (truncate(&line, MAX_SUMMARY_CHARS), is_error)
        }
    }
}

/// 结果的单行摘要（JSON 的 `ToolCall.summary` 与人类输出的共用来源）。
pub fn format_tool_summary(_call: &ReadyToolCall, outcome: &ToolOutcome) -> Option<String> {
    let ToolOutcome::Executed(result) = outcome else {
        return None;
    };
    Some(truncate(&summarize_result(result), MAX_SUMMARY_CHARS))
}

fn summarize_result(result: &ToolExecutionResult) -> String {
    match result {
        ToolExecutionResult::Read { kind, .. } => {
            let summary = to_summary(result);
            match kind {
                ReadKind::File => format!("Read {} lines", summary.read_lines.unwrap_or_default()),
                ReadKind::Directory => {
                    format!("Listed {} entries", summary.total_lines.unwrap_or_default())
                }
                ReadKind::Image => "Read image".to_string(),
            }
        }
        ToolExecutionResult::Search {
            matches, truncated, ..
        } => {
            let suffix = if *truncated { " (truncated)" } else { "" };
            format!("{} matches{suffix}", matches.len())
        }
        ToolExecutionResult::Edit { changes, partial } => {
            let suffix = if *partial { " · partial" } else { "" };
            format!("{} changed{suffix}", changes.len())
        }
        ToolExecutionResult::Write {
            created,
            added,
            removed,
            ..
        } => {
            if *created {
                format!("Created (+{added} -{removed})")
            } else {
                format!("Overwrote (+{added} -{removed})")
            }
        }
        ToolExecutionResult::Process(process) => process_summary(process),
        ToolExecutionResult::Plan(snapshot) => format!(
            "{}/{} steps completed",
            snapshot.completed(),
            snapshot.steps.len()
        ),
        ToolExecutionResult::Ask { questions } => {
            format!("Answered {} questions", questions.len())
        }
        ToolExecutionResult::Subagent(outcome) => collab_summary(outcome),
        ToolExecutionResult::CurrentTime { current_time } => current_time.clone(),
        ToolExecutionResult::Mcp(details) => {
            format!("{} · {} completed", details.server, details.tool)
        }
        ToolExecutionResult::WebSearch(search) => {
            format!(
                "Searched \"{}\" · {}",
                search.queries.join(", "),
                result_count_label(search.num_results)
            )
        }
        ToolExecutionResult::Failed { code, message, .. } => {
            format!("{}: {message}", code.as_str())
        }
    }
}

fn process_summary(process: &crate::chat::process::ProcessResult) -> String {
    use crate::chat::process::ProcessState;
    match &process.state {
        ProcessState::Starting | ProcessState::Running => "running".to_string(),
        ProcessState::Exited { code: Some(code) } => format!("exit {code}"),
        ProcessState::Exited { code: None } => "exited".to_string(),
        ProcessState::Stopped => "stopped".to_string(),
        ProcessState::TimedOut => "timed out".to_string(),
        ProcessState::Interrupted => "interrupted".to_string(),
        ProcessState::Failed { code } => format!("failed: {code}"),
    }
}

fn process_summary_line(process: &crate::chat::process::ProcessResult, elapsed_ms: i64) -> String {
    use crate::chat::process::ProcessState;
    let elapsed = elapsed_ms.max(0);
    match &process.state {
        ProcessState::Starting | ProcessState::Running => {
            format!("running in background ({elapsed}ms)")
        }
        ProcessState::Exited { code: Some(0) } => format!("succeeded in {elapsed}ms"),
        ProcessState::Exited { code: Some(code) } => {
            format!("exited with code {code} in {elapsed}ms")
        }
        ProcessState::Exited { code: None } => format!("exited in {elapsed}ms"),
        ProcessState::Stopped => "stopped".to_string(),
        ProcessState::TimedOut => format!("timed out after {elapsed}ms"),
        ProcessState::Interrupted => "interrupted".to_string(),
        ProcessState::Failed { code } => format!("failed: {code}"),
    }
}

fn collab_summary(outcome: &SubagentOutcome) -> String {
    match outcome {
        SubagentOutcome::Spawned {
            agent_id,
            nickname,
            role,
            ..
        } => match clean_optional(nickname.as_deref()) {
            Some(nickname) => format!(
                "spawned {} ({agent_id})",
                crate::chat::agent::format_agent_identity(Some(nickname.as_str()), role.as_str())
            ),
            None => format!("spawned {agent_id} ({})", role.as_str()),
        },
        SubagentOutcome::InputAccepted { agent_id, .. } => {
            format!("input accepted for {agent_id}")
        }
        SubagentOutcome::Interrupted {
            agent_id,
            nickname,
            previous_status,
            queued_count,
        } => {
            let nickname = clean_optional(nickname.as_deref());
            let target = nickname.as_deref().unwrap_or(agent_id.as_str());
            let mut line = format!("interrupted {target} (was {previous_status}");
            if *queued_count > 0 {
                line.push_str(&format!(" · {queued_count} paused"));
            }
            line.push(')');
            line
        }
        SubagentOutcome::Resumed { agent_id, .. } => format!("resumed {agent_id}"),
        SubagentOutcome::Waited {
            timed_out, agents, ..
        } => {
            if *timed_out {
                "wait timed out".to_string()
            } else {
                format!("{} agent(s) settled", agents.len())
            }
        }
        SubagentOutcome::Closed { agent_id, .. } => format!("closed {agent_id}"),
        SubagentOutcome::Listed { agents } => format!("{} agent(s)", agents.len()),
        SubagentOutcome::InputCancelled { agent_id, .. } => {
            format!("cancelled pending input for {agent_id}")
        }
    }
}

/// 委派工具的展示身份：起始参数与结算结果都归一到同一形状。
#[derive(Debug, Default)]
struct CollabIdentity {
    agent_id: Option<String>,
    role: Option<String>,
    title: Option<String>,
    nickname: Option<String>,
    model: Option<String>,
    fork_context: Option<bool>,
}

fn collab_identity(outcome: &SubagentOutcome) -> CollabIdentity {
    match outcome {
        SubagentOutcome::Spawned {
            agent_id,
            nickname,
            role,
            title,
            model,
            reasoning_effort,
            fork_context,
        } => CollabIdentity {
            agent_id: Some(agent_id.clone()),
            role: Some(role.as_str().to_string()),
            title: Some(title.clone()),
            nickname: clean_optional(nickname.as_deref()),
            model: collab_model_label(model.as_deref(), reasoning_effort.as_deref()),
            fork_context: (*fork_context).then_some(true),
        },
        SubagentOutcome::Interrupted {
            agent_id, nickname, ..
        } => CollabIdentity {
            agent_id: Some(agent_id.clone()),
            nickname: clean_optional(nickname.as_deref()),
            ..CollabIdentity::default()
        },
        SubagentOutcome::InputAccepted { agent_id, .. }
        | SubagentOutcome::Resumed { agent_id, .. }
        | SubagentOutcome::Closed { agent_id, .. }
        | SubagentOutcome::InputCancelled { agent_id, .. } => CollabIdentity {
            agent_id: Some(agent_id.clone()),
            ..CollabIdentity::default()
        },
        SubagentOutcome::Waited { agents, .. } => match agents.as_slice() {
            [] => CollabIdentity::default(),
            [agent] => CollabIdentity {
                agent_id: Some(agent.agent_id.clone()),
                role: Some(agent.role.as_str().to_string()),
                title: Some(agent.title.clone()),
                nickname: clean_optional(agent.nickname.as_deref()),
                ..CollabIdentity::default()
            },
            agents => {
                let ids: Vec<&str> = agents.iter().map(|agent| agent.agent_id.as_str()).collect();
                // 角色只在全体一致时才有意义，否则留空避免误导。
                let role = agents
                    .first()
                    .map(|agent| agent.role)
                    .filter(|role| agents.iter().all(|agent| agent.role == *role))
                    .map(|role| role.as_str().to_string());
                let titles: Vec<&str> = agents
                    .iter()
                    .map(|agent| agent.title.as_str())
                    .filter(|title| !title.is_empty())
                    .collect();
                let title = if titles.is_empty() {
                    None
                } else {
                    Some(titles.join(", "))
                };
                CollabIdentity {
                    agent_id: Some(ids.join(", ")),
                    role,
                    title,
                    ..CollabIdentity::default()
                }
            }
        },
        SubagentOutcome::Listed { agents } => {
            let ids: Vec<&str> = agents.iter().map(|agent| agent.agent_id.as_str()).collect();
            CollabIdentity {
                agent_id: if ids.is_empty() {
                    None
                } else {
                    Some(ids.join(", "))
                },
                ..CollabIdentity::default()
            }
        }
    }
}

/// 子 agent 控制工具的起始标识：各工具 schema 的目标字段名并不统一。
fn collab_start_identity(name: &str, arguments: &Value) -> CollabIdentity {
    let agent_id = arg_str(arguments, "agent_id")
        .or_else(|| arg_str(arguments, "target"))
        .or_else(|| arg_str(arguments, "id"))
        .or_else(|| arg_str(arguments, "submission_id"))
        .map(str::to_string)
        .or_else(|| wait_targets(name, arguments));
    CollabIdentity {
        agent_id,
        role: arg_str(arguments, "role").map(str::to_string),
        title: arg_str(arguments, "title").map(str::to_string),
        nickname: clean_optional(arg_str(arguments, "nickname")),
        model: collab_model_label(
            arg_str(arguments, "model"),
            arg_str(arguments, "reasoning_effort"),
        ),
        fork_context: (arguments.get("fork_context").and_then(Value::as_bool) == Some(true))
            .then_some(true),
    }
}

/// 非空字符串（去首尾空白），否则 `None`。
fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// `model` + `reasoning_effort` → 紧凑展示标签（与 TUI 卡片同款）。
fn collab_model_label(model: Option<&str>, reasoning_effort: Option<&str>) -> Option<String> {
    let model = model.map(str::trim).filter(|value| !value.is_empty());
    let effort = reasoning_effort
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match (model, effort) {
        (Some(model), Some(effort)) => Some(format!("{model} ({effort})")),
        (Some(model), None) => Some(model.to_string()),
        (None, Some(effort)) => Some(format!("effort:{effort}")),
        (None, None) => None,
    }
}

/// `wait_agent` 的 `targets` 数组；空数组代表「全部」，不产生目标标识。
fn wait_targets(name: &str, arguments: &Value) -> Option<String> {
    if name != "wait_agent" {
        return None;
    }
    let targets: Vec<&str> = arguments
        .get("targets")
        .and_then(Value::as_array)?
        .iter()
        .filter_map(Value::as_str)
        .filter(|target| !target.is_empty())
        .collect();
    if targets.is_empty() {
        None
    } else {
        Some(targets.join(", "))
    }
}

/// `wait_agent` 超时且没有任何子 agent 结束：按失败处理。
fn collab_wait_failed(result: Option<&ToolExecutionResult>) -> bool {
    matches!(
        result,
        Some(ToolExecutionResult::Subagent(outcome))
            if matches!(
                outcome.as_ref(),
                SubagentOutcome::Waited {
                    timed_out: true,
                    agents,
                    ..
                } if agents.is_empty()
            )
    )
}

/// `web_search` 命中数文案。
fn result_count_label(count: usize) -> String {
    if count == 1 {
        "1 result".to_string()
    } else {
        format!("{count} results")
    }
}

fn file_changes_of(result: &ToolExecutionResult) -> Vec<FileChangeEntry> {
    match result {
        ToolExecutionResult::Edit { changes, .. } | ToolExecutionResult::Failed { changes, .. } => {
            changes.iter().map(change_entry).collect()
        }
        ToolExecutionResult::Write { path, created, .. } => vec![FileChangeEntry {
            path: path.display().to_string(),
            kind: if *created { "added" } else { "updated" }.to_string(),
        }],
        _ => Vec::new(),
    }
}

fn change_entry(change: &FileChange) -> FileChangeEntry {
    FileChangeEntry {
        path: change.path.display().to_string(),
        kind: match &change.kind {
            FileChangeKind::Added => "added",
            FileChangeKind::Updated => "updated",
            FileChangeKind::Deleted => "deleted",
            FileChangeKind::Moved { .. } => "moved",
        }
        .to_string(),
    }
}

fn file_changes_from_arguments(name: &str, arguments: &Value) -> Vec<FileChangeEntry> {
    match name {
        "write" => arg_str(arguments, "path")
            .map(|path| {
                vec![FileChangeEntry {
                    path: path.to_string(),
                    kind: "updated".to_string(),
                }]
            })
            .unwrap_or_default(),
        "edit" => arg_str(arguments, "patch")
            .map(patch_changes)
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn patch_changes(patch: &str) -> Vec<FileChangeEntry> {
    let mut changes = Vec::new();
    for line in patch.lines() {
        let line = line.trim_start();
        let (kind, rest) = if let Some(rest) = line.strip_prefix("*** Update File: ") {
            ("updated", rest)
        } else if let Some(rest) = line.strip_prefix("*** Add File: ") {
            ("added", rest)
        } else if let Some(rest) = line.strip_prefix("*** Delete File: ") {
            ("deleted", rest)
        } else {
            continue;
        };
        let path = rest.trim();
        if !path.is_empty() {
            changes.push(FileChangeEntry {
                path: path.to_string(),
                kind: kind.to_string(),
            });
        }
    }
    changes
}

fn command_from_arguments(name: &str, arguments: &Value) -> String {
    match name {
        "exec_command" => arg_str(arguments, "cmd").unwrap_or_default().to_string(),
        "write_stdin" => arguments
            .get("session_id")
            .and_then(Value::as_i64)
            .map(|session| format!("write_stdin session {session}"))
            .unwrap_or_else(|| "write_stdin".to_string()),
        _ => String::new(),
    }
}

fn web_queries(arguments: &Value) -> Vec<String> {
    let mut queries: Vec<String> = arguments
        .get("queries")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if queries.is_empty()
        && let Some(query) = arg_str(arguments, "query")
    {
        queries.push(query.to_string());
    }
    queries
}

fn plan_items(arguments: &Value) -> Vec<TodoEntry> {
    arguments
        .get("plan")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|step| {
                    Some(TodoEntry {
                        step: step.get("step")?.as_str()?.to_string(),
                        status: step
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("pending")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn mcp_parts(name: &str, server_hint: Option<&str>) -> (String, String) {
    if let Some((server, tool)) = crate::mcp::naming::split_qualified(name) {
        return (server.to_string(), tool.to_string());
    }
    let server = server_hint.unwrap_or_default().to_string();
    let tool = name.trim_start_matches(crate::mcp::MCP_PREFIX).to_string();
    (server, tool)
}

fn status_of(outcome: &ToolOutcome) -> ItemStatus {
    match outcome {
        ToolOutcome::Cancelled => ItemStatus::Cancelled,
        ToolOutcome::Executed(result) => match result {
            ToolExecutionResult::Process(process) => match &process.state {
                crate::chat::process::ProcessState::Starting
                | crate::chat::process::ProcessState::Running => ItemStatus::InProgress,
                _ if result.is_failure() => ItemStatus::Failed,
                _ => ItemStatus::Completed,
            },
            _ if result.is_failure() => ItemStatus::Failed,
            _ => ItemStatus::Completed,
        },
    }
}

fn failure_message(result: &ToolExecutionResult) -> Option<String> {
    match result {
        ToolExecutionResult::Failed { message, .. } => Some(message.clone()),
        ToolExecutionResult::Process(process) => process.failure_message(),
        _ => None,
    }
}

fn is_collab_tool(name: &str) -> bool {
    crate::chat::tools::SUBAGENT_TOOL_NAMES.contains(&name)
}

fn parse_arguments(call: &ReadyToolCall) -> Value {
    serde_json::from_str(&call.arguments_json).unwrap_or(Value::Null)
}

fn arg_str<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments.get(key).and_then(Value::as_str)
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

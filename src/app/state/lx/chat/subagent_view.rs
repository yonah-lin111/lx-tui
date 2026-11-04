//! 子 agent 控制工具（`spawn_agent` / `wait_agent` / `send_input` / `interrupt_agent` 等）的卡片详情视图。

use serde_json::Value;

/// `spawn_agent` / `wait_agent` / `close_agent` / `interrupt_agent` 等共用的详情（按 `name` 取标题）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentDetailView {
    /// spawn 的角色。
    pub role: Option<crate::chat::agent::AgentRole>,
    /// 子 agent 唯一昵称（如 `"Turing"`；无昵称时为 None）。
    pub nickname: Option<String>,
    /// spawn 的短标题（卡片标题；不是 task 原文）。
    pub title: Option<String>,
    /// spawn 的完整任务文本（只保留在状态里；卡片不再显示提示词）。
    pub task: Option<String>,
    /// spawn/close/interrupt 的 agent id。
    pub agent_id: Option<String>,
    /// agent 的规范层级路径（如 `"/root/turing"`；旧记录为 None）。
    pub agent_path: Option<String>,
    /// spawn 对应子 agent 的实时/历史状态（不由工具调用状态代替）。
    pub agent_status: Option<String>,
    /// 运行中子 agent 的单行实时动作摘要。
    pub live_activity: Option<String>,
    /// spawn 显式或角色覆写模型的展示标签（如 `"gpt-4.1-mini (low)"`）。
    pub model_label: Option<String>,
    /// spawn 是否开启了 `fork_context`。
    pub fork_context: bool,
    /// wait 的目标 id；为空表示等待本会话全部子 agent。
    pub targets: Vec<String>,
    /// wait 是否以全部子 agent 为目标。
    pub all: bool,
    /// wait 的完成条件。
    pub mode: crate::chat::agent::WaitMode,
    /// wait 调用配置的等待超时（毫秒）；旧记录/结果回放可能缺失。
    pub timeout_ms: Option<u64>,
    /// wait 是否超时。
    pub timed_out: bool,
    /// wait 结算到的 agent 状态。
    pub agents: Vec<crate::chat::agent::AgentStatusView>,
    /// spawn 卡片展开时显示的子 agent 最终报告。
    pub report: Option<crate::chat::agent::SubagentReportView>,
    /// close/interrupt 前的状态。
    pub closed_status: Option<String>,
}

impl SubagentDetailView {
    /// 只由参数构造：工具已收到完整参数、结果还没回来时也要显示卡片树体。
    pub fn from_arguments(name: &str, arguments_json: &str) -> Option<Self> {
        let op = crate::chat::agent::parse_control(name, arguments_json)?.ok()?;
        let mut view = Self::empty();
        view.apply_op(op);
        Some(view)
    }

    fn empty() -> Self {
        Self {
            role: None,
            nickname: None,
            title: None,
            task: None,
            agent_id: None,
            agent_path: None,
            agent_status: None,
            live_activity: None,
            model_label: None,
            fork_context: false,
            targets: Vec::new(),
            all: false,
            mode: crate::chat::agent::WaitMode::All,
            timeout_ms: None,
            timed_out: false,
            agents: Vec::new(),
            report: None,
            closed_status: None,
        }
    }

    fn apply_op(&mut self, op: crate::chat::agent::SubagentOp) {
        match op {
            crate::chat::agent::SubagentOp::Spawn {
                role,
                title,
                task,
                fork_context,
                fork_turns: _,
                max_tokens: _,
                model,
                reasoning_effort,
                ..
            } => {
                self.role = Some(role);
                self.title = Some(title);
                self.task = Some(task);
                if fork_context {
                    self.fork_context = true;
                }
                if self.model_label.is_none() {
                    self.model_label =
                        format_spawn_model_label(model.as_deref(), reasoning_effort.as_deref());
                }
            }
            crate::chat::agent::SubagentOp::SendInput {
                target,
                items,
                interrupt,
                trigger_turn: _,
            } => {
                if self.agent_id.is_none() {
                    self.agent_id = Some(target);
                }
                self.task = Some(crate::chat::agent::joined_input_text(&items));
                if interrupt && self.agent_status.is_none() {
                    self.agent_status = Some("interrupt".into());
                }
            }
            crate::chat::agent::SubagentOp::Interrupt { target } => {
                if self.agent_id.is_none() {
                    self.agent_id = Some(target);
                }
            }
            crate::chat::agent::SubagentOp::Resume { id } => {
                if self.agent_id.is_none() {
                    self.agent_id = Some(id);
                }
            }
            crate::chat::agent::SubagentOp::Wait {
                targets,
                mode,
                timeout_ms,
            } => {
                self.all = targets.is_empty();
                self.targets = targets;
                self.mode = mode;
                self.timeout_ms = Some(timeout_ms);
            }
            crate::chat::agent::SubagentOp::Close { target } => {
                if self.agent_id.is_none() {
                    self.agent_id = Some(target);
                }
            }
            crate::chat::agent::SubagentOp::List { path_prefix: _ } => {}
            crate::chat::agent::SubagentOp::CancelInput { submission_id } => {
                self.agent_id = Some(submission_id);
            }
        }
    }

    /// 由工具名 + 参数 + 落库结果 JSON 构造（实时结算与历史重载共用）；结果体缺失时
    /// 回退到调用参数，保证崩溃恢复后的 `interrupted`/失败工具仍保留昵称/目标/模型卡片。
    pub fn from_tool(name: &str, arguments_json: &str, result_json: Option<&str>) -> Option<Self> {
        let arguments = Self::from_arguments(name, arguments_json);
        let outcome = result_json
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .and_then(|value| {
                Self::from_outcome_with_arguments(&value, Some((name, arguments_json)))
            });
        let Some(mut detail) = outcome else {
            return arguments;
        };
        detail.fill_missing_from_arguments(arguments);
        Some(detail)
    }

    /// 用调用参数补齐结果体里没有的字段（昵称、角色、标题、任务、模型、fork）。
    fn fill_missing_from_arguments(&mut self, arguments: Option<Self>) {
        let Some(arguments) = arguments else {
            return;
        };
        // send_input / resume_agent / close_agent / interrupt_agent 允许用昵称寻址：
        // 结果体只带解析后的 `sub-...` id，昵称要从参数里补回。
        if self.nickname.is_none()
            && let Some(target) = arguments.agent_id.as_deref()
            && self.agent_id.as_deref() != Some(target)
            && !target.starts_with("sub-")
            && !target.starts_with("sub_")
        {
            self.nickname = Some(target.to_string());
        }
        self.nickname = self.nickname.take().or(arguments.nickname);
        self.role = self.role.take().or(arguments.role);
        self.title = self.title.take().or(arguments.title);
        self.task = self.task.take().or(arguments.task);
        self.model_label = self.model_label.take().or(arguments.model_label);
        self.fork_context |= arguments.fork_context;
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
            view.apply_op(op);
        }
        Some(view)
    }

    /// 由落库结果 JSON 的形状构造（无参数侧信息）。
    pub fn from_outcome_value(value: &Value) -> Option<Self> {
        let mut view = Self::empty();
        match value.get("tool").and_then(Value::as_str)? {
            "spawn_agent" => {
                view.role = value
                    .get("role")
                    .and_then(Value::as_str)
                    .and_then(crate::chat::agent::AgentRole::parse);
                view.nickname = value
                    .get("nickname")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                view.title = value
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                view.agent_id = value
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                view.agent_path = value
                    .get("agent_path")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                view.fork_context = value
                    .get("fork_context")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                view.model_label = format_spawn_model_label(
                    value.get("model").and_then(Value::as_str),
                    value.get("reasoning_effort").and_then(Value::as_str),
                );
            }
            "wait_agent" => {
                view.timed_out = value
                    .get("timed_out")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                view.all = value.get("all").and_then(Value::as_bool).unwrap_or(false);
                view.mode = crate::chat::agent::WaitMode::parse(
                    value
                        .get("wait_for")
                        .and_then(Value::as_str)
                        .unwrap_or("all"),
                );
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
            "interrupt_agent" => {
                view.agent_id = value
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                view.nickname = value
                    .get("nickname")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let prev = value
                    .get("previous_status")
                    .and_then(Value::as_str)
                    .unwrap_or("running");
                let queued = value
                    .get("queued_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                view.closed_status = Some(if queued > 0 {
                    format!("{prev} · {queued} paused")
                } else {
                    prev.to_string()
                });
            }
            "send_input" | "resume_agent" => {
                view.agent_id = value
                    .get("agent_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let status = value
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let queued = value
                    .get("queued_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                if !status.is_empty() {
                    view.agent_status = Some(if queued > 0 {
                        format!("{status} · {queued} queued")
                    } else {
                        status.to_string()
                    });
                }
            }
            "list_agents" => {
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
                        let status = agent
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let queued = agent
                            .get("queued_count")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                        view.agents.push(crate::chat::agent::AgentStatusView {
                            agent_id: agent_id.to_string(),
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
                            role,
                            status: if queued > 0 {
                                format!("{status} · {queued} queued")
                            } else {
                                status.to_string()
                            },
                            report: None,
                        });
                    }
                }
            }
            "cancel_agent_input" => {
                view.agent_id = value
                    .get("submission_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                view.agent_status = value
                    .get("status")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            _ => return None,
        }
        Some(view)
    }
}

/// 组装紧凑的覆写模型标签（如 `"gpt-4.1-mini (low)"` 或 `"high"`）。
fn format_spawn_model_label(model: Option<&str>, reasoning_effort: Option<&str>) -> Option<String> {
    let clean_model = model.map(str::trim).filter(|s| !s.is_empty());
    let clean_effort = reasoning_effort.map(str::trim).filter(|s| !s.is_empty());
    match (clean_model, clean_effort) {
        (Some(m), Some(e)) => Some(format!("{m} ({e})")),
        (Some(m), None) => Some(m.to_string()),
        (None, Some(e)) => Some(format!("effort:{e}")),
        (None, None) => None,
    }
}

//! 子 agent 展示态：一个运行记录（含完整消息模型）与会话级集合。
//!
//! 数据来源是事件层的回投（`AppEvent`）；`app/` 不持有 DB 连接，也不执行 IO。
//! 渲染复用主历史那套 `LxChat` 步骤时间线，因此这里只保存数据与滚动状态。

pub use crate::chat::agent::SubagentReportView;

use crate::chat::agent::AgentRole;
use crate::db::chat::{SubagentRecord, Usage};

use super::chat::{AssistantStepView, ChatMessageView, ChatPartView, LxChat};
use super::compact::LxCompact;
use crate::app::state::{RequestKey, RequestState};
use crate::chat::events::PartKind;

/// 一个 subagent 的展示态。
#[derive(Debug)]
pub struct SubagentRun {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 可选唯一昵称（如 `"Turing"`；旧记录为 None）。
    pub nickname: Option<String>,
    /// 规范层级路径（如 `"/root/turing"`、`"/root/turing/curie"`）。
    pub agent_path: String,
    /// 子会话 external id（delta/工具回投的定位键）。
    pub child_session_id: String,
    /// 父会话 external id：切会话后拒绝迟到回投把旧 run 塞进新会话。
    pub parent_session_id: String,
    /// 角色。
    pub role: AgentRole,
    /// 面向用户的短标题（列表与卡片标题）；旧记录可能为空。
    pub title: String,
    /// 任务原文（只用于 live 种子消息；历史从 DB 消息流加载）。
    pub task: String,
    /// `running` / `idle` / `blocked` / `completed` / `failed` / `cancelled` / `interrupted`。
    pub status: String,
    /// 当前未处理的排队输入数。
    pub queued_count: usize,
    /// 启动时刻（UTC 毫秒）。
    pub started_at_ms: i64,
    /// 结束时刻（运行中为 None）。
    pub completed_at_ms: Option<i64>,
    /// 完整消息模型：与主历史同一套字段与增量方法。
    pub chat: LxChat,
    /// 子会话无 compact，恒为空（`HistorySource` 需要）。
    pub compact: LxCompact,
    /// 消息流是否已从 DB 懒加载（历史 run）。
    pub loaded: bool,
    /// 最终报告（完成后才有）。
    pub report: Option<SubagentReportView>,
    /// 该轮输入 token。
    pub input_tokens: Option<i64>,
    /// 该轮输出 token。
    pub output_tokens: Option<i64>,
    /// 详情区滚动状态（与 Processes 的 `output_scroll` 分开）。
    pub scroll: usize,
    /// 详情区可见行数（事件循环在绘制前写入）。
    pub visible: usize,
    /// 详情区总行数（事件循环在绘制前写入；翻页上限用）。
    pub total_rows: usize,
    /// 消息流自上次量行数后是否变化（避免每帧重排 markdown）。
    pub rows_dirty: bool,
    /// 是否跟随底部（用户上翻即停）。
    pub follow: bool,
}

impl SubagentRun {
    /// 由持久化元数据构造（消息流待懒加载）。
    pub fn from_record(record: &SubagentRecord) -> Self {
        Self {
            agent_id: record.agent_id.clone(),
            nickname: record.nickname.clone(),
            agent_path: record.agent_path.clone(),
            child_session_id: record.child_session_id.clone(),
            parent_session_id: record.parent_session_id.clone(),
            role: record.role,
            title: record.title.clone(),
            // 旧记录没有单独 title；这里不拿 task 首行冒充标题，保持为空。
            task: record.title.clone(),
            status: record.status.clone(),
            queued_count: 0,
            started_at_ms: record.created_at_ms,
            completed_at_ms: record.completed_at_ms,
            chat: LxChat::new(),
            compact: LxCompact::default(),
            loaded: false,
            report: record
                .report_json
                .as_deref()
                .and_then(SubagentReportView::from_json),
            input_tokens: None,
            output_tokens: None,
            scroll: 0,
            visible: 0,
            total_rows: 0,
            rows_dirty: true,
            follow: true,
        }
    }

    /// 已运行时长（毫秒）；结束的按结束时刻算。
    pub fn elapsed_ms(&self, now_ms: i64) -> i64 {
        let end = self.completed_at_ms.unwrap_or(now_ms);
        end.saturating_sub(self.started_at_ms).max(0)
    }

    /// 是否仍在运行。
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }

    /// 层级深度：`/root` -> 0，`/root/turing` -> 1，`/root/turing/curie` -> 2。
    pub fn depth(&self) -> usize {
        crate::chat::agent::path_depth(&self.agent_path)
    }

    /// 本轮 token 总量：终态取回投的输入/输出，运行中读实时聚合槽。
    pub fn total_tokens(&self) -> Option<i64> {
        if self.input_tokens.is_some() || self.output_tokens.is_some() {
            let total = self
                .input_tokens
                .unwrap_or(0)
                .saturating_add(self.output_tokens.unwrap_or(0));
            if total > 0 {
                return Some(total);
            }
        }
        if self.is_running() {
            let step = &self.chat.step_usage;
            let total = step
                .input
                .unwrap_or(0)
                .saturating_add(step.output.unwrap_or(0));
            if total > 0 {
                return Some(total);
            }
        }
        None
    }

    /// 运行中子 agent 的实时动作摘要（主历史 `spawn_agent` 卡片用）。
    ///
    /// 只读本地消息模型：先看最新 assistant 消息里是否还有在途工具，
    /// 否则退回最后一步的思考/正文预览；终态返回 None。
    pub fn live_activity(&self) -> Option<String> {
        if !self.is_running() {
            return None;
        }
        let message = self
            .chat
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "assistant")?;
        for step in message.steps.iter().rev() {
            let AssistantStepView::Tool { call, .. } = step else {
                continue;
            };
            if matches!(
                call.status,
                crate::app::state::ToolStatus::Pending | crate::app::state::ToolStatus::Running
            ) {
                return Some(tool_activity(call));
            }
        }
        let Some(AssistantStepView::Part { part, .. }) = message.steps.last() else {
            return None;
        };
        if part.reasoning_open() {
            return Some("Thinking...".to_string());
        }
        if part.kind != PartKind::Text {
            return None;
        }
        let line = part
            .content
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())?
            .trim();
        (!line.is_empty()).then(|| line.to_string())
    }

    /// 列表标题；旧记录没有单独标题时为空（不显示 task 提示词）。
    pub fn display_title(&self) -> &str {
        if self.title.is_empty() {
            ""
        } else {
            self.title.as_str()
        }
    }

    /// 列表与卡片可读身份：`Turing [explorer]`（无昵称时回退 `explorer`）。
    pub fn display_identity(&self) -> String {
        crate::chat::agent::format_agent_identity(self.nickname.as_deref(), self.role.as_str())
    }
}

/// 在途工具的单行动作摘要（`$ cmd` / `edit src/a.rs` / `search pat`）。
fn tool_activity(call: &crate::app::state::ToolCallView) -> String {
    match call.name.as_str() {
        "exec_command" | "write_stdin" => {
            let command = serde_json::from_str::<serde_json::Value>(&call.arguments_json)
                .ok()
                .and_then(|value| {
                    value
                        .get("command")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                });
            match command {
                Some(command) => {
                    let first = command.lines().next().unwrap_or_default().trim();
                    if first.is_empty() {
                        call.name.clone()
                    } else {
                        format!("$ {first}")
                    }
                }
                None => call.name.clone(),
            }
        }
        "read" | "edit" | "write" => call
            .summary
            .as_ref()
            .and_then(|summary| summary.path.as_deref())
            .map_or_else(|| call.name.clone(), |path| format!("{} {path}", call.name)),
        "search" | "web_search" => call
            .summary
            .as_ref()
            .and_then(|summary| summary.pattern.as_deref())
            .map_or_else(
                || "search".to_string(),
                |pattern| format!("search {pattern}"),
            ),
        "spawn_agent" => "spawning subagent...".to_string(),
        "wait_agent" => "waiting for subagents...".to_string(),
        _ => call.name.clone(),
    }
}

/// 给运行中的子 agent 填充初始消息模型（user task + streaming assistant）。
///
/// 子 turn 的增量由事件层同时写入这里的模型（持久化来源）与 UI 的 run.chat；
/// 两条路使用同一套 `LxChat` 方法，因此步骤位置不会分叉。
pub fn seed_chat(run: &mut SubagentRun, key: RequestKey, assistant_id: &str) {
    seed_chat_model(
        &mut run.chat,
        &run.child_session_id,
        &run.task,
        key,
        assistant_id,
    );
    run.loaded = true;
}

/// 只填消息模型（事件层句柄与 UI run 各持一份，同一批增量）。
pub fn seed_chat_model(
    chat: &mut LxChat,
    child_session_id: &str,
    task: &str,
    key: RequestKey,
    assistant_id: &str,
) {
    chat.session_id = Some(child_session_id.to_string());
    chat.messages.push(ChatMessageView {
        id: format!("{assistant_id}:user"),
        sequence: 0,
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        turn_index: 0,
        shell: None,
        steps: vec![AssistantStepView::Part {
            position: 0,
            part: ChatPartView {
                kind: PartKind::Text,
                content: task.to_string(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        }],
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    chat.messages.push(ChatMessageView {
        id: assistant_id.to_string(),
        sequence: 1,
        kind: "chat".into(),
        role: "assistant".into(),
        status: "streaming".into(),
        turn_index: 0,
        shell: None,
        steps: Vec::new(),
        model_label: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        duration_ms: None,
        reports: Vec::new(),
        attachments: Vec::new(),
    });
    chat.request = RequestState::Streaming {
        key,
        assistant_id: assistant_id.to_string(),
    };
    // 新一轮从空聚合槽开始：不能把上一轮的 output 带进第一次采样。
    chat.step_usage = Usage::default();
    // 子 agent 详情也走同一套步骤渲染：没有 status.streaming，运行中的步骤标记
    // 会被当成终态而不呼吸。终态由 `on_subagent_finished` 复位。
    chat.status.streaming = true;
}

/// 面板数据：本窗格的 subagent 集合（面板关闭时依然保留）。
#[derive(Debug, Default)]
pub struct AgentsState {
    /// 运行记录：按 `agent_path` 树形前序排列（兄弟间保持创建顺序）。
    pub runs: Vec<SubagentRun>,
}

impl AgentsState {
    /// 按子会话 id 查找。
    pub fn by_session(&self, child_session: &str) -> Option<&SubagentRun> {
        self.runs
            .iter()
            .find(|run| run.child_session_id == child_session)
    }

    /// 按子会话 id 查找可变引用。
    pub fn by_session_mut(&mut self, child_session: &str) -> Option<&mut SubagentRun> {
        self.runs
            .iter_mut()
            .find(|run| run.child_session_id == child_session)
    }

    /// 按 agent id 查找。
    pub fn by_agent(&self, agent_id: &str) -> Option<&SubagentRun> {
        self.runs.iter().find(|run| run.agent_id == agent_id)
    }

    /// 按 agent id 查找可变引用。
    pub fn by_agent_mut(&mut self, agent_id: &str) -> Option<&mut SubagentRun> {
        self.runs.iter_mut().find(|run| run.agent_id == agent_id)
    }

    /// 按 `agent_path` 的树形前序重排：父节点之后紧跟其全部后代，再轮到下一个兄弟。
    ///
    /// 同层与兄弟之间保持创建顺序；父 run 不在列表里（历史缺失）时按根节点处理。
    pub fn sort_runs_tree_order(&mut self) {
        if self.runs.len() < 2 {
            return;
        }
        let index_by_path: std::collections::HashMap<String, usize> = self
            .runs
            .iter()
            .enumerate()
            .map(|(index, run)| (run.agent_path.to_ascii_lowercase(), index))
            .collect();
        let mut children = vec![Vec::new(); self.runs.len()];
        let mut roots = Vec::new();
        for (index, run) in self.runs.iter().enumerate() {
            let parent = run
                .agent_path
                .trim_end_matches('/')
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_ascii_lowercase());
            match parent.and_then(|parent| index_by_path.get(&parent).copied()) {
                Some(parent) if parent != index => children[parent].push(index),
                _ => roots.push(index),
            }
        }
        let mut pending: Vec<Option<SubagentRun>> = std::mem::take(&mut self.runs)
            .into_iter()
            .map(Some)
            .collect();
        let mut stack: Vec<usize> = roots.into_iter().rev().collect();
        let mut ordered = Vec::with_capacity(pending.len());
        while let Some(index) = stack.pop() {
            let Some(run) = pending[index].take() else {
                continue;
            };
            ordered.push(run);
            for child in children[index].iter().rev() {
                stack.push(*child);
            }
        }
        // 防御：理论上不可达的节点（坏路径/重复路径）按原顺序补回。
        ordered.extend(pending.into_iter().flatten());
        self.runs = ordered;
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// 仍在运行的子 agent 数（底边框运行指标）。
    pub fn running_count(&self) -> usize {
        self.runs.iter().filter(|run| run.is_running()).count()
    }
}

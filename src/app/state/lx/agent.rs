//! 子 agent 展示态：一个运行记录（含完整消息模型）与会话级集合。
//!
//! 数据来源是事件层的回投（`AppEvent`）；`app/` 不持有 DB 连接，也不执行 IO。
//! 渲染复用主历史那套 `LxChat` 步骤时间线，因此这里只保存数据与滚动状态。

pub use crate::chat::agent::SubagentReportView;

use crate::chat::agent::AgentRole;
use crate::db::chat::SubagentRecord;

use super::chat::{AssistantStepView, ChatMessageView, ChatPartView, LxChat};
use super::compact::LxCompact;
use crate::app::state::{RequestKey, RequestState};
use crate::chat::events::PartKind;

/// 一个 subagent 的展示态。
#[derive(Debug)]
pub struct SubagentRun {
    /// 稳定 agent id。
    pub agent_id: String,
    /// 子会话 external id（delta/工具回投的定位键）。
    pub child_session_id: String,
    /// 角色。
    pub role: AgentRole,
    /// 任务原文（面板与详情标题）。
    pub task: String,
    /// `running` / `completed` / `failed` / `cancelled`。
    pub status: String,
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
            child_session_id: record.child_session_id.clone(),
            role: record.role,
            task: record.task_title.clone(),
            status: record.status.clone(),
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

    /// 任务首行（列表行显示）。
    pub fn task_head(&self) -> &str {
        self.task.lines().next().unwrap_or_default()
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
    });
    chat.request = RequestState::Streaming {
        key,
        assistant_id: assistant_id.to_string(),
    };
}

/// 面板数据：本窗格的 subagent 集合（面板关闭时依然保留）。
#[derive(Debug, Default)]
pub struct AgentsState {
    /// 按创建顺序排列的运行记录。
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

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
}

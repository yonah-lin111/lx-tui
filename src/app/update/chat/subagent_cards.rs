//! 子 agent 卡片状态回填：`spawn_agent` 跟随 run 状态，`wait_agent` 目标实时结算。
//!
//! 主会话与每个 run 自己的消息流都要刷新：depth-1 子 agent 的消息里可能嵌着
//! 孙 agent 的 `spawn_agent` / `wait_agent` 卡片。

use crate::app::state::{
    AppState, AssistantStepView, ChatMessageView, SubagentReportView, ToolDetailView, ToolStatus,
};
use crate::chat::agent::{AgentRole, AgentStatusView};
use crate::layout::PaneId;

/// 刷新时使用的 run 元数据快照（借用期内不再读 `AgentsState`）。
struct RunCardSnapshot {
    agent_id: String,
    nickname: Option<String>,
    agent_path: String,
    role: AgentRole,
    title: String,
    status: String,
    report: Option<SubagentReportView>,
    live_activity: Option<String>,
}

impl RunCardSnapshot {
    /// 由当前 run 快照生成 wait/list 共用的结算视图。
    fn status_view(&self, reports: &[SubagentReportView]) -> AgentStatusView {
        AgentStatusView {
            agent_id: self.agent_id.clone(),
            nickname: self.nickname.clone(),
            agent_path: Some(self.agent_path.clone()),
            title: self.title.clone(),
            role: self.role,
            status: self.status.clone(),
            report: self
                .report
                .as_ref()
                .map(|report| report.body.clone())
                .or_else(|| {
                    reports
                        .iter()
                        .find(|report| report.agent_id == self.agent_id)
                        .map(|report| report.body.clone())
                }),
        }
    }
}

/// 子 agent 卡片状态回填：
/// - `spawn_agent` 标题状态跟对应 run 的 running/completed 走；
/// - 运行中的 `wait_agent` 目标状态用当前 run 快照实时填充。
///
/// spawn 的工具调用在子 agent 启动时就已经返回，不能拿工具状态冒充运行状态。
pub fn refresh_subagent_cards(state: &mut AppState, pane: PaneId) -> bool {
    let Some(target) = state.pane_mut_anywhere(pane) else {
        return false;
    };
    let runs: Vec<RunCardSnapshot> = target
        .lx
        .agents
        .runs
        .iter()
        .map(|run| RunCardSnapshot {
            agent_id: run.agent_id.clone(),
            nickname: run.nickname.clone(),
            agent_path: run.agent_path.clone(),
            role: run.role,
            title: run.title.clone(),
            status: run.status.clone(),
            report: run.report.clone(),
            live_activity: run.live_activity(),
        })
        .collect();
    let mut changed = refresh_messages_with_runs(
        &mut target.lx.chat.messages,
        &runs,
        crate::chat::agent::ROOT_AGENT_PATH,
    );
    // 子 agent 的消息流里还有更下一层的工具卡片；相对路径按该 run 的位置解析。
    for run in &mut target.lx.agents.runs {
        changed |= refresh_messages_with_runs(&mut run.chat.messages, &runs, &run.agent_path);
    }
    changed
}

/// 刷新一组消息里的委派卡片；`caller_path` 是该消息流所属 agent 的路径。
fn refresh_messages_with_runs(
    messages: &mut [ChatMessageView],
    runs: &[RunCardSnapshot],
    caller_path: &str,
) -> bool {
    // 历史会话不一定回投了 Agents 列表；父消息里的汇报同样带终态。
    let reports: Vec<SubagentReportView> = messages
        .iter()
        .flat_map(|message| &message.reports)
        .cloned()
        .collect();
    let mut changed = false;
    for message in messages {
        for step in &mut message.steps {
            let AssistantStepView::Tool { call, .. } = step else {
                continue;
            };
            let Some(ToolDetailView::Subagent(view)) = call.detail.as_deref_mut() else {
                continue;
            };
            if call.name == "spawn_agent"
                && let Some(agent_id) = view.agent_id.as_deref()
            {
                if let Some(run) = runs.iter().find(|r| r.agent_id == agent_id) {
                    if view.agent_status.as_deref() != Some(run.status.as_str()) {
                        view.agent_status = Some(run.status.clone());
                        changed = true;
                    }
                    if view.nickname.is_none() && run.nickname.is_some() {
                        view.nickname = run.nickname.clone();
                        changed = true;
                    }
                    if view.agent_path.is_none() {
                        view.agent_path = Some(run.agent_path.clone());
                        changed = true;
                    }
                    if view.title.as_deref().unwrap_or("").is_empty() && !run.title.is_empty() {
                        view.title = Some(run.title.clone());
                        changed = true;
                    }
                    if view.report != run.report {
                        view.report = run.report.clone();
                        changed = true;
                    }
                    if view.live_activity != run.live_activity {
                        view.live_activity = run.live_activity.clone();
                        changed = true;
                    }
                } else if let Some(report) =
                    reports.iter().find(|report| report.agent_id == agent_id)
                {
                    if view.agent_status.as_deref() != Some(report.status.as_str()) {
                        view.agent_status = Some(report.status.clone());
                        changed = true;
                    }
                    if view.nickname.is_none() && report.nickname.is_some() {
                        view.nickname = report.nickname.clone();
                        changed = true;
                    }
                    if view.title.as_deref().unwrap_or("").is_empty() && !report.title.is_empty() {
                        view.title = Some(report.title.clone());
                        changed = true;
                    }
                    if view.report.as_ref() != Some(report) {
                        view.report = Some(report.clone());
                        changed = true;
                    }
                }
            }
            if matches!(
                call.name.as_str(),
                "send_input" | "interrupt_agent" | "resume_agent" | "close_agent"
            ) && let Some(target_key) = view.agent_id.as_deref()
                && let Some(matched) = runs.iter().find(|r| {
                    crate::chat::agent::matches_agent_target(
                        &r.agent_id,
                        r.nickname.as_deref(),
                        Some(r.agent_path.as_str()),
                        caller_path,
                        target_key,
                    )
                })
            {
                if view.nickname != matched.nickname {
                    view.nickname = matched.nickname.clone();
                    changed = true;
                }
                if view.role != Some(matched.role) {
                    view.role = Some(matched.role);
                    changed = true;
                }
            }
            if call.name != "wait_agent"
                || !matches!(call.status, ToolStatus::Pending | ToolStatus::Running)
            {
                continue;
            }
            let mut agents: Vec<AgentStatusView> = Vec::new();
            if view.all || view.targets.is_empty() {
                // 全量等待：先刷新已有条目，再补上等待期间新派发的 run。
                for existing in &view.agents {
                    if let Some(run) = runs.iter().find(|run| run.agent_id == existing.agent_id) {
                        agents.push(run.status_view(&reports));
                    } else {
                        agents.push(existing.clone());
                    }
                }
                for run in runs {
                    if !agents.iter().any(|agent| agent.agent_id == run.agent_id) {
                        agents.push(run.status_view(&reports));
                    }
                }
            } else {
                for target_key in &view.targets {
                    if let Some(run) = runs.iter().find(|run| {
                        crate::chat::agent::matches_agent_target(
                            &run.agent_id,
                            run.nickname.as_deref(),
                            Some(run.agent_path.as_str()),
                            caller_path,
                            target_key,
                        )
                    }) {
                        agents.push(run.status_view(&reports));
                        continue;
                    }
                    // 目标已结束且不在活动列表里：保留上一次结算的快照。
                    if let Some(existing) = view.agents.iter().find(|agent| {
                        crate::chat::agent::matches_agent_target(
                            &agent.agent_id,
                            agent.nickname.as_deref(),
                            agent.agent_path.as_deref(),
                            caller_path,
                            target_key,
                        )
                    }) {
                        agents.push(existing.clone());
                    }
                }
            }
            if view.agents != agents {
                view.agents = agents;
                changed = true;
            }
        }
    }
    changed
}

//! 子 agent 展示态测试：增量分流、列表幂等、懒加载、汇报挂载、复位与 tab 切换。

use super::*;
use crate::app::state::lx::agent::{AgentsState, SubagentRun, seed_chat};
use crate::app::state::{
    AssistantStepView, ChatMessageView, SubagentDetailView, ToolCallView, ToolDetailView,
    ToolStatus,
};
use crate::app::update;
use crate::chat::agent::{AgentRole, SubagentReportView};
use crate::chat::events::PartDelta;
use crate::db::chat::SubagentRecord;

fn run(child_session: &str, agent_id: &str, role: AgentRole) -> SubagentRun {
    SubagentRun::from_record(&SubagentRecord {
        agent_id: agent_id.into(),
        nickname: None,
        agent_path: "/root/agent".into(),
        child_session_id: child_session.into(),
        parent_session_id: "sess-parent".into(),
        spawn_message_id: None,
        role,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "running".into(),
        title: "investigate".into(),
        created_at_ms: 1_000,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    })
}

fn key(pane: PaneId, child_session: &str) -> RequestKey {
    RequestKey {
        pane,
        session: child_session.into(),
        generation: 1,
    }
}

fn text_delta(text: &str) -> ChatDelta {
    ChatDelta::Part(PartDelta {
        kind: PartKind::Text,
        text: text.into(),
        metadata: None,
    })
}

#[test]
fn interleaved_deltas_do_not_cross_runs() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut first = run("sess-a", "sub-a", AgentRole::Explorer);
    seed_chat(&mut first, key(pane, "sess-a"), "msg-a");
    let mut second = run("sess-b", "sub-b", AgentRole::Executor);
    seed_chat(&mut second, key(pane, "sess-b"), "msg-b");
    assert!(update::on_subagent_started(&mut state, pane, first));
    assert!(update::on_subagent_started(&mut state, pane, second));

    assert!(update::on_subagent_delta(
        &mut state,
        "sess-a",
        &[text_delta("alpha")]
    ));
    assert!(update::on_subagent_delta(
        &mut state,
        "sess-b",
        &[text_delta("beta")]
    ));

    let agents = &state.pane_anywhere(pane).expect("pane").lx.agents;
    let a = agents.by_session("sess-a").expect("a");
    let b = agents.by_session("sess-b").expect("b");
    assert_eq!(a.chat.messages.last().expect("assistant").text(), "alpha");
    assert_eq!(b.chat.messages.last().expect("assistant").text(), "beta");
}

#[test]
fn set_subagent_list_is_idempotent_and_keeps_local_runs() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut started = run("sess-a", "sub-a", AgentRole::Explorer);
    seed_chat(&mut started, key(pane, "sess-a"), "msg-a");
    update::on_subagent_started(&mut state, pane, started);
    let record = SubagentRecord {
        agent_id: "sub-a".into(),
        nickname: None,
        agent_path: "/root/agent".into(),
        child_session_id: "sess-a".into(),
        parent_session_id: "sess-parent".into(),
        spawn_message_id: None,
        role: AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "completed".into(),
        title: "investigate".into(),
        created_at_ms: 1_000,
        completed_at_ms: Some(2_000),
        report_json: Some(
            SubagentReportView {
                agent_id: "sub-a".into(),
                nickname: None,
                title: "test title".into(),
                role: AgentRole::Explorer,
                status: "completed".into(),
                files_changed: 1,
                changed_paths: Vec::new(),
                duration_ms: Some(1_000),
                body: "done".into(),
                delivered_at_ms: 2_000,
            }
            .to_json(),
        ),
        report_delivered: false,
    };
    assert!(update::set_subagent_list(
        &mut state,
        pane,
        "sess-parent",
        std::slice::from_ref(&record),
    ));
    // 第二次回投内容相同：不置脏。
    assert!(!update::set_subagent_list(
        &mut state,
        pane,
        "sess-parent",
        &[record]
    ));
    let agents = &state.pane_anywhere(pane).expect("pane").lx.agents;
    assert_eq!(agents.runs.len(), 1);
    assert_eq!(agents.runs[0].status, "completed");
    assert!(agents.runs[0].report.is_some());
    // live 消息模型不被列表回投覆盖。
    assert!(!agents.runs[0].chat.messages.is_empty());
}

#[test]
fn loaded_transcript_only_fills_unloaded_runs() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut history = run("sess-h", "sub-h", AgentRole::Executor);
    history.loaded = false;
    update::on_subagent_started(&mut state, pane, history);
    let messages = vec![crate::db::chat::ChatMessage {
        external_id: "msg-1".into(),
        sequence: 0,
        turn_index: 0,
        kind: "chat".into(),
        role: "user".into(),
        status: "complete".into(),
        model_provider: None,
        model_id: None,
        variant: None,
        stop_reason: None,
        error_code: None,
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_write_tokens: None,
        created_at_ms: 0,
        completed_at_ms: None,
        user_shell: None,
        subagent_reports: None,
        parts: Vec::new(),
        tools: Vec::new(),
        attachments: Vec::new(),
    }];
    assert!(update::on_subagent_loaded(
        &mut state, pane, "sub-h", &messages
    ));
    assert!(!update::on_subagent_loaded(
        &mut state, pane, "sub-h", &messages
    ));
    let agents = &state.pane_anywhere(pane).expect("pane").lx.agents;
    assert!(agents.by_agent("sub-h").expect("run").loaded);
}

#[test]
fn corrupted_report_json_decodes_to_empty_reports() {
    let view = crate::app::update::chat::view_message_of(
        &crate::db::chat::ChatMessage {
            external_id: "msg".into(),
            sequence: 0,
            turn_index: 0,
            kind: "chat".into(),
            role: "user".into(),
            status: "complete".into(),
            model_provider: None,
            model_id: None,
            variant: None,
            stop_reason: None,
            error_code: None,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            created_at_ms: 0,
            completed_at_ms: None,
            user_shell: None,
            subagent_reports: Some("{not json".into()),
            parts: Vec::new(),
            tools: Vec::new(),
            attachments: Vec::new(),
        },
        &ModelsSnapshot::default(),
    );
    assert!(view.reports.is_empty());
}

#[test]
fn stop_selected_subagent_requests_only_that_agent() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-parent".into());
    }
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-a", "sub-a", AgentRole::Explorer),
    );
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-b", "sub-b", AgentRole::Executor),
    );
    assert!(update::open_agent_page(&mut state, pane, "sub-b"));
    state.effects.clear();
    assert!(update::stop_selected_subagent(&mut state, pane));
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::CancelSubagents { session_id, agent_id }
            if session_id == "sess-parent" && agent_id.as_deref() == Some("sub-b")
    )));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .toast
            .as_ref()
            .is_some_and(|toast| toast.message.contains('1')),
        "提示只停一个"
    );
}

#[test]
fn loading_another_session_clears_agents_and_resets_gates() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-a".into());
        target.lx.chat.main_stopped = true;
        target.lx.chat.stop_armed_at = Some(std::time::Instant::now());
    }
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-a", "sub-a", AgentRole::Explorer),
    );
    assert!(update::on_session_loaded_full(
        &mut state,
        pane,
        "sess-b",
        &[],
        &[],
        0,
        None,
    ));
    let target = state.pane_anywhere(pane).expect("pane");
    assert_eq!(target.lx.chat.session_id.as_deref(), Some("sess-b"));
    assert!(target.lx.agents.is_empty(), "切会话必须清掉上一会话的 run");
    assert!(!target.lx.chat.main_stopped, "切会话重置报告闸门");
    assert!(target.lx.chat.stop_armed_at.is_none());
}

#[test]
fn stale_subagent_replies_are_ignored_by_session() {
    use crate::db::chat::SubagentRecord;

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("sess-b".into());
    let record = SubagentRecord {
        agent_id: "sub-a".into(),
        nickname: None,
        agent_path: "/root/agent".into(),
        child_session_id: "sess-a".into(),
        parent_session_id: "sess-a-parent".into(),
        spawn_message_id: None,
        role: AgentRole::Explorer,
        model_provider: String::new(),
        model_id: String::new(),
        model_variant: None,
        status: "running".into(),
        title: "old".into(),
        created_at_ms: 1_000,
        completed_at_ms: None,
        report_json: None,
        report_delivered: false,
    };
    assert!(!update::set_subagent_list(
        &mut state,
        pane,
        "sess-a-parent",
        std::slice::from_ref(&record),
    ));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .is_empty(),
        "旧会话列表不能污染新会话"
    );

    let mut old = SubagentRun::from_record(&record);
    old.parent_session_id = "sess-a-parent".into();
    assert!(
        !update::on_subagent_started(&mut state, pane, old),
        "旧会话 spawn 回执不能污染新会话"
    );
}

#[test]
fn new_session_clears_agents() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-a", "sub-a", AgentRole::Explorer),
    );
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .is_empty()
    );
    assert!(update::new_session(&mut state, pane));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .is_empty()
    );
}

#[test]
fn opening_an_agent_link_opens_the_agents_page_and_selects_it() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.view = crate::app::state::PaneView::Lx;
        target.lx.chat.session_id = Some("sess-parent".into());
    }
    assert!(update::on_subagent_started(
        &mut state,
        pane,
        run("sess-a", "sub-a", AgentRole::Explorer)
    ));
    assert!(update::open_agent_page(&mut state, pane, "sub-a"));
    let target = state.pane_anywhere(pane).expect("pane");
    let page = target.lx.task.as_ref().expect("agents page");
    assert_eq!(page.tab, crate::app::state::TaskTab::Agents);
    assert_eq!(page.agents_window.active, 0);
    assert_eq!(
        target.lx.agents.runs[0].agent_id, "sub-a",
        "定位到被点击的 agent"
    );
    assert!(state.effects.iter().any(|effect| matches!(
        effect,
        crate::app::effects::Effect::ListSubagents { pane: p, .. } if *p == pane
    )));
}

#[test]
fn task_page_switches_between_process_and_agent_lists() {
    use crate::app::state::{TaskPage, TaskTab};
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-a", "sub-a", AgentRole::Explorer),
    );
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-b", "sub-b", AgentRole::Executor),
    );
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task = Some(TaskPage::default());
    }
    let agents = AgentsState {
        runs: vec![run("sess-a", "sub-a", AgentRole::Explorer)],
    };
    let mut page = TaskPage::default();
    assert_eq!(page.len(&agents), 0, "进程 tab 没有条目");
    page.tab = TaskTab::Agents;
    assert_eq!(page.len(&agents), 1);
    assert!(!page.move_active(&agents, 1), "只有一项时不动");
    assert_eq!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .runs
            .len(),
        2
    );
}

#[test]
fn scrolling_the_agents_detail_stops_following() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut started = run("sess-a", "sub-a", AgentRole::Explorer);
    seed_chat(&mut started, key(pane, "sess-a"), "msg-a");
    started.visible = 10;
    started.total_rows = 100;
    started.follow = true;
    started.scroll = 90;
    update::on_subagent_started(&mut state, pane, started);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task = Some(crate::app::state::TaskPage {
            tab: crate::app::state::TaskTab::Agents,
            ..crate::app::state::TaskPage::default()
        });
    }
    assert!(update::scroll_agents_detail(&mut state, pane, -20));
    let run = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_session("sess-a")
        .expect("run");
    assert_eq!(run.scroll, 70);
    assert!(!run.follow, "上翻后停止跟随");
}

#[test]
fn auto_compact_notice_becomes_a_checkpoint_and_toggles() {
    use crate::app::state::{CardTarget, TaskPage, TaskTab};
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut started = run("sess-a", "sub-a", AgentRole::Explorer);
    seed_chat(&mut started, key(pane, "sess-a"), "msg-a");
    update::on_subagent_started(&mut state, pane, started);
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Agents,
            ..TaskPage::default()
        });
    }
    let notice = crate::chat::compact::AutoCompactNotice {
        model_label: "m".into(),
        summary: "handoff".into(),
        estimated_tokens: 190_000,
        context_limit: 200_000,
    };
    assert!(update::on_subagent_auto_compact(
        &mut state, "sess-a", &notice
    ));
    let run = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_session("sess-a")
        .expect("run");
    assert_eq!(run.compact.checkpoints.len(), 1);
    assert!(run.compact.checkpoints[0].auto);
    assert_eq!(run.compact.checkpoints[0].summary, "handoff");
    let id = run.compact.checkpoints[0].id.clone();
    assert!(update::toggle_agent_card(
        &mut state,
        pane,
        CardTarget::Compact {
            checkpoint_id: id.clone()
        },
    ));
    let run = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_session("sess-a")
        .expect("run");
    assert!(run.compact.is_expanded(&id));
}

#[test]
fn selecting_an_agents_row_follows_the_detail_tail() {
    use crate::app::state::{TaskPage, TaskTab};
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut first = run("sess-a", "sub-a", AgentRole::Explorer);
    first.follow = false;
    first.scroll = 3;
    update::on_subagent_started(&mut state, pane, first);
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-b", "sub-b", AgentRole::Executor),
    );
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Agents,
            ..TaskPage::default()
        });
    }
    assert!(update::select_task_agent(&mut state, pane, 0));
    let target = state.pane_anywhere(pane).expect("pane");
    let page = target.lx.task.as_ref().expect("task");
    assert_eq!(page.agents_window.active, 0);
    let run = target.lx.agents.by_session("sess-a").expect("run");
    assert!(run.follow);
    assert_eq!(run.scroll, usize::MAX);
}

#[test]
fn agents_page_advances_subagent_loading_frames() {
    use crate::app::state::{TaskPage, TaskTab};
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-a", "sub-a", AgentRole::Explorer),
    );
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Agents,
            ..TaskPage::default()
        });
    }
    assert!(update::advance_lx_loading(&mut state));
    let target = state.pane_anywhere(pane).expect("pane");
    assert!(target.lx.chat.loading_frame > 0);
    assert!(
        target.lx.agents.runs[0].chat.loading_frame > 0,
        "详情步骤标记使用同一帧呼吸"
    );
}

#[test]
fn agents_scrollbar_offset_updates_follow_at_bottom() {
    use crate::app::state::{TaskPage, TaskTab};
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    update::on_subagent_started(
        &mut state,
        pane,
        run("sess-a", "sub-a", AgentRole::Explorer),
    );
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Agents,
            ..TaskPage::default()
        });
        let run = &mut target.lx.agents.runs[0];
        run.visible = 10;
        run.total_rows = 100;
    }
    assert!(update::set_task_agents_scroll(&mut state, pane, 20));
    let run = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .runs
        .first()
        .expect("run");
    assert_eq!(run.scroll, 20);
    assert!(!run.follow);
    assert!(update::set_task_agents_scroll(&mut state, pane, 90));
    let run = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .runs
        .first()
        .expect("run");
    assert_eq!(run.scroll, 90);
    assert!(run.follow, "贴底恢复跟随");
}

#[test]
fn finished_run_records_status_and_tokens() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    let mut started = run("sess-a", "sub-a", AgentRole::Explorer);
    seed_chat(&mut started, key(pane, "sess-a"), "msg-a");
    update::on_subagent_started(&mut state, pane, started);
    assert!(update::on_subagent_finished(
        &mut state,
        "sess-a",
        "completed",
        None,
        Usage {
            input: Some(7),
            output: Some(3),
            ..Usage::default()
        },
    ));
    let agents = &state.pane_anywhere(pane).expect("pane").lx.agents;
    let run = agents.by_session("sess-a").expect("run");
    assert_eq!(run.status, "completed");
    assert_eq!(run.input_tokens, Some(7));
    assert_eq!(run.output_tokens, Some(3));
    assert!(!run.chat.status.streaming, "终态停止步骤呼吸");
    let message = run
        .chat
        .messages
        .iter()
        .find(|message| message.role == "assistant")
        .expect("assistant");
    assert_eq!(message.status, "complete", "统计行只在终态消息上出现");
    assert_eq!(message.input_tokens, Some(7));
    assert_eq!(message.output_tokens, Some(3));
}

/// 父消息里的一张委派工具卡（`spawn_agent` / `wait_agent`）。
fn parent_tool_message(
    name: &str,
    detail: SubagentDetailView,
    status: ToolStatus,
) -> ChatMessageView {
    ChatMessageView {
        id: format!("parent-{name}"),
        sequence: 0,
        kind: "chat".into(),
        role: "assistant".into(),
        status: "complete".into(),
        turn_index: 0,
        shell: None,
        steps: vec![AssistantStepView::Tool {
            position: 0,
            call: ToolCallView {
                call_id: format!("call-{name}"),
                name: name.into(),
                position: 0,
                arguments_json: "{}".into(),
                status,
                summary: None,
                error: None,
                result_json: None,
                detail: Some(Box::new(ToolDetailView::Subagent(detail))),
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
    }
}

/// 取父消息里第一张委派卡片的详情视图。
fn first_subagent_view(state: &AppState, pane: PaneId) -> SubagentDetailView {
    state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .iter()
        .flat_map(|message| &message.steps)
        .find_map(|step| match step {
            AssistantStepView::Tool { call, .. } => match call.detail.as_deref() {
                Some(ToolDetailView::Subagent(view)) => Some(view.clone()),
                _ => None,
            },
            AssistantStepView::Part { .. } => None,
        })
        .expect("subagent detail")
}

#[test]
fn spawn_card_backfills_agent_path_and_live_activity() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-parent".into());
    }
    let mut started = run("sess-a", "sub-a", AgentRole::Executor);
    started.agent_path = "/root/turing".into();
    seed_chat(&mut started, key(pane, "sess-a"), "msg-a");
    started
        .chat
        .messages
        .last_mut()
        .expect("assistant")
        .steps
        .push(AssistantStepView::Tool {
            position: 0,
            call: ToolCallView {
                call_id: "call-exec".into(),
                name: "exec_command".into(),
                position: 0,
                arguments_json: r#"{"command":"cargo test --all\n--nocapture"}"#.into(),
                status: ToolStatus::Running,
                summary: None,
                error: None,
                result_json: None,
                detail: None,
            },
        });
    assert!(update::on_subagent_started(&mut state, pane, started));
    let mut detail = SubagentDetailView::from_arguments(
        "spawn_agent",
        r#"{"role":"executor","task":"run tests","title":"run tests"}"#,
    )
    .expect("spawn detail");
    detail.agent_id = Some("sub-a".into());
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .push(parent_tool_message(
            "spawn_agent",
            detail,
            ToolStatus::Completed,
        ));

    assert!(update::refresh_subagent_cards(&mut state, pane));
    let view = first_subagent_view(&state, pane);
    assert_eq!(view.agent_path.as_deref(), Some("/root/turing"));
    assert_eq!(view.live_activity.as_deref(), Some("$ cargo test --all"));
}

#[test]
fn wait_all_card_tracks_every_running_subagent() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-parent".into());
    }
    let mut first = run("sess-a", "sub-a", AgentRole::Explorer);
    first.agent_path = "/root/turing".into();
    let mut second = run("sess-b", "sub-b", AgentRole::Executor);
    second.agent_path = "/root/turing/curie".into();
    assert!(update::on_subagent_started(&mut state, pane, first));
    assert!(update::on_subagent_started(&mut state, pane, second));
    let detail = SubagentDetailView::from_arguments("wait_agent", "{}").expect("wait detail");
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .push(parent_tool_message(
            "wait_agent",
            detail,
            ToolStatus::Running,
        ));

    assert!(update::refresh_subagent_cards(&mut state, pane));
    let view = first_subagent_view(&state, pane);
    assert!(view.all, "空 targets 表示等待全部");
    assert_eq!(view.agents.len(), 2, "逐目标回填而不是只画 All");
    assert_eq!(view.agents[0].agent_path.as_deref(), Some("/root/turing"));
    assert_eq!(
        view.agents[1].agent_path.as_deref(),
        Some("/root/turing/curie")
    );
    assert!(view.agents.iter().all(|agent| agent.status == "running"));
}

#[test]
fn wait_targets_match_agent_path() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.chat.session_id = Some("sess-parent".into());
    }
    let mut first = run("sess-a", "sub-a", AgentRole::Explorer);
    first.agent_path = "/root/turing".into();
    let mut second = run("sess-b", "sub-b", AgentRole::Executor);
    second.agent_path = "/root/curie".into();
    assert!(update::on_subagent_started(&mut state, pane, first));
    assert!(update::on_subagent_started(&mut state, pane, second));
    let detail = SubagentDetailView::from_arguments("wait_agent", r#"{"targets":["/root/curie"]}"#)
        .expect("wait detail");
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .push(parent_tool_message(
            "wait_agent",
            detail,
            ToolStatus::Running,
        ));

    assert!(update::refresh_subagent_cards(&mut state, pane));
    let view = first_subagent_view(&state, pane);
    assert_eq!(view.agents.len(), 1);
    assert_eq!(view.agents[0].agent_id, "sub-b");
}

#[test]
fn depth_two_spawn_matches_inner_parent_session_and_sorts_tree() {
    use crate::app::state::{TaskPage, TaskTab};

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("sess-parent".into());

    let mut first = run("sess-a", "sub-a", AgentRole::Executor);
    first.agent_path = "/root/a".into();
    let mut sibling = run("sess-b", "sub-b", AgentRole::Executor);
    sibling.agent_path = "/root/b".into();
    assert!(update::on_subagent_started(&mut state, pane, first));
    assert!(update::on_subagent_started(&mut state, pane, sibling));
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target.lx.task = Some(TaskPage {
            tab: TaskTab::Agents,
            ..TaskPage::default()
        });
        target.lx.task.as_mut().expect("task").agents_window.active = 1;
    }

    let mut grandchild = run("sess-c", "sub-c", AgentRole::Explorer);
    grandchild.agent_path = "/root/a/c".into();
    grandchild.parent_session_id = "sess-a".into();
    assert!(
        update::on_subagent_started(&mut state, pane, grandchild),
        "孙 agent 的父会话是 depth-1 子会话"
    );

    let target = state.pane_anywhere(pane).expect("pane");
    let order: Vec<&str> = target
        .lx
        .agents
        .runs
        .iter()
        .map(|run| run.agent_id.as_str())
        .collect();
    assert_eq!(order, ["sub-a", "sub-c", "sub-b"], "树形前序");
    let page = target.lx.task.as_ref().expect("task");
    assert_eq!(
        target.lx.agents.runs[page.agents_window.active].agent_id, "sub-b",
        "重排后高亮仍指向原 agent"
    );
}

/// 已加载的终态 run（LRU 回收测试用）。
fn completed_run(
    child_session: &str,
    agent_id: &str,
    completed_ms: i64,
    last_accessed_ms: i64,
) -> SubagentRun {
    let mut run = run(child_session, agent_id, AgentRole::Explorer);
    run.status = "completed".into();
    run.completed_at_ms = Some(completed_ms);
    run.loaded = true;
    run.chat.session_id = Some(child_session.into());
    run.last_accessed_ms = last_accessed_ms;
    run
}

#[test]
fn idle_chat_lru_evicts_oldest_and_protects_running_and_selected() {
    use crate::app::state::lx::agent::MAX_RESIDENT_SUBAGENT_CHATS;

    assert_eq!(MAX_RESIDENT_SUBAGENT_CHATS, 6);
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("sess-parent".into());

    let mut runs: Vec<SubagentRun> = (0..8)
        .map(|index| {
            completed_run(
                &format!("sess-{index}"),
                &format!("sub-{index}"),
                1_000 + index,
                index,
            )
        })
        .collect();
    let mut running = run("sess-running", "sub-running", AgentRole::Executor);
    running.loaded = true;
    running.last_accessed_ms = -1;
    runs.push(running);
    state.pane_mut_anywhere(pane).expect("pane").lx.agents.runs = runs;

    let evicted = state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .evict_excess_idle_chats(Some("sub-1"));
    assert_eq!(evicted, ["sub-0", "sub-2"], "最久未访问的空闲会话先卸载");
    let agents = &state.pane_anywhere(pane).expect("pane").lx.agents;
    assert_eq!(
        agents
            .runs
            .iter()
            .filter(|run| !run.is_running() && run.loaded)
            .count(),
        MAX_RESIDENT_SUBAGENT_CHATS,
        "驻留数收敛到上限"
    );
    assert!(
        agents.by_agent("sub-running").expect("running").loaded,
        "运行中不卸载"
    );
    assert!(
        agents.by_agent("sub-1").expect("selected").loaded,
        "选中项受保护"
    );
    assert!(!agents.by_agent("sub-0").expect("evicted").loaded);
    assert!(
        agents
            .by_agent("sub-0")
            .expect("evicted")
            .chat
            .session_id
            .is_none(),
        "内存消息树已清空"
    );
}

#[test]
fn reselecting_an_evicted_run_requests_a_lazy_load() {
    use crate::app::state::{TaskPage, TaskTab};

    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("sess-parent".into());
    let runs: Vec<SubagentRun> = (0..7)
        .map(|index| {
            completed_run(
                &format!("sess-{index}"),
                &format!("sub-{index}"),
                1_000 + index,
                index,
            )
        })
        .collect();
    let target = state.pane_mut_anywhere(pane).expect("pane");
    target.lx.agents.runs = runs;
    let evicted = target.lx.agents.evict_excess_idle_chats(None);
    assert_eq!(evicted, ["sub-0"], "只卸载最旧的一条");
    target.lx.task = Some(TaskPage {
        tab: TaskTab::Agents,
        ..TaskPage::default()
    });
    state.effects.clear();

    assert!(update::select_task_agent(&mut state, pane, 0));
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::LoadSubagentSession { pane: p, agent_id, child_session }
                if *p == pane && agent_id == "sub-0" && child_session == "sess-0"
        )),
        "重新选中已卸载 run 时请求懒加载：{:?}",
        state.effects
    );
}

#[test]
fn subagent_loaded_and_finished_trigger_idle_eviction() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("sess-parent".into());
    let mut runs: Vec<SubagentRun> = (0..7)
        .map(|index| {
            completed_run(
                &format!("sess-{index}"),
                &format!("sub-{index}"),
                1_000 + index,
                index,
            )
        })
        .collect();
    let mut pending = run("sess-target", "sub-target", AgentRole::Explorer);
    pending.status = "completed".into();
    pending.completed_at_ms = Some(2_000);
    pending.loaded = false;
    runs.push(pending);
    state.pane_mut_anywhere(pane).expect("pane").lx.agents.runs = runs;

    assert!(update::on_subagent_loaded(
        &mut state,
        pane,
        "sub-target",
        &[]
    ));
    let agents = &state.pane_anywhere(pane).expect("pane").lx.agents;
    assert!(agents.by_agent("sub-target").expect("target").loaded);
    assert_eq!(
        agents
            .runs
            .iter()
            .filter(|run| !run.is_running() && run.loaded)
            .count(),
        6,
        "加载完成后自动回收超限会话"
    );

    let mut running = run("sess-running", "sub-running", AgentRole::Executor);
    running.loaded = true;
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .runs
        .push(running);
    assert!(update::on_subagent_finished(
        &mut state,
        "sess-running",
        "completed",
        None,
        crate::db::chat::Usage::default(),
    ));
    let agents = &state.pane_anywhere(pane).expect("pane").lx.agents;
    assert!(
        agents.by_agent("sub-running").expect("finished").loaded,
        "刚结束的会话视为最近访问，不立即卸载"
    );
    assert_eq!(
        agents
            .runs
            .iter()
            .filter(|run| !run.is_running() && run.loaded)
            .count(),
        6,
        "终态后同样回收超限会话"
    );
}

#[test]
fn refresh_subagent_cards_updates_nested_spawn_cards() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    state
        .pane_mut_anywhere(pane)
        .expect("pane")
        .lx
        .chat
        .session_id = Some("sess-parent".into());

    let mut parent = run("sess-a", "sub-a", AgentRole::Executor);
    parent.agent_path = "/root/a".into();
    let mut detail = SubagentDetailView::from_arguments(
        "spawn_agent",
        r#"{"role":"explorer","task":"scan","title":"scan"}"#,
    )
    .expect("spawn detail");
    detail.agent_id = Some("sub-c".into());
    parent.chat.messages.push(parent_tool_message(
        "spawn_agent",
        detail,
        ToolStatus::Completed,
    ));
    assert!(update::on_subagent_started(&mut state, pane, parent));

    let mut child = run("sess-c", "sub-c", AgentRole::Explorer);
    child.agent_path = "/root/a/c".into();
    child.parent_session_id = "sess-a".into();
    child.title = "scan".into();
    assert!(update::on_subagent_started(&mut state, pane, child));

    assert!(update::refresh_subagent_cards(&mut state, pane));
    let target = state.pane_anywhere(pane).expect("pane");
    let parent = target.lx.agents.by_agent("sub-a").expect("parent run");
    let view = parent
        .chat
        .messages
        .iter()
        .flat_map(|message| &message.steps)
        .find_map(|step| match step {
            AssistantStepView::Tool { call, .. } => match call.detail.as_deref() {
                Some(ToolDetailView::Subagent(view)) => Some(view.clone()),
                _ => None,
            },
            AssistantStepView::Part { .. } => None,
        })
        .expect("nested spawn detail");
    assert_eq!(view.agent_status.as_deref(), Some("running"));
    assert_eq!(view.agent_path.as_deref(), Some("/root/a/c"));
}

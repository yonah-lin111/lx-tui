//! 子 agent 展示态测试：增量分流、列表幂等、懒加载、汇报挂载、复位与 tab 切换。

use super::*;
use crate::app::state::lx::agent::{AgentsState, SubagentRun, seed_chat};
use crate::app::update;
use crate::chat::agent::{AgentRole, SubagentReportView};
use crate::chat::events::PartDelta;
use crate::db::chat::SubagentRecord;

fn run(child_session: &str, agent_id: &str, role: AgentRole) -> SubagentRun {
    SubagentRun::from_record(&SubagentRecord {
        agent_id: agent_id.into(),
        child_session_id: child_session.into(),
        parent_session_id: "sess-parent".into(),
        spawn_message_id: None,
        role,
        status: "running".into(),
        task_title: "investigate".into(),
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
        child_session_id: "sess-a".into(),
        parent_session_id: "sess-parent".into(),
        spawn_message_id: None,
        role: AgentRole::Explorer,
        status: "completed".into(),
        task_title: "investigate".into(),
        created_at_ms: 1_000,
        completed_at_ms: Some(2_000),
        report_json: Some(
            SubagentReportView {
                agent_id: "sub-a".into(),
                role: AgentRole::Explorer,
                status: "completed".into(),
                files_changed: 1,
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
fn reports_attach_to_the_parent_message_once() {
    let mut state = AppState::demo();
    let pane = state.active_tab().layout.focus();
    {
        let target = state.pane_mut_anywhere(pane).expect("pane");
        target
            .lx
            .chat
            .messages
            .push(crate::app::state::ChatMessageView {
                id: "msg-report".into(),
                sequence: 0,
                kind: "chat".into(),
                role: "user".into(),
                status: "complete".into(),
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
    }
    let report = SubagentReportView {
        agent_id: "sub-1".into(),
        role: AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 2,
        duration_ms: Some(1_400),
        body: "found it".into(),
        delivered_at_ms: 1,
    };
    assert!(update::on_subagent_report(
        &mut state,
        pane,
        "msg-report",
        report.clone()
    ));
    assert!(!update::on_subagent_report(
        &mut state,
        pane,
        "msg-report",
        report
    ));
    let messages = &state.pane_anywhere(pane).expect("pane").lx.chat.messages;
    assert_eq!(messages[0].reports.len(), 1);
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
        },
        &ModelsSnapshot::default(),
    );
    assert!(view.reports.is_empty());
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
}

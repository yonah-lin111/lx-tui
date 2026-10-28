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
        child_session_id: "sess-a".into(),
        parent_session_id: "sess-parent".into(),
        spawn_message_id: None,
        role: AgentRole::Explorer,
        status: "completed".into(),
        title: "investigate".into(),
        created_at_ms: 1_000,
        completed_at_ms: Some(2_000),
        report_json: Some(
            SubagentReportView {
                agent_id: "sub-a".into(),
                title: "test title".into(),
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
        child_session_id: "sess-a".into(),
        parent_session_id: "sess-a-parent".into(),
        spawn_message_id: None,
        role: AgentRole::Explorer,
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
fn agent_card_toggle_waits_for_click_window_and_cancels() {
    use crate::app::state::{CardTarget, CardToggleSurface, TaskPage, TaskTab};
    use std::time::{Duration, Instant};

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
    let id = state
        .pane_anywhere(pane)
        .expect("pane")
        .lx
        .agents
        .by_session("sess-a")
        .expect("run")
        .compact
        .checkpoints[0]
        .id
        .clone();
    let target = CardTarget::Compact {
        checkpoint_id: id.clone(),
    };
    let now = Instant::now();
    update::schedule_card_toggle(
        &mut state,
        pane,
        target.clone(),
        CardToggleSurface::AgentDetail,
        now,
    );
    assert!(state.pending_card_toggle.is_some());
    assert!(!update::tick(&mut state, now + Duration::from_millis(249)));
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .by_session("sess-a")
            .expect("run")
            .compact
            .is_expanded(&id),
        "延迟窗口内不得切换"
    );

    // 新一次按下会清选区；同一路径必须取消待定折叠。
    update::clear_selection(&mut state);
    assert!(state.pending_card_toggle.is_none());
    update::tick(&mut state, now + Duration::from_millis(250));
    assert!(
        !state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .by_session("sess-a")
            .expect("run")
            .compact
            .is_expanded(&id),
        "取消后不得在到期时切换"
    );

    update::schedule_card_toggle(
        &mut state,
        pane,
        target,
        CardToggleSurface::AgentDetail,
        now,
    );
    assert!(update::tick(&mut state, now + Duration::from_millis(250)));
    assert!(
        state
            .pane_anywhere(pane)
            .expect("pane")
            .lx
            .agents
            .by_session("sess-a")
            .expect("run")
            .compact
            .is_expanded(&id),
        "未取消的单击在到期后切换"
    );
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

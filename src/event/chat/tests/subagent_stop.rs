//! 用户主动停止语义：排队取消、不自动恢复、wait/list 标记与显式唤醒。

use super::subagent::{fixture_with_parent, op, spawn_explorer};
use super::*;
use crate::chat::agent::{AgentRole, SubagentInputItem, SubagentOp, SubagentOutcome};

#[tokio::test]
async fn user_stop_marks_stopped_and_cancels_queued_inputs() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first").await;
    // 运行中排队第二条输入：停止时它必须一起取消。
    let queued = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "later".into(),
            }],
            interrupt: false,
            trigger_turn: true,
        },
    )
    .await
    .expect("queued input");
    assert_eq!(queued.tool_name(), "send_input");

    let stopped = crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&agent_id),
    );
    assert_eq!(stopped, 1);
    assert!(streams.user_stopped.contains(&agent_id));
    let handle = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle");
    assert!(handle.stopped_by_user);
    assert!(*handle.cancel.borrow(), "在途轮次收到取消信号");

    let database = persistence.database().expect("db");
    let queues = database
        .subagent_queue_states(fixture.workspace(), &parent)
        .expect("queues");
    assert_eq!(queues.get(&agent_id).map_or(0, |q| q.queued_count), 0);
    let run = state
        .pane_anywhere(key.pane)
        .unwrap()
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(run.stopped_by_user);
    assert_eq!(run.display_status(), "stopped");
    assert_eq!(run.queued_count, 0);
}

#[tokio::test]
async fn stop_all_does_not_rewrite_completed_agents_as_stopped() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "done").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .map(|handle| handle.child_session_id.clone())
        .expect("handle");
    crate::event::chat::subagent::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session,
            generation: key.generation,
        },
        crate::chat::events::StopReason::Complete,
        crate::db::chat::Usage::default(),
    );
    let stopped = crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        None,
    );
    assert_eq!(stopped, 0, "已完成的 agent 不进入用户停止集合");
    assert!(streams.user_stopped.is_empty());
    let run = state
        .pane_anywhere(key.pane)
        .unwrap()
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(!run.stopped_by_user);
    assert_eq!(run.display_status(), "completed");
}

#[tokio::test]
async fn user_stop_produces_no_report_on_settlement() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "report?").await;
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&agent_id),
    );
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .map(|handle| handle.child_session_id.clone())
        .expect("handle");
    crate::event::chat::subagent::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session,
            generation: key.generation,
        },
        crate::chat::events::StopReason::Cancelled,
        crate::db::chat::Usage::default(),
    );
    let pending = persistence
        .database()
        .expect("db")
        .pending_reports(fixture.workspace(), &parent)
        .expect("pending");
    assert!(pending.is_empty(), "用户主动停止不得生成报告");
}

#[tokio::test]
async fn wait_and_list_mark_stopped_by_user() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "stop me").await;
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&agent_id),
    );
    // 取消后句柄仍在结算路径；wait 结算必须带 stopped_by_user。
    let waited = crate::event::chat::subagent::settle_wait(
        &mut streams,
        &persistence,
        &key,
        std::slice::from_ref(&agent_id),
        false,
        false,
        crate::chat::agent::WaitMode::Any,
    )
    .expect("wait");
    match waited {
        SubagentOutcome::Waited { agents, .. } => {
            assert_eq!(agents.len(), 1);
            assert!(agents[0].stopped_by_user);
        }
        other => panic!("expected waited, got {other:?}"),
    }

    let listed = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::List {
            path_prefix: None,
            include_private: false,
        },
    )
    .await
    .expect("list");
    match listed {
        SubagentOutcome::Listed { agents } => {
            assert!(
                agents
                    .iter()
                    .any(|agent| agent.agent_id == agent_id && agent.stopped_by_user),
                "list_agents 必须标记 stopped_by_user"
            );
        }
        other => panic!("expected listed, got {other:?}"),
    }
}

#[tokio::test]
async fn send_input_wakes_user_stopped_agent() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "wake me").await;
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&agent_id),
    );
    let resumed = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::SendInput {
            target: agent_id.clone(),
            items: vec![SubagentInputItem::Text {
                text: "keep going".into(),
            }],
            interrupt: false,
            trigger_turn: true,
        },
    )
    .await
    .expect("wake via send_input");
    assert_eq!(resumed.tool_name(), "send_input");
    assert!(
        !streams.user_stopped.contains(&agent_id),
        "显式唤醒后解除用户停止标记"
    );
}

#[tokio::test]
async fn resume_wakes_user_stopped_agent() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "resume me").await;
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&agent_id),
    );
    let resumed = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::Resume {
            id: agent_id.clone(),
        },
    )
    .await
    .expect("resume");
    assert_eq!(resumed.tool_name(), "resume_agent");
    assert!(!streams.user_stopped.contains(&agent_id));
}

#[test]
fn deleting_the_parent_session_cancels_live_children() {
    let (fixture, _state, key, persistence, mut streams, parent) = fixture_with_parent();
    // 手工登记一个在册句柄：子 turn 的接收端由测试自己持有，信号可确定性观测。
    let (cancel, mut cancel_rx) = tokio::sync::watch::channel(false);
    let (done, _done_rx) = tokio::sync::watch::channel(false);
    streams.subagents.insert(
        "sess-child".into(),
        super::super::subagent::SubagentHandle {
            parent: key.clone(),
            agent_id: "sub-agent".into(),
            nickname: None,
            agent_path: "/root/agent".into(),
            root_session: parent.clone(),
            depth: 1,
            child_session_id: "sess-child".into(),
            child_assistant_id: "msg-child".into(),
            child_user_message_id: "msg-user".into(),
            turn_user_message_ids: vec!["msg-user".into()],
            role: AgentRole::Explorer,
            title: "work".into(),
            task: "work".into(),
            cancel,
            done,
            status: crate::event::chat::subagent::STATUS_RUNNING,
            started_at_ms: 0,
            max_tokens: None,
            closed_without_report: false,
            stopped_by_user: false,
            notify_parent: true,
            mutation_user_id: std::sync::Arc::new(std::sync::RwLock::new(String::new())),
            chat: crate::app::state::LxChat::new(),
            workspace: fixture.workspace().to_path_buf(),
        },
    );
    assert!(
        persistence
            .database()
            .expect("db")
            .delete_chat_session(fixture.workspace(), &parent)
            .is_ok()
    );
    assert!(crate::event::chat::subagent::cancel_for_session(
        &mut streams,
        &parent,
        None,
        None,
    ));
    assert!(cancel_rx.has_changed().expect("receiver alive"));
    assert!(*cancel_rx.borrow_and_update(), "子 turn 收到取消信号");
}

#[test]
fn wait_timeout_is_not_an_error_and_carries_no_report() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(spawn_explorer(
            &mut state,
            &mut streams,
            &persistence,
            &key,
            "slow",
        ));
    // 到期结算：仍在跑的目标只报告状态，不带正文、不写错误。
    let outcome = crate::event::chat::subagent::settle_wait(
        &mut streams,
        &persistence,
        &key,
        &[agent_id],
        true,
        false,
        crate::chat::agent::WaitMode::All,
    )
    .expect("settled");
    let SubagentOutcome::Waited {
        timed_out, agents, ..
    } = outcome
    else {
        panic!("expected waited");
    };
    assert!(timed_out);
    assert_eq!(agents[0].status, "running");
    assert_eq!(agents[0].report, None, "超时不带报告");
}

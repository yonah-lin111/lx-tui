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
    let run = state
        .pane_anywhere(key.pane)
        .unwrap()
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(!run.stopped_by_user, "唤醒后 UI 投影不得继续显示 stopped");
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
    let run = state
        .pane_anywhere(key.pane)
        .unwrap()
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(!run.stopped_by_user, "resume 同步清除 UI stopped 投影");
}

#[test]
fn stop_settles_a_handle_whose_cancel_receiver_is_already_gone() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(spawn_explorer(
            &mut state,
            &mut streams,
            &persistence,
            &key,
            "dead handle",
        ));
    // 模拟子 loop 已退出、但 Finished 尚未处理：cancel 接收端已销毁。
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .map(|handle| handle.child_session_id.clone())
        .expect("handle");
    let mut handle = streams.subagents.remove(&child_session).expect("handle");
    let (cancel, receiver) = watch::channel(false);
    drop(receiver);
    handle.cancel = cancel;
    streams.subagents.insert(child_session.clone(), handle);

    let stopped = crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&agent_id),
    );
    assert_eq!(stopped, 1);
    assert!(
        !streams.subagents.contains_key(&child_session),
        "迟到的结算路径必须收掉句柄，不能留下永久 running"
    );
    let messages = persistence
        .database()
        .expect("db")
        .load_chat_session(fixture.workspace(), &child_session)
        .expect("load")
        .unwrap_or_default();
    assert!(
        messages.iter().all(|message| message.status != "streaming"),
        "死句柄必须被结算，DB 不能永久停在 streaming"
    );
    assert!(streams.user_stopped.contains(&agent_id));
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

/// 测试没有常驻事件循环：手动消费 `SubagentWaitReady`，模拟事件层读库结算并回执。
async fn drive_wait(
    streams: &mut ChatStreams,
    persistence: &Persistence,
    events: &mut mpsc::UnboundedReceiver<AppEvent>,
    receiver: &mut tokio::sync::oneshot::Receiver<Result<SubagentOutcome, HostError>>,
) -> Result<SubagentOutcome, HostError> {
    loop {
        tokio::select! {
            result = &mut *receiver => return result.expect("wait reply"),
            event = events.recv() => {
                let Some(event) = event else {
                    panic!("event channel closed before wait reply");
                };
                if let AppEvent::SubagentWaitReady {
                    key,
                    targets,
                    timed_out,
                    all,
                    mode,
                    timeout_ms,
                    auto_continue,
                    reply,
                } = event
                {
                    crate::event::chat::subagent::handle_subagent_wait_ready(
                        streams,
                        persistence,
                        key,
                        targets,
                        timed_out,
                        all,
                        mode,
                        timeout_ms,
                        auto_continue,
                        reply,
                    );
                }
            }
        }
    }
}

/// 用户停止正在被 `wait_agent` 等待的 subagent：停止是终态，等待立即结算并
/// 把 `stopped_by_user` 交回模型；主轮不取消、不挂起。
#[tokio::test]
async fn user_stop_settles_the_wait_without_stopping_the_parent() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "wait on me").await;
    let (cancel_tx, cancel_rx) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_tx);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<AppEvent>();
    streams.set_event_sender(events_tx);
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    crate::event::chat::subagent::handle_subagent_op(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        SubagentOp::Wait {
            targets: vec![agent_id.clone()],
            mode: crate::chat::agent::WaitMode::All,
            timeout_ms: 60_000,
        },
        reply,
    );
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&agent_id),
    );
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        drive_wait(&mut streams, &persistence, &mut events_rx, &mut receiver),
    )
    .await
    .expect("停止必须立即结算等待，不能永久挂起")
    .expect("停止是正常结果，不是取消错误");
    match outcome {
        SubagentOutcome::Waited {
            timed_out, agents, ..
        } => {
            assert!(!timed_out);
            assert_eq!(agents.len(), 1);
            assert!(agents[0].stopped_by_user);
            assert_eq!(agents[0].status, "stopped");
            assert!(agents[0].report.is_none(), "用户停止不产生报告");
        }
        other => panic!("expected waited, got {other:?}"),
    }
    assert!(!*cancel_rx.borrow(), "停止 subagent 不得取消主轮");
}

/// All 模式等多目标：停止其中一个后继续等剩余目标，剩余目标结算时一并返回。
#[tokio::test]
async fn user_stop_keeps_waiting_for_remaining_targets() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let first = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first").await;
    let second = spawn_explorer(&mut state, &mut streams, &persistence, &key, "second").await;
    let (cancel_tx, _cancel_rx) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_tx);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<AppEvent>();
    streams.set_event_sender(events_tx);
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    crate::event::chat::subagent::handle_subagent_op(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        SubagentOp::Wait {
            targets: vec![first.clone(), second.clone()],
            mode: crate::chat::agent::WaitMode::All,
            timeout_ms: 60_000,
        },
        reply,
    );
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&first),
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(150), &mut receiver)
            .await
            .is_err(),
        "还有目标在跑：等待不得提前结算"
    );
    assert!(
        events_rx.try_recv().is_err(),
        "还有目标在跑：不得提前派发结算事件"
    );
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == second)
        .map(|handle| handle.child_session_id.clone())
        .expect("second handle");
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
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        drive_wait(&mut streams, &persistence, &mut events_rx, &mut receiver),
    )
    .await
    .expect("剩余目标结算后等待必须返回")
    .expect("wait outcome");
    match outcome {
        SubagentOutcome::Waited { agents, .. } => {
            assert_eq!(agents.len(), 2);
            let stopped = agents
                .iter()
                .find(|agent| agent.agent_id == first)
                .expect("stopped target");
            assert!(stopped.stopped_by_user);
            assert_eq!(stopped.status, "stopped");
            let done = agents
                .iter()
                .find(|agent| agent.agent_id == second)
                .expect("finished target");
            assert!(!done.stopped_by_user);
            assert_eq!(done.status, "completed");
        }
        other => panic!("expected waited, got {other:?}"),
    }
}

/// Any 模式等待中停止一个目标：停止即终态，等待立即返回，不阻塞到另一个目标结束。
#[tokio::test]
async fn user_stop_settles_an_in_flight_any_wait() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let stopped_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "stop me").await;
    let running =
        spawn_explorer(&mut state, &mut streams, &persistence, &key, "keep running").await;
    let (cancel_tx, _cancel_rx) = watch::channel(false);
    streams.cancels.insert(key.clone(), cancel_tx);
    let (events_tx, mut events_rx) = mpsc::unbounded_channel::<AppEvent>();
    streams.set_event_sender(events_tx);
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    crate::event::chat::subagent::handle_subagent_op(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        SubagentOp::Wait {
            targets: vec![stopped_id.clone(), running],
            mode: crate::chat::agent::WaitMode::Any,
            timeout_ms: 60_000,
        },
        reply,
    );
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&stopped_id),
    );
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        drive_wait(&mut streams, &persistence, &mut events_rx, &mut receiver),
    )
    .await
    .expect("Any 等待必须在目标停止后立即返回")
    .expect("wait outcome");
    let SubagentOutcome::Waited {
        timed_out, agents, ..
    } = outcome
    else {
        panic!("expected waited");
    };
    assert!(!timed_out);
    let stopped = agents
        .iter()
        .find(|agent| agent.agent_id == stopped_id)
        .expect("stopped target");
    assert!(stopped.stopped_by_user);
    assert_eq!(stopped.status, "stopped");
}

/// 等待发起前目标就已经被停止：立即结算，不再阻塞。
#[tokio::test]
async fn wait_on_already_stopped_agent_returns_immediately() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        "already stopped",
    )
    .await;
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&agent_id),
    );
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    crate::event::chat::subagent::handle_subagent_op(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        SubagentOp::Wait {
            targets: vec![agent_id.clone()],
            mode: crate::chat::agent::WaitMode::All,
            timeout_ms: 60_000,
        },
        reply,
    );
    let outcome = tokio::time::timeout(Duration::from_secs(2), &mut receiver)
        .await
        .expect("已停止目标必须立即结算，不能进入等待")
        .expect("wait reply")
        .expect("wait outcome");
    match outcome {
        SubagentOutcome::Waited { agents, .. } => {
            assert_eq!(agents.len(), 1);
            assert!(agents[0].stopped_by_user);
            assert_eq!(agents[0].status, "stopped");
        }
        other => panic!("expected waited, got {other:?}"),
    }
}

/// Any 模式：目标在等待发起前已停止时同样立即结算，不因另一个目标仍在跑而阻塞。
#[tokio::test]
async fn wait_any_returns_when_a_target_was_stopped_before_the_wait() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let stopped = spawn_explorer(&mut state, &mut streams, &persistence, &key, "stopped").await;
    let running = spawn_explorer(&mut state, &mut streams, &persistence, &key, "running").await;
    crate::event::chat::subagent::stop_subagents_by_user(
        &mut state,
        &mut streams,
        &persistence,
        key.pane,
        &parent,
        Some(&stopped),
    );
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    crate::event::chat::subagent::handle_subagent_op(
        &mut state,
        &mut streams,
        &persistence,
        key.clone(),
        SubagentOp::Wait {
            targets: vec![stopped.clone(), running],
            mode: crate::chat::agent::WaitMode::Any,
            timeout_ms: 60_000,
        },
        reply,
    );
    let outcome = tokio::time::timeout(Duration::from_secs(2), &mut receiver)
        .await
        .expect("Any 模式不得等待已停止目标")
        .expect("wait reply")
        .expect("wait outcome");
    match outcome {
        SubagentOutcome::Waited { agents, .. } => {
            assert_eq!(agents.len(), 2);
            assert!(
                agents
                    .iter()
                    .any(|agent| agent.agent_id == stopped && agent.stopped_by_user)
            );
        }
        other => panic!("expected waited, got {other:?}"),
    }
}

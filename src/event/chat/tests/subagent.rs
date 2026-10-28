//! 子 agent 事件层测试：spawn 落库与限额、wait 结算（恰好一次）、close 无报告、
//! delta 分流、窗格已关闭时的工具握手、启动恢复与进程回收白名单。

use super::*;
use crate::chat::agent::{
    AgentRole, LIMIT_REACHED_CODE, MAX_CONCURRENT_SUBAGENTS, NOT_FOUND_CODE, SubagentOp,
    SubagentOutcome,
};

/// 带父会话的事件层 fixture：冻结模型与工作区已登记，事件通道常驻。
fn fixture_with_parent() -> (
    Fixture,
    AppState,
    RequestKey,
    Persistence,
    ChatStreams,
    String,
) {
    let fixture = Fixture::new("subagent");
    let (state, key, persistence, _session) = streaming_state(&fixture);
    let frozen = crate::app::state::FrozenModel::resolve(
        &state.models,
        &crate::config::ai::ModelSelection {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
    )
    .expect("frozen");
    let mut streams = ChatStreams::default();
    streams.frozen.insert(key.session.clone(), frozen);
    streams
        .db_workspaces
        .insert(key.clone(), fixture.workspace().to_path_buf());
    let (sender, receiver) = mpsc::unbounded_channel::<AppEvent>();
    streams.set_event_sender(sender);
    // 子 turn 的异步回投需要通道可写；测试只关心同步路径，故不消费。
    std::mem::forget(receiver);
    // 子会话的进程运行实例：事件层从这里取 manager（生产由 `set_process_sender` 注入）。
    let (process_sender, process_receiver) =
        mpsc::unbounded_channel::<crate::chat::process::ProcessUiEvent>();
    streams.set_process_sender(process_sender);
    std::mem::forget(process_receiver);
    let parent = key.session.clone();
    (fixture, state, key, persistence, streams, parent)
}

/// 执行一次控制操作并等待回执。
async fn op(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    op: SubagentOp,
) -> Result<SubagentOutcome, HostError> {
    let (reply, receiver) = tokio::sync::oneshot::channel();
    crate::event::chat::subagent::handle_subagent_op(
        state,
        streams,
        persistence,
        key.clone(),
        op,
        reply,
    );
    receiver.await.expect("reply")
}

async fn spawn_explorer(
    state: &mut AppState,
    streams: &mut ChatStreams,
    persistence: &Persistence,
    key: &RequestKey,
    task: &str,
) -> String {
    let outcome = op(
        state,
        streams,
        persistence,
        key,
        SubagentOp::Spawn {
            role: AgentRole::Explorer,
            title: "test title".into(),
            task: task.into(),
        },
    )
    .await
    .expect("spawn");
    match outcome {
        SubagentOutcome::Spawned { agent_id, .. } => agent_id,
        other => panic!("expected spawned, got {other:?}"),
    }
}

#[tokio::test]
async fn main_cancel_releases_wait_but_keeps_child_running() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "wait me").await;
    let child_cancel = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("child handle")
        .cancel
        .subscribe();
    let (cancel, _cancel_rx) = tokio::sync::watch::channel(false);
    streams.cancels.insert(key.clone(), cancel);
    let (reply, receiver) = tokio::sync::oneshot::channel();
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
    assert!(streams.cancel(&key));
    let result = tokio::time::timeout(std::time::Duration::from_secs(1), receiver)
        .await
        .expect("wait reply after cancel")
        .expect("reply channel");
    assert!(
        matches!(result, Err(HostError::Cancelled)),
        "主轮 Esc 释放 wait 回执：{result:?}"
    );
    assert!(!*child_cancel.borrow(), "子 agent 本身继续运行");
    assert!(
        streams
            .subagents
            .values()
            .any(|handle| handle.agent_id == agent_id),
        "句柄仍在册"
    );
}

#[tokio::test]
async fn main_cancel_keeps_children_running() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let first = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first").await;
    let second = spawn_explorer(&mut state, &mut streams, &persistence, &key, "second").await;
    let first_cancel = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == first)
        .expect("first handle")
        .cancel
        .subscribe();
    let second_cancel = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == second)
        .expect("second handle")
        .cancel
        .subscribe();
    let (cancel, _cancel_rx) = tokio::sync::watch::channel(false);
    streams.cancels.insert(key.clone(), cancel);
    assert!(streams.cancel(&key));
    assert!(!*first_cancel.borrow());
    assert!(!*second_cancel.borrow());
}

#[tokio::test]
async fn fourth_spawn_is_rejected_and_earlier_runs_stay_running() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    for index in 0..MAX_CONCURRENT_SUBAGENTS {
        spawn_explorer(
            &mut state,
            &mut streams,
            &persistence,
            &key,
            &format!("task {index}"),
        )
        .await;
    }
    let error = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::Spawn {
            role: AgentRole::Executor,
            title: "too many".into(),
            task: "one too many".into(),
        },
    )
    .await
    .expect_err("limit");
    assert_eq!(
        error,
        HostError::Failed {
            code: LIMIT_REACHED_CODE.to_string()
        }
    );
    let records = persistence
        .database()
        .expect("db")
        .list_subagents(fixture.workspace(), &parent)
        .expect("list");
    assert_eq!(records.len(), MAX_CONCURRENT_SUBAGENTS);
    assert!(records.iter().all(|record| record.status == "running"));
}

#[tokio::test]
async fn wait_claims_the_report_exactly_once() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id =
        spawn_explorer(&mut state, &mut streams, &persistence, &key, "investigate").await;
    // 等价于 `finish_subagent` 的 DB 效果：agent 行 completed + report_json。
    let report = crate::chat::agent::SubagentReportView {
        agent_id: agent_id.clone(),
        title: "test title".into(),
        role: AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 1,
        duration_ms: Some(20),
        body: "the answer".into(),
        delivered_at_ms: 1,
    };
    {
        let database = persistence.database().expect("db");
        database
            .connection()
            .execute(
                "UPDATE agent_subagent SET status = 'completed', report_json = ?1 \
                 WHERE external_id = ?2",
                rusqlite::params![report.to_json(), agent_id],
            )
            .expect("update");
    }
    // wait 只从 DB 结算；在册句柄不影响结果。
    streams.subagents.clear();

    async fn wait(
        state: &mut AppState,
        streams: &mut ChatStreams,
        persistence: &Persistence,
        key: &RequestKey,
        agent_id: &str,
    ) -> (bool, Vec<crate::chat::agent::AgentStatusView>) {
        let outcome = op(
            state,
            streams,
            persistence,
            key,
            SubagentOp::Wait {
                targets: vec![agent_id.to_string()],
                mode: crate::chat::agent::WaitMode::All,
                timeout_ms: 1_000,
            },
        )
        .await
        .expect("wait");
        match outcome {
            SubagentOutcome::Waited {
                timed_out, agents, ..
            } => (timed_out, agents),
            other => panic!("expected waited, got {other:?}"),
        }
    }
    let (timed_out, agents) = wait(&mut state, &mut streams, &persistence, &key, &agent_id).await;
    assert!(!timed_out);
    assert_eq!(agents[0].report.as_deref(), Some("the answer"));
    let (_, agents) = wait(&mut state, &mut streams, &persistence, &key, &agent_id).await;
    assert_eq!(agents[0].report, None, "报告恰好一次");
    let records = persistence
        .database()
        .expect("db")
        .list_subagents(fixture.workspace(), &parent)
        .expect("list");
    assert!(records[0].report_delivered);
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .chat
            .messages
            .iter()
            .all(|message| message.reports.is_empty()),
        "wait 消费的报告只进工具结果，不插消息行"
    );
}

#[tokio::test]
async fn idle_parent_batches_pending_reports_into_one_turn() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let first = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first").await;
    let second = spawn_explorer(&mut state, &mut streams, &persistence, &key, "second").await;
    for (agent_id, body) in [(&first, "first answer"), (&second, "second answer")] {
        let report = crate::chat::agent::SubagentReportView {
            agent_id: agent_id.clone(),
            title: format!("title {agent_id}"),
            role: AgentRole::Explorer,
            status: "completed".into(),
            files_changed: 0,
            duration_ms: Some(20),
            body: body.into(),
            delivered_at_ms: 1,
        };
        persistence
            .database()
            .expect("db")
            .connection()
            .execute(
                "UPDATE agent_subagent SET status = 'completed', report_json = ?1 \
                 WHERE external_id = ?2",
                rusqlite::params![report.to_json(), agent_id],
            )
            .expect("update report");
    }
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.chat.request = crate::app::state::RequestState::Idle;
        target.lx.chat.status.streaming = false;
        target.lx.chat.queue.clear();
    }
    let before = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .len();
    assert!(crate::event::chat::subagent::drain_reports(
        &mut state,
        &persistence,
        key.pane
    ));
    {
        let target = state.pane_anywhere(key.pane).expect("pane");
        let messages = &target.lx.chat.messages;
        assert_eq!(messages.len(), before + 2, "一条报告消息 + 一个 assistant");
        let reports = &messages[before];
        assert_eq!(reports.reports.len(), 2, "多个报告合并进同一张大卡消息");
        assert_eq!(messages[before + 1].role, "assistant");
        assert_eq!(messages[before + 1].status, "streaming");
        assert!(matches!(
            target.lx.chat.request,
            crate::app::state::RequestState::Streaming { .. }
        ));
        assert!(target.lx.chat.status.streaming);
    }
    assert!(
        state
            .effects
            .iter()
            .any(|effect| matches!(effect, crate::app::effects::Effect::StartChatStream { .. }))
    );
    let records = persistence
        .database()
        .expect("db")
        .list_subagents(fixture.workspace(), &parent)
        .expect("list");
    assert!(records.iter().all(|record| record.report_delivered));
}

#[tokio::test]
async fn idle_parent_finish_starts_report_batch_immediately() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id =
        spawn_explorer(&mut state, &mut streams, &persistence, &key, "investigate").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .map(|handle| handle.child_session_id.clone())
        .expect("child");
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.chat.request = crate::app::state::RequestState::Idle;
        target.lx.chat.status.streaming = false;
        target.lx.chat.queue.clear();
    }
    let before = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .len();
    assert!(crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        crate::app::state::RequestKey {
            pane: key.pane,
            session: child_session,
            generation: key.generation,
        },
        crate::chat::events::StopReason::Complete,
        crate::db::chat::Usage::default(),
    ));
    let target = state.pane_anywhere(key.pane).expect("pane");
    assert_eq!(target.lx.chat.messages.len(), before + 2);
    assert_eq!(target.lx.chat.messages[before].reports.len(), 1);
    assert_eq!(target.lx.chat.messages[before + 1].status, "streaming");
    let records = persistence
        .database()
        .expect("db")
        .list_subagents(fixture.workspace(), &parent)
        .expect("list");
    assert!(records[0].report_delivered);
}

#[tokio::test]
async fn queued_user_chat_wins_before_report_drain() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first").await;
    let report = crate::chat::agent::SubagentReportView {
        agent_id: agent_id.clone(),
        title: "title".into(),
        role: AgentRole::Explorer,
        status: "completed".into(),
        files_changed: 0,
        duration_ms: Some(20),
        body: "answer".into(),
        delivered_at_ms: 1,
    };
    persistence
        .database()
        .expect("db")
        .connection()
        .execute(
            "UPDATE agent_subagent SET status = 'completed', report_json = ?1 \
             WHERE external_id = ?2",
            rusqlite::params![report.to_json(), agent_id],
        )
        .expect("update report");
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        target.lx.chat.request = crate::app::state::RequestState::Idle;
        target.lx.chat.status.streaming = false;
        target
            .lx
            .chat
            .queue
            .push(crate::app::state::QueuedSubmission::Chat {
                draft: "user first".into(),
            });
    }
    assert!(
        !crate::event::chat::subagent::drain_reports(&mut state, &persistence, key.pane),
        "用户排队输入优先，报告延后"
    );
}

#[tokio::test]
async fn wait_all_without_targets_returns_all_records() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let outcome = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::Wait {
            targets: Vec::new(),
            mode: crate::chat::agent::WaitMode::All,
            timeout_ms: 10,
        },
    )
    .await
    .expect("wait all");
    let SubagentOutcome::Waited {
        timed_out,
        agents,
        all,
        mode,
    } = outcome
    else {
        panic!("expected waited");
    };
    assert!(!timed_out);
    assert!(agents.is_empty());
    assert!(all);
    assert_eq!(mode, crate::chat::agent::WaitMode::All);
}

#[tokio::test]
async fn wait_any_returns_immediately_when_a_target_already_finished() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let first = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first").await;
    let second = spawn_explorer(&mut state, &mut streams, &persistence, &key, "second").await;
    let first_child = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == first)
        .map(|handle| handle.child_session_id.clone())
        .expect("first child");
    assert!(crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        crate::app::state::RequestKey {
            pane: key.pane,
            session: first_child,
            generation: key.generation,
        },
        crate::chat::events::StopReason::Complete,
        crate::db::chat::Usage::default(),
    ));
    assert_eq!(
        persistence
            .database()
            .expect("db")
            .list_subagents(fixture.workspace(), &key.session)
            .expect("list")
            .iter()
            .find(|record| record.agent_id == first)
            .map(|record| record.status.as_str()),
        Some("completed")
    );
    let outcome = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::Wait {
            targets: vec![first.clone(), second.clone()],
            mode: crate::chat::agent::WaitMode::Any,
            timeout_ms: 5_000,
        },
    )
    .await
    .expect("wait any");
    let SubagentOutcome::Waited {
        timed_out,
        agents,
        mode,
        ..
    } = outcome
    else {
        panic!("expected waited");
    };
    assert!(!timed_out, "已有目标完成，不应等到超时");
    assert_eq!(mode, crate::chat::agent::WaitMode::Any);
    assert_eq!(agents.len(), 2);
    assert_eq!(
        agents
            .iter()
            .find(|agent| agent.agent_id == first)
            .map(|agent| agent.status.as_str()),
        Some("completed")
    );
    assert_eq!(
        agents
            .iter()
            .find(|agent| agent.agent_id == second)
            .map(|agent| agent.status.as_str()),
        Some("running")
    );
}

#[tokio::test]
async fn wait_all_auto_continues_instead_of_returning_timeout() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        "still running",
    )
    .await;
    let (reply, mut receiver) = tokio::sync::oneshot::channel();
    let changed = crate::event::chat::subagent::handle_subagent_wait_ready(
        &mut streams,
        &persistence,
        key.clone(),
        Vec::new(),
        true,
        true,
        crate::chat::agent::WaitMode::All,
        10,
        true,
        reply,
    );
    assert!(!changed);
    assert!(
        receiver.try_recv().is_err(),
        "目标仍在跑：auto wait 不得把 timeout 交回模型"
    );
    assert_eq!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .agents
            .by_agent(&agent_id)
            .expect("agent")
            .status,
        "running"
    );
}

#[tokio::test]
async fn wait_rejects_unknown_targets() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let error = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::Wait {
            targets: vec!["sub-nope".into()],
            mode: crate::chat::agent::WaitMode::All,
            timeout_ms: 10,
        },
    )
    .await
    .expect_err("not found");
    assert_eq!(
        error,
        HostError::Failed {
            code: NOT_FOUND_CODE.to_string()
        }
    );
}

#[tokio::test]
async fn close_cancels_and_produces_no_report() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "stop me").await;
    let outcome = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::Close {
            target: agent_id.clone(),
        },
    )
    .await
    .expect("close");
    assert!(matches!(outcome, SubagentOutcome::Closed { .. }));
    let records = persistence
        .database()
        .expect("db")
        .list_subagents(fixture.workspace(), &parent)
        .expect("list");
    assert!(records[0].report_json.is_none(), "close 不产生报告");
}

#[tokio::test]
async fn child_delta_never_reaches_the_parent_conversation() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        "read something",
    )
    .await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .map(|handle| handle.child_session_id.clone())
        .expect("handle");
    let parent_messages = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .len();
    let changed = handle_net_event(
        &mut state,
        &mut streams,
        &persistence,
        ChatNetEvent::Delta {
            key: RequestKey {
                pane: key.pane,
                session: child_session,
                generation: key.generation,
            },
            deltas: vec![ChatDelta::Part(crate::chat::events::PartDelta {
                kind: PartKind::Text,
                text: "child text".into(),
                metadata: None,
            })],
        },
    );
    assert!(changed);
    assert_eq!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .chat
            .messages
            .len(),
        parent_messages,
        "子 delta 不进主对话"
    );
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert_eq!(
        run.chat.messages.last().expect("assistant").text(),
        "child text"
    );
    let _ = fixture;
}

#[tokio::test]
async fn child_tool_handshake_works_after_the_pane_is_gone() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        "touch nothing",
    )
    .await;
    let (child_session, child_assistant) = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .map(|handle| {
            (
                handle.child_session_id.clone(),
                handle.child_assistant_id.clone(),
            )
        })
        .expect("handle");
    let child_key = RequestKey {
        pane: key.pane,
        session: child_session.clone(),
        generation: key.generation,
    };
    // 窗格彻底移除：主轮归属校验会失败，子轮必须仍能握手。
    // 再开一个窗格用于移除原窗格：主轮归属校验会失败，子轮必须仍能握手。
    let second = state
        .active_tab_mut()
        .split_pane(key.pane, crate::app::state::Direction::Horizontal)
        .expect("split");
    assert_ne!(second, key.pane);
    assert!(state.active_tab_mut().remove_pane(key.pane));
    streams
        .db_workspaces
        .insert(child_key.clone(), fixture.workspace().to_path_buf());
    let (reply, receiver) = tokio::sync::oneshot::channel();
    let changed = begin_tool(
        &mut state,
        &mut streams,
        &persistence,
        child_key.clone(),
        crate::chat::agent_loop::ReadyToolCall {
            call_id: "child-read".into(),
            name: "read".into(),
            arguments_json: r#"{"path":"a.txt"}"#.into(),
            ordinal: 0,
        },
        reply,
    );
    // 窗格已关闭时 UI 不会变化（`changed` 为 false 合理），但握手与回执必须成功。
    let _ = changed;
    let ticket = receiver.await.expect("reply").expect("ticket");
    assert_eq!(ticket.session_id, child_session);
    assert_eq!(ticket.message_id, child_assistant);

    let (reply, receiver) = tokio::sync::oneshot::channel();
    finish_tool(
        &mut state,
        &mut streams,
        &persistence,
        child_key,
        &ticket,
        crate::chat::agent_loop::ToolOutcome::Executed(ToolExecutionResult::Failed {
            code: crate::chat::tools::ToolErrorCode::Io,
            message: "test failure".into(),
            changes: Vec::new(),
        }),
        reply,
    );
    assert!(receiver.await.expect("reply").is_ok());
}

#[tokio::test]
async fn recover_running_subagents_cancels_leftovers() {
    let (fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    spawn_explorer(&mut state, &mut streams, &persistence, &key, "leftover").await;
    let database = persistence.database().expect("db");
    assert_eq!(database.recover_running_subagents().expect("recover"), 1);
    let records = database
        .list_subagents(fixture.workspace(), &parent)
        .expect("list");
    assert_eq!(records[0].status, "cancelled");
}

#[tokio::test]
async fn reap_orphan_runtimes_keeps_running_children() {
    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id =
        spawn_explorer(&mut state, &mut streams, &persistence, &key, "keep running").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .map(|handle| handle.child_session_id.clone())
        .expect("handle");
    // 窗格已不再绑定任何会话：运行中的子会话不算孤儿。
    let (sender, receiver) = mpsc::unbounded_channel::<AppEvent>();
    std::mem::forget(receiver);
    pump_chat_streams(&mut state, &mut streams, &sender, &persistence);
    assert!(
        streams.runtimes.get(&child_session).is_some(),
        "子会话运行实例不被回收"
    );
    let _ = fixture;
}

#[tokio::test]
async fn escape_cancels_only_the_parent_turn() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "keep going").await;
    assert!(update::escape_stop(&mut state, key.pane));
    assert!(update::stop_stream(&mut state, key.pane));
    // 主轮取消不影响子句柄：仍在册且没有收到取消信号。
    let handle = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle survived");
    assert_eq!(handle.status, crate::event::chat::subagent::STATUS_RUNNING);
    assert!(!*handle.cancel.borrow());
}

#[tokio::test]
async fn selected_cancel_only_stops_the_target_child() {
    let (_fixture, mut state, key, persistence, mut streams, parent) = fixture_with_parent();
    let first = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first").await;
    let second = spawn_explorer(&mut state, &mut streams, &persistence, &key, "second").await;
    let first_cancel = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == first)
        .expect("first handle")
        .cancel
        .subscribe();
    let second_cancel = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == second)
        .expect("second handle")
        .cancel
        .subscribe();
    assert!(crate::event::chat::subagent::cancel_for_session(
        &mut streams,
        &parent,
        Some(&first),
    ));
    assert!(*first_cancel.borrow(), "选中的子 agent 收到取消");
    assert!(!*second_cancel.borrow(), "其他 running 子 agent 继续");
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
            child_session_id: "sess-child".into(),
            child_assistant_id: "msg-child".into(),
            child_user_message_id: "msg-user".into(),
            role: AgentRole::Explorer,
            title: "work".into(),
            task: "work".into(),
            cancel,
            done,
            status: crate::event::chat::subagent::STATUS_RUNNING,
            started_at_ms: 0,
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

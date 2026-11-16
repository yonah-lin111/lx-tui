//! 详情 `/compact` 事件层：忙碌拒绝、无执行回执丢弃与 BTW 检查点恢复到视图。

use super::subagent::{fixture_with_parent, spawn_explorer};
use super::*;
use crate::app::state::DetailTarget;
use crate::event::chat::compact::CompactOutcome;

#[tokio::test]
async fn compact_detail_busy_agent_is_rejected() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let started = crate::event::chat::detail_compact::prepare(
        &mut state,
        &sender,
        &persistence,
        key.pane,
        DetailTarget::Agent(agent_id.clone()),
    );
    assert!(started, "忙碌拒绝仍算已处理");
    assert!(
        !state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .agents
            .by_agent(&agent_id)
            .expect("run")
            .compact
            .is_running(),
        "忙碌时不得进入压缩"
    );
    assert!(receiver.try_recv().is_err(), "忙碌拒绝不应发摘要请求或回执");
}

#[tokio::test]
async fn compact_detail_rejects_reentry_while_compacting() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session,
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent_mut(&agent_id)
        .expect("run")
        .compact
        .start("existing-op".into(), 1);
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    assert!(crate::event::chat::detail_compact::prepare(
        &mut state,
        &sender,
        &persistence,
        key.pane,
        DetailTarget::Agent(agent_id.clone()),
    ));
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert_eq!(
        run.compact.running.as_deref(),
        Some("existing-op"),
        "第二次 /compact 不得覆盖执行中的幂等键"
    );
    assert!(
        receiver.try_recv().is_err(),
        "压缩中再次 /compact 不得发第二次摘要请求"
    );
}

#[tokio::test]
async fn compact_detail_finished_without_running_is_ignored() {
    let (fixture, mut state, key, _persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id =
        spawn_explorer(&mut state, &mut streams, &_persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    assert!(!crate::event::chat::detail_compact::handle_finished(
        &mut state,
        &_persistence,
        key.pane,
        &child_session,
        fixture.workspace(),
        "stale-op",
        &CompactOutcome::Empty,
    ));
}

/// 用本地 mock SSE 让详情压缩真正跑完：检查点落在子会话，不碰主会话。
#[tokio::test]
async fn compact_detail_commits_checkpoint_for_child_session() {
    use super::compact::{spawn_mock, summary_stream};

    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    let base_url = spawn_mock("200 OK", summary_stream()).await;
    state
        .models
        .settings
        .providers
        .get_mut("p")
        .expect("provider")
        .options
        .base_url = base_url;
    let persistence = Persistence::with_database_at(fixture.database(), fixture.db_path());
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    assert!(
        crate::event::chat::detail_compact::prepare(
            &mut state,
            &sender,
            &persistence,
            key.pane,
            DetailTarget::Agent(agent_id.clone()),
        ),
        "详情压缩应启动"
    );
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .agents
            .by_agent(&agent_id)
            .expect("run")
            .compact
            .is_running(),
        "启动后进入 compacting"
    );
    let AppEvent::CompactFinished {
        pane,
        session_id,
        workspace,
        operation_id,
        outcome,
    } = tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
        .await
        .expect("压缩必须在超时前回投")
        .expect("compact 事件")
    else {
        panic!("expected CompactFinished");
    };
    assert_eq!(outcome, CompactOutcome::Committed);
    assert_eq!(pane, key.pane);
    assert_eq!(session_id, child_session);
    assert!(crate::event::chat::compact::handle_compact_finished(
        &mut state,
        &persistence,
        pane,
        &session_id,
        &workspace,
        &operation_id,
        outcome,
    ));
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(!run.compact.is_running(), "回执后结束执行态");
    assert_eq!(run.compact.checkpoints.len(), 1, "检查点回填到子 agent");
    assert!(
        run.compact.checkpoints[0]
            .summary
            .contains("ship the harness"),
        "摘要来自本轮：{:?}",
        run.compact.checkpoints[0].summary
    );
}

/// 空摘要不写检查点，详情只提示 Nothing new to compact。
#[tokio::test]
async fn compact_detail_empty_reports_nothing_to_compact() {
    use super::compact::spawn_mock;

    let (fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session.clone(),
            generation: key.generation,
        },
        StopReason::Complete,
        Usage::default(),
    );
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"   \"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{}, \"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let base_url = spawn_mock("200 OK", body).await;
    state
        .models
        .settings
        .providers
        .get_mut("p")
        .expect("provider")
        .options
        .base_url = base_url;
    let persistence = Persistence::with_database_at(fixture.database(), fixture.db_path());
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    assert!(crate::event::chat::detail_compact::prepare(
        &mut state,
        &sender,
        &persistence,
        key.pane,
        DetailTarget::Agent(agent_id.clone()),
    ));
    let AppEvent::CompactFinished {
        pane,
        session_id,
        workspace,
        operation_id,
        outcome,
    } = tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
        .await
        .expect("压缩必须在超时前回投")
        .expect("compact 事件")
    else {
        panic!("expected CompactFinished");
    };
    assert!(
        matches!(&outcome, CompactOutcome::Failed { message } if message.contains("empty")),
        "空白摘要必须失败：{outcome:?}"
    );
    assert!(crate::event::chat::compact::handle_compact_finished(
        &mut state,
        &persistence,
        pane,
        &session_id,
        &workspace,
        &operation_id,
        outcome,
    ));
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(!run.compact.is_running());
    assert!(run.compact.checkpoints.is_empty(), "失败不得写检查点");
    assert!(
        fixture
            .reopen()
            .compact_checkpoints(fixture.workspace(), &child_session)
            .expect("checkpoints")
            .is_empty()
    );
}

/// BTW 线程同样能压缩，检查点回填到线程视图。
#[tokio::test]
async fn btw_compact_persists_checkpoint_and_reloads_thread() {
    use super::compact::{spawn_mock, summary_stream};

    let (fixture, mut state, key, _persistence, _streams, _parent) = fixture_with_parent();
    let thread = super::detail_rewind::btw_fixture(&fixture, &mut state, &key);
    let base_url = spawn_mock("200 OK", summary_stream()).await;
    state
        .models
        .settings
        .providers
        .get_mut("p")
        .expect("provider")
        .options
        .base_url = base_url;
    let persistence = Persistence::with_database_at(fixture.database(), fixture.db_path());
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    assert!(crate::event::chat::detail_compact::prepare(
        &mut state,
        &sender,
        &persistence,
        key.pane,
        DetailTarget::Btw(thread.id.clone()),
    ));
    let AppEvent::CompactFinished {
        pane,
        session_id,
        workspace,
        operation_id,
        outcome,
    } = tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
        .await
        .expect("压缩必须在超时前回投")
        .expect("compact 事件")
    else {
        panic!("expected CompactFinished");
    };
    assert_eq!(outcome, CompactOutcome::Committed);
    assert_eq!(session_id, thread.child_session_id);
    assert!(crate::event::chat::compact::handle_compact_finished(
        &mut state,
        &persistence,
        pane,
        &session_id,
        &workspace,
        &operation_id,
        outcome,
    ));
    let thread_state = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .btw
        .thread_by_thread_id(&thread.id)
        .expect("thread kept");
    assert!(!thread_state.compact.is_running());
    assert_eq!(thread_state.compact.checkpoints.len(), 1);
}

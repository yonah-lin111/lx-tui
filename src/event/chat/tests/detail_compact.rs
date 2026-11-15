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

//! `/report` 一次性总结：效应存活、真实请求、检查点落库与主输入框芯片。

use super::subagent::{fixture_with_parent, spawn_explorer};
use super::*;

#[tokio::test]
async fn report_effect_survives_persistence_pump_and_starts_one_shot_summary() {
    let (_fixture, mut state, key, mut persistence, mut streams, parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "report").await;
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
    state.effects.clear();
    state
        .effects
        .push(crate::app::effects::Effect::SendSubagentReport {
            pane: key.pane,
            session_id: parent,
            agent_id: agent_id.clone(),
            note: String::new(),
        });
    // 复现用户现场：事件循环先跑持久化泵，再把报告效应交给一次性总结泵。
    persistence.pump(&mut state);
    assert!(
        state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentReport { .. }
        )),
        "持久化泵不得吞掉报告效应：{:?}",
        state.effects
    );
    let (sender, _receiver) = mpsc::unbounded_channel::<AppEvent>();
    crate::event::chat::apply_compact_effects(&mut state, &sender, &persistence);
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            crate::app::effects::Effect::SendSubagentReport { .. }
        )),
        "报告效应必须被一次性总结泵消费"
    );
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(run.compact.running_report, "报告必须进入执行态");
    assert!(
        !state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .agents
            .by_agent(&agent_id)
            .expect("run")
            .chat
            .messages
            .iter()
            .any(|message| message.role == "user"
                && crate::chat::agent::is_report_request(&message.text())),
        "一次性总结不得创建 user 消息"
    );
}

/// `/report` 端到端：一次性请求 → `kind=report` 检查点 → 主输入框芯片；
/// 子会话不得新增 user/assistant 报告消息。
#[tokio::test]
async fn report_generates_one_shot_checkpoint_and_fills_main_input() {
    let (fixture, mut state, key, _old, mut streams, parent) = fixture_with_parent();
    let persistence = Persistence::with_database_at(fixture.database(), fixture.db_path());
    let base_url = super::compact::spawn_mock("200 OK", super::compact::summary_stream()).await;
    if let Some(provider) = state.models.settings.providers.get_mut("p") {
        provider.options.base_url = base_url;
    }
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "report me").await;
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
            session: child_session.clone(),
            generation: key.generation,
        },
        crate::chat::events::StopReason::Complete,
        crate::db::chat::Usage::default(),
    );
    let before = persistence
        .database()
        .expect("db")
        .load_chat_session(fixture.workspace(), &child_session)
        .expect("messages")
        .map(|messages| messages.len())
        .unwrap_or_default();
    state.effects.clear();
    state
        .effects
        .push(crate::app::effects::Effect::SendSubagentReport {
            pane: key.pane,
            session_id: parent,
            agent_id: agent_id.clone(),
            note: "focus".into(),
        });
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    assert!(crate::event::chat::apply_compact_effects(
        &mut state,
        &sender,
        &persistence
    ));
    let event = tokio::time::timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("report 必须在超时前回投")
        .expect("report 事件");
    let AppEvent::CompactFinished {
        pane,
        session_id,
        workspace,
        operation_id,
        outcome,
    } = event
    else {
        panic!("unexpected event: {event:?}");
    };
    assert_eq!(
        outcome,
        crate::event::chat::compact::CompactOutcome::Committed
    );
    assert!(crate::event::chat::handle_compact_finished(
        &mut state,
        &persistence,
        pane,
        &session_id,
        &workspace,
        &operation_id,
        outcome,
    ));
    let after = persistence
        .database()
        .expect("db")
        .load_chat_session(fixture.workspace(), &child_session)
        .expect("messages")
        .map(|messages| messages.len())
        .unwrap_or_default();
    assert_eq!(after, before, "报告不得向子会话追加消息");
    let run = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .expect("run");
    assert!(!run.compact.is_running());
    assert!(
        run.compact
            .checkpoints
            .iter()
            .any(|checkpoint| checkpoint.report),
        "报告必须作为可撤销检查点出现在面板"
    );
    let input = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .input
        .clone();
    let tokens = crate::app::paste::folded_tokens(&input);
    assert_eq!(tokens.len(), 1);
    assert_eq!(
        tokens[0].kind,
        crate::app::paste::FoldedKind::SubagentReport
    );
    assert!(tokens[0].body.contains("[Subagent report]"));
    assert!(tokens[0].body.contains("ship the harness"));
}

//! 私聊 / 汇报 / steer 的回归：私聊不覆盖父可见任务、report 模式聚合整轮、异常结束退回草稿。

use super::subagent::{fixture_with_parent, op, spawn_explorer};
use super::*;
use crate::chat::agent::{SubagentOp, SubagentOutcome};

/// 往指定 assistant 追加一段文本步骤（报告正文只读 `handle.chat`）。
fn push_text(chat: &mut crate::app::state::LxChat, message_id: &str, text: &str) {
    let Some(message) = chat
        .messages
        .iter_mut()
        .find(|message| message.id == message_id)
    else {
        return;
    };
    message
        .steps
        .push(crate::app::state::AssistantStepView::Part {
            position: i64::try_from(message.steps.len()).unwrap_or(i64::MAX),
            part: crate::app::state::ChatPartView {
                kind: crate::chat::events::PartKind::Text,
                content: text.to_string(),
                provider_metadata_json: None,
                started_at_ms: None,
                ended_at_ms: None,
                completed_at_ms: None,
            },
        });
}

/// 私聊轮不能覆盖 `run.task`；该字段会进入主 agent 的 `@` 引用块。
#[test]
fn private_follow_up_keeps_the_parent_visible_task() {
    use crate::app::state::lx::agent::visible_task;

    assert_eq!(
        visible_task("PRIVATE-MARKER", false, Some("original task"), "title"),
        "original task"
    );
    assert_eq!(
        visible_task("REPORT-MARKER", true, Some("original task"), "title"),
        "REPORT-MARKER"
    );
    assert_eq!(
        visible_task("PRIVATE-MARKER", false, None, "title"),
        "title",
        "缺少原任务时退回非私聊标题"
    );
    // 显式 /report 的输入是指令，不得覆盖父可见任务。
    let report_request = crate::chat::agent::report_request_message("check the log");
    assert_eq!(
        visible_task(&report_request, true, Some("original task"), "title"),
        "original task"
    );
}

/// report 模式下 steer 一次，报告仍覆盖 steer 前后两个 assistant 片段。
#[tokio::test]
async fn report_mode_steer_keeps_both_segments_in_the_report() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    let first_assistant = streams.subagents[&child_session].child_assistant_id.clone();
    if let Some(handle) = streams.subagents.get_mut(&child_session) {
        assert!(handle.notify_parent, "spawn 默认汇报");
        push_text(&mut handle.chat, &first_assistant, "FIRST HALF");
    }
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent_mut(&agent_id)
        .expect("run")
        .chat
        .pending_steers
        .push(crate::app::state::lx::chat::steer::PendingSteer {
            text: "steer this turn".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        });
    let child_key = RequestKey {
        pane: key.pane,
        session: child_session.clone(),
        generation: key.generation,
    };
    let (reply, receiver) = tokio::sync::oneshot::channel();
    assert!(crate::event::chat::steer::deliver(
        &mut state,
        &mut streams,
        &persistence,
        child_key.clone(),
        reply,
    ));
    receiver.await.expect("reply").expect("steer inputs");
    let second_assistant = streams.subagents[&child_session].child_assistant_id.clone();
    assert_ne!(first_assistant, second_assistant);
    assert_eq!(
        streams.subagents[&child_session]
            .turn_user_message_ids
            .len(),
        2,
        "本轮应记录 claim + steer 两个 user 消息"
    );
    if let Some(handle) = streams.subagents.get_mut(&child_session) {
        push_text(&mut handle.chat, &second_assistant, "SECOND HALF");
    }

    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        child_key,
        StopReason::Complete,
        Usage::default(),
    );
    let report = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .and_then(|run| run.report.as_ref())
        .expect("report");
    assert!(report.body.contains("FIRST HALF"), "body: {}", report.body);
    assert!(report.body.contains("SECOND HALF"), "body: {}", report.body);
}

/// 停止/失败后未消费的 steer 退回 composer，不自动重启 agent。
#[tokio::test]
async fn abnormal_settle_restores_undelivered_steer_to_the_composer() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    state
        .pane_mut_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent_mut(&agent_id)
        .expect("run")
        .chat
        .pending_steers
        .push(crate::app::state::lx::chat::steer::PendingSteer {
            text: "late steer".into(),
            attachments: Vec::new(),
            model_images: Vec::new(),
            model: crate::db::chat::ModelSnapshot {
                provider: "p".into(),
                model: "m".into(),
                variant: None,
            },
        });
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        RequestKey {
            pane: key.pane,
            session: child_session,
            generation: key.generation,
        },
        StopReason::Cancelled,
        Usage::default(),
    );
    assert!(
        !state.effects.iter().any(|effect| matches!(
            effect,
            Effect::SendSubagentUserInput { text, .. } if text == "late steer"
        )),
        "异常结束不得自动重启：{:?}",
        state.effects
    );
    let draft = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .composer(&agent_id)
        .map(|composer| composer.text().to_string());
    assert_eq!(draft.as_deref(), Some("late steer"), "文本应退回草稿");
}

/// 私聊追问：不生成报告、不唤醒主 agent；显式 include_private 才能读到内容。
#[tokio::test]
async fn private_panel_follow_up_stays_out_of_the_parent_and_is_readable_on_request() {
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
    // 清掉首轮自动汇报轮留下的 UI/请求状态，让父窗格回到 Idle。
    let pane = state.pane_mut_anywhere(key.pane).expect("pane");
    pane.lx.chat.request = crate::app::state::RequestState::Idle;
    pane.lx.chat.status.streaming = false;
    // 把首轮的正常报告投递掉，确保下面观察到的只可能是私聊轮产生的消息。
    assert!(crate::event::chat::subagent::drain_reports(
        &mut state,
        &persistence,
        key.pane
    ));
    let pane = state.pane_mut_anywhere(key.pane).expect("pane");
    pane.lx.chat.request = crate::app::state::RequestState::Idle;
    pane.lx.chat.status.streaming = false;
    pane.lx.chat.messages.clear();
    state.effects.clear();
    let before = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .chat
        .messages
        .len();

    // 私聊追问：notify_parent=false。
    state.effects.push(Effect::SendSubagentUserInput {
        pane: key.pane,
        session_id: key.session.clone(),
        agent_id: agent_id.clone(),
        text: "secret".into(),
        notify_parent: false,
    });
    assert!(crate::event::chat::apply_subagent_effects(
        &mut state,
        &mut streams,
        &persistence
    ));
    assert!(
        streams
            .subagents
            .values()
            .any(|handle| handle.agent_id == agent_id && handle.status == "running"),
        "私聊仍应立即起子轮"
    );
    let changed = crate::event::chat::handle_subagent_net_event(
        &mut state,
        &mut streams,
        ChatNetEvent::Delta {
            key: RequestKey {
                pane: key.pane,
                session: child_session.clone(),
                generation: key.generation,
            },
            deltas: vec![ChatDelta::Part(crate::chat::events::PartDelta {
                kind: PartKind::Text,
                text: "private reply".into(),
                metadata: None,
            })],
        },
    );
    assert!(changed);
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

    let database = persistence.database().expect("db");
    assert!(
        database
            .pending_reports(fixture.workspace(), &key.session)
            .expect("pending")
            .is_empty(),
        "私聊轮不得生成待投递报告"
    );
    assert_eq!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .chat
            .messages
            .len(),
        before,
        "父对话不得因为私聊轮新增报告消息"
    );
    let tail = database
        .private_turn_tail(fixture.workspace(), &key.session, &agent_id)
        .expect("query")
        .expect("private tail");
    assert!(tail.contains("secret"), "私聊 user 文本：{tail}");
    assert!(
        tail.contains("private reply"),
        "私聊 assistant 文本：{tail}"
    );
    assert!(
        database
            .private_turn_tail(fixture.workspace(), &key.session, "missing")
            .expect("query")
            .is_none(),
        "未知 agent 无私聊"
    );
    let listed_default = op(
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
    .expect("list default");
    let SubagentOutcome::Listed { agents } = listed_default else {
        panic!("expected listed");
    };
    assert!(
        agents.iter().all(|agent| agent.private_tail.is_none()),
        "默认列表不得携带私聊转录"
    );
    let listed = op(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        SubagentOp::List {
            path_prefix: None,
            include_private: true,
        },
    )
    .await
    .expect("list");
    let SubagentOutcome::Listed { agents } = listed else {
        panic!("expected listed");
    };
    let tail = agents
        .iter()
        .find(|agent| agent.agent_id == agent_id)
        .and_then(|agent| agent.private_tail.as_deref())
        .expect("list_agents include_private tail");
    assert!(tail.contains("secret") && tail.contains("private reply"));
}

/// 先私聊多轮，再 `/report`：报告正文只含报告轮 assistant 输出，闲聊不得进入主 agent。
#[tokio::test]
async fn report_turn_body_excludes_previous_private_chat() {
    let (_fixture, mut state, key, persistence, mut streams, _parent) = fixture_with_parent();
    let agent_id = spawn_explorer(&mut state, &mut streams, &persistence, &key, "first task").await;
    let child_session = streams
        .subagents
        .values()
        .find(|handle| handle.agent_id == agent_id)
        .expect("handle")
        .child_session_id
        .clone();
    let child_key = RequestKey {
        pane: key.pane,
        session: child_session.clone(),
        generation: key.generation,
    };
    // 结束 spawn 轮并投递首轮报告，清空父对话。
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        child_key.clone(),
        StopReason::Complete,
        Usage::default(),
    );
    {
        let pane = state.pane_mut_anywhere(key.pane).expect("pane");
        pane.lx.chat.request = crate::app::state::RequestState::Idle;
        pane.lx.chat.status.streaming = false;
    }
    assert!(crate::event::chat::subagent::drain_reports(
        &mut state,
        &persistence,
        key.pane
    ));
    {
        let pane = state.pane_mut_anywhere(key.pane).expect("pane");
        pane.lx.chat.request = crate::app::state::RequestState::Idle;
        pane.lx.chat.status.streaming = false;
        pane.lx.chat.messages.clear();
        if let Some(run) = pane.lx.agents.by_agent_mut(&agent_id) {
            run.report = None;
        }
    }

    // 一轮纯私聊：不得产生报告。
    assert!(send_child(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &agent_id,
        "今天天气不错",
        false,
    ));
    assert!(crate::event::chat::handle_subagent_net_event(
        &mut state,
        &mut streams,
        ChatNetEvent::Delta {
            key: child_key.clone(),
            deltas: vec![ChatDelta::Part(crate::chat::events::PartDelta {
                kind: PartKind::Text,
                text: "PRIVATE CHAT".into(),
                metadata: None,
            })],
        },
    ));
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        child_key.clone(),
        StopReason::Complete,
        Usage::default(),
    );
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .agents
            .by_agent(&agent_id)
            .and_then(|run| run.report.as_ref())
            .is_none(),
        "私聊轮不产生报告"
    );

    // 显式汇报轮：报告正文只取本轮输出。
    assert!(send_child(
        &mut state,
        &mut streams,
        &persistence,
        &key,
        &agent_id,
        &crate::chat::agent::report_request_message("locate the login bug"),
        true,
    ));
    assert!(crate::event::chat::handle_subagent_net_event(
        &mut state,
        &mut streams,
        ChatNetEvent::Delta {
            key: child_key.clone(),
            deltas: vec![ChatDelta::Part(crate::chat::events::PartDelta {
                kind: PartKind::Text,
                text: "REPORT BODY".into(),
                metadata: None,
            })],
        },
    ));
    crate::event::chat::finish_subagent(
        &mut state,
        &mut streams,
        &persistence,
        child_key,
        StopReason::Complete,
        Usage::default(),
    );
    let report = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .agents
        .by_agent(&agent_id)
        .and_then(|run| run.report.as_ref())
        .expect("report");
    assert!(
        report.body.contains("REPORT BODY"),
        "报告正文应含本轮输出：{}",
        report.body
    );
    assert!(
        !report.body.contains("PRIVATE CHAT"),
        "闲聊不得进入报告：{}",
        report.body
    );
}

/// 从面板发起一轮子 agent 输入（复用事件层入口）。
fn send_child(
    state: &mut AppState,
    streams: &mut crate::event::chat::ChatStreams,
    persistence: &crate::db::service::Persistence,
    key: &RequestKey,
    agent_id: &str,
    text: &str,
    notify_parent: bool,
) -> bool {
    state.effects.push(Effect::SendSubagentUserInput {
        pane: key.pane,
        session_id: key.session.clone(),
        agent_id: agent_id.to_string(),
        text: text.to_string(),
        notify_parent,
    });
    crate::event::chat::apply_subagent_effects(state, streams, persistence)
}

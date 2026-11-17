//! BTW `/report`：一次性总结请求不创建 user/assistant 消息，只登记可撤销报告检查点。

use super::*;
use crate::app::effects::BtwEffect;
use crate::db::chat::{BtwTargetSpec, ModelSnapshot};

fn default_model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
}

/// 建一条已存在问答的 BTW 线程：QA 锚点来自主会话，子会话由 DB 创建。
fn thread_fixture(
    fixture: &Fixture,
    persistence: &Persistence,
    session: &str,
) -> (String, String, String) {
    let database = persistence.database().expect("db");
    let anchor = database
        .qa_anchors(fixture.workspace(), session)
        .expect("anchors")
        .into_iter()
        .next()
        .expect("qa anchor");
    let accepted = database
        .accept_btw_batch(
            fixture.workspace(),
            session,
            &anchor.message_id,
            "side thread",
            "btw-seed-1",
            "seed question",
            &[BtwTargetSpec::default_assistant(default_model())],
        )
        .expect("seed btw batch");
    (
        anchor.message_id,
        accepted.thread.id,
        accepted.thread.child_session_id,
    )
}

fn report_effect(key: &RequestKey, session: &str, thread_id: &str) -> BtwEffect {
    BtwEffect::Report {
        pane: key.pane,
        parent_session_id: session.to_string(),
        thread_id: thread_id.to_string(),
        note: "focus".into(),
    }
}

#[tokio::test]
async fn btw_report_starts_without_creating_messages() {
    let fixture = Fixture::new("btw-report");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let (qa_id, thread_id, child_session) = thread_fixture(&fixture, &persistence, &session);
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        let thread = target.lx.btw.ensure_thread(&qa_id);
        thread.thread_id = Some(thread_id.clone());
        thread.child_session_id = Some(child_session);
    }
    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let (process_sender, _process_receiver) = mpsc::unbounded_channel();
    let mut streams = ChatStreams::default();
    streams.set_event_sender(sender.clone());
    streams.set_process_sender(process_sender);
    assert!(crate::event::chat::btw::apply_effect(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        report_effect(&key, &session, &thread_id),
    ));
    let thread = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert!(thread.compact.running_report, "报告进入执行态");
    assert!(
        !thread
            .chat
            .messages
            .iter()
            .any(|message| message.role == "user"
                && crate::chat::agent::is_report_request(&message.text())),
        "一次性总结不得创建 user 消息"
    );

    // 没有可回放的完整问答 → Empty 回执立即结束执行态。
    let event = receiver.try_recv().expect("compact finished");
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
    assert!(matches!(
        outcome,
        crate::event::chat::compact::CompactOutcome::Empty
    ));
    assert!(crate::event::chat::handle_compact_finished(
        &mut state,
        &persistence,
        pane,
        &session_id,
        &workspace,
        &operation_id,
        outcome,
    ));
    let thread = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .btw
        .active_thread()
        .expect("thread");
    assert!(!thread.compact.is_running(), "空报告必须收口执行态");
    let toast = state
        .pane_anywhere(key.pane)
        .expect("pane")
        .lx
        .toast
        .as_ref()
        .expect("toast");
    assert_eq!(
        toast.message,
        crate::ui::text::detail::LX_BTW_REPORT_UNAVAILABLE
    );
}

#[tokio::test]
async fn btw_report_without_thread_is_rejected() {
    let fixture = Fixture::new("btw-report-missing");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let sender = mpsc::unbounded_channel::<AppEvent>().0;
    let (process_sender, _process_receiver) = mpsc::unbounded_channel();
    let mut streams = ChatStreams::default();
    streams.set_event_sender(sender.clone());
    streams.set_process_sender(process_sender);
    // 线程不存在：事件层直接拒绝，不登记执行态。
    assert!(crate::event::chat::btw::apply_effect(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        BtwEffect::Report {
            pane: key.pane,
            parent_session_id: session,
            thread_id: "btw-missing".into(),
            note: String::new(),
        },
    ));
    assert!(
        state
            .pane_anywhere(key.pane)
            .expect("pane")
            .lx
            .toast
            .is_some(),
        "无线程报告必须给出反馈"
    );
}

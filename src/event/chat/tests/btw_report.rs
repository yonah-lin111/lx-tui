//! BTW `/report`：多目标批次登记、接受回执与聚合结算。

use super::*;
use crate::app::effects::{BtwEffect, BtwSubmission};
use crate::db::chat::{BtwTargetSpec, ModelSnapshot};

fn default_model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    }
}

/// 建一条已存在回答的 BTW 线程：QA 锚点来自主会话，子会话由 DB 创建。
fn thread_fixture(fixture: &Fixture, persistence: &Persistence, session: &str) -> (String, String) {
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
    (anchor.message_id, accepted.thread.id)
}

fn report_submission(
    fixture: &Fixture,
    key: &RequestKey,
    session: &str,
    qa_id: &str,
    thread_id: &str,
) -> BtwSubmission {
    report_submission_with_model(fixture, key, session, qa_id, thread_id, default_model())
}

fn report_submission_with_model(
    fixture: &Fixture,
    key: &RequestKey,
    session: &str,
    qa_id: &str,
    thread_id: &str,
    model: ModelSnapshot,
) -> BtwSubmission {
    BtwSubmission {
        pane: key.pane,
        parent_session_id: session.to_string(),
        qa_id: qa_id.to_string(),
        workspace: fixture.workspace().to_path_buf(),
        cwd: fixture.workspace().to_path_buf(),
        thread_id: Some(thread_id.to_string()),
        submit_id: "btw-report-1".into(),
        draft_revision: 0,
        text: crate::chat::agent::report_request_message("focus"),
        report_to_main_input: true,
        targets: Vec::new(),
        default_model: model,
        default_variant_params: None,
        idle_timeout_ms: 30_000,
    }
}

#[tokio::test]
async fn btw_report_submission_registers_batch_and_projects_accepted() {
    let fixture = Fixture::new("btw-report");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let (qa_id, thread_id) = thread_fixture(&fixture, &persistence, &session);
    // UI 侧已加载线程并处于编辑态：`on_accepted` 需要匹配 pending_submit。
    let revision = {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        let thread = target.lx.btw.ensure_thread(&qa_id);
        thread.thread_id = Some(thread_id.clone());
        thread.pending_submit = Some(("btw-report-1".into(), thread.composer.revision()));
        thread.composer.revision()
    };
    let _ = revision;

    let (sender, mut receiver) = mpsc::unbounded_channel::<AppEvent>();
    let (process_sender, _process_receiver) = mpsc::unbounded_channel();
    let mut streams = ChatStreams::default();
    streams.set_event_sender(sender.clone());
    streams.set_process_sender(process_sender);
    let submission = report_submission(&fixture, &key, &session, &qa_id, &thread_id);
    assert!(crate::event::chat::btw::apply_effect(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        BtwEffect::Submit(Box::new(submission)),
    ));
    assert_eq!(
        streams.btw.report_batches.len(),
        1,
        "报告批次必须登记，结算时才能聚合"
    );
    let runs: Vec<_> = streams
        .btw
        .runs
        .values()
        .map(|run| {
            (
                run.thread_id.clone(),
                run.batch_id.clone(),
                run.label.clone(),
            )
        })
        .collect();
    let mut events: Vec<String> = Vec::new();
    let mut arrived: Vec<AppEvent> = Vec::new();
    while let Ok(event) = receiver.try_recv() {
        let label = match &event {
            AppEvent::Btw(crate::event::chat::btw::BtwEvent::Failed { message, .. }) => {
                format!("failed: {message}")
            }
            AppEvent::Btw(_) => "btw-event".to_string(),
            other => format!("other: {other:?}"),
        };
        events.push(label);
        arrived.push(event);
    }
    eprintln!("runs={runs:?} events={events:?}");
    assert!(
        streams
            .btw
            .runs
            .values()
            .any(|run| run.thread_id == thread_id),
        "报告回答者必须真正启动"
    );

    // Accepted 回执把报告请求投影进 BTW 线程。
    let mut accepted = false;
    for event in arrived {
        if let AppEvent::Btw(btw_event) = event {
            accepted |= matches!(
                btw_event,
                crate::event::chat::btw::BtwEvent::Accepted { .. }
            );
            crate::event::chat::btw::apply_event_to_state(&mut state, btw_event);
        }
    }
    assert!(accepted, "必须回投 Accepted");
    let target = state.pane_anywhere(key.pane).expect("pane");
    let thread = target.lx.btw.active_thread().expect("thread");
    assert_eq!(thread.thread_id.as_deref(), Some(thread_id.as_str()));
    assert!(
        thread
            .chat
            .messages
            .iter()
            .any(|message| message.role == "user"
                && crate::chat::agent::is_report_request(&message.text())),
        "报告请求必须出现在 BTW 线程里"
    );
}

#[tokio::test]
async fn btw_report_start_failure_settles_batch_and_toasts() {
    let fixture = Fixture::new("btw-report-fail");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let (qa_id, thread_id) = thread_fixture(&fixture, &persistence, &session);
    let (sender, _receiver) = mpsc::unbounded_channel::<AppEvent>();
    let (process_sender, _process_receiver) = mpsc::unbounded_channel();
    let mut streams = ChatStreams::default();
    streams.set_event_sender(sender.clone());
    streams.set_process_sender(process_sender);
    let submission = report_submission_with_model(
        &fixture,
        &key,
        &session,
        &qa_id,
        &thread_id,
        ModelSnapshot {
            provider: "missing".into(),
            model: "missing".into(),
            variant: None,
        },
    );
    assert!(crate::event::chat::btw::apply_effect(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        BtwEffect::Submit(Box::new(submission)),
    ));
    assert!(
        streams.btw.report_batches.is_empty(),
        "启动失败也必须收口报告批次"
    );
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

#[test]
fn btw_report_batch_aggregates_sections_and_failures() {
    use crate::event::chat::btw::BtwReportBatch;

    let mut streams = ChatStreams::default();
    streams.btw.report_batches.insert(
        "batch-1".into(),
        BtwReportBatch {
            pane: crate::tiling::PaneId::alloc(),
            parent_session_id: "sess".into(),
            thread_id: "thread-1".into(),
            title: "side thread".into(),
            expected: 2,
            done: 0,
            sections: Vec::new(),
            failed: Vec::new(),
        },
    );
    assert!(
        streams
            .btw
            .record_report_outcome("batch-1", "Curie", Some("first body".into()))
            .is_none(),
        "未收口不填芯片"
    );
    let ready = streams
        .btw
        .record_report_outcome("batch-1", "Turing", None)
        .expect("全部收口");
    assert_eq!(
        ready.sections,
        vec![("Curie".to_string(), "first body".to_string())]
    );
    assert_eq!(ready.failed, vec!["Turing".to_string()]);
    assert!(streams.btw.report_batches.is_empty());
}

#[test]
fn btw_report_batch_treats_blank_body_as_failure() {
    use crate::event::chat::btw::BtwReportBatch;

    let mut streams = ChatStreams::default();
    streams.btw.report_batches.insert(
        "batch-2".into(),
        BtwReportBatch {
            pane: crate::tiling::PaneId::alloc(),
            parent_session_id: "sess".into(),
            thread_id: "thread-2".into(),
            title: String::new(),
            expected: 1,
            done: 0,
            sections: Vec::new(),
            failed: Vec::new(),
        },
    );
    let ready = streams
        .btw
        .record_report_outcome("batch-2", "BTW", Some("  \n ".into()))
        .expect("收口");
    assert!(ready.sections.is_empty());
    assert_eq!(ready.failed, vec!["BTW".to_string()]);
}

#[tokio::test]
async fn btw_report_without_thread_creates_one_and_runs() {
    let fixture = Fixture::new("btw-report-new");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let qa_id = persistence
        .database()
        .expect("db")
        .qa_anchors(fixture.workspace(), &session)
        .expect("anchors")
        .into_iter()
        .next()
        .expect("qa anchor")
        .message_id;
    {
        let target = state.pane_mut_anywhere(key.pane).expect("pane");
        let thread = target.lx.btw.ensure_thread(&qa_id);
        thread.pending_submit = Some(("btw-report-1".into(), thread.composer.revision()));
    }
    let (sender, _receiver) = mpsc::unbounded_channel::<AppEvent>();
    let (process_sender, _process_receiver) = mpsc::unbounded_channel();
    let mut streams = ChatStreams::default();
    streams.set_event_sender(sender.clone());
    streams.set_process_sender(process_sender);
    let mut submission = report_submission(&fixture, &key, &session, &qa_id, "");
    submission.thread_id = None;
    assert!(crate::event::chat::btw::apply_effect(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        BtwEffect::Submit(Box::new(submission)),
    ));
    assert_eq!(
        streams.btw.report_batches.len(),
        1,
        "新 QA 的报告批次必须登记"
    );
    assert!(
        streams
            .btw
            .runs
            .values()
            .any(|run| run.thread_id.starts_with("btw-")),
        "事件层必须按 QA 锚点创建线程并启动报告"
    );
    let threads = persistence
        .database()
        .expect("db")
        .list_btw_threads(fixture.workspace(), &session)
        .expect("threads");
    assert_eq!(threads.len(), 1, "报告线程落库");
}

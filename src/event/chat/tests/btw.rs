//! BTW 多回答者：同一线程的多个目标必须共享 session guard 并全部真正启动。

use super::*;
use crate::app::effects::{BtwEffect, BtwSubmission, BtwTargetChoice};
use crate::db::chat::ModelSnapshot;

fn target(agent_id: &str, nickname: &str) -> BtwTargetChoice {
    BtwTargetChoice {
        agent_id: agent_id.to_string(),
        display_nickname: nickname.to_string(),
        role_name: "explorer".to_string(),
        source_child_session_id: format!("{agent_id}-session"),
    }
}

/// 同一 BTW 线程的第二个目标不能因为文件锁不可重入而永远停在 queued：
/// 两个目标都必须进入 running，否则重启恢复会把从未执行的回答者误标为 interrupted。
#[tokio::test]
async fn multi_target_btw_batch_starts_every_recipient() {
    let fixture = Fixture::new("btw-multi-target");
    let (mut state, key, persistence, session) = streaming_state(&fixture);
    let database = persistence.database().expect("db");
    let qa_id = database
        .qa_anchors(fixture.workspace(), &session)
        .expect("anchors")
        .into_iter()
        .next()
        .expect("qa anchor")
        .message_id;
    let (sender, receiver) = mpsc::unbounded_channel::<AppEvent>();
    std::mem::forget(receiver);
    let (process_sender, process_receiver) = mpsc::unbounded_channel();
    std::mem::forget(process_receiver);
    let mut streams = ChatStreams::default();
    streams.set_event_sender(sender.clone());
    streams.set_process_sender(process_sender);

    let submission = BtwSubmission {
        pane: key.pane,
        parent_session_id: session,
        qa_id,
        workspace: fixture.workspace().to_path_buf(),
        cwd: fixture.workspace().to_path_buf(),
        submit_id: "btw-multi-1".into(),
        draft_revision: 0,
        text: "compare".into(),
        targets: vec![target("sub-a", "A"), target("sub-b", "B")],
        default_model: ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: None,
        },
        default_variant_params: None,
        idle_timeout_ms: 30_000,
    };
    assert!(crate::event::chat::btw::apply_effect(
        &mut state,
        &mut streams,
        &persistence,
        &sender,
        BtwEffect::Submit(Box::new(submission)),
    ));
    assert_eq!(
        streams.btw.runs.len(),
        2,
        "同一线程的两个目标都必须启动，而不是第二个被 session guard 挡回 queued"
    );
    let thread_id = streams
        .btw
        .runs
        .values()
        .next()
        .map(|run| run.thread_id.clone())
        .expect("run");
    let view = database
        .load_btw_thread(fixture.workspace(), &thread_id)
        .expect("load")
        .expect("thread");
    let statuses: Vec<&str> = view.batches[0]
        .recipients
        .iter()
        .map(|recipient| recipient.status.as_str())
        .collect();
    assert_eq!(statuses, vec!["running", "running"]);
}

//! 旁路问答持久化：QA 锚点、线程/批次/回答者、级联删除与启动恢复。

use std::path::PathBuf;

use super::*;
use crate::db::chat::{BtwTargetSpec, TurnOutcome, Usage};
use rusqlite::params;

fn workspace(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "lx-btw-{label}-{}-{}",
        std::process::id(),
        crate::db::now_ms()
    ));
    std::fs::create_dir_all(&path).expect("workspace dir");
    path
}

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "test".into(),
        model: "model".into(),
        variant: None,
    }
}

fn default_target() -> BtwTargetSpec {
    BtwTargetSpec::default_assistant(model())
}

fn seeded_session(database: &Database, workspace: &std::path::Path) -> (String, String) {
    let turn = database
        .begin_turn_with_attachments(workspace, None, "main question", &[], &model())
        .expect("begin turn");
    database
        .finish(
            workspace,
            &turn.assistant_message_id,
            &[],
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
        )
        .expect("finish");
    (turn.session_id, turn.user_message_id)
}

#[test]
fn latest_qa_anchor_skips_steer_report_and_shell() {
    let workspace = workspace("anchor");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, first) = seeded_session(&database, &workspace);
    let second = database
        .begin_turn(&workspace, Some(&session), "second question", &model())
        .expect("second turn")
        .user_message_id;
    // steer / 自动报告 / shell 都不是新的正式 QA。
    for (sequence, id, role, kind, stop, report) in [
        (99, "msg_steer", "user", "chat", Some("steer"), None),
        (100, "msg_report", "user", "chat", None, Some("{}")),
        (101, "msg_shell", "user", "user_shell", None, None),
    ] {
        database
            .conn
            .execute(
                r#"INSERT INTO agent_message
                       (external_id, session_id, sequence, turn_index, role, status, kind,
                        stop_reason, subagent_report_json, created_at_ms)
                   VALUES (?1, ?2, ?7, ?7, ?3, 'complete', ?4, ?5, ?6, 0)"#,
                params![id, session, role, kind, stop, report, sequence],
            )
            .expect("insert special message");
    }
    assert_eq!(
        database
            .latest_qa_anchor(&workspace, &session)
            .expect("anchor")
            .map(|anchor| anchor.message_id),
        Some(second)
    );
    let anchors = database.qa_anchors(&workspace, &session).expect("anchors");
    assert_eq!(anchors.len(), 2);
    assert_eq!(anchors[0].message_id, first);
}

#[test]
fn accept_batch_is_idempotent_and_deduplicates_targets() {
    let workspace = workspace("accept");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let targets = vec![default_target()];
    let first = database
        .accept_btw_batch(
            &workspace, &session, &qa, "side", "submit-1", "why?", &targets,
        )
        .expect("accept");
    let again = database
        .accept_btw_batch(
            &workspace, &session, &qa, "side", "submit-1", "why?", &targets,
        )
        .expect("accept again");
    assert_eq!(first.thread.id, again.thread.id);
    assert_eq!(first.recipients.len(), 1);
    assert_eq!(again.recipients.len(), 1);
    assert_eq!(first.user_message_id, again.user_message_id);

    let view = database
        .load_btw_thread(&workspace, &first.thread.id)
        .expect("load")
        .expect("thread");
    assert_eq!(view.batches.len(), 1);
    assert_eq!(view.batches[0].recipients.len(), 1);
    assert_eq!(view.batches[0].recipients[0].status, "queued");
    assert!(view.batches[0].recipients[0].assistant_message_id.is_none());
}

#[test]
fn recipient_claim_creates_streaming_message_once() {
    let workspace = workspace("claim");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-claim",
            "why?",
            &[default_target()],
        )
        .expect("accept");
    let recipient = accepted.recipients[0].id.clone();
    let start = database
        .start_btw_recipient(&workspace, &recipient)
        .expect("start")
        .expect("claimed");
    assert!(start.assistant_message_id.starts_with("msg"));
    assert_eq!(start.question, "why?");
    // 第二次认领：已不是 queued，返回 None。
    assert!(
        database
            .start_btw_recipient(&workspace, &recipient)
            .expect("start again")
            .is_none()
    );
    let messages = database
        .load_chat_session(&workspace, &start.child_session_id)
        .expect("load")
        .expect("session");
    assert!(
        messages
            .iter()
            .any(|message| message.external_id == start.assistant_message_id
                && message.status == "streaming")
    );
}

#[test]
fn finish_recipient_aggregates_batch_and_is_idempotent() {
    let workspace = workspace("finish");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-finish",
            "why?",
            &[default_target()],
        )
        .expect("accept");
    let recipient = accepted.recipients[0].id.clone();
    let start = database
        .start_btw_recipient(&workspace, &recipient)
        .expect("start")
        .expect("claimed");
    let completion = database
        .finish_btw_recipient(
            &workspace,
            &recipient,
            &start.assistant_message_id,
            Some(&[]),
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
            Usage::default(),
        )
        .expect("finish");
    assert!(completion.batch_terminal);
    assert_eq!(completion.recipient_status, "completed");
    // 重复结算不改写批次状态、不重复累计用量。
    let again = database
        .finish_btw_recipient(
            &workspace,
            &recipient,
            &start.assistant_message_id,
            Some(&[]),
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
            Usage::default(),
        )
        .expect("finish again");
    assert_eq!(again.batch_id, completion.batch_id);
    let view = database
        .load_btw_thread(&workspace, &accepted.thread.id)
        .expect("load")
        .expect("thread");
    assert_eq!(view.batches[0].status, "completed");
}

#[test]
fn deleting_btw_thread_removes_its_child_session() {
    let workspace = workspace("delete");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-delete",
            "why?",
            &[default_target()],
        )
        .expect("accept");
    let child = accepted.thread.child_session_id.clone();
    database
        .delete_btw_thread(&workspace, &accepted.thread.id)
        .expect("delete");
    assert!(
        database
            .load_chat_session(&workspace, &child)
            .expect("load child")
            .is_none()
    );
    assert!(
        database
            .btw_thread(&workspace, &accepted.thread.id)
            .expect("thread")
            .is_none()
    );
}

#[test]
fn deleting_the_qa_anchor_cascades_to_the_btw_child_session() {
    let workspace = workspace("cascade");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-cascade",
            "why?",
            &[default_target()],
        )
        .expect("accept");
    let child = accepted.thread.child_session_id.clone();
    database
        .conn
        .execute(
            "DELETE FROM agent_message WHERE external_id = ?1",
            params![qa],
        )
        .expect("delete anchor");
    assert!(
        database
            .load_chat_session(&workspace, &child)
            .expect("load child")
            .is_none()
    );
}

#[test]
fn native_source_guard_excludes_btw_sessions() {
    let workspace = workspace("guard");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-guard",
            "why?",
            &[default_target()],
        )
        .expect("accept");
    assert!(
        database
            .is_native_session(&workspace, &session)
            .expect("native")
    );
    assert!(
        !database
            .is_native_session(&workspace, &accepted.thread.child_session_id)
            .expect("btw")
    );
    assert!(
        database
            .is_btw_session(&workspace, &accepted.thread.child_session_id)
            .expect("btw")
    );
}

#[test]
fn startup_recovery_interrupts_pending_batches_without_deleting_text() {
    let workspace = workspace("recover");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-recover",
            "why?",
            &[default_target()],
        )
        .expect("accept");
    let recipient = accepted.recipients[0].id.clone();
    let start = database
        .start_btw_recipient(&workspace, &recipient)
        .expect("start")
        .expect("claimed");
    database.recover_btw_startup().expect("recover");
    let view = database
        .load_btw_thread(&workspace, &accepted.thread.id)
        .expect("load")
        .expect("thread");
    assert_eq!(view.batches[0].status, "interrupted");
    assert_eq!(view.batches[0].recipients[0].status, "interrupted");
    let messages = database
        .load_chat_session(&workspace, &start.child_session_id)
        .expect("messages")
        .expect("session");
    assert!(
        messages
            .iter()
            .any(|message| message.external_id == start.assistant_message_id
                && message.status == "interrupted")
    );
}

#[test]
fn finishing_without_in_memory_parts_keeps_checkpoint_text() {
    // 面板关闭后回答才结束：事件层拿不到内存投影，必须用 `None` 结算，
    // 不能拿空数组覆盖最后一次 checkpoint 的正文。
    let workspace = workspace("keep-parts");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-keep",
            "why?",
            &[default_target()],
        )
        .expect("accept");
    let recipient = accepted.recipients[0].id.clone();
    let start = database
        .start_btw_recipient(&workspace, &recipient)
        .expect("start")
        .expect("claimed");
    let parts = vec![crate::db::chat::ChatPart {
        position: 0,
        kind: "text".to_string(),
        content: "streamed answer".to_string(),
        provider_metadata_json: None,
        started_at_ms: Some(1),
        ended_at_ms: Some(2),
        completed_at_ms: Some(2),
    }];
    database
        .checkpoint(&workspace, &start.assistant_message_id, &parts)
        .expect("checkpoint");
    database
        .finish_btw_recipient(
            &workspace,
            &recipient,
            &start.assistant_message_id,
            None,
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
            Usage::default(),
        )
        .expect("finish");
    let messages = database
        .load_chat_session(&workspace, &start.child_session_id)
        .expect("load")
        .expect("session");
    let assistant = messages
        .iter()
        .find(|message| message.external_id == start.assistant_message_id)
        .expect("assistant");
    assert_eq!(assistant.status, "complete");
    assert_eq!(assistant.parts.len(), 1);
    assert_eq!(assistant.parts[0].content, "streamed answer");
}

/// 启动恢复不得把从未启动的回答者标成“被打断”：多目标批次里第二个目标
/// 若一直排队，恢复后应为 cancelled，批次按已完成/已取消聚合为 partial。
#[test]
fn startup_recovery_cancels_queued_recipients_instead_of_interrupting_them() {
    fn subagent_target(key: &str, name: &str) -> BtwTargetSpec {
        BtwTargetSpec {
            target_kind: "subagent".to_string(),
            target_key: key.to_string(),
            agent_id: Some(key.to_string()),
            source_child_session_id: Some(format!("{key}-session")),
            display_nickname: name.to_string(),
            role_name: Some("explorer".to_string()),
            model: model(),
        }
    }

    let workspace = workspace("recover-queued");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-recover-queued",
            "why?",
            &[subagent_target("sub-a", "A"), subagent_target("sub-b", "B")],
        )
        .expect("accept");
    // 只有第一个目标真正执行并完成；第二个一直排队（例如被文件锁挡掉）。
    let first = accepted.recipients[0].id.clone();
    let start = database
        .start_btw_recipient(&workspace, &first)
        .expect("start")
        .expect("claimed");
    database
        .finish_btw_recipient(
            &workspace,
            &first,
            &start.assistant_message_id,
            Some(&[]),
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
            Usage::default(),
        )
        .expect("finish");

    database.recover_btw_startup().expect("recover");
    let view = database
        .load_btw_thread(&workspace, &accepted.thread.id)
        .expect("load")
        .expect("thread");
    assert_eq!(view.batches[0].recipients[0].status, "completed");
    assert_eq!(
        view.batches[0].recipients[1].status, "cancelled",
        "从未启动的目标不得标成 interrupted"
    );
    assert_eq!(
        view.batches[0].status, "partial",
        "已完成 + 从未启动应按 partial 聚合"
    );
}

/// 用户停止 BTW 回答：回答者与批次都是 interrupted，不能显示成 failed（红叉）。
#[test]
fn interrupted_recipient_marks_batch_interrupted_not_failed() {
    let workspace = workspace("interrupted-batch");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-interrupted",
            "why?",
            &[default_target()],
        )
        .expect("accept");
    let recipient = accepted.recipients[0].id.clone();
    let start = database
        .start_btw_recipient(&workspace, &recipient)
        .expect("start")
        .expect("claimed");
    database
        .finish_btw_recipient(
            &workspace,
            &recipient,
            &start.assistant_message_id,
            None,
            &TurnOutcome::Interrupted,
            Usage::default(),
            Usage::default(),
        )
        .expect("finish");
    let view = database
        .load_btw_thread(&workspace, &accepted.thread.id)
        .expect("load")
        .expect("thread");
    assert_eq!(view.batches[0].recipients[0].status, "interrupted");
    assert_eq!(
        view.batches[0].status, "interrupted",
        "停止不是失败，批次不得折叠成 failed"
    );
}

/// 恢复时「已完成 + 运行中被截断」与「已完成 + 从未启动」同属 partial，
/// 不能因为存在 interrupted 就把整批写成 interrupted。
#[test]
fn startup_recovery_keeps_mixed_completed_and_interrupted_batch_partial() {
    let workspace = workspace("recover-mixed-interrupted");
    let database = Database::open_in_memory().expect("db");
    database.ensure_workspace(&workspace).expect("ws");
    let (session, qa) = seeded_session(&database, &workspace);
    let target = |key: &str| BtwTargetSpec {
        target_kind: "subagent".to_string(),
        target_key: key.to_string(),
        agent_id: Some(key.to_string()),
        source_child_session_id: Some(format!("{key}-session")),
        display_nickname: key.to_string(),
        role_name: Some("explorer".to_string()),
        model: model(),
    };
    let accepted = database
        .accept_btw_batch(
            &workspace,
            &session,
            &qa,
            "side",
            "submit-recover-mixed",
            "why?",
            &[target("sub-a"), target("sub-b")],
        )
        .expect("accept");
    let first = accepted.recipients[0].id.clone();
    let second = accepted.recipients[1].id.clone();
    let start = database
        .start_btw_recipient(&workspace, &first)
        .expect("start first")
        .expect("claimed first");
    database
        .finish_btw_recipient(
            &workspace,
            &first,
            &start.assistant_message_id,
            None,
            &TurnOutcome::Complete {
                stop_reason: Some("complete".into()),
            },
            Usage::default(),
            Usage::default(),
        )
        .expect("finish first");
    database
        .start_btw_recipient(&workspace, &second)
        .expect("start second")
        .expect("claimed second");

    database.recover_btw_startup().expect("recover");
    let view = database
        .load_btw_thread(&workspace, &accepted.thread.id)
        .expect("load")
        .expect("thread");
    assert_eq!(view.batches[0].recipients[0].status, "completed");
    assert_eq!(view.batches[0].recipients[1].status, "interrupted");
    assert_eq!(
        view.batches[0].status, "partial",
        "混合批次必须按回答者聚合，不得整批写成 interrupted"
    );
}

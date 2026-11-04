//! fork 持久化测试：边界复制、空 fork、streaming 拒绝、检查点/模型切换与图片路径。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::params;

use super::*;
use crate::db::chat::{
    ChatPart, CompactCommit, ModelSnapshot, ModelSwitchCommit, TurnCommitted, TurnOutcome, Usage,
};

fn temp_paths(label: &str) -> (PathBuf, PathBuf) {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let suffix = format!(
        "{label}-{nanos:x}-{}",
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let root = std::env::temp_dir().join(format!("lx-fork-test-{suffix}"));
    (root.join("db").join("lx.db"), root.join("workspace"))
}

fn database(label: &str) -> (Database, PathBuf) {
    let (db_path, workspace) = temp_paths(label);
    let database = Database::open(&db_path).expect("database opens");
    database
        .trust_workspace(&workspace, "workspace")
        .expect("trust");
    (database, workspace)
}

fn model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "example".into(),
        model: "example-chat".into(),
        variant: Some("high".into()),
    }
}

fn text_part(content: &str) -> ChatPart {
    ChatPart {
        position: 0,
        kind: "text".into(),
        content: content.into(),
        provider_metadata_json: None,
        started_at_ms: None,
        ended_at_ms: None,
        completed_at_ms: None,
    }
}

fn complete_turn(
    database: &Database,
    workspace: &Path,
    existing: Option<&str>,
    question: &str,
    answer: &str,
    usage: Usage,
) -> TurnCommitted {
    let committed = database
        .begin_turn(workspace, existing, question, &model())
        .expect("begin turn");
    database
        .finish(
            workspace,
            &committed.assistant_message_id,
            &[text_part(answer)],
            &TurnOutcome::Complete {
                stop_reason: Some("end_turn".into()),
            },
            usage,
        )
        .expect("finish turn");
    committed
}

fn count_rows(database: &Database, table: &str, session_id: &str) -> i64 {
    database
        .conn
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE session_id = ?1"),
            params![session_id],
            |row| row.get(0),
        )
        .expect("count")
}

#[test]
fn fork_before_second_user_message_copies_first_turn() {
    let (database, workspace) = database("before-second");
    let first = complete_turn(
        &database,
        &workspace,
        None,
        "first question",
        "first answer",
        Usage {
            input: Some(10),
            output: Some(5),
            ..Default::default()
        },
    );
    let ticket = database
        .begin_tool_call(
            &workspace,
            &first.session_id,
            &first.assistant_message_id,
            "call-1",
            "read_file",
            r#"{"path":"a.txt"}"#,
        )
        .expect("tool");
    assert_eq!(ticket.position, 1);
    let second = complete_turn(
        &database,
        &workspace,
        Some(&first.session_id),
        "second question",
        "second answer",
        Usage::default(),
    );

    let forked = database
        .fork_chat_session(&workspace, &first.session_id, Some(&second.user_message_id))
        .expect("fork");
    let messages = database
        .load_chat_session(&workspace, &forked.session_id)
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 2, "只复制第二轮用户消息之前的两条");
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[1].role, "assistant");
    assert_ne!(messages[0].external_id, first.user_message_id);
    assert_ne!(messages[1].external_id, first.assistant_message_id);
    assert_eq!(messages[0].parts[0].content, "first question");
    assert_eq!(messages[1].parts[0].content, "first answer");
    assert_eq!(messages[1].tools.len(), 1, "工具调用随消息复制");
    assert_eq!(messages[1].tools[0].call_id, "call-1");
    assert_eq!(forked.restore_text.as_deref(), Some("second question"));

    let sessions = database
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    let fork = sessions
        .iter()
        .find(|session| session.external_id == forked.session_id)
        .expect("fork session");
    assert_eq!(fork.title, "first question (fork #1)");
    assert_eq!(fork.total_input_tokens, 10);
    assert_eq!(fork.total_output_tokens, 5);

    let source = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load source")
        .expect("source exists");
    assert_eq!(source.len(), 4, "源会话保持四行");
    assert_eq!(
        count_rows(&database, "agent_turn_file_scope", &forked.session_id),
        0,
        "fork 不继承文件撤销 scope"
    );
    assert_eq!(
        count_rows(&database, "agent_file_mutation", &forked.session_id),
        0
    );
}

#[test]
fn fork_full_session_copies_all_messages() {
    let (database, workspace) = database("full");
    let first = complete_turn(&database, &workspace, None, "q1", "a1", Usage::default());
    complete_turn(
        &database,
        &workspace,
        Some(&first.session_id),
        "q2",
        "a2",
        Usage::default(),
    );
    let forked = database
        .fork_chat_session(&workspace, &first.session_id, None)
        .expect("fork");
    let messages = database
        .load_chat_session(&workspace, &forked.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(messages.len(), 4);
    assert!(forked.restore_text.is_none());
}

#[test]
fn fork_first_message_creates_empty_session() {
    let (database, workspace) = database("empty");
    let first = complete_turn(
        &database,
        &workspace,
        None,
        "only question",
        "only answer",
        Usage::default(),
    );
    let forked = database
        .fork_chat_session(&workspace, &first.session_id, Some(&first.user_message_id))
        .expect("fork");
    let messages = database
        .load_chat_session(&workspace, &forked.session_id)
        .expect("load")
        .expect("fork session exists");
    assert!(messages.is_empty(), "第一条消息之前允许空 fork");
    assert_eq!(forked.restore_text.as_deref(), Some("only question"));
}

#[test]
fn fork_rejects_missing_or_non_chat_boundary() {
    let (database, workspace) = database("bad-boundary");
    let first = complete_turn(&database, &workspace, None, "q", "a", Usage::default());
    assert!(matches!(
        database.fork_chat_session(&workspace, &first.session_id, Some("missing")),
        Err(DbError::ForkBoundaryMissing)
    ));
    assert!(matches!(
        database.fork_chat_session(
            &workspace,
            &first.session_id,
            Some(&first.assistant_message_id)
        ),
        Err(DbError::ForkBoundaryMissing)
    ));
}

#[test]
fn fork_rejects_streaming_source() {
    let (database, workspace) = database("streaming");
    let committed = database
        .begin_turn(&workspace, None, "in flight", &model())
        .expect("begin");
    assert!(matches!(
        database.fork_chat_session(&workspace, &committed.session_id, None),
        Err(DbError::SessionBusy)
    ));
}

#[test]
fn fork_copies_only_checkpoints_before_boundary() {
    let (database, workspace) = database("checkpoint");
    let first = complete_turn(&database, &workspace, None, "q1", "a1", Usage::default());
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    database
        .commit_compact(
            &workspace,
            &CompactCommit {
                session_id: first.session_id.clone(),
                expected_revision: revision,
                base_checkpoint_id: None,
                start_sequence: 0,
                end_sequence: 1,
                summary_text: "summary 1".into(),
                model_label: "M".into(),
                operation_id: "op-1".into(),
            },
        )
        .expect("compact 1");
    let second = complete_turn(
        &database,
        &workspace,
        Some(&first.session_id),
        "q2",
        "a2",
        Usage::default(),
    );
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    database
        .commit_compact(
            &workspace,
            &CompactCommit {
                session_id: first.session_id.clone(),
                expected_revision: revision,
                base_checkpoint_id: Some("first".into()),
                start_sequence: 0,
                end_sequence: 3,
                summary_text: "summary 2".into(),
                model_label: "M".into(),
                operation_id: "op-2".into(),
            },
        )
        .expect("compact 2");

    let forked = database
        .fork_chat_session(&workspace, &first.session_id, Some(&second.user_message_id))
        .expect("fork");
    let checkpoints = database
        .compact_checkpoints(&workspace, &forked.session_id)
        .expect("checkpoints");
    assert_eq!(checkpoints.len(), 1, "只复制 end_sequence 在边界内的检查点");
    assert_eq!(checkpoints[0].end_sequence, 1);
    assert_ne!(checkpoints[0].operation_id, "op-1");
    assert_eq!(checkpoints[0].state, "active");
}

#[test]
fn fork_copies_model_switches_before_boundary() {
    let (database, workspace) = database("switch");
    let first = complete_turn(&database, &workspace, None, "q1", "a1", Usage::default());
    let committed = database
        .record_model_switch(
            &workspace,
            &ModelSwitchCommit {
                session_id: first.session_id.clone(),
                model: ModelSnapshot {
                    provider: "example2".into(),
                    model: "example-chat-2".into(),
                    variant: None,
                },
                from_label: Some("Example".into()),
                to_label: "Example 2".into(),
                updated_sources: vec!["AGENTS.md".into()],
                injected_prompt: "<system-update>model</system-update>".into(),
                cwd: None,
                to_identity: crate::chat::agent::ModelIdentity {
                    display_name: "Example 2".into(),
                    provider_id: "example2".into(),
                    model_id: "example-chat-2".into(),
                    variant: None,
                    supports_image_input: false,
                },
            },
        )
        .expect("switch")
        .expect("switch record");
    let second = complete_turn(
        &database,
        &workspace,
        Some(&first.session_id),
        "q2",
        "a2",
        Usage::default(),
    );
    let forked = database
        .fork_chat_session(&workspace, &first.session_id, Some(&second.user_message_id))
        .expect("fork");
    let switches = database
        .model_switches(&workspace, &forked.session_id)
        .expect("switches");
    assert_eq!(switches.len(), 1);
    assert_eq!(switches[0].to_provider, "example2");
    assert_ne!(switches[0].id, committed.id, "fork 的记录必须使用新 id");
    assert_eq!(switches[0].sequence, committed.sequence);
}

#[test]
fn fork_remaps_checkpoint_base_ids() {
    let (database, workspace) = database("checkpoint-base");
    let first = complete_turn(&database, &workspace, None, "q1", "a1", Usage::default());
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    let first_checkpoint = database
        .commit_compact(
            &workspace,
            &CompactCommit {
                session_id: first.session_id.clone(),
                expected_revision: revision,
                base_checkpoint_id: None,
                start_sequence: 0,
                end_sequence: 1,
                summary_text: "summary 1".into(),
                model_label: "M".into(),
                operation_id: "op-1".into(),
            },
        )
        .expect("compact 1")
        .checkpoint;
    let second = complete_turn(
        &database,
        &workspace,
        Some(&first.session_id),
        "q2",
        "a2",
        Usage::default(),
    );
    let revision = database
        .session_revision(&first.session_id)
        .expect("revision");
    database
        .commit_compact(
            &workspace,
            &CompactCommit {
                session_id: first.session_id.clone(),
                expected_revision: revision,
                base_checkpoint_id: Some(first_checkpoint.id.clone()),
                start_sequence: 0,
                end_sequence: 3,
                summary_text: "summary 2".into(),
                model_label: "M".into(),
                operation_id: "op-2".into(),
            },
        )
        .expect("compact 2");
    let third = complete_turn(
        &database,
        &workspace,
        Some(&first.session_id),
        "q3",
        "a3",
        Usage::default(),
    );
    let forked = database
        .fork_chat_session(&workspace, &first.session_id, Some(&third.user_message_id))
        .expect("fork");
    let checkpoints = database
        .compact_checkpoints(&workspace, &forked.session_id)
        .expect("checkpoints");
    assert_eq!(checkpoints.len(), 2);
    assert_eq!(checkpoints[0].base_checkpoint_id, None);
    assert_eq!(
        checkpoints[1].base_checkpoint_id.as_deref(),
        Some(checkpoints[0].id.as_str()),
        "base_checkpoint_id 必须重映射到 fork 内的新检查点"
    );
    assert_ne!(checkpoints[1].operation_id, "op-2");
    assert!(!second.user_message_id.is_empty());
}

#[test]
fn fork_copies_attachments() {
    let (database, workspace) = database("attachments");
    let committed = database
        .begin_turn_with_attachments(
            &workspace,
            None,
            "q with file",
            &[crate::db::chat::MessageAttachmentInput {
                kind: crate::db::chat::MessageAttachmentKind::File,
                label: "a.txt".into(),
                marker_start: Some(0),
                marker_end: Some(5),
                source_path: Some("/tmp/a.txt".into()),
                mime: None,
                width: None,
                height: None,
            }],
            &model(),
        )
        .expect("begin with attachment");
    database
        .finish(
            &workspace,
            &committed.assistant_message_id,
            &[text_part("answer")],
            &TurnOutcome::Complete {
                stop_reason: Some("end_turn".into()),
            },
            Usage::default(),
        )
        .expect("finish");
    let forked = database
        .fork_chat_session(&workspace, &committed.session_id, None)
        .expect("fork");
    let messages = database
        .load_chat_session(&workspace, &forked.session_id)
        .expect("load")
        .expect("exists");
    assert_eq!(messages[0].attachments.len(), 1);
    assert_eq!(messages[0].attachments[0].label, "a.txt");
    assert_eq!(
        messages[0].attachments[0].source_path.as_deref(),
        Some("/tmp/a.txt")
    );
}

#[test]
fn fork_rewrites_managed_image_prefix() {
    let rewrite = Some((
        "/root/session/source/image".to_string(),
        "/root/session/target/image".to_string(),
    ));
    assert_eq!(
        super::rewrite("/root/session/source/image/a.png", &rewrite),
        "/root/session/target/image/a.png"
    );
    assert_eq!(
        super::rewrite("/workspace/photo.png", &rewrite),
        "/workspace/photo.png"
    );
    assert_eq!(
        super::rewrite("/root/session/source/image/a.png", &None),
        "/root/session/source/image/a.png"
    );
    let win_rewrite = Some((
        "C:\\sessions\\source\\image".to_string(),
        "C:\\sessions\\target\\image".to_string(),
    ));
    assert_eq!(
        super::rewrite(
            r#"{"path":"C:\\sessions\\source\\image\\a.png"}"#,
            &win_rewrite
        ),
        r#"{"path":"C:\\sessions\\target\\image\\a.png"}"#
    );
}

#[test]
fn fork_title_increments_existing_suffix() {
    assert_eq!(fork_title("Work"), "Work (fork #1)");
    assert_eq!(fork_title("Work (fork #1)"), "Work (fork #2)");
    assert_eq!(fork_title("Work (fork #9)"), "Work (fork #10)");
    assert_eq!(fork_title("Work (fork #x)"), "Work (fork #x) (fork #1)");
    assert_eq!(fork_title(""), "Session (fork #1)");
}

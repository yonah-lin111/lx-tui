//! steer 轮次事务：旧片段封口、同事务开启新轮、附件归档与 rewind scope 切换。

use super::{database_with_two_workspaces, model, part};
use crate::db::DbError;
use crate::db::chat::rewind::{SCOPE_CAPTURING, SCOPE_COMPLETE};
use crate::db::chat::{MessageAttachmentInput, MessageAttachmentKind, TurnOutcome, Usage};

fn file_attachment() -> MessageAttachmentInput {
    MessageAttachmentInput {
        kind: MessageAttachmentKind::File,
        label: "src/main.rs".into(),
        marker_start: Some(0),
        marker_end: Some(11),
        source_path: Some("src/main.rs".into()),
        mime: None,
        width: None,
        height: None,
    }
}

fn image_attachment(path: &std::path::Path) -> MessageAttachmentInput {
    MessageAttachmentInput {
        kind: MessageAttachmentKind::Image,
        label: "steer.png".into(),
        marker_start: Some(12),
        marker_end: Some(40),
        source_path: Some(path.to_string_lossy().into_owned()),
        mime: Some("image/png".into()),
        width: Some(4),
        height: Some(4),
    }
}

/// 写一张真实 PNG，验证 steer 附件与普通轮次走同一归档路径。
fn write_test_png(path: &std::path::Path) {
    let image: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode png");
    std::fs::write(path, bytes).expect("write png");
}

#[test]
fn steer_turn_closes_previous_assistant_and_opens_next_turn() {
    let (database, workspace, _) = database_with_two_workspaces("steer-success");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let image_path = workspace.join("steer.png");
    write_test_png(&image_path);

    let first = database
        .begin_turn(&workspace, None, "first", &model())
        .expect("begin first");
    database
        .checkpoint(
            &workspace,
            &first.assistant_message_id,
            &[part(0, "text", "partial")],
        )
        .expect("checkpoint");
    assert_eq!(
        database
            .turn_scope(&first.user_message_id)
            .expect("scope")
            .expect("present")
            .state,
        SCOPE_CAPTURING
    );

    let previous_parts = vec![part(0, "text", "final partial answer")];
    let attachments = vec![file_attachment(), image_attachment(&image_path)];
    let committed = database
        .begin_steer_turn(
            &workspace,
            &first.session_id,
            &first.assistant_message_id,
            &previous_parts,
            "steer text",
            &attachments,
            &model(),
        )
        .expect("begin steer turn");

    assert_eq!(committed.session_id, first.session_id);
    assert_eq!(committed.turn_index, 1);
    assert_eq!(committed.user_sequence, 2);
    assert_eq!(committed.assistant_sequence, 3);
    assert_eq!(committed.revision, 2);
    assert_eq!(committed.attachments.len(), 2);

    let messages = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 4, "旧片段 + 新 user/assistant 同事务出现");
    assert_eq!(
        messages
            .iter()
            .map(|message| message.sequence)
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(
        messages
            .iter()
            .map(|message| message.turn_index)
            .collect::<Vec<_>>(),
        vec![0, 0, 1, 1]
    );

    let previous = &messages[1];
    assert_eq!(previous.external_id, first.assistant_message_id);
    assert_eq!(previous.status, "complete");
    assert_eq!(previous.stop_reason.as_deref(), Some("steer"));
    assert!(previous.completed_at_ms.is_some());
    assert_eq!(previous.parts, previous_parts, "旧片段写入最终 parts");
    assert_eq!(previous.input_tokens, None, "旧片段不写假 0 usage");
    assert_eq!(previous.output_tokens, None);

    let user = &messages[2];
    assert_eq!(user.external_id, committed.user_message_id);
    assert_eq!(user.role, "user");
    assert_eq!(user.status, "complete");
    assert_eq!(user.parts.len(), 1);
    assert_eq!(user.parts[0].position, 0);
    assert_eq!(user.parts[0].kind, "text");
    assert_eq!(user.parts[0].content, "steer text");
    assert_eq!(user.attachments.len(), 2);
    assert_eq!(user.attachments[0].label, "src/main.rs");
    let archived = user.attachments[1]
        .source_path
        .as_deref()
        .expect("archived image path");
    assert!(
        archived.contains(&format!("/session/{}/image/", committed.session_id)),
        "图片应归档到会话目录: {archived}"
    );

    let assistant = &messages[3];
    assert_eq!(assistant.external_id, committed.assistant_message_id);
    assert_eq!(assistant.role, "assistant");
    assert_eq!(assistant.status, "streaming");
    assert_eq!(assistant.model_provider.as_deref(), Some("example"));
    assert_eq!(assistant.model_id.as_deref(), Some("example-chat"));
    assert_eq!(assistant.variant.as_deref(), Some("high"));

    assert_eq!(
        database
            .turn_scope(&first.user_message_id)
            .expect("old scope")
            .expect("present")
            .state,
        SCOPE_COMPLETE,
        "旧轮次 scope 已封口"
    );
    assert_eq!(
        database
            .turn_scope(&committed.user_message_id)
            .expect("new scope")
            .expect("present")
            .state,
        SCOPE_CAPTURING,
        "新轮次 scope 重新开始捕获"
    );

    assert_eq!(
        database
            .session_revision(&first.session_id)
            .expect("revision"),
        2
    );
    let sessions = database
        .list_chat_sessions(&workspace, 0, 10)
        .expect("list");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].updated_at_ms, committed.created_at_ms);
    assert_eq!(sessions[0].last_message_at_ms, committed.created_at_ms);
    assert_eq!(sessions[0].total_input_tokens, 0, "steer 不累计 usage");
    assert_eq!(sessions[0].total_output_tokens, 0);

    crate::platform::remove_session_images(&committed.session_id);
}

#[test]
fn non_streaming_previous_assistant_is_rejected_without_new_rows() {
    let (database, workspace, _) = database_with_two_workspaces("steer-not-streaming");
    let first = database
        .begin_turn(&workspace, None, "first", &model())
        .expect("begin");
    database
        .finish(
            &workspace,
            &first.assistant_message_id,
            &[part(0, "text", "final")],
            &TurnOutcome::Complete { stop_reason: None },
            Usage::default(),
        )
        .expect("finish");

    let error = database
        .begin_steer_turn(
            &workspace,
            &first.session_id,
            &first.assistant_message_id,
            &[],
            "steer text",
            &[],
            &model(),
        )
        .expect_err("finished assistant cannot be steered");
    assert!(matches!(error, DbError::MessageNotStreaming));

    let messages = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 2, "失败不产生新行");
    assert_eq!(messages[1].status, "complete");
    assert_eq!(messages[1].parts[0].content, "final");
    assert_eq!(
        database
            .session_revision(&first.session_id)
            .expect("revision"),
        1
    );
    let scope_count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM agent_turn_file_scope", [], |row| {
            row.get(0)
        })
        .expect("scope count");
    assert_eq!(scope_count, 1, "不产生新 scope");
}

#[test]
fn session_or_workspace_mismatch_is_rejected_without_new_rows() {
    let (database, workspace_a, workspace_b) = database_with_two_workspaces("steer-mismatch");
    let first = database
        .begin_turn(&workspace_a, None, "first", &model())
        .expect("begin");

    let error = database
        .begin_steer_turn(
            &workspace_b,
            &first.session_id,
            &first.assistant_message_id,
            &[],
            "steer text",
            &[],
            &model(),
        )
        .expect_err("session must belong to workspace");
    assert!(matches!(error, DbError::SessionMissing));

    let error = database
        .begin_steer_turn(
            &workspace_a,
            "sess-unknown",
            &first.assistant_message_id,
            &[],
            "steer text",
            &[],
            &model(),
        )
        .expect_err("unknown session");
    assert!(matches!(error, DbError::SessionMissing));

    let messages = database
        .load_chat_session(&workspace_a, &first.session_id)
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].status, "streaming");
    assert_eq!(
        database
            .session_revision(&first.session_id)
            .expect("revision"),
        1
    );
    let scope_count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM agent_turn_file_scope", [], |row| {
            row.get(0)
        })
        .expect("scope count");
    assert_eq!(scope_count, 1);
}

#[test]
fn repeated_steer_on_same_assistant_fails_without_new_rows() {
    let (database, workspace, _) = database_with_two_workspaces("steer-repeat");
    let first = database
        .begin_turn(&workspace, None, "first", &model())
        .expect("begin");
    let committed = database
        .begin_steer_turn(
            &workspace,
            &first.session_id,
            &first.assistant_message_id,
            &[part(0, "text", "closed")],
            "steer text",
            &[],
            &model(),
        )
        .expect("first steer");

    let error = database
        .begin_steer_turn(
            &workspace,
            &first.session_id,
            &first.assistant_message_id,
            &[part(0, "text", "must not apply")],
            "duplicate",
            &[],
            &model(),
        )
        .expect_err("previous assistant already closed");
    assert!(matches!(error, DbError::MessageNotStreaming));

    let messages = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 4, "重复调用不追加消息");
    assert_eq!(messages[1].parts[0].content, "closed");
    assert_eq!(messages[3].external_id, committed.assistant_message_id);
    assert_eq!(messages[3].status, "streaming");
    assert_eq!(
        database
            .session_revision(&first.session_id)
            .expect("revision"),
        committed.revision
    );
    let scope_count: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM agent_turn_file_scope", [], |row| {
            row.get(0)
        })
        .expect("scope count");
    assert_eq!(scope_count, 2);
}

#[test]
fn invalid_previous_parts_roll_back_the_whole_steer_transaction() {
    let (database, workspace, _) = database_with_two_workspaces("steer-rollback");
    let first = database
        .begin_turn(&workspace, None, "first", &model())
        .expect("begin");
    database
        .checkpoint(
            &workspace,
            &first.assistant_message_id,
            &[part(0, "text", "keep")],
        )
        .expect("checkpoint");

    let error = database
        .begin_steer_turn(
            &workspace,
            &first.session_id,
            &first.assistant_message_id,
            &[part(0, "text", "a"), part(0, "text", "b")],
            "steer text",
            &[],
            &model(),
        )
        .expect_err("duplicate positions rejected");
    assert!(matches!(error, DbError::PositionConflict));

    let messages = database
        .load_chat_session(&workspace, &first.session_id)
        .expect("load")
        .expect("session exists");
    assert_eq!(messages.len(), 2, "事务回滚不产生半轮");
    assert_eq!(
        messages[1].status, "streaming",
        "旧 assistant 保持可继续流式"
    );
    assert_eq!(messages[1].parts[0].content, "keep", "旧 parts 未被删除");
    assert_eq!(
        database
            .session_revision(&first.session_id)
            .expect("revision"),
        1
    );
    assert_eq!(
        database
            .turn_scope(&first.user_message_id)
            .expect("scope")
            .expect("present")
            .state,
        SCOPE_CAPTURING
    );
}

use std::path::PathBuf;

use crate::db::chat::{Database, MessageAttachmentInput, MessageAttachmentKind, ModelSnapshot};
use crate::db::{DbError, SCHEMA_VERSION};

fn temp_db(name: &str) -> (Database, PathBuf) {
    let dir = std::env::temp_dir().join(format!("lx-test-att-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create dir");
    let path = dir.join("test.db");
    let db = Database::open(&path).expect("open");
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create ws");
    db.trust_workspace(&workspace, "ws").expect("trust");
    (db, workspace)
}

fn test_model() -> ModelSnapshot {
    ModelSnapshot {
        provider: "openai".into(),
        model: "gpt-4o".into(),
        variant: None,
    }
}

#[test]
fn begin_turn_with_attachments_and_load_metadata() {
    let (db, workspace) = temp_db("begin_turn_att");
    let attachments = vec![
        MessageAttachmentInput {
            kind: MessageAttachmentKind::File,
            label: "src/main.rs".into(),
            marker_start: Some(0),
            marker_end: Some(15),
            source_path: Some("src/main.rs".into()),
            mime: None,
            width: None,
            height: None,
            image_bytes: None,
        },
        MessageAttachmentInput {
            kind: MessageAttachmentKind::Image,
            label: "test.png".into(),
            marker_start: None,
            marker_end: None,
            source_path: Some("/tmp/test.png".into()),
            mime: Some("image/png".into()),
            width: Some(100),
            height: Some(50),
            image_bytes: Some(vec![1, 2, 3, 4, 5]),
        },
    ];

    let committed = db
        .begin_turn_with_attachments(
            &workspace,
            None,
            "Check this image",
            &attachments,
            &test_model(),
        )
        .expect("begin turn with attachments");

    let messages = db
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load session")
        .expect("session exists");

    assert_eq!(messages.len(), 2);
    let user_msg = &messages[0];
    assert_eq!(user_msg.role, "user");
    assert_eq!(user_msg.attachments.len(), 2);

    let att0 = &user_msg.attachments[0];
    assert_eq!(att0.position, 0);
    assert_eq!(att0.kind, MessageAttachmentKind::File);
    assert_eq!(att0.label, "src/main.rs");
    assert_eq!(att0.marker_start, Some(0));
    assert_eq!(att0.marker_end, Some(15));

    let att1 = &user_msg.attachments[1];
    assert_eq!(att1.position, 1);
    assert_eq!(att1.kind, MessageAttachmentKind::Image);
    assert_eq!(att1.label, "test.png");
    assert_eq!(att1.mime.as_deref(), Some("image/png"));
    assert_eq!(att1.width, Some(100));
    assert_eq!(att1.height, Some(50));

    // 校验 load_image_payloads
    let payloads = db
        .load_image_payloads(&workspace, &committed.session_id, &[&att1.id])
        .expect("load payloads");
    assert_eq!(payloads.get(&att1.id), Some(&vec![1, 2, 3, 4, 5]));

    // 跨 session 读取或查不存在的附件应报错
    let cross = db.load_image_payloads(&workspace, "other_sess", &[&att1.id]);
    assert!(cross.is_err());
}

#[test]
fn image_only_turn_accepted() {
    let (db, workspace) = temp_db("image_only");
    let attachments = vec![MessageAttachmentInput {
        kind: MessageAttachmentKind::Image,
        label: "screenshot.png".into(),
        marker_start: None,
        marker_end: None,
        source_path: None,
        mime: Some("image/png".into()),
        width: Some(200),
        height: Some(100),
        image_bytes: Some(vec![10, 20, 30]),
    }];

    let committed = db
        .begin_turn_with_attachments(&workspace, None, "   ", &attachments, &test_model())
        .expect("begin image only");

    let messages = db
        .load_chat_session(&workspace, &committed.session_id)
        .expect("load session")
        .expect("session exists");

    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].attachments.len(), 1);
}

#[test]
fn empty_text_and_attachments_rejected() {
    let (db, workspace) = temp_db("empty_rejected");
    let err = db
        .begin_turn_with_attachments(&workspace, None, "   ", &[], &test_model())
        .unwrap_err();
    assert!(matches!(err, DbError::InvalidDraft));
}

#[test]
fn cascade_delete_cleans_attachments() {
    let (db, workspace) = temp_db("cascade_att");
    let attachments = vec![MessageAttachmentInput {
        kind: MessageAttachmentKind::Image,
        label: "img.png".into(),
        marker_start: None,
        marker_end: None,
        source_path: None,
        mime: Some("image/png".into()),
        width: Some(50),
        height: Some(50),
        image_bytes: Some(vec![42]),
    }];

    let committed = db
        .begin_turn_with_attachments(&workspace, None, "hi", &attachments, &test_model())
        .expect("begin");

    let count: i64 = db
        .conn
        .query_row("SELECT count(*) FROM agent_message_attachment", [], |r| {
            r.get(0)
        })
        .expect("count");
    assert_eq!(count, 1);

    db.delete_chat_session(&workspace, &committed.session_id)
        .expect("delete session");

    let count_after: i64 = db
        .conn
        .query_row("SELECT count(*) FROM agent_message_attachment", [], |r| {
            r.get(0)
        })
        .expect("count after");
    assert_eq!(count_after, 0);
}

#[test]
fn v18_schema_integrity_and_foreign_keys() {
    let (db, _) = temp_db("schema_check");
    let version: i64 = db
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .expect("version");
    assert_eq!(version, SCHEMA_VERSION);
    let fk_violations: Vec<String> = db
        .conn
        .prepare("PRAGMA foreign_key_check")
        .expect("prep fk")
        .query_map([], |r| Ok(format!("{:?}", r.get::<_, String>(0))))
        .expect("query fk")
        .filter_map(std::result::Result::ok)
        .collect();
    assert!(
        fk_violations.is_empty(),
        "foreign key violations: {fk_violations:?}"
    );

    let integrity: String = db
        .conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .expect("integrity");
    assert_eq!(integrity, "ok");
}

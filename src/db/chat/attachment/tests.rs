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
    let img_path = workspace.join("test.png");
    let img: image::ImageBuffer<image::Rgba<u8>, Vec<u8>> =
        image::ImageBuffer::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
    let mut png_bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageFormat::Png,
    )
    .expect("encode png");
    std::fs::write(&img_path, &png_bytes).expect("write test png");
    let img_path_str = img_path.to_string_lossy().into_owned();

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
        },
        MessageAttachmentInput {
            kind: MessageAttachmentKind::Image,
            label: "test.png".into(),
            marker_start: Some(16),
            marker_end: Some(40),
            source_path: Some(img_path_str.clone()),
            mime: Some("image/png".into()),
            width: Some(4),
            height: Some(4),
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
    let archived_path = att1.source_path.as_deref().expect("archived path");
    assert!(
        archived_path.contains(&format!("/images/{}/", committed.session_id)),
        "图片应归档在 ~/.lx-t[-dev]/images/<session_id>/ 下: {archived_path}"
    );
    assert_eq!(att1.mime.as_deref(), Some("image/png"));
    assert_eq!(att1.width, Some(4));
    assert_eq!(att1.height, Some(4));

    // 删除原始桌面/工作区图片后，仍可从 ~/.lx-t[-dev]/images/<session_id>/ 归档副本读取
    std::fs::remove_file(&img_path).expect("remove original image");
    let refs = db.referenced_attachment_paths().expect("referenced paths");
    assert!(refs.contains(archived_path));

    let payloads = db
        .load_image_payloads(&workspace, &committed.session_id, &[&att1.id])
        .expect("load payloads after original deleted");
    assert!(payloads.get(&att1.id).is_some_and(|b| !b.is_empty()));

    let _ = std::fs::remove_file(archived_path);

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
        marker_start: Some(0),
        marker_end: Some(32),
        source_path: Some("/tmp/screenshot.png".into()),
        mime: Some("image/png".into()),
        width: Some(200),
        height: Some(100),
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
        marker_start: Some(0),
        marker_end: Some(25),
        source_path: Some("/tmp/img.png".into()),
        mime: Some("image/png".into()),
        width: Some(50),
        height: Some(50),
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

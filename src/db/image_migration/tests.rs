use super::*;
use crate::db::chat::{MessageAttachmentInput, MessageAttachmentKind, ModelSnapshot};

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lx-image-migration-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn db_with_image_attachment(label: &str, old_path: &Path) -> (Database, PathBuf) {
    let dir = temp_dir(label);
    let db = Database::open(&dir.join("test.db")).expect("open db");
    let workspace = dir.join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    db.trust_workspace(&workspace, "ws").expect("trust");
    let model = ModelSnapshot {
        provider: "openai".into(),
        model: "gpt".into(),
        variant: None,
    };
    db.begin_turn_with_attachments(
        &workspace,
        None,
        "hello",
        &[MessageAttachmentInput {
            kind: MessageAttachmentKind::File,
            label: "old.png".into(),
            marker_start: Some(0),
            marker_end: Some(3),
            source_path: Some(old_path.to_string_lossy().into_owned()),
            mime: None,
            width: None,
            height: None,
        }],
        &model,
    )
    .expect("begin turn");
    db.conn
        .execute(
            "UPDATE agent_message_attachment SET kind = 'image', mime = 'image/png',
                    width = 4, height = 4, source_path = ?1 WHERE label = 'old.png'",
            rusqlite::params![old_path.to_string_lossy()],
        )
        .expect("mark as image");
    (db, dir)
}

#[test]
fn moves_files_and_rewrites_attachment_paths() {
    let dir = temp_dir("move");
    let old = dir.join("images/sess-1/a.png");
    std::fs::create_dir_all(old.parent().expect("parent")).expect("old dir");
    std::fs::write(&old, b"png").expect("old file");
    let (db, db_dir) = db_with_image_attachment("move-db", &old);
    let changed = migrate_legacy_session_images_in(&db, &dir);
    assert_eq!(changed, 1);
    let new = dir.join("session/sess-1/image/a.png");
    assert!(new.is_file());
    assert!(!old.exists());
    let stored: String = db
        .conn
        .query_row(
            "SELECT source_path FROM agent_message_attachment WHERE label = 'old.png'",
            [],
            |row| row.get(0),
        )
        .expect("path");
    assert_eq!(stored, new.to_string_lossy());
    assert!(!dir.join("images").exists());
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(db_dir);
}

#[test]
fn compensation_rewrites_rows_when_only_the_new_file_exists() {
    let dir = temp_dir("compensate");
    let old = dir.join("images/sess-2/a.png");
    let new = dir.join("session/sess-2/image/a.png");
    std::fs::create_dir_all(new.parent().expect("parent")).expect("new dir");
    std::fs::write(&new, b"png").expect("new file");
    let (db, db_dir) = db_with_image_attachment("compensate-db", &old);
    assert_eq!(migrate_legacy_session_images_in(&db, &dir), 1);
    let stored: String = db
        .conn
        .query_row(
            "SELECT source_path FROM agent_message_attachment WHERE label = 'old.png'",
            [],
            |row| row.get(0),
        )
        .expect("path");
    assert_eq!(stored, new.to_string_lossy());
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(db_dir);
}

use std::path::PathBuf;

use rusqlite::params;

use super::*;
use crate::db::Database;
use crate::db::chat::TurnCommitted;

fn setup() -> (Database, PathBuf, TurnCommitted) {
    let db = Database::open_in_memory().expect("db");
    let workspace = PathBuf::from("/tmp/lx-recap-db-test");
    db.ensure_workspace(&workspace).expect("workspace");
    let model = ModelSnapshot {
        provider: "p".into(),
        model: "m".into(),
        variant: None,
    };
    let turn = db
        .begin_turn(&workspace, None, "hello", &model)
        .expect("turn");
    (db, workspace, turn)
}

fn commit(turn: &TurnCommitted, summary: &str) -> RecapCommit {
    RecapCommit {
        session_id: turn.session_id.clone(),
        message_id: turn.assistant_message_id.clone(),
        summary: summary.to_string(),
        next_action: "next".to_string(),
        model: ModelSnapshot {
            provider: "p".into(),
            model: "m".into(),
            variant: Some("high".into()),
        },
        model_label: "Model".into(),
        end_turn: 0,
        end_sequence: turn.assistant_sequence,
    }
}

#[test]
fn commit_and_load_recap() {
    let (db, workspace, turn) = setup();
    let record = db
        .commit_recap(&workspace, &commit(&turn, "summary"))
        .expect("commit");
    assert_eq!(record.summary, "summary");
    assert_eq!(record.message_id, turn.assistant_message_id);
    let loaded = db.load_recaps(&workspace, &turn.session_id).expect("load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].summary, "summary");
    assert_eq!(loaded[0].model_label, "Model");
}

#[test]
fn commit_upserts_by_message_id() {
    let (db, workspace, turn) = setup();
    db.commit_recap(&workspace, &commit(&turn, "first"))
        .expect("first");
    db.commit_recap(&workspace, &commit(&turn, "second"))
        .expect("second");
    let loaded = db.load_recaps(&workspace, &turn.session_id).expect("load");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].summary, "second");
}

#[test]
fn commit_rejects_user_message() {
    let (db, workspace, turn) = setup();
    let mut commit = commit(&turn, "summary");
    commit.message_id = turn.user_message_id;
    assert!(db.commit_recap(&workspace, &commit).is_err());
}

#[test]
fn load_is_workspace_scoped() {
    let (db, workspace, turn) = setup();
    db.commit_recap(&workspace, &commit(&turn, "summary"))
        .expect("commit");
    let other = PathBuf::from("/tmp/lx-recap-db-other");
    db.ensure_workspace(&other).expect("other");
    assert!(
        db.load_recaps(&other, &turn.session_id)
            .expect("load")
            .is_empty()
    );
}

#[test]
fn delete_message_cascades_recap() {
    let (db, workspace, turn) = setup();
    db.commit_recap(&workspace, &commit(&turn, "summary"))
        .expect("commit");
    db.connection()
        .execute(
            "DELETE FROM agent_message WHERE external_id = ?1",
            params![turn.assistant_message_id],
        )
        .expect("delete");
    let loaded = db.load_recaps(&workspace, &turn.session_id).expect("load");
    assert!(loaded.is_empty());
}
